//! Every running process, with CPU rates from the change in accumulated CPU
//! time between two reads.
//!
//! `ps` may read the CPU time of every user's processes, so it supplies the
//! list and each process's owner. This user's own processes are then re-read
//! from the kernel, which gives an exact CPU time and the footprint. sysinfo's
//! per-process rate is not used: it keeps a process's last busy value once
//! the process goes idle.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::macos::{self, Kernel};
use crate::tool::run_timed;

const PS: &str = "/bin/ps";
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub pid: u32,
    pub uid: u32,
    pub name: String,
    /// Accumulated CPU time, read at `at`.
    pub cpu_secs: f64,
    pub at: Instant,
    /// Physical footprint; `None` for processes of macOS and other users,
    /// whose memory macOS shows only to administrator tools.
    pub footprint_bytes: Option<u64>,
}

/// Lists every process but zombies, as `top` does.
pub fn read(kernel: &Kernel) -> Result<Vec<Sample>, String> {
    let started = Instant::now();
    // Names come from the kernel: asking ps for them doubles its cost.
    let (output, ended) = run_timed(PS, &["-axo", "pid=,uid=,time="], TIMEOUT)?;
    // ps reads the processes one by one while it runs.
    let ps_at = started + (ended - started) / 2;
    let me = unsafe { libc::getuid() };
    Ok(parse(&output)
        .filter_map(|(pid, uid, cpu_secs)| {
            // No name: a zombie, or a process that ended after ps listed it.
            let name = macos::process_name(pid)?;
            let mut sample = Sample { pid, uid, name, cpu_secs, at: ps_at, footprint_bytes: None };
            if uid == me {
                // One of this user's processes without a reading is ending.
                let usage = kernel.usage(pid)?;
                sample.footprint_bytes = Some(usage.footprint_bytes);
                if let Some(cpu_secs) = usage.cpu_secs {
                    (sample.cpu_secs, sample.at) = (cpu_secs, Instant::now());
                }
            }
            Some(sample)
        })
        .collect())
}

/// Parses `ps -axo pid=,uid=,time=` into pid, owner and CPU seconds. Lines
/// that do not fit are skipped.
pub fn parse(output: &str) -> impl Iterator<Item = (u32, u32, f64)> + '_ {
    output.lines().filter_map(|line| {
        let mut fields = line.split_whitespace();
        let pid = fields.next()?.parse().ok()?;
        let uid = fields.next()?.parse().ok()?;
        Some((pid, uid, parse_cpu_time(fields.next()?)?))
    })
}

/// `[[dd-]hh:]mm:ss.ss`, as macOS ps prints accumulated CPU time.
pub fn parse_cpu_time(s: &str) -> Option<f64> {
    let (days, clock) = match s.split_once('-') {
        Some((days, clock)) => (number(days)?, clock),
        None => (0.0, s),
    };
    let mut secs = 0.0;
    for part in clock.split(':') {
        secs = secs * 60.0 + number(part)?;
    }
    Some(days * 86_400.0 + secs)
}

fn number(s: &str) -> Option<f64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    s.parse().ok().filter(|v: &f64| v.is_finite())
}

struct Previous {
    uid: u32,
    name: String,
    cpu_secs: f64,
    at: Instant,
}

pub struct CpuRates {
    last: HashMap<u32, Previous>,
    max_window: Duration,
}

impl CpuRates {
    /// Rates averaged over more than `max_window` describe the past, not now.
    pub fn new(max_window: Duration) -> Self {
        Self { last: HashMap::new(), max_window }
    }

    /// Forgets the previous readings, so the next update measures nothing:
    /// for a gap the rates must not span.
    pub fn reset(&mut self) {
        self.last.clear();
    }

    /// Each sample's CPU use since the previous update, as a percent of one
    /// core. `None` for a process not seen then (new, or its pid reused), or
    /// one whose previous reading is older than `max_window`.
    pub fn update(&mut self, samples: &[Sample]) -> Vec<Option<f32>> {
        let rates = samples
            .iter()
            .map(|s| {
                let prev = self.last.get(&s.pid)?;
                let window = s.at.checked_duration_since(prev.at).filter(|w| !w.is_zero() && *w <= self.max_window)?;
                // A pid with a new owner or name is a new process. A new owner
                // also means its CPU time now comes from the other source.
                (prev.uid == s.uid && prev.name == s.name && prev.cpu_secs <= s.cpu_secs)
                    .then(|| ((s.cpu_secs - prev.cpu_secs) / window.as_secs_f64() * 100.0) as f32)
            })
            .collect();
        self.last = samples
            .iter()
            .map(|s| (s.pid, Previous { uid: s.uid, name: s.name.clone(), cpu_secs: s.cpu_secs, at: s.at }))
            .collect();
        rates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::run;

    #[test]
    fn parses_cpu_time_formats() {
        assert_eq!(parse_cpu_time("0:00.94"), Some(0.94));
        assert_eq!(parse_cpu_time("607:35.46"), Some(607.0 * 60.0 + 35.46));
        assert_eq!(parse_cpu_time("1:02:03.50"), Some(3723.5));
        assert_eq!(parse_cpu_time("2-01:00:00.00"), Some(2.0 * 86_400.0 + 3600.0));
        for bad in ["abc", "", "inf-00:00.00", "NaN:00.00", "1:-5.00", "+1:00.00", "1e3:00.00"] {
            assert_eq!(parse_cpu_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn parses_padded_rows_and_skips_others() {
        let rows: Vec<_> = parse("  160     0 607:35.46\n43418   501   7:27.07\ngarbage line\n  172 0 1-02:03:04.50\n").collect();
        assert_eq!(rows, [(160, 0, 607.0 * 60.0 + 35.46), (43418, 501, 447.07), (172, 0, 93_784.5)]);
    }

    fn sample(pid: u32, uid: u32, name: &str, cpu_secs: f64, at: Instant) -> Sample {
        Sample { pid, uid, name: name.into(), cpu_secs, at, footprint_bytes: None }
    }

    #[test]
    fn a_process_that_goes_idle_drops_to_zero() {
        let mut rates = CpuRates::new(Duration::from_secs(15));
        let (t0, ms) = (Instant::now(), Duration::from_millis);
        assert_eq!(rates.update(&[sample(7, 501, "worker", 1.0, t0)]), [None]);
        assert_eq!(rates.update(&[sample(7, 501, "worker", 1.05, t0 + ms(100))]), [Some(50.0)]);
        assert_eq!(rates.update(&[sample(7, 501, "worker", 1.05, t0 + ms(200))]), [Some(0.0)]);
        rates.reset();
        assert_eq!(rates.update(&[sample(7, 501, "worker", 1.05, t0 + ms(300))]), [None]);
    }

    #[test]
    fn no_rate_for_a_changed_process_or_an_unfit_window() {
        let mut rates = CpuRates::new(Duration::from_secs(15));
        let (t0, secs) = (Instant::now(), Duration::from_secs);
        rates.update(&[sample(7, 0, "worker", 5.0, t0), sample(8, 0, "daemon", 5.0, t0), sample(9, 0, "daemon", 1.0, t0)]);
        let next = [
            // Reused pid.
            sample(7, 0, "other", 5.5, t0 + secs(1)),
            // Same program, now running as this user: timed by the kernel instead of ps.
            sample(8, 501, "daemon", 5.5, t0 + secs(1)),
            // Not seen before.
            sample(10, 0, "new", 0.1, t0 + secs(1)),
            // Previous reading too old to describe now.
            sample(9, 0, "daemon", 1.2, t0 + secs(20)),
        ];
        assert_eq!(rates.update(&next), [None, None, None, None]);
        // Same instant as the previous reading.
        assert_eq!(rates.update(&[sample(9, 0, "daemon", 1.3, t0 + secs(20))]), [None]);
    }

    #[test]
    fn live_read_covers_every_user() {
        let kernel = Kernel::new();
        let me = unsafe { libc::getuid() };
        let mut rates = CpuRates::new(Duration::from_secs(15));
        let first = read(&kernel).unwrap();
        assert!(rates.update(&first).iter().all(Option::is_none));
        // launchd belongs to root: listed by ps, named by the kernel, its memory hidden.
        assert!(first.iter().any(|s| s.pid == 1 && s.uid == 0 && s.name == "launchd" && s.footprint_bytes.is_none()));
        assert!(first.iter().any(|s| s.pid == std::process::id() && s.uid == me && s.footprint_bytes.is_some()));
        // Memory is readable for exactly this user's processes.
        assert!(first.iter().all(|s| (s.uid == me) == s.footprint_bytes.is_some()));
        std::thread::sleep(Duration::from_millis(300));
        let second = read(&kernel).unwrap();
        let rates = rates.update(&second);
        assert!(rates.iter().flatten().all(|r| *r >= 0.0));
        let rate_of = |pid| second.iter().zip(&rates).find(|(s, _)| s.pid == pid).and_then(|(_, r)| *r);
        // Timed by the kernel and by ps respectively.
        assert!(rate_of(std::process::id()).is_some() && rate_of(1).is_some());
    }

    /// Catches a wrong tick length: rusage times are not nanoseconds on Apple Silicon.
    #[test]
    fn kernel_cpu_time_agrees_with_ps() {
        let busy = Instant::now();
        while busy.elapsed() < Duration::from_millis(100) {
            std::hint::black_box(0u64);
        }
        let (pid, kernel) = (std::process::id(), Kernel::new());
        let cpu = || kernel.usage(pid).and_then(|u| u.cpu_secs).unwrap();
        let before = cpu();
        let ps = parse_cpu_time(run(PS, &["-o", "time=", "-p", &pid.to_string()], TIMEOUT).unwrap().trim()).unwrap();
        let after = cpu();
        // ps reads between the two and rounds to the nearest 10 ms.
        assert!((before - 0.0051..=after + 0.0051).contains(&ps), "{before} <= {ps} <= {after}, give or take 5 ms");
    }
}
