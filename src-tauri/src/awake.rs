//! What is keeping the Mac or its display awake: the power assertions that
//! `pmset -g assertions` lists by owning process.

use std::time::Duration;

use serde::Serialize;

use crate::apps;
use crate::tool::run;

const PMSET: &str = "/usr/bin/pmset";
const TIMEOUT: Duration = Duration::from_secs(5);
/// powerd holds one assertion for as long as the display is on; it is a
/// consequence of the display being awake, not a cause.
const POWERD: &str = "powerd";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Keeps {
    /// The display stays on.
    Display,
    /// The Mac does not idle-sleep, though the display may turn off.
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    pub pid: u32,
    pub process: String,
    /// The process a service holds it for, as coreaudiod does for an app playing sound.
    pub for_pid: Option<u32>,
    /// The app responsible: `for_pid`'s if it is still running, else the holder's.
    pub app: String,
    pub keeps: Keeps,
    pub reason: String,
    pub held_secs: Option<u64>,
    /// When the assertion releases itself, if it has a timeout.
    pub ends_in_secs: Option<u64>,
}

/// Runs pmset and names the app behind each assertion.
pub fn read() -> Result<Vec<Blocker>, String> {
    let mut blockers = parse(&run(PMSET, &["-g", "assertions"], TIMEOUT)?);
    for b in &mut blockers {
        if let Some((app, _)) = b.for_pid.and_then(apps::identify).or_else(|| apps::identify(b.pid)) {
            b.app = app;
        }
    }
    Ok(blockers)
}

/// The assertions against display or idle sleep, with `app` set to the
/// holding process until `read` resolves it.
pub fn parse(output: &str) -> Vec<Blocker> {
    let mut blockers: Vec<Blocker> = Vec::new();
    // Whether the latest line was an assertion that is kept, so its detail lines apply to it.
    let mut kept = false;
    let mut listing = false;
    for line in output.lines() {
        if line.starts_with("Listed by owning process:") {
            listing = true;
            continue;
        }
        if !listing {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            break;
        }
        if let Some(detail) = line.strip_prefix('\t') {
            if let Some(last) = blockers.last_mut().filter(|_| kept) {
                apply_detail(last, detail.trim());
            }
            continue;
        }
        kept = false;
        if let Some(b) = assertion(line.trim()) {
            if b.process != POWERD {
                blockers.push(b);
                kept = true;
            }
        }
    }
    blockers
}

/// `pid 730(Google Chrome): [0x...] 00:44:40 NoIdleSleepAssertion named: "Playing audio"`
fn assertion(line: &str) -> Option<Blocker> {
    let rest = line.strip_prefix("pid ")?;
    let (pid, rest) = rest.split_once('(')?;
    // A name can contain parentheses, as in "Helper (Renderer)"; the first "): " ends it.
    let (process, rest) = rest.split_once("): ")?;
    let rest = rest.strip_prefix('[')?.split_once("] ")?.1;
    let mut fields = rest.splitn(3, ' ');
    let held = fields.next()?;
    let keeps = match fields.next()? {
        "PreventUserIdleDisplaySleep" | "NoDisplaySleepAssertion" | "InternalPreventDisplaySleep" => Keeps::Display,
        "PreventUserIdleSystemSleep" | "NoIdleSleepAssertion" | "PreventSystemSleep" | "InternalPreventSleep" => Keeps::System,
        _ => return None,
    };
    let name = fields.next()?.trim().strip_prefix("named: ")?.trim();
    let name = name.strip_prefix('"').and_then(|n| n.strip_suffix('"')).unwrap_or(name);
    Some(Blocker {
        pid: pid.parse().ok()?,
        process: process.to_string(),
        for_pid: None,
        app: process.to_string(),
        keeps,
        reason: audio_reason(name).unwrap_or_else(|| name.to_string()),
        held_secs: clock_secs(held),
        ends_in_secs: None,
    })
}

/// Core Audio names its assertions after the device, as in
/// `com.apple.audio.BuiltInSpeakerDevice.context.preventuseridlesleep`.
fn audio_reason(name: &str) -> Option<String> {
    let device = name.strip_prefix("com.apple.audio.")?.split('.').next().filter(|d| !d.is_empty())?;
    Some(format!("Audio in use ({device})"))
}

fn apply_detail(b: &mut Blocker, detail: &str) {
    if let Some(pid) = detail.strip_prefix("Created for PID: ") {
        b.for_pid = pid.trim_end_matches(['.', ' ']).parse().ok();
    } else if let Some(rest) = detail.strip_prefix("Timeout will fire in ") {
        b.ends_in_secs = rest.split_whitespace().next().and_then(|s| s.parse().ok());
    }
}

/// `hh:mm:ss`, where the hours can pass 24.
fn clock_secs(s: &str) -> Option<u64> {
    let mut parts = s.split(':');
    let (h, m, sec) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let n = |p: &str| p.parse::<u64>().ok().filter(|_| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    Some(n(h)? * 3600 + n(m)? * 60 + n(sec)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_output() {
        let b = parse(include_str!("../tests/fixtures/pmset_assertions.txt"));
        let summary: Vec<_> = b.iter().map(|b| (b.pid, b.process.as_str(), b.keeps, b.reason.as_str(), b.held_secs)).collect();
        assert_eq!(
            summary,
            [
                (30933, "caffeinate", Keeps::System, "caffeinate command-line tool", Some(188)),
                (730, "Google Chrome", Keeps::System, "Playing audio", Some(44 * 60 + 40)),
                (72382, "zoom.us", Keeps::Display, "Disabling display sleep", Some(2 * 3600 + 16 * 60 + 48)),
                (172, "coreaudiod", Keeps::System, "Audio in use (BuiltInSpeakerDevice)", Some(165)),
                (172, "coreaudiod", Keeps::System, "Audio in use (BuiltInSpeakerDevice)", Some(3600 + 52 * 60 + 17)),
                (519, "sharingd", Keeps::System, "Handoff", Some(15)),
            ]
        );
        assert_eq!(b[0].ends_in_secs, Some(111));
        assert_eq!((b[3].for_pid, b[4].for_pid), (Some(774), Some(72382)));
        // WindowServer's UserIsActive is not a sleep blocker, so its timeout is not misapplied.
        assert_eq!(b[5].ends_in_secs, None);
    }

    #[test]
    fn names_with_parentheses_and_unknown_types() {
        let out = "Listed by owning process:\n   pid 9(Helper (Renderer)): [0x1] 100:00:01 PreventUserIdleDisplaySleep named: \"Video\"  \n   pid 8(x): [0x2] 00:00:01 BackgroundTask named: \"y\"  \nNo kernel assertions.\n   pid 7(after): [0x3] 00:00:01 PreventSystemSleep named: \"z\"\n";
        let b = parse(out);
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].process.as_str(), b[0].keeps, b[0].held_secs), ("Helper (Renderer)", Keeps::Display, Some(360_001)));
        let internal = |kind: &str| assertion(&format!("pid 3(sharingd): [0x4] 01:00:00 {kind} named: \"x\"")).map(|b| b.keeps);
        assert_eq!(internal("InternalPreventDisplaySleep"), Some(Keeps::Display));
        assert_eq!(internal("InternalPreventSleep"), Some(Keeps::System));
    }

    #[test]
    fn malformed_lines_are_skipped() {
        for bad in ["pid x(a): [0x1] 00:00:01 PreventSystemSleep named: \"z\"", "pid 1(a) [0x1] 00:00:01 PreventSystemSleep", "pid 1(a): 00:00:01", ""] {
            assert_eq!(assertion(bad), None, "{bad}");
        }
        assert_eq!(clock_secs("1:2"), None);
        assert_eq!(clock_secs("01:02:-3"), None);
        assert!(parse("garbage\n\tCreated for PID: 5.\n").is_empty());
    }

    #[test]
    fn live_read_matches_pmset() {
        // Retried, since assertions come and go between the two pmset calls.
        let consistent = || {
            let raw = run(PMSET, &["-g", "assertions"], TIMEOUT).unwrap();
            let blockers = read().unwrap();
            blockers.iter().all(|b| raw.contains(&format!("pid {}({})", b.pid, b.process)) && !b.app.is_empty())
        };
        assert!((0..3).any(|_| consistent()));
    }
}
