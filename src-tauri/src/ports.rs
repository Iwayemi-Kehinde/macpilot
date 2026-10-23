//! TCP ports this user's processes are listening on, from `lsof`, with the
//! folder each server runs in. macOS lets `lsof` see only this user's
//! processes without administrator rights.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;

use crate::apps;
use crate::macos;
use crate::tool::run;

const LSOF: &str = "/usr/sbin/lsof";
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listener {
    pub port: u16,
    pub pid: u32,
    pub process: String,
    pub app: String,
    /// Addresses it accepts connections on, such as `*` or `127.0.0.1`.
    pub addresses: Vec<String>,
    /// Reachable from other devices: bound to every interface or to a non-loopback address.
    pub network: bool,
    /// The working folder, for servers started from a project, with the home folder as `~`.
    pub folder: Option<String>,
    /// The executable, to tell macOS's own services from others.
    pub path: Option<String>,
}

pub fn read() -> Result<Vec<Listener>, String> {
    // -n and -P keep addresses and ports numeric, so nothing is looked up.
    let output = matched(run(LSOF, &["-nP", "-iTCP", "-sTCP:LISTEN", "-FpcLn"], TIMEOUT))?;
    let mut listeners = parse(&output);
    let home = std::env::var("HOME").ok();
    for l in &mut listeners {
        if let Some((app, path)) = apps::identify(l.pid) {
            l.app = app;
            l.path = path;
        }
        l.folder = macos::working_dir(l.pid).filter(|dir| dir != "/").map(|dir| tilde(&dir, home.as_deref()));
    }
    Ok(listeners)
}

/// Parses `lsof -F pcLn` output: a `p` line starts each process, followed by
/// its command, login and one `f`/`n` pair per socket. One entry per process
/// and port, with IPv4 and IPv6 sockets merged.
pub fn parse(output: &str) -> Vec<Listener> {
    let mut found: BTreeMap<(u16, u32), Listener> = BTreeMap::new();
    let (mut pid, mut process) = (None, String::new());
    for line in output.lines() {
        let mut chars = line.chars();
        let Some(tag) = chars.next() else { continue };
        let value = chars.as_str();
        match tag {
            'p' => (pid, process) = (value.parse::<u32>().ok(), String::new()),
            'c' => process = value.to_string(),
            'n' => {
                let (Some(pid), Some((address, port))) = (pid, value.rsplit_once(':')) else { continue };
                let Ok(port) = port.parse::<u16>() else { continue };
                let address = address.trim_start_matches('[').trim_end_matches(']').to_string();
                let entry = found.entry((port, pid)).or_insert_with(|| Listener {
                    port,
                    pid,
                    process: process.clone(),
                    app: process.clone(),
                    addresses: Vec::new(),
                    network: false,
                    folder: None,
                    path: None,
                });
                entry.network |= !is_loopback(&address);
                if !entry.addresses.contains(&address) {
                    entry.addresses.push(address);
                }
            }
            _ => {}
        }
    }
    found.into_values().collect()
}

/// lsof exits with 1 both when no socket matches and on errors; only errors say why.
fn matched(result: Result<String, String>) -> Result<String, String> {
    match result {
        Err(e) if e == format!("{LSOF} exited with exit status: 1") => Ok(String::new()),
        other => other,
    }
}

fn tilde(path: &str, home: Option<&str>) -> String {
    match home.filter(|h| !h.is_empty() && *h != "/").and_then(|h| path.strip_prefix(h)) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

fn is_loopback(address: &str) -> bool {
    address == "localhost" || address == "::1" || address.starts_with("127.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured on macOS 26.6, with one app renamed.
    const OUTPUT: &str = "p445\ncControlCenter\nLme\nf8\nn*:7000\nf11\nn*:7000\nf12\nn*:5000\nf13\nn*:5000\np458\ncrapportd\nLme\nf10\nn*:56223\np1922\nclimactl\nLme\nf13\nn127.0.0.1:49272\np43117\ncnode\nLme\nf59\nn127.0.0.1:5173\nf60\nn[::1]:5173\n";

    #[test]
    fn parses_lsof_fields() {
        let ports = parse(OUTPUT);
        let summary: Vec<_> = ports.iter().map(|l| (l.port, l.process.as_str(), l.network, l.addresses.join(" "))).collect();
        assert_eq!(
            summary,
            [
                (5000, "ControlCenter", true, "*".to_string()),
                (5173, "node", false, "127.0.0.1 ::1".to_string()),
                (7000, "ControlCenter", true, "*".to_string()),
                (49272, "limactl", false, "127.0.0.1".to_string()),
                (56223, "rapportd", true, "*".to_string()),
            ]
        );
    }

    #[test]
    fn specific_addresses_and_junk() {
        let ports = parse("p1\ncsrv\nn192.168.1.20:8080\nn[fe80::1%en0]:9\nnbad\nn*:99999\nxjunk\np\nn*:1\n");
        assert_eq!(ports.len(), 2);
        assert!(ports.iter().all(|l| l.network));
        assert!(parse("").is_empty());
    }

    #[test]
    fn folders_under_home_use_a_tilde() {
        let home = Some("/Users/me");
        assert_eq!(tilde("/Users/me/Documents/app", home), "~/Documents/app");
        assert_eq!(tilde("/Users/me", home), "~");
        assert_eq!(tilde("/Users/meg/app", home), "/Users/meg/app");
        assert_eq!(tilde("/opt/app", None), "/opt/app");
    }

    #[test]
    fn no_match_is_empty_but_an_error_is_kept() {
        // Nothing serves port 1 on macOS, and BOGUS is not a TCP state.
        assert_eq!(matched(run(LSOF, &["-nP", "-iTCP:1", "-sTCP:LISTEN", "-FpcLn"], TIMEOUT)), Ok(String::new()));
        let err = matched(run(LSOF, &["-nP", "-iTCP", "-sTCP:BOGUS", "-FpcLn"], TIMEOUT)).unwrap_err();
        assert!(err.contains("unknown TCP state name"), "{err}");
    }

    #[test]
    fn live_read_matches_lsof() {
        // Retried, since servers can start or stop between the two lsof calls.
        let consistent = || {
            let raw = run(LSOF, &["-nP", "-iTCP", "-sTCP:LISTEN"], TIMEOUT).unwrap_or_default();
            let ports = read().unwrap();
            let listed = |l: &Listener| {
                raw.contains(&format!(":{} (LISTEN)", l.port))
                    && raw.lines().any(|line| line.split_whitespace().nth(1) == Some(&l.pid.to_string()))
            };
            ports.iter().all(listed) && ports.is_empty() == raw.is_empty()
        };
        assert!((0..3).any(|_| consistent()));
    }
}
