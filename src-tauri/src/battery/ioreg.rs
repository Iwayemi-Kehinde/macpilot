//! Parser for `ioreg -rn AppleSmartBattery`.
//!
//! Only top-level `"Key" = value` lines are read. Nested dictionaries such as
//! `BatteryData` sit on a single line and are kept as opaque values.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum IoValue {
    Int(u64),
    Bool(bool),
    Str(String),
    Other,
}

pub fn parse_properties(output: &str) -> HashMap<String, IoValue> {
    let mut props = HashMap::new();
    for line in output.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('"') else { continue };
        let Some((key, value)) = rest.split_once("\" = ") else { continue };
        let value = value.trim();
        let parsed = if value == "Yes" {
            IoValue::Bool(true)
        } else if value == "No" {
            IoValue::Bool(false)
        } else if let Ok(n) = value.parse::<u64>() {
            IoValue::Int(n)
        } else if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            IoValue::Str(value[1..value.len() - 1].to_string())
        } else {
            IoValue::Other
        };
        // Keep the first occurrence if ioreg ever lists more than one object.
        props.entry(key.to_string()).or_insert(parsed);
    }
    props
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IoregBattery {
    pub installed: Option<bool>,
    pub cycle_count: Option<u32>,
    pub design_capacity_mah: Option<u32>,
    /// `AppleRawMaxCapacity`: the pack's current full-charge capacity.
    pub full_charge_capacity_mah: Option<u32>,
    /// `AppleRawCurrentCapacity`: charge currently held.
    pub current_charge_mah: Option<u32>,
    pub voltage_mv: Option<u32>,
    /// Negative while discharging.
    pub amperage_ma: Option<i64>,
    pub temperature_c: Option<f64>,
    pub is_charging: Option<bool>,
    pub external_connected: Option<bool>,
}

pub fn parse(output: &str) -> IoregBattery {
    let p = parse_properties(output);
    let int = |k: &str| match p.get(k) {
        Some(IoValue::Int(n)) => Some(*n),
        _ => None,
    };
    let boolean = |k: &str| match p.get(k) {
        Some(IoValue::Bool(b)) => Some(*b),
        _ => None,
    };
    let mah = |k: &str| int(k).filter(|v| (1..=100_000).contains(v)).map(|v| v as u32);

    IoregBattery {
        installed: boolean("BatteryInstalled"),
        cycle_count: int("CycleCount").filter(|v| *v < 100_000).map(|v| v as u32),
        design_capacity_mah: mah("DesignCapacity"),
        full_charge_capacity_mah: mah("AppleRawMaxCapacity"),
        current_charge_mah: int("AppleRawCurrentCapacity")
            .filter(|v| *v <= 100_000)
            .map(|v| v as u32),
        voltage_mv: int("Voltage").filter(|v| (1..=30_000).contains(v)).map(|v| v as u32),
        // ioreg prints signed amperage as an unsigned 64-bit two's complement value.
        amperage_ma: int("InstantAmperage")
            .or_else(|| int("Amperage"))
            .map(|v| v as i64)
            .filter(|v| v.abs() <= 20_000),
        // Reported in hundredths of a degree Celsius.
        temperature_c: int("Temperature")
            .map(|v| v as f64 / 100.0)
            .filter(|c| (-20.0..=100.0).contains(c)),
        is_charging: boolean("IsCharging"),
        external_connected: boolean("ExternalConnected"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_fixture() {
        let b = parse(include_str!("../../tests/fixtures/ioreg_battery.txt"));
        assert_eq!(b.installed, Some(true));
        assert_eq!(b.cycle_count, Some(555));
        assert_eq!(b.design_capacity_mah, Some(8694));
        assert_eq!(b.full_charge_capacity_mah, Some(7627));
        assert_eq!(b.current_charge_mah, Some(6230));
        assert_eq!(b.voltage_mv, Some(12106));
        assert_eq!(b.amperage_ma, Some(-1024));
        assert_eq!(b.temperature_c, Some(30.77));
        assert_eq!(b.is_charging, Some(false));
        assert_eq!(b.external_connected, Some(false));
    }

    #[test]
    fn nested_dictionaries_are_opaque() {
        let p = parse_properties(include_str!("../../tests/fixtures/ioreg_battery.txt"));
        assert_eq!(p.get("BatteryData"), Some(&IoValue::Other));
        assert_eq!(p.get("Serial"), Some(&IoValue::Str("REDACTED".into())));
    }

    #[test]
    fn empty_output_gives_all_none() {
        assert_eq!(parse(""), IoregBattery::default());
    }

    #[test]
    fn rejects_out_of_range_values() {
        let b = parse("  \"DesignCapacity\" = 0\n  \"Temperature\" = 99999\n  \"Voltage\" = 500000\n");
        assert_eq!(b.design_capacity_mah, None);
        assert_eq!(b.temperature_c, None);
        assert_eq!(b.voltage_mv, None);
    }

    #[test]
    fn positive_amperage_while_charging() {
        let b = parse("  \"InstantAmperage\" = 2150\n  \"IsCharging\" = Yes\n");
        assert_eq!(b.amperage_ma, Some(2150));
        assert_eq!(b.is_charging, Some(true));
    }
}
