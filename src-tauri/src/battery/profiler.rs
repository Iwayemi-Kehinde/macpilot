//! Parser for `system_profiler SPPowerDataType -json`, used only for the
//! values Apple computes itself: battery condition and "Maximum Capacity".

use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfilerBattery {
    pub condition: Option<String>,
    pub max_capacity_percent: Option<u8>,
}

pub fn parse(json: &str) -> ProfilerBattery {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return ProfilerBattery::default();
    };
    let health = root["SPPowerDataType"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| item["_name"] == "spbattery_information")
        .map(|item| &item["sppower_battery_health_info"]);
    let Some(health) = health else {
        return ProfilerBattery::default();
    };

    ProfilerBattery {
        condition: health["sppower_battery_health"].as_str().map(str::to_string),
        max_capacity_percent: health["sppower_battery_health_maximum_capacity"]
            .as_str()
            .and_then(|s| s.trim().trim_end_matches('%').parse::<u8>().ok())
            .filter(|p| *p <= 100),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_fixture() {
        let b = parse(include_str!("../../tests/fixtures/system_profiler_power.json"));
        assert_eq!(b.condition.as_deref(), Some("Good"));
        assert_eq!(b.max_capacity_percent, Some(85));
    }

    #[test]
    fn missing_battery_section() {
        assert_eq!(parse(r#"{"SPPowerDataType":[{"_name":"sppower_information"}]}"#), ProfilerBattery::default());
        assert_eq!(parse("not json"), ProfilerBattery::default());
    }
}
