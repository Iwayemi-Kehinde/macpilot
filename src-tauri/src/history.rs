//! Local battery history in SQLite, plus the discharge-rate estimate.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;

/// Samples further apart than this are treated as separate segments (the Mac
/// was likely asleep or MacPilot was closed), so no rate spans the gap.
pub const MAX_SAMPLE_GAP_SECS: i64 = 15 * 60;
/// How far back the drain estimate reads, whatever range the chart shows. A
/// run on battery is shorter than this; earlier runs calibrate the percentage.
const ESTIMATE_LOOKBACK_SECS: i64 = 7 * 24 * 3600;
const MIN_WINDOW_SECS: i64 = 10 * 60;
const MIN_SAMPLES: usize = 3;
/// Whole-number readings at fewer different percentages than this fit too rough a line.
const MIN_SCALE_PERCENTS: usize = 11;
/// A sample within this many seconds of an existing one in the same power state
/// is skipped, so a second running copy of MacPilot cannot double the history,
/// while a sample marking the adapter being connected or removed always lands.
const MIN_SPACING_SECS: i64 = 45;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatterySample {
    /// Unix seconds.
    pub ts: i64,
    pub percent: u8,
    pub charging: bool,
    pub external_power: bool,
    pub charge_mah: Option<u32>,
    /// The gauge's full-charge capacity when the sample was taken.
    pub full_charge_mah: Option<u32>,
    pub amperage_ma: Option<i64>,
    pub voltage_mv: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DischargeEstimate {
    pub percent_per_hour: f64,
    /// Average current from the change in held charge, when ioreg reported it.
    pub average_ma: Option<f64>,
    pub window_minutes: f64,
    pub sample_count: usize,
    pub projected_minutes_to_empty: Option<f64>,
    pub confidence: Confidence,
    pub confidence_reason: String,
}

pub struct History {
    conn: Connection,
}

impl History {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> rusqlite::Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS battery_samples (
                ts INTEGER NOT NULL,
                percent INTEGER NOT NULL,
                charging INTEGER NOT NULL,
                external_power INTEGER NOT NULL,
                charge_mah INTEGER,
                amperage_ma INTEGER,
                voltage_mv INTEGER,
                full_charge_mah INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_battery_samples_ts ON battery_samples(ts);",
        )?;
        // Files created before full_charge_mah existed keep their rows; those read
        // back as NULL. The write lock stops two copies from both adding it.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let has_full: bool = tx.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('battery_samples') WHERE name = 'full_charge_mah'",
            [],
            |r| r.get::<_, i64>(0).map(|n| n > 0),
        )?;
        if !has_full {
            tx.execute("ALTER TABLE battery_samples ADD COLUMN full_charge_mah INTEGER", [])?;
        }
        tx.commit()?;
        Ok(Self { conn })
    }

    /// Returns whether the sample was stored.
    pub fn insert(&self, s: &BatterySample) -> rusqlite::Result<bool> {
        let inserted = self.conn.execute(
            "INSERT INTO battery_samples (ts, percent, charging, external_power, charge_mah, amperage_ma, voltage_mv, full_charge_mah)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
             WHERE NOT EXISTS (
                 SELECT 1 FROM battery_samples WHERE ts > ?1 - ?9 AND ts < ?1 + ?9 AND external_power = ?4
             )",
            params![
                s.ts,
                s.percent,
                s.charging,
                s.external_power,
                s.charge_mah,
                s.amperage_ma,
                s.voltage_mv,
                s.full_charge_mah,
                MIN_SPACING_SECS
            ],
        )?;
        Ok(inserted == 1)
    }

    fn since(&self, since_ts: i64) -> rusqlite::Result<Vec<BatterySample>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, percent, charging, external_power, charge_mah, amperage_ma, voltage_mv, full_charge_mah
             FROM battery_samples WHERE ts >= ?1 ORDER BY ts ASC",
        )?;
        stmt.query_map([since_ts], |r| {
            Ok(BatterySample {
                ts: r.get(0)?,
                percent: r.get(1)?,
                charging: r.get(2)?,
                external_power: r.get(3)?,
                charge_mah: r.get(4)?,
                amperage_ma: r.get(5)?,
                voltage_mv: r.get(6)?,
                full_charge_mah: r.get(7)?,
            })
        })?
        .collect()
    }

    pub fn prune_before(&self, ts: i64) -> rusqlite::Result<usize> {
        self.conn.execute("DELETE FROM battery_samples WHERE ts < ?1", [ts])
    }

    /// The samples of the last `chart_secs`, and the drain estimate, which
    /// reads the last `ESTIMATE_LOOKBACK_SECS` whatever range the chart shows.
    pub fn chart_and_estimate(
        &self,
        now: i64,
        chart_secs: i64,
    ) -> rusqlite::Result<(Vec<BatterySample>, Result<DischargeEstimate, String>)> {
        let (chart_from, estimate_from) = (now - chart_secs, now - ESTIMATE_LOOKBACK_SECS);
        let all = self.since(chart_from.min(estimate_from))?;
        let from = |ts: i64| all.partition_point(|s| s.ts < ts);
        let estimate = estimate_discharge(&all[from(estimate_from)..], now);
        Ok((all[from(chart_from)..].to_vec(), estimate))
    }
}

/// macOS's battery percentage as a straight line in the gauge's share of its
/// full charge. The two differ: macOS holds 100% for a while after a full
/// charge, and from there its percentage falls a few percent faster than the
/// share does. Fitting the share, not mAh, keeps the line when the gauge
/// relearns its full charge.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PercentScale {
    /// Points of macOS's percentage per point of the gauge's share.
    slope: f64,
    share_at_zero: f64,
}

impl PercentScale {
    /// The gauge's share itself, until there is enough to fit.
    const GAUGE: Self = Self { slope: 1.0, share_at_zero: 0.0 };

    fn percent(self, share: f64) -> f64 {
        self.slope * (share - self.share_at_zero)
    }
}

fn gauge_share(mah: f64, full_mah: u32) -> f64 {
    mah / f64::from(full_mah) * 100.0
}

/// Fits the scale by least squares over samples on battery, once they show
/// `MIN_SCALE_PERCENTS` different percentages.
fn percent_scale(samples: &[BatterySample]) -> Option<PercentScale> {
    // 100% is left out: macOS holds it while the charge falls.
    let points: Vec<(f64, u8)> = samples
        .iter()
        .filter(|s| !s.charging && !s.external_power && s.percent < 100)
        .filter_map(|s| Some((gauge_share(f64::from(s.charge_mah?), s.full_charge_mah.filter(|f| *f > 0)?), s.percent)))
        .collect();
    if points.iter().map(|&(_, p)| p).collect::<BTreeSet<_>>().len() < MIN_SCALE_PERCENTS {
        return None;
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|&(x, _)| x).sum::<f64>() / n;
    let my = points.iter().map(|&(_, y)| f64::from(y)).sum::<f64>() / n;
    let sxy: f64 = points.iter().map(|&(x, y)| (x - mx) * (f64::from(y) - my)).sum();
    let sxx: f64 = points.iter().map(|&(x, _)| (x - mx).powi(2)).sum();
    let slope = sxy / sxx;
    (slope.is_finite() && slope > 0.0).then(|| PercentScale { slope, share_at_zero: mx - my / slope })
}

fn gauge_confidence(gauge_minutes: f64) -> (Confidence, String) {
    if gauge_minutes >= 60.0 {
        (Confidence::High, format!("Measured by the battery gauge over {gauge_minutes:.0} min of continuous samples."))
    } else if gauge_minutes >= 20.0 {
        (Confidence::Medium, format!("Measured by the battery gauge over {gauge_minutes:.0} min; it follows recent load and settles as samples build up."))
    } else {
        (Confidence::Low, format!("Measured by the battery gauge, but only over {gauge_minutes:.0} min, so it reflects whatever ran in that time."))
    }
}

/// Estimates the drain rate, in the percentage macOS shows, from the most
/// recent unbroken run of samples on battery power. Every sample on battery
/// calibrates the gauge against the percentage. Returns a plain-language
/// reason when it cannot.
fn estimate_discharge(samples: &[BatterySample], now: i64) -> Result<DischargeEstimate, String> {
    let Some(latest) = samples.last() else {
        return Err("No battery samples yet.".into());
    };
    if now - latest.ts > MAX_SAMPLE_GAP_SECS {
        return Err(format!(
            "No battery reading in the last {} min, so there is no current rate. Readings pause while the Mac sleeps or MacPilot is closed, and the estimate resumes with the next one.",
            (now - latest.ts) / 60
        ));
    }
    let mut start = samples.len();
    while start > 0 {
        let s = &samples[start - 1];
        if s.charging || s.external_power {
            break;
        }
        if start < samples.len() && samples[start].ts - s.ts > MAX_SAMPLE_GAP_SECS {
            break;
        }
        start -= 1;
    }
    let run = &samples[start..];
    if run.is_empty() {
        return Err("Not running on battery, so there is no discharge to measure.".into());
    }
    let (first, last) = (&run[0], &run[run.len() - 1]);
    let window_secs = last.ts - first.ts;
    if run.len() < MIN_SAMPLES || window_secs < MIN_WINDOW_SECS {
        return Err(format!(
            "Collecting data: {} min of unplugged history so far, {} min needed.",
            window_secs / 60,
            MIN_WINDOW_SECS / 60
        ));
    }

    let minutes = window_secs as f64 / 60.0;
    // A single sample can miss its gauge reading, so the outermost ones that have it are used.
    let gauge = run
        .iter()
        .find_map(|s| Some((s.ts, s.charge_mah?)))
        .zip(run.iter().rev().find_map(|s| Some((s.ts, s.charge_mah?))))
        .filter(|((t0, _), (t1, _))| t1 - t0 >= MIN_WINDOW_SECS)
        .map(|((t0, first_mah), (t1, held))| {
            let gauge_minutes = (t1 - t0) as f64 / 60.0;
            (gauge_minutes, (first_mah as f64 - held as f64) / (gauge_minutes / 60.0), held)
        });
    let average_ma = gauge.map(|(_, ma, _)| ma);
    let full_mah = run.iter().rev().find_map(|s| s.full_charge_mah).filter(|f| *f > 0);

    // The gauge counts charge in mAh, about 100 times finer than the whole
    // percents pmset reports, so it is used whenever it has a long enough run.
    let (percent_per_hour, projected_minutes_to_empty, confidence, confidence_reason) = match (gauge, full_mah) {
        (Some((gauge_minutes, ma, _)), _) if ma <= 1.0 => (
            0.0,
            None,
            Confidence::Low,
            format!("The battery gauge's charge did not fall over {gauge_minutes:.0} min, so there is no drain to project."),
        ),
        (Some((gauge_minutes, ma, held)), Some(full)) => {
            let (confidence, reason) = gauge_confidence(gauge_minutes);
            let (scale, reason) = match percent_scale(samples) {
                Some(scale) => (scale, reason),
                None => (
                    PercentScale::GAUGE,
                    format!("{reason} Until MacPilot has read {MIN_SCALE_PERCENTS} different percentages on battery, the rate is a share of the gauge's full charge, which can differ from the percentage macOS shows."),
                ),
            };
            let percent_per_hour = scale.slope * gauge_share(ma, full);
            let fitted = scale.percent(gauge_share(f64::from(held), full));
            // Under the fitted 0% point the line is extrapolated, so the shown percentage counts there.
            let left = if fitted > 0.0 { fitted } else { f64::from(last.percent) };
            (percent_per_hour, Some(left / percent_per_hour * 60.0), confidence, reason)
        }
        _ => {
            let drop = first.percent as f64 - last.percent as f64;
            let percent_per_hour = drop / (minutes / 60.0);
            let projected = (percent_per_hour > 0.1).then(|| last.percent as f64 / percent_per_hour * 60.0);
            // pmset reports whole percents, so small drops carry a large rounding error.
            let (confidence, reason) = if minutes >= 60.0 && drop >= 5.0 {
                (Confidence::High, format!("{drop:.0}% drop over {minutes:.0} min of continuous samples."))
            } else if minutes >= 20.0 && drop >= 2.0 {
                (Confidence::Medium, format!("{drop:.0}% drop over {minutes:.0} min; percent readings are whole numbers, so the rate may shift by a few %/h."))
            } else {
                (Confidence::Low, format!("Only a {drop:.0}% drop over {minutes:.0} min; 1% rounding dominates the result."))
            };
            (percent_per_hour, projected, confidence, reason)
        }
    };

    Ok(DischargeEstimate {
        percent_per_hour: round2(percent_per_hour),
        average_ma: average_ma.map(|v| v.round()),
        window_minutes: minutes.round(),
        sample_count: run.len(),
        projected_minutes_to_empty: projected_minutes_to_empty.map(|v| v.round()),
        confidence,
        confidence_reason,
    })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ts: i64, percent: u8, external_power: bool, charge_mah: Option<u32>) -> BatterySample {
        BatterySample {
            ts,
            percent,
            charging: false,
            external_power,
            charge_mah,
            full_charge_mah: None,
            amperage_ma: None,
            voltage_mv: None,
        }
    }

    fn gauge_sample(ts: i64, percent: u8, charge_mah: u32) -> BatterySample {
        BatterySample { full_charge_mah: Some(7620), ..sample(ts, percent, false, Some(charge_mah)) }
    }

    /// Estimates as of the newest sample.
    fn estimate(samples: &[BatterySample]) -> Result<DischargeEstimate, String> {
        estimate_discharge(samples, samples.last().map_or(0, |s| s.ts))
    }

    /// Six hours unplugged, as recorded on an M1 Pro with a 7637 mAh full
    /// charge: the gauge falls 577 mA, and macOS's percentage is 1.0738 times
    /// the gauge's share less 1.44, so it falls 7% faster than the share.
    fn macos_run(start: i64, full: u32) -> Vec<BatterySample> {
        (0..=360)
            .map(|m| {
                let charge = 6605 * full / 7637 - (577 * m / 60) as u32;
                let percent = (1.0738 * (gauge_share(f64::from(charge), full) - 1.44)).round() as u8;
                BatterySample { full_charge_mah: Some(full), ..sample(start + m * 60, percent, false, Some(charge)) }
            })
            .collect()
    }

    #[test]
    fn stores_and_reads_back_in_order() {
        let h = History::open_in_memory().unwrap();
        h.insert(&sample(200, 90, false, Some(7000))).unwrap();
        h.insert(&sample(100, 91, true, None)).unwrap();
        let all = h.since(0).unwrap();
        assert_eq!(all.iter().map(|s| s.ts).collect::<Vec<_>>(), vec![100, 200]);
        assert_eq!(all[0].charge_mah, None);
        assert!(all[0].external_power);
        assert_eq!(h.since(150).unwrap().len(), 1);
        assert_eq!(h.prune_before(150).unwrap(), 1);
        assert_eq!(h.since(0).unwrap().len(), 1);
    }

    #[test]
    fn skips_samples_too_close_to_an_existing_one() {
        let h = History::open_in_memory().unwrap();
        assert!(h.insert(&sample(1000, 80, false, None)).unwrap());
        // A second MacPilot instance sampling 20 s later, or 20 s earlier.
        assert!(!h.insert(&sample(1020, 80, false, None)).unwrap());
        assert!(!h.insert(&sample(980, 80, false, None)).unwrap());
        assert!(h.insert(&sample(1060, 79, false, None)).unwrap());
        // The adapter connected 10 s later: recorded at once.
        assert!(h.insert(&sample(1070, 79, true, None)).unwrap());
        assert_eq!(h.since(0).unwrap().len(), 3);
    }

    #[test]
    fn retention_survives_reopening_the_file() {
        let path = std::env::temp_dir().join(format!("macpilot-history-test-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let h = History::open(&path).unwrap();
            h.insert(&sample(100, 90, false, None)).unwrap();
            h.insert(&sample(10_000, 80, false, None)).unwrap();
        }
        let h = History::open(&path).unwrap();
        assert_eq!(h.prune_before(5_000).unwrap(), 1);
        assert_eq!(h.since(0).unwrap().iter().map(|s| s.ts).collect::<Vec<_>>(), vec![10_000]);
        drop(h);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn steady_hour_of_drain_is_high_confidence() {
        // 80% -> 70% over 60 minutes, one sample a minute.
        let samples: Vec<_> = (0..=60)
            .map(|m| sample(m * 60, 80 - (m / 6) as u8, false, Some(7000 - m as u32 * 13)))
            .collect();
        let e = estimate(&samples).unwrap();
        assert_eq!(e.percent_per_hour, 10.0);
        assert_eq!(e.average_ma, Some(780.0));
        assert_eq!(e.projected_minutes_to_empty, Some(420.0));
        assert_eq!(e.confidence, Confidence::High);
        assert_eq!(e.sample_count, 61);
    }

    #[test]
    fn rate_is_in_the_percentage_macos_shows() {
        let samples = macos_run(1_000_000, 7637);
        let e = estimate(&samples).unwrap();
        // 577 mA on the percentage's scale. The gauge's share of full charge
        // would read 577 / 7637 = 7.6 %/h.
        assert!((e.percent_per_hour - 577.0 * 0.014_06).abs() < 0.05, "{}", e.percent_per_hour);
        let (first, last) = (&samples[0], &samples[samples.len() - 1]);
        let shown_per_hour = f64::from(first.percent - last.percent) / 6.0;
        assert!((e.percent_per_hour - shown_per_hour).abs() < 0.2, "{} vs {shown_per_hour}", e.percent_per_hour);
        assert_eq!(e.average_ma, Some(577.0));
        // To the 0% point, not to an empty gauge: (3143 - 110) mAh at 577 mA.
        let to_empty = e.projected_minutes_to_empty.unwrap();
        assert!((to_empty - 315.0).abs() < 3.0, "{to_empty}");
        assert!(!e.confidence_reason.contains("share of the gauge"), "{}", e.confidence_reason);
    }

    #[test]
    fn percent_scale_needs_eleven_percentages_on_battery() {
        let run = |percents: &[u8]| -> Vec<_> {
            percents.iter().enumerate().map(|(i, p)| gauge_sample(i as i64 * 60, *p, 7000 - i as u32 * 70)).collect()
        };
        let falling: Vec<u8> = (89..=100).rev().collect();
        // 100% does not count: macOS holds it while the charge falls.
        assert_eq!(percent_scale(&run(&falling[..11])), None);
        let scale = percent_scale(&run(&falling)).unwrap();
        // A point for every 70 mAh of the 7620 mAh gauge.
        assert!((scale.slope - 7620.0 / 7000.0).abs() < 1e-9, "{scale:?}");
        // Two whole-number readings 10 points apart could put the line 10% out.
        assert_eq!(percent_scale(&run(&[60, 50])), None);
        let mut plugged = run(&falling);
        plugged[5].external_power = true;
        assert_eq!(percent_scale(&plugged), None);
    }

    #[test]
    fn calibration_holds_when_the_gauge_relearns_its_full_charge() {
        let mut samples = macos_run(1_000_000, 7637);
        samples.extend(macos_run(1_000_000 + 2 * 24 * 3600, 7400));
        let e = estimate(&samples).unwrap();
        let expected = 1.0738 * gauge_share(577.0, 7400);
        assert!((e.percent_per_hour - expected).abs() < 0.05, "{} vs {expected}", e.percent_per_hour);
    }

    #[test]
    fn under_the_fitted_zero_the_shown_percentage_is_projected() {
        // 600 mA down to 60 mAh, under the fitted 0% point of 110 mAh, while
        // macOS still shows 2%.
        let mut samples: Vec<_> = (0..=150)
            .map(|m| {
                let charge = 1560 - 10 * m as u32;
                let percent = (1.0738 * (gauge_share(f64::from(charge), 7637) - 1.44)).round().max(0.0) as u8;
                BatterySample { full_charge_mah: Some(7637), ..sample(m * 60, percent, false, Some(charge)) }
            })
            .collect();
        samples.last_mut().unwrap().percent = 2;
        let e = estimate(&samples).unwrap();
        let to_empty = e.projected_minutes_to_empty.unwrap();
        assert!((to_empty - 2.0 / e.percent_per_hour * 60.0).abs() < 1.0, "{to_empty}");
    }

    #[test]
    fn estimate_reads_the_whole_run_whatever_the_chart_range() {
        let h = History::open_in_memory().unwrap();
        let mut samples = macos_run(1_000_000, 7637);
        samples.insert(0, BatterySample { charging: true, external_power: true, ..samples[0].clone() });
        samples[0].ts -= 60;
        for s in &samples {
            assert!(h.insert(s).unwrap());
        }
        let now = samples.last().unwrap().ts + 30;
        let (hour, from_hour) = h.chart_and_estimate(now, 3600).unwrap();
        let (_, from_day) = h.chart_and_estimate(now, 24 * 3600).unwrap();
        assert_eq!(hour.len(), 60);
        assert_eq!(from_hour, from_day);
        assert_eq!(from_hour.unwrap().window_minutes, 360.0);
    }

    #[test]
    fn no_current_rate_after_a_long_silence() {
        let samples = macos_run(1_000_000, 7637);
        let now = samples.last().unwrap().ts + 20 * 60;
        let err = estimate_discharge(&samples, now).unwrap_err();
        assert!(err.contains("last 20 min"), "{err}");
    }

    #[test]
    fn only_counts_samples_since_unplugging() {
        let mut samples = vec![sample(0, 50, true, None), sample(600, 60, true, None)];
        samples.extend((0..=20).map(|m| sample(1200 + m * 60, 60 - (m / 10) as u8, false, None)));
        let e = estimate(&samples).unwrap();
        assert_eq!(e.sample_count, 21);
        assert_eq!(e.window_minutes, 20.0);
        assert_eq!(e.average_ma, None);
        assert_eq!(e.confidence, Confidence::Medium);
    }

    #[test]
    fn gap_splits_the_run() {
        let mut samples: Vec<_> = (0..=30).map(|m| sample(m * 60, 90, false, None)).collect();
        // Two hours asleep, then five minutes awake.
        samples.extend((0..=5).map(|m| sample(30 * 60 + 7200 + m * 60, 80, false, None)));
        let err = estimate(&samples).unwrap_err();
        assert!(err.contains("5 min"), "{err}");
    }

    #[test]
    fn plugged_in_has_no_rate() {
        let samples = vec![sample(0, 80, true, None), sample(600, 81, true, None)];
        assert!(estimate(&samples).is_err());
        assert!(estimate(&[]).is_err());
    }

    /// macOS can hold 100% for a while after a full charge while the gauge
    /// already shows the drain.
    #[test]
    fn gauge_rate_is_used_while_the_percent_has_not_moved() {
        let samples: Vec<_> = (0..=16).map(|m| gauge_sample(m * 60, 100, 7548 - m as u32 * 13)).collect();
        let e = estimate(&samples).unwrap();
        assert_eq!(e.average_ma, Some(780.0));
        assert_eq!(e.percent_per_hour, 10.24);
        // 7340 mAh left at 780 mA.
        assert_eq!(e.projected_minutes_to_empty, Some(565.0));
        assert_eq!(e.confidence, Confidence::Low);
        assert!(e.confidence_reason.contains("battery gauge"), "{}", e.confidence_reason);
    }

    #[test]
    fn gauge_confidence_grows_with_the_window() {
        let run = |mins: i64| -> Vec<_> { (0..=mins).map(|m| gauge_sample(m * 60, 90, 7000 - m as u32 * 10)).collect() };
        assert_eq!(estimate(&run(25)).unwrap().confidence, Confidence::Medium);
        assert_eq!(estimate(&run(60)).unwrap().confidence, Confidence::High);
    }

    #[test]
    fn flat_or_rising_gauge_has_no_rate_or_confidence() {
        let samples: Vec<_> = (0..=60).map(|m| gauge_sample(m * 60, 90, 7000 + (m / 30) as u32 * 10)).collect();
        let e = estimate(&samples).unwrap();
        assert_eq!(e.percent_per_hour, 0.0);
        assert_eq!(e.projected_minutes_to_empty, None);
        assert_eq!(e.confidence, Confidence::Low);
    }

    #[test]
    fn a_missing_gauge_reading_at_the_start_still_uses_the_gauge() {
        let mut samples: Vec<_> = (0..=30).map(|m| gauge_sample(m * 60, 100, 7500 - m as u32 * 13)).collect();
        samples[0].charge_mah = None;
        let e = estimate(&samples).unwrap();
        assert_eq!(e.average_ma, Some(780.0));
        assert!(e.confidence_reason.contains("battery gauge over 29 min"), "{}", e.confidence_reason);
    }

    #[test]
    fn opening_an_old_file_adds_the_new_column() {
        let path = std::env::temp_dir().join(format!("macpilot-migrate-test-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE battery_samples (ts INTEGER NOT NULL, percent INTEGER NOT NULL, charging INTEGER NOT NULL,
                 external_power INTEGER NOT NULL, charge_mah INTEGER, amperage_ma INTEGER, voltage_mv INTEGER);
                 INSERT INTO battery_samples VALUES (100, 90, 0, 0, 7000, -800, 12000);",
            )
            .unwrap();
        }
        let h = History::open(&path).unwrap();
        h.insert(&gauge_sample(200, 89, 6990)).unwrap();
        let all = h.since(0).unwrap();
        assert_eq!(all[0].full_charge_mah, None);
        assert_eq!(all[0].charge_mah, Some(7000));
        assert_eq!(all[1].full_charge_mah, Some(7620));
        drop(h);
        History::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn flat_reading_gives_no_projection() {
        let samples: Vec<_> = (0..=12).map(|m| sample(m * 60, 77, false, None)).collect();
        let e = estimate(&samples).unwrap();
        assert_eq!(e.percent_per_hour, 0.0);
        assert_eq!(e.projected_minutes_to_empty, None);
        assert_eq!(e.confidence, Confidence::Low);
    }
}
