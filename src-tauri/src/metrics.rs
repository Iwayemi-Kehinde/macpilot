//! CPU, memory, disk, network and process metrics via `sysinfo`, which reads
//! the native Mach and BSD interfaces on macOS.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sysinfo::{DiskRefreshKind, Disks, Networks, ProcessRefreshKind, ProcessesToUpdate, System, MINIMUM_CPU_UPDATE_INTERVAL};

const TOP_PROCESSES: usize = 12;

/// Rates averaged over a longer window than this describe the past, not now
/// (the window was hidden or the Mac slept), so they are withheld for a sample.
const MAX_RATE_WINDOW: Duration = Duration::from_secs(15);
/// Free space changes slowly, and reading it costs more than the rest of a
/// snapshot together, so it is re-read this often or when a volume comes or goes.
const DISK_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub hostname: Option<String>,
    pub os_version: Option<String>,
    pub kernel_version: Option<String>,
    pub cpu_brand: Option<String>,
    pub physical_cores: Option<usize>,
    pub logical_cores: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuMetrics {
    /// A rate between two refreshes at least 0.2 s apart. Snapshots always
    /// measure one; `None` stays in the format so readers handle a missing rate.
    pub usage_percent: Option<f32>,
    pub per_core_percent: Vec<f32>,
    pub load_average: [f64; 3],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    /// App memory + wired + compressed, the same definition Activity Monitor
    /// uses for "Memory Used". The rest is file cache and free pages.
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskMetrics {
    pub name: String,
    pub mount_point: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub removable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkMetrics {
    pub interface: String,
    /// See `CpuMetrics::usage_percent`.
    pub rx_bytes_per_sec: Option<f64>,
    pub tx_bytes_per_sec: Option<f64>,
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMetrics {
    pub pid: u32,
    pub name: String,
    /// Percent of one core, so values above 100 are possible.
    pub cpu_percent: Option<f32>,
    /// Resident memory. Activity Monitor's Memory column shows the footprint,
    /// which is a different measure.
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSummary {
    pub total: usize,
    /// Processes owned by other users (root, system daemons). macOS does not let
    /// an unprivileged app read their CPU or memory, so they are left out of the
    /// rankings instead of being shown as idle.
    pub unreadable: usize,
    /// Empty until CPU rates are available.
    pub top_by_cpu: Vec<ProcessMetrics>,
    pub top_by_memory: Vec<ProcessMetrics>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    pub timestamp_ms: u64,
    pub uptime_secs: u64,
    pub host: HostInfo,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub disks: Vec<DiskMetrics>,
    pub network: Vec<NetworkMetrics>,
    pub processes: ProcessSummary,
}

pub struct Monitor {
    sys: System,
    disks: Disks,
    networks: Networks,
    host: HostInfo,
    cpu_stamp: Option<Stamp>,
    net_stamp: Option<Stamp>,
    disks_read: Instant,
}

#[derive(Clone, Copy)]
struct Stamp {
    mono: Instant,
    wall: SystemTime,
}

impl Stamp {
    fn now() -> Self {
        Self { mono: Instant::now(), wall: SystemTime::now() }
    }

    /// The window since `self`, if it is fit for a rate. The monotonic clock
    /// pauses while the Mac sleeps, so wall time is checked too.
    fn window(self) -> Option<Duration> {
        rate_window(self.mono.elapsed(), self.wall.elapsed().ok())
    }
}

fn rate_window(mono: Duration, wall: Option<Duration>) -> Option<Duration> {
    let wall = wall?;
    let fits = |d: Duration| (MINIMUM_CPU_UPDATE_INTERVAL..=MAX_RATE_WINDOW).contains(&d);
    (fits(mono) && fits(wall)).then_some(mono)
}

impl Monitor {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_all();
        let host = HostInfo {
            hostname: System::host_name(),
            os_version: System::long_os_version(),
            kernel_version: System::kernel_version(),
            cpu_brand: sys.cpus().first().map(|c| c.brand().trim().to_string()).filter(|b| !b.is_empty()),
            physical_cores: System::physical_core_count(),
            logical_cores: sys.cpus().len(),
        };
        Self {
            sys,
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            host,
            cpu_stamp: None,
            net_stamp: None,
            disks_read: Instant::now(),
        }
    }

    fn refresh_disks(&mut self) {
        let mounts = |d: &Disks| d.list().iter().map(|d| d.mount_point().to_path_buf()).collect::<Vec<_>>();
        let before = mounts(&self.disks);
        // Listing volumes is cheap; their free space is not.
        self.disks.refresh_specifics(true, DiskRefreshKind::nothing());
        if mounts(&self.disks) != before || self.disks_read.elapsed() >= DISK_REFRESH_INTERVAL {
            self.disks.refresh_specifics(true, DiskRefreshKind::nothing().with_storage());
            self.disks_read = Instant::now();
        }
    }

    fn refresh_rates(&mut self) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
    }

    /// Blocks for up to `MINIMUM_CPU_UPDATE_INTERVAL` when it needs a fresh
    /// rate window, so call it off the async runtime.
    pub fn snapshot(&mut self) -> SystemSnapshot {
        match self.cpu_stamp.map(|s| (s.mono.elapsed(), s.window())) {
            // Called again within the minimum interval: wait out the rest.
            Some((elapsed, _)) if elapsed < MINIMUM_CPU_UPDATE_INTERVAL => {
                std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL - elapsed)
            }
            Some((_, Some(_))) => {}
            // First call, or the last one was too long ago (window hidden, Mac
            // asleep) to describe now: take a fresh short sample instead.
            _ => {
                self.refresh_rates();
                self.networks.refresh(true);
                self.net_stamp = Some(Stamp::now());
                std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
            }
        }
        self.cpu_stamp = Some(Stamp::now());
        self.refresh_rates();
        self.sys.refresh_memory();
        self.refresh_disks();

        // Timed on its own so slow process refreshes do not skew throughput.
        let net_secs = self.net_stamp.and_then(Stamp::window).map(|d| d.as_secs_f64());
        self.net_stamp = Some(Stamp::now());
        self.networks.refresh(true);

        let load = System::load_average();
        let cpu = CpuMetrics {
            usage_percent: Some(self.sys.global_cpu_usage()),
            per_core_percent: self.sys.cpus().iter().map(|c| c.cpu_usage()).collect(),
            load_average: [load.one, load.five, load.fifteen],
        };

        let memory = MemoryMetrics {
            total_bytes: self.sys.total_memory(),
            used_bytes: self.sys.used_memory(),
            free_bytes: self.sys.free_memory(),
            swap_total_bytes: self.sys.total_swap(),
            swap_used_bytes: self.sys.used_swap(),
        };

        // APFS volumes in one container (`/` and `/System/Volumes/Data`) share a
        // name and capacity; keep the first so space is not double counted. Free
        // space is not part of the key because the two are read moments apart
        // and differ by a few bytes whenever something writes to disk.
        let mut seen = HashSet::new();
        let disks = self
            .disks
            .list()
            .iter()
            .filter(|d| seen.insert((d.name().to_os_string(), d.total_space())))
            .map(|d| DiskMetrics {
                name: d.name().to_string_lossy().into_owned(),
                mount_point: d.mount_point().to_string_lossy().into_owned(),
                file_system: d.file_system().to_string_lossy().into_owned(),
                total_bytes: d.total_space(),
                available_bytes: d.available_space(),
                removable: d.is_removable(),
            })
            .collect();

        let mut network: Vec<NetworkMetrics> = self
            .networks
            .list()
            .iter()
            .filter(|(_, n)| n.total_received() + n.total_transmitted() > 0)
            .map(|(name, n)| NetworkMetrics {
                interface: name.clone(),
                rx_bytes_per_sec: net_secs.map(|s| n.received() as f64 / s),
                tx_bytes_per_sec: net_secs.map(|s| n.transmitted() as f64 / s),
                total_rx_bytes: n.total_received(),
                total_tx_bytes: n.total_transmitted(),
            })
            .collect();
        network.sort_by(|a, b| {
            (b.total_rx_bytes + b.total_tx_bytes)
                .cmp(&(a.total_rx_bytes + a.total_tx_bytes))
                .then_with(|| a.interface.cmp(&b.interface))
        });

        let total = self.sys.processes().len();
        // Without privileges, other users' processes come back with no memory
        // (and no start time). Anything without resident memory, those plus
        // zombies, has nothing to rank.
        let readable: Vec<ProcessMetrics> = self
            .sys
            .processes()
            .values()
            .filter(|p| p.memory() != 0)
            .map(|p| ProcessMetrics {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_percent: Some(p.cpu_usage()),
                memory_bytes: p.memory(),
            })
            .collect();
        let processes = ProcessSummary {
            total,
            unreadable: total - readable.len(),
            top_by_cpu: top_n(&readable, |a, b| {
                let (a_cpu, b_cpu) = (a.cpu_percent.unwrap_or(0.0), b.cpu_percent.unwrap_or(0.0));
                b_cpu.total_cmp(&a_cpu).then(a.pid.cmp(&b.pid))
            }),
            top_by_memory: top_n(&readable, |a, b| b.memory_bytes.cmp(&a.memory_bytes).then(a.pid.cmp(&b.pid))),
        };

        SystemSnapshot {
            timestamp_ms: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
            uptime_secs: System::uptime(),
            host: self.host.clone(),
            cpu,
            memory,
            disks,
            network,
            processes,
        }
    }
}

fn top_n(all: &[ProcessMetrics], cmp: impl Fn(&ProcessMetrics, &ProcessMetrics) -> Ordering) -> Vec<ProcessMetrics> {
    let mut sorted = all.to_vec();
    sorted.sort_by(cmp);
    sorted.truncate(TOP_PROCESSES);
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the live system: the first call measures a short window of its own,
    /// and a call within the minimum interval waits for one instead of skipping rates.
    #[test]
    fn live_snapshot_always_has_rates() {
        let mut m = Monitor::new();
        let first = m.snapshot();
        assert!(first.cpu.usage_percent.is_some());
        assert!(first.network.iter().all(|n| n.rx_bytes_per_sec.is_some()));
        assert!(!first.processes.top_by_cpu.is_empty());
        assert!(first.memory.total_bytes > 0);
        assert!(first.memory.used_bytes <= first.memory.total_bytes);
        let mounts: Vec<_> = first.disks.iter().map(|d| d.mount_point.as_str()).collect();
        assert!(!(mounts.contains(&"/") && mounts.contains(&"/System/Volumes/Data")), "{mounts:?}");
        assert!(first.processes.total > 0);
        assert!(first.host.logical_cores > 0);

        let started = Instant::now();
        let second = m.snapshot();
        assert!(started.elapsed() >= MINIMUM_CPU_UPDATE_INTERVAL / 2, "waited {:?}", started.elapsed());
        let usage = second.cpu.usage_percent.expect("cpu usage after the minimum interval");
        assert!((0.0..=100.0).contains(&usage));
        assert_eq!(second.cpu.per_core_percent.len(), second.host.logical_cores);
        assert!(second.network.iter().all(|n| n.rx_bytes_per_sec.is_some()));

        let procs = &second.processes;
        assert!(!procs.top_by_cpu.is_empty() && procs.top_by_cpu.len() <= TOP_PROCESSES);
        assert!(procs.unreadable < procs.total);
        // Unreadable processes must not be ranked as idle ones.
        assert!(procs.top_by_memory.iter().all(|p| p.memory_bytes > 0));
        let cpus: Vec<f32> = procs.top_by_cpu.iter().map(|p| p.cpu_percent.unwrap()).collect();
        assert!(cpus.windows(2).all(|w| w[0] >= w[1]), "{cpus:?}");
    }

    #[test]
    fn rate_window_rejects_too_short_too_long_and_slept() {
        let secs = Duration::from_secs;
        assert_eq!(rate_window(secs(2), Some(secs(2))), Some(secs(2)));
        assert_eq!(rate_window(Duration::from_millis(50), Some(Duration::from_millis(50))), None);
        // Window hidden for a minute: the average no longer describes now.
        assert_eq!(rate_window(secs(60), Some(secs(60))), None);
        // Slept for an hour: the monotonic clock paused, wall time did not.
        assert_eq!(rate_window(secs(3), Some(secs(3600))), None);
        // Wall clock moved backwards.
        assert_eq!(rate_window(secs(2), None), None);
    }

    #[test]
    #[ignore = "prints a live snapshot for manual inspection"]
    fn print_live_snapshot() {
        let mut m = Monitor::new();
        m.snapshot();
        std::thread::sleep(std::time::Duration::from_secs(1));
        println!("{}", serde_json::to_string_pretty(&m.snapshot()).unwrap());
    }
}
