//! Local battery history in SQLite, plus the discharge-rate estimate.

use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;

/// Samples further apart than this are treated as separate segments (the Mac
/// was likely asleep or MacPilot was closed), so no rate spans the gap.
pub const MAX_SAMPLE_GAP_SECS: i64 = 15 * 60;
const MIN_WINDOW_SECS: i64 = 10 * 60;
const MIN_SAMPLES: usize = 3;
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

    pub fn since(&self, since_ts: i64) -> rusqlite::Result<Vec<BatterySample>> {
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
}

/// Estimates the drain rate from the most recent unbroken run of samples taken
/// on battery power. Returns a plain-language reason when it cannot.
pub fn estimate_discharge(samples: &[BatterySample]) -> Result<DischargeEstimate, String> {
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
        (Some((gauge_minutes, ma, _)), Some(_)) if ma <= 1.0 => (
            0.0,
            None,
            Confidence::Low,
            format!("The battery gauge's charge did not fall over {gauge_minutes:.0} min, so there is no drain to project."),
        ),
        (Some((gauge_minutes, ma, held)), Some(full)) => {
            let (confidence, reason) = if gauge_minutes >= 60.0 {
                (Confidence::High, format!("Measured by the battery gauge over {gauge_minutes:.0} min of continuous samples."))
            } else if gauge_minutes >= 20.0 {
                (Confidence::Medium, format!("Measured by the battery gauge over {gauge_minutes:.0} min; it follows recent load and settles as samples build up."))
            } else {
                (Confidence::Low, format!("Measured by the battery gauge, but only over {gauge_minutes:.0} min, so it reflects whatever ran in that time."))
            };
            (ma / full as f64 * 100.0, Some(held as f64 / ma * 60.0), confidence, reason)
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
        let e = estimate_discharge(&samples).unwrap();
        assert_eq!(e.percent_per_hour, 10.0);
        assert_eq!(e.average_ma, Some(780.0));
        assert_eq!(e.projected_minutes_to_empty, Some(420.0));
        assert_eq!(e.confidence, Confidence::High);
        assert_eq!(e.sample_count, 61);
    }

    #[test]
    fn only_counts_samples_since_unplugging() {
        let mut samples = vec![sample(0, 50, true, None), sample(600, 60, true, None)];
        samples.extend((0..=20).map(|m| sample(1200 + m * 60, 60 - (m / 10) as u8, false, None)));
        let e = estimate_discharge(&samples).unwrap();
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
        let err = estimate_discharge(&samples).unwrap_err();
        assert!(err.contains("5 min"), "{err}");
    }

    #[test]
    fn plugged_in_has_no_rate() {
        let samples = vec![sample(0, 80, true, None), sample(600, 81, true, None)];
        assert!(estimate_discharge(&samples).is_err());
        assert!(estimate_discharge(&[]).is_err());
    }

    /// macOS can hold 100% for a while after a full charge while the gauge
    /// already shows the drain.
    #[test]
    fn gauge_rate_is_used_while_the_percent_has_not_moved() {
        let samples: Vec<_> = (0..=16).map(|m| gauge_sample(m * 60, 100, 7548 - m as u32 * 13)).collect();
        let e = estimate_discharge(&samples).unwrap();
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
        assert_eq!(estimate_discharge(&run(25)).unwrap().confidence, Confidence::Medium);
        assert_eq!(estimate_discharge(&run(60)).unwrap().confidence, Confidence::High);
    }

    #[test]
    fn flat_or_rising_gauge_has_no_rate_or_confidence() {
        let samples: Vec<_> = (0..=60).map(|m| gauge_sample(m * 60, 90, 7000 + (m / 30) as u32 * 10)).collect();
        let e = estimate_discharge(&samples).unwrap();
        assert_eq!(e.percent_per_hour, 0.0);
        assert_eq!(e.projected_minutes_to_empty, None);
        assert_eq!(e.confidence, Confidence::Low);
    }

    #[test]
    fn a_missing_gauge_reading_at_the_start_still_uses_the_gauge() {
        let mut samples: Vec<_> = (0..=30).map(|m| gauge_sample(m * 60, 100, 7500 - m as u32 * 13)).collect();
        samples[0].charge_mah = None;
        let e = estimate_discharge(&samples).unwrap();
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
        let e = estimate_discharge(&samples).unwrap();
        assert_eq!(e.percent_per_hour, 0.0);
        assert_eq!(e.projected_minutes_to_empty, None);
        assert_eq!(e.confidence, Confidence::Low);
    }
}
