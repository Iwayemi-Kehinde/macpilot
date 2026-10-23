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
    pub power_source: Option<String>,
    pub percent: Option<u8>,
    pub state: ChargeState,
    pub external_power: Option<bool>,
    pub time_remaining_min: Option<u32>,
    pub time_remaining_kind: Option<TimeKind>,
    pub cycle_count: Option<u32>,
    pub design_capacity_mah: Option<u32>,
    pub full_charge_capacity_mah: Option<u32>,
    pub current_charge_mah: Option<u32>,
    /// Full-charge capacity divided by design capacity, from raw gauge values.
    pub raw_capacity_percent: Option<f64>,
    /// "Maximum Capacity" exactly as macOS reports it in System Settings.
    pub apple_max_capacity_percent: Option<u8>,
    pub condition: Option<String>,
    pub voltage_mv: Option<u32>,
    pub amperage_ma: Option<i64>,
    /// Instantaneous power; negative while the battery is draining.
    pub power_w: Option<f64>,
    pub temperature_c: Option<f64>,
    /// Why specific values are missing, in plain language.
    pub notes: Vec<String>,
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
            design_capacity_mah: None,
            full_charge_capacity_mah: None,
            current_charge_mah: None,
            raw_capacity_percent: None,
            apple_max_capacity_percent: None,
            condition: None,
            voltage_mv: None,
            amperage_ma: None,
            power_w: None,
            temperature_c: None,
            notes,
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

    let raw_capacity_percent = match (io.full_charge_capacity_mah, io.design_capacity_mah) {
        (Some(full), Some(design)) => Some(round1(full as f64 / design as f64 * 100.0)),
        _ => {
            notes.push("Raw capacity ratio unavailable: ioreg did not report both capacities.".to_string());
            None
        }
    };
    if prof.max_capacity_percent.is_none() {
        notes.push("Apple's maximum-capacity figure was not reported by system_profiler.".to_string());
    }

    let power_w = match (io.voltage_mv, io.amperage_ma) {
        (Some(v), Some(a)) => Some(round2(v as f64 * a as f64 / 1_000_000.0)),
        _ => None,
    };
    if power_w.is_none() {
        notes.push("Power draw unavailable: voltage or current not reported.".to_string());
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
        design_capacity_mah: io.design_capacity_mah,
        full_charge_capacity_mah: io.full_charge_capacity_mah,
        current_charge_mah: io.current_charge_mah,
        raw_capacity_percent,
        apple_max_capacity_percent: prof.max_capacity_percent,
        condition: prof.condition.clone(),
        voltage_mv: io.voltage_mv,
        amperage_ma: io.amperage_ma,
        power_w,
        temperature_c: io.temperature_c,
        notes,
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
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
        assert_eq!(b.raw_capacity_percent, Some(87.7));
        assert_eq!(b.apple_max_capacity_percent, Some(85));
        assert_eq!(b.condition.as_deref(), Some("Normal"));
        // 12.106 V x -1.024 A
        assert_eq!(b.power_w, Some(-12.4));
        assert!(b.notes.is_empty(), "{:?}", b.notes);
    }

    #[test]
    fn missing_values_are_explained_not_invented() {
        let (pm, _, _) = fixtures();
        let b = combine(&pm, &ioreg::IoregBattery::default(), &profiler::ProfilerBattery::default());
        assert_eq!(b.cycle_count, None);
        assert_eq!(b.raw_capacity_percent, None);
        assert_eq!(b.power_w, None);
        assert_eq!(b.notes.len(), 4);
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
