//! Battery data from native macOS tools: `pmset` for charge and the system
//! time estimate, `ioreg` for the gas-gauge readings, and `system_profiler`
//! for Apple's own condition and maximum-capacity figures.

pub mod ioreg;
pub mod pmset;
pub mod profiler;

use std::time::{Duration, Instant};

use serde::Serialize;

pub use pmset::ChargeState;

use crate::tool::run;

const PMSET: &str = "/usr/bin/pmset";
const IOREG: &str = "/usr/sbin/ioreg";
const SYSTEM_PROFILER: &str = "/usr/sbin/system_profiler";

/// Apple's condition and maximum-capacity values change over weeks, so the
/// slower `system_profiler` call is cached.
const PROFILER_TTL: Duration = Duration::from_secs(15 * 60);
const GAUGE_TIMEOUT: Duration = Duration::from_secs(5);
const PROFILER_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeKind {
    UntilEmpty,
    UntilFull,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryInfo {
    pub present: bool,
    #[serde(skip)]
    pub power_source: Option<String>,
    pub percent: Option<u8>,
    pub state: ChargeState,
    pub external_power: Option<bool>,
    pub time_remaining_min: Option<u32>,
    pub time_remaining_kind: Option<TimeKind>,
    pub cycle_count: Option<u32>,
    /// "Maximum Capacity" exactly as macOS reports it in System Settings.
    pub apple_max_capacity_percent: Option<u8>,
    pub condition: Option<String>,
    pub temperature_c: Option<f64>,
    /// What the whole Mac draws right now, screen included.
    pub system_power_w: Option<f64>,
    /// What the adapter supplies; 0 on battery.
    pub adapter_power_w: Option<f64>,
    /// Into the battery while positive, out of it while negative.
    pub battery_power_w: Option<f64>,
    pub adapter_rating_w: Option<u32>,
    /// On battery, the drain the gauge's present current amounts to, in the
    /// percentage macOS shows per hour. Set from the history's calibration.
    pub drain_percent_per_hour: Option<f64>,
    /// False while the drain is the gauge's share, before there is enough history to calibrate.
    pub drain_calibrated: bool,
    /// Why specific values are missing, in plain language.
    pub notes: Vec<String>,
    #[serde(skip)]
    pub full_charge_capacity_mah: Option<u32>,
    #[serde(skip)]
    pub current_charge_mah: Option<u32>,
    #[serde(skip)]
    pub voltage_mv: Option<u32>,
    #[serde(skip)]
    pub amperage_ma: Option<i64>,
}

pub fn combine(
    pm: &pmset::PmsetBattery,
    io: &ioreg::IoregBattery,
    prof: &profiler::ProfilerBattery,
) -> BatteryInfo {
    let present = pm.present && io.installed != Some(false);
    let mut notes = Vec::new();

    if !present {
        notes.push("No internal battery reported by macOS.".to_string());
        return BatteryInfo {
            present,
            power_source: pm.power_source.clone(),
            percent: None,
            state: ChargeState::Unknown,
            external_power: io.external_connected,
            time_remaining_min: None,
            time_remaining_kind: None,
            cycle_count: None,
            apple_max_capacity_percent: None,
            condition: None,
            temperature_c: None,
            system_power_w: io.system_load_mw.map(milliwatts),
            adapter_power_w: io.adapter_input_mw.map(milliwatts),
            battery_power_w: None,
            adapter_rating_w: io.adapter_watts,
            drain_percent_per_hour: None,
            drain_calibrated: false,
            notes,
            full_charge_capacity_mah: None,
            current_charge_mah: None,
            voltage_mv: None,
            amperage_ma: None,
        };
    }

    let time_remaining_kind = match pm.state {
        ChargeState::Discharging => Some(TimeKind::UntilEmpty),
        ChargeState::Charging | ChargeState::FinishingCharge => Some(TimeKind::UntilFull),
        _ => None,
    };
    if time_remaining_kind.is_some() && pm.time_remaining_min.is_none() {
        notes.push("macOS has not produced a time estimate yet.".to_string());
    }
    if prof.max_capacity_percent.is_none() {
        notes.push("Apple's maximum-capacity figure was not reported by system_profiler.".to_string());
    }

    // The telemetry counters, or voltage times current where they are missing.
    let battery_power_w = io.battery_power_mw.map(|mw| mw as f64 / 1000.0).or_else(|| match (io.voltage_mv, io.amperage_ma) {
        (Some(v), Some(a)) => Some(v as f64 * a as f64 / 1_000_000.0),
        _ => None,
    });
    let on_battery = io.external_connected == Some(false);
    let system_power_w = io
        .system_load_mw
        .map(milliwatts)
        .or_else(|| battery_power_w.filter(|w| on_battery && *w <= 0.0).map(f64::abs));
    if system_power_w.is_none() {
        notes.push("The Mac's power draw was not reported by the battery's power telemetry.".to_string());
    }
    if io.temperature_c.is_none() {
        notes.push("Battery temperature not reported.".to_string());
    }

    BatteryInfo {
        present,
        power_source: pm.power_source.clone(),
        percent: pm.percent,
        state: pm.state,
        external_power: io.external_connected,
        time_remaining_min: time_remaining_kind.and(pm.time_remaining_min),
        time_remaining_kind,
        cycle_count: io.cycle_count,
        apple_max_capacity_percent: prof.max_capacity_percent,
        condition: prof.condition.clone(),
        temperature_c: io.temperature_c,
        system_power_w: system_power_w.map(round2),
        adapter_power_w: io.adapter_input_mw.map(milliwatts),
        battery_power_w: battery_power_w.map(round2),
        adapter_rating_w: io.adapter_watts.filter(|_| io.external_connected != Some(false)),
        drain_percent_per_hour: None,
        drain_calibrated: false,
        notes,
        full_charge_capacity_mah: io.full_charge_capacity_mah,
        current_charge_mah: io.current_charge_mah,
        voltage_mv: io.voltage_mv,
        amperage_ma: io.amperage_ma,
    }
}

fn milliwatts(mw: u64) -> f64 {
    round2(mw as f64 / 1000.0)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}


#[derive(Default)]
pub struct BatteryReader {
    profiler_cache: Option<(Instant, profiler::ProfilerBattery)>,
}

impl BatteryReader {
    /// Fast read for history sampling: pmset and ioreg only.
    pub fn read_gauge() -> Result<(pmset::PmsetBattery, ioreg::IoregBattery), String> {
        let pm = pmset::parse(&run(PMSET, &["-g", "batt"], GAUGE_TIMEOUT)?);
        // Depth 1 keeps child objects' properties out; -w0 disables line wrapping.
        let io = ioreg::parse(&run(IOREG, &["-rd1", "-w0", "-n", "AppleSmartBattery"], GAUGE_TIMEOUT)?);
        Ok((pm, io))
    }

    pub fn read(&mut self) -> Result<BatteryInfo, String> {
        let (pm, io) = Self::read_gauge()?;
        let prof = self.profiler();
        Ok(combine(&pm, &io, &prof))
    }

    fn profiler(&mut self) -> profiler::ProfilerBattery {
        if let Some((at, cached)) = &self.profiler_cache {
            if at.elapsed() < PROFILER_TTL {
                return cached.clone();
            }
        }
        // A failed call is not cached, so the next read retries.
        match run(SYSTEM_PROFILER, &["SPPowerDataType", "-json"], PROFILER_TIMEOUT) {
            Ok(json) => {
                let parsed = profiler::parse(&json);
                self.profiler_cache = Some((Instant::now(), parsed.clone()));
                parsed
            }
            Err(_) => profiler::ProfilerBattery::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> (pmset::PmsetBattery, ioreg::IoregBattery, profiler::ProfilerBattery) {
        (
            pmset::parse(include_str!("../../tests/fixtures/pmset_discharging.txt")),
            ioreg::parse(include_str!("../../tests/fixtures/ioreg_battery.txt")),
            profiler::parse(include_str!("../../tests/fixtures/system_profiler_power.json")),
        )
    }

    #[test]
    fn combines_real_fixtures() {
        let (pm, io, prof) = fixtures();
        let b = combine(&pm, &io, &prof);
        assert!(b.present);
        assert_eq!(b.percent, Some(86));
        assert_eq!(b.time_remaining_kind, Some(TimeKind::UntilEmpty));
        assert_eq!(b.time_remaining_min, Some(726));
        assert_eq!(b.apple_max_capacity_percent, Some(85));
        assert_eq!(b.condition.as_deref(), Some("Normal"));
        // On battery the Mac's whole draw comes out of the battery.
        assert_eq!((b.system_power_w, b.adapter_power_w, b.battery_power_w), (Some(11.78), Some(0.0), Some(-11.78)));
        assert_eq!(b.adapter_rating_w, None);
        assert!(b.notes.is_empty(), "{:?}", b.notes);
    }

    #[test]
    fn charging_splits_the_adapter_power_between_the_mac_and_the_battery() {
        let (_, _, prof) = fixtures();
        let pm = pmset::parse(" -InternalBattery-0 (id=1)\t99%; charging; 0:20 remaining present: true\n");
        let b = combine(&pm, &ioreg::parse(include_str!("../../tests/fixtures/ioreg_battery_d1.txt")), &prof);
        assert_eq!((b.adapter_power_w, b.system_power_w, b.battery_power_w), (Some(14.32), Some(9.15), Some(5.17)));
        assert_eq!(b.adapter_rating_w, Some(61));
        assert_eq!(b.time_remaining_kind, Some(TimeKind::UntilFull));
    }

    #[test]
    fn power_falls_back_to_voltage_times_current() {
        let (pm, mut io, prof) = fixtures();
        (io.system_load_mw, io.battery_power_mw) = (None, None);
        let b = combine(&pm, &io, &prof);
        // 12.106 V x -1.024 A, all of it drawn by the Mac.
        assert_eq!((b.battery_power_w, b.system_power_w), (Some(-12.4), Some(12.4)));
    }

    #[test]
    fn missing_values_are_explained_not_invented() {
        let (pm, _, _) = fixtures();
        let b = combine(&pm, &ioreg::IoregBattery::default(), &profiler::ProfilerBattery::default());
        assert_eq!(b.cycle_count, None);
        assert_eq!((b.system_power_w, b.battery_power_w), (None, None));
        assert_eq!(b.notes.len(), 3, "{:?}", b.notes);
    }

    /// Every prefix of real output (a truncated read) and pseudo-random bytes
    /// must parse without panicking and without producing out-of-range values.
    #[test]
    fn malformed_output_never_panics_or_invents_values() {
        let pm_src = include_str!("../../tests/fixtures/pmset_discharging.txt");
        let io_src = include_str!("../../tests/fixtures/ioreg_battery.txt");
        let prof_src = include_str!("../../tests/fixtures/system_profiler_power.json");
        let prefixes = |s: &'static str| s.char_indices().map(move |(i, _)| &s[..i]);

        for p in prefixes(pm_src) {
            let b = pmset::parse(p);
            assert!(b.percent.is_none_or(|v| v <= 100));
        }
        for p in prefixes(io_src) {
            let b = ioreg::parse(p);
            assert!(b.temperature_c.is_none_or(|t| (-20.0..=100.0).contains(&t)));
        }
        for p in prefixes(prof_src) {
            assert!(profiler::parse(p).max_capacity_percent.is_none_or(|v| v <= 100));
        }

        const NOISE: &[u8] = b"\"=;:%() \n\t0123456789abcYesNo-{}<>";
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..500 {
            let noise: String = (0..256)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    char::from(NOISE[(seed % NOISE.len() as u64) as usize])
                })
                .collect();
            let b = combine(&pmset::parse(&noise), &ioreg::parse(&noise), &profiler::parse(&noise));
            assert!(b.percent.is_none_or(|v| v <= 100));
        }
    }

    #[test]
    fn no_battery_clears_everything() {
        let pm = pmset::parse("Now drawing from 'AC Power'\n");
        let b = combine(&pm, &ioreg::IoregBattery::default(), &profiler::ProfilerBattery::default());
        assert!(!b.present);
        assert_eq!(b.percent, None);
        assert_eq!(b.power_source.as_deref(), Some("AC Power"));
    }
}
