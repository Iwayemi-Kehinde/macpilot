//! Parser for `pmset -g batt`.
//!
//! Typical output:
//! ```text
//! Now drawing from 'Battery Power'
//!  -InternalBattery-0 (id=6684771)	93%; discharging; 11:41 remaining present: true
//! ```

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChargeState {
    Charging,
    Discharging,
    Charged,
    AcAttached,
    FinishingCharge,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PmsetBattery {
    pub power_source: Option<String>,
    pub present: bool,
    pub percent: Option<u8>,
    pub state: ChargeState,
    /// Minutes as estimated by macOS. `None` when pmset gives no estimate
    /// or when the estimate does not apply (charged, AC attached).
    pub time_remaining_min: Option<u32>,
}

pub fn parse(output: &str) -> PmsetBattery {
    let power_source = output.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("Now drawing from '")?;
        Some(rest.split('\'').next()?.to_string())
    });

    let Some(line) = output.lines().find(|l| l.contains("InternalBattery")) else {
        return PmsetBattery {
            power_source,
            present: false,
            percent: None,
            state: ChargeState::Unknown,
            time_remaining_min: None,
        };
    };

    let present = !line.contains("present: false");
    // Fields follow the "(id=...)" block, separated by ';'.
    let fields_part = line.split_once(')').map_or(line, |(_, rest)| rest);
    let fields: Vec<&str> = fields_part.split(';').map(str::trim).collect();

    let percent = fields
        .first()
        .and_then(|f| f.strip_suffix('%'))
        .and_then(|p| p.trim().parse::<u8>().ok())
        .filter(|p| *p <= 100);

    let state = match fields.get(1).map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("charging") => ChargeState::Charging,
        Some("discharging") => ChargeState::Discharging,
        Some("charged") => ChargeState::Charged,
        Some("ac attached") => ChargeState::AcAttached,
        Some("finishing charge") => ChargeState::FinishingCharge,
        _ => ChargeState::Unknown,
    };

    let time_remaining_min = match state {
        ChargeState::Charging | ChargeState::Discharging | ChargeState::FinishingCharge => {
            fields.iter().skip(2).find_map(|f| parse_remaining(f))
        }
        _ => None,
    };

    PmsetBattery {
        power_source,
        present,
        percent,
        state,
        time_remaining_min,
    }
}

/// Parses "11:41 remaining ..." into minutes. "(no estimate)" yields `None`.
fn parse_remaining(field: &str) -> Option<u32> {
    let mut words = field.split_whitespace();
    let time = words.next()?;
    if words.next()? != "remaining" {
        return None;
    }
    let (h, m) = time.split_once(':')?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    (m < 60).then_some(h * 60 + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_discharging_output() {
        let b = parse(include_str!("../../tests/fixtures/pmset_discharging.txt"));
        assert_eq!(b.power_source.as_deref(), Some("Battery Power"));
        assert!(b.present);
        assert_eq!(b.percent, Some(86));
        assert_eq!(b.state, ChargeState::Discharging);
        assert_eq!(b.time_remaining_min, Some(12 * 60 + 6));
    }

    #[test]
    fn parses_charging_with_time_to_full() {
        let b = parse("Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t54%; charging; 1:05 remaining present: true\n");
        assert_eq!(b.power_source.as_deref(), Some("AC Power"));
        assert_eq!(b.percent, Some(54));
        assert_eq!(b.state, ChargeState::Charging);
        assert_eq!(b.time_remaining_min, Some(65));
    }

    #[test]
    fn no_estimate_is_none() {
        let b = parse("Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t71%; discharging; (no estimate) present: true\n");
        assert_eq!(b.state, ChargeState::Discharging);
        assert_eq!(b.time_remaining_min, None);
    }

    #[test]
    fn charged_and_ac_attached_have_no_time() {
        let charged = parse("Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t100%; charged; 0:00 remaining present: true\n");
        assert_eq!(charged.state, ChargeState::Charged);
        assert_eq!(charged.time_remaining_min, None);

        let held = parse("Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t80%; AC attached; not charging present: true\n");
        assert_eq!(held.state, ChargeState::AcAttached);
        assert_eq!(held.percent, Some(80));
        assert_eq!(held.time_remaining_min, None);
    }

    #[test]
    fn desktop_without_battery() {
        let b = parse("Now drawing from 'AC Power'\n");
        assert!(!b.present);
        assert_eq!(b.percent, None);
        assert_eq!(b.state, ChargeState::Unknown);
    }

    #[test]
    fn garbage_is_tolerated() {
        let b = parse(" -InternalBattery-0 (id=1)\t250%; exploding; soon remaining\n");
        assert_eq!(b.percent, None);
        assert_eq!(b.state, ChargeState::Unknown);
        assert_eq!(b.time_remaining_min, None);
        assert_eq!(parse("").percent, None);
    }
}
