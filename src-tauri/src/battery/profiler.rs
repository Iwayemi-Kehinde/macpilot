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
        condition: health["sppower_battery_health"].as_str().map(shown_condition),
        max_capacity_percent: health["sppower_battery_health_maximum_capacity"]
            .as_str()
            .and_then(|s| s.trim().trim_end_matches('%').parse::<u8>().ok())
            .filter(|p| *p <= 100),
    }
}

/// The JSON gives the condition's internal name; macOS shows it through the
/// report's string table (SPPowerReporter's Localizable.loctable), so "Good"
/// reads "Normal" there and in System Settings.
fn shown_condition(raw: &str) -> String {
    match raw {
        "Good" => "Normal",
        "Fair" | "Poor" | "Check Battery" => "Service Recommended",
        other => other,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_fixture() {
        let b = parse(include_str!("../../tests/fixtures/system_profiler_power.json"));
        // The fixture says "Good"; System Settings and the text report say "Normal".
        assert_eq!(b.condition.as_deref(), Some("Normal"));
        assert_eq!(b.max_capacity_percent, Some(85));
    }

    #[test]
    fn conditions_read_as_macos_shows_them() {
        let condition = |raw: &str| {
            parse(&format!(
                r#"{{"SPPowerDataType":[{{"_name":"spbattery_information","sppower_battery_health_info":{{"sppower_battery_health":"{raw}"}}}}]}}"#
            ))
            .condition
        };
        assert_eq!(condition("Fair").as_deref(), Some("Service Recommended"));
        assert_eq!(condition("Check Battery").as_deref(), Some("Service Recommended"));
        assert_eq!(condition("Something New").as_deref(), Some("Something New"));
    }

    #[test]
    fn missing_battery_section() {
        assert_eq!(parse(r#"{"SPPowerDataType":[{"_name":"sppower_information"}]}"#), ProfilerBattery::default());
        assert_eq!(parse("not json"), ProfilerBattery::default());
    }
}
