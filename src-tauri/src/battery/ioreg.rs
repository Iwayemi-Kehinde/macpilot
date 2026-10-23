//! Parser for `ioreg -rd1 -w0 -n AppleSmartBattery`.
//!
//! Top-level `"Key" = value` lines are read. Nested dictionaries sit on a
//! single line; the power telemetry and adapter details are read from theirs.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum IoValue {
    Int(u64),
    Bool(bool),
    Str(String),
    Other,
}

fn value(text: &str) -> IoValue {
    let text = text.trim();
    if text == "Yes" {
        IoValue::Bool(true)
    } else if text == "No" {
        IoValue::Bool(false)
    } else if let Ok(n) = text.parse::<u64>() {
        IoValue::Int(n)
    } else if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        IoValue::Str(text[1..text.len() - 1].to_string())
    } else {
        IoValue::Other
    }
}

pub fn parse_properties(output: &str) -> HashMap<String, IoValue> {
    let mut props = HashMap::new();
    for line in output.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('"') else { continue };
        let Some((key, text)) = rest.split_once("\" = ") else { continue };
        // Keep the first occurrence if ioreg ever lists more than one object.
        props.entry(key.to_string()).or_insert_with(|| value(text));
    }
    props
}

/// The top-level entries of a one-line dictionary such as
/// `{"SystemLoad"=5259,"Menu"=({"Index"=0}),"Description"="pd charger"}`.
/// Nested values are kept as `Other`.
pub fn parse_dictionary(text: &str) -> HashMap<String, IoValue> {
    let mut entries = HashMap::new();
    let Some(inner) = text.trim().strip_prefix('{').and_then(|t| t.strip_suffix('}')) else { return entries };
    let (mut depth, mut quoted, mut start) = (0usize, false, 0);
    let mut add = |entry: &str| {
        let Some((key, text)) = entry.split_once('=') else { return };
        let Some(key) = key.trim().strip_prefix('"').and_then(|k| k.strip_suffix('"')) else { return };
        entries.entry(key.to_string()).or_insert_with(|| value(text));
    };
    for (i, c) in inner.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '{' | '(' | '<' if !quoted => depth += 1,
            '}' | ')' | '>' if !quoted => depth = depth.saturating_sub(1),
            ',' if !quoted && depth == 0 => {
                add(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    add(&inner[start..]);
    entries
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
    /// What the whole Mac draws, from `PowerTelemetryData`.
    pub system_load_mw: Option<u64>,
    /// What the adapter supplies.
    pub adapter_input_mw: Option<u64>,
    /// Into the battery while positive, out of it while negative.
    pub battery_power_mw: Option<i64>,
    /// The connected adapter's rating, from `AdapterDetails`.
    pub adapter_watts: Option<u32>,
}

/// ioreg prints signed values as unsigned 64-bit two's complement.
fn signed(n: u64) -> i64 {
    n as i64
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
    let dictionary = |k: &str| match p.get(k) {
        Some(IoValue::Other) => output
            .lines()
            .find_map(|l| l.trim().strip_prefix(&format!("\"{k}\" = ")).map(parse_dictionary))
            .unwrap_or_default(),
        _ => HashMap::new(),
    };
    let telemetry = dictionary("PowerTelemetryData");
    let adapter = dictionary("AdapterDetails");
    let milliwatts = |d: &HashMap<String, IoValue>, k: &str| match d.get(k) {
        Some(IoValue::Int(n)) => Some(*n),
        _ => None,
    };
    // A Mac draws well under 1 kW; anything larger is a misread.
    let plausible_mw = |mw: &u64| *mw <= 1_000_000;

    IoregBattery {
        installed: boolean("BatteryInstalled"),
        cycle_count: int("CycleCount").filter(|v| *v < 100_000).map(|v| v as u32),
        design_capacity_mah: mah("DesignCapacity"),
        full_charge_capacity_mah: mah("AppleRawMaxCapacity"),
        current_charge_mah: int("AppleRawCurrentCapacity")
            .filter(|v| *v <= 100_000)
            .map(|v| v as u32),
        voltage_mv: int("Voltage").filter(|v| (1..=30_000).contains(v)).map(|v| v as u32),
        amperage_ma: int("InstantAmperage").or_else(|| int("Amperage")).map(signed).filter(|v| v.abs() <= 20_000),
        // Reported in hundredths of a degree Celsius.
        temperature_c: int("Temperature")
            .map(|v| v as f64 / 100.0)
            .filter(|c| (-20.0..=100.0).contains(c)),
        is_charging: boolean("IsCharging"),
        external_connected: boolean("ExternalConnected"),
        system_load_mw: milliwatts(&telemetry, "SystemLoad").filter(plausible_mw),
        adapter_input_mw: milliwatts(&telemetry, "SystemPowerIn").filter(plausible_mw),
        battery_power_mw: milliwatts(&telemetry, "BatteryPower").map(signed).filter(|mw| mw.unsigned_abs() <= 1_000_000),
        adapter_watts: milliwatts(&adapter, "Watts").filter(|w| (1..=1000).contains(w)).map(|w| w as u32),
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
        // On battery the whole draw comes out of the battery, and no adapter is described.
        assert_eq!((b.system_load_mw, b.adapter_input_mw, b.battery_power_mw), (Some(11783), Some(0), Some(-11783)));
        assert_eq!(b.adapter_watts, None);
    }

    /// Captured with the flags the app uses (`-rd1 -w0`), while charging.
    #[test]
    fn parses_depth_limited_unwrapped_output_while_charging() {
        let b = parse(include_str!("../../tests/fixtures/ioreg_battery_d1.txt"));
        assert_eq!(b.installed, Some(true));
        assert_eq!(b.cycle_count, Some(555));
        assert_eq!(b.design_capacity_mah, Some(8694));
        assert_eq!(b.full_charge_capacity_mah, Some(7625));
        assert_eq!(b.current_charge_mah, Some(7548));
        assert_eq!(b.voltage_mv, Some(12794));
        assert_eq!(b.amperage_ma, Some(388));
        assert_eq!(b.temperature_c, Some(30.95));
        assert_eq!(b.is_charging, Some(true));
        assert_eq!(b.external_connected, Some(true));
        // 14.32 W in from a 61 W adapter: 9.15 W for the Mac, 5.17 W into the battery.
        assert_eq!((b.system_load_mw, b.adapter_input_mw, b.battery_power_mw), (Some(9151), Some(14320), Some(5169)));
        assert_eq!(b.adapter_watts, Some(61));
    }

    #[test]
    fn dictionaries_keep_top_level_entries_only() {
        let d = parse_dictionary(r#"{"Watts"=61,"Menu"=({"Index"=0,"Max"=5000},{"Index"=1}),"Description"="pd, charger","Neg"=18446744073709546357,"IsWireless"=No}"#);
        assert_eq!(d.get("Watts"), Some(&IoValue::Int(61)));
        assert_eq!(d.get("Menu"), Some(&IoValue::Other));
        assert_eq!(d.get("Index"), None);
        assert_eq!(d.get("Description"), Some(&IoValue::Str("pd, charger".into())));
        assert_eq!(d.get("Neg").map(|v| match v { IoValue::Int(n) => signed(*n), _ => 0 }), Some(-5259));
        assert_eq!(d.get("IsWireless"), Some(&IoValue::Bool(false)));
        assert!(parse_dictionary("not a dictionary").is_empty());
        assert!(parse_dictionary("{}").is_empty());
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
