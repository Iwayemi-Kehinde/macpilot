//! CPU, memory, disk, network and process metrics. `sysinfo` reads the native
//! Mach and BSD interfaces; `macos` and `processes` cover what it does not.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use crate::macos::{Kernel, MemoryBreakdown};
use crate::processes::{self, CpuRates, Sample};
use sysinfo::{DiskRefreshKind, Disks, Networks, System, MINIMUM_CPU_UPDATE_INTERVAL};

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
    /// Activity Monitor's "Memory Used" (see `macos::Memory`), or sysinfo's
    /// figure if the kernel did not answer, in which case `breakdown` is `None`.
    pub used_bytes: u64,
    pub breakdown: Option<MemoryBreakdown>,
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
    /// Percent of one core, so values above 100 are possible. `None` for a
    /// process that started since the previous reading.
    pub cpu_percent: Option<f32>,
    /// Physical footprint, as in Activity Monitor's Memory column. `None` for
    /// system processes: macOS shows their memory only to administrator tools.
    pub memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSummary {
    /// Processes `ps` lists; kernel_task is hidden from apps.
    pub total: usize,
    /// Processes of macOS and other users, whose memory macOS shows only to
    /// administrator tools. They are ranked by CPU only.
    pub system: usize,
    /// Why there is no process list, if `ps` failed.
    pub error: Option<String>,
    pub top_by_cpu: Vec<ProcessMetrics>,
    /// This user's processes only, since the others have no readable memory.
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
    kernel: Kernel,
    cpu_rates: CpuRates,
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
            kernel: Kernel::new(),
            cpu_rates: CpuRates::new(MAX_RATE_WINDOW),
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
                self.sys.refresh_cpu_usage();
                // Process rates must not span the gap either: forget the old
                // readings, then take a baseline. If ps fails here, the read
                // that follows reports it.
                self.cpu_rates.reset();
                if let Ok(samples) = processes::read(&self.kernel) {
                    self.cpu_rates.update(&samples);
                }
                self.networks.refresh(true);
                self.net_stamp = Some(Stamp::now());
                std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
            }
        }
        self.cpu_stamp = Some(Stamp::now());
        self.sys.refresh_cpu_usage();
        // Straight after the CPU refresh, so process rates share its window.
        let processes = match processes::read(&self.kernel) {
            Ok(samples) => {
                let rates = self.cpu_rates.update(&samples);
                summarize(samples, rates)
            }
            Err(error) => ProcessSummary { error: Some(error), ..ProcessSummary::default() },
        };
        self.sys.refresh_memory();
        self.refresh_disks();

        // Timed on its own so a slow disk refresh does not skew throughput.
        let net_secs = self.net_stamp.and_then(Stamp::window).map(|d| d.as_secs_f64());
        self.net_stamp = Some(Stamp::now());
        self.networks.refresh(true);

        let load = System::load_average();
        let cpu = CpuMetrics {
            usage_percent: Some(self.sys.global_cpu_usage()),
            per_core_percent: self.sys.cpus().iter().map(|c| c.cpu_usage()).collect(),
            load_average: [load.one, load.five, load.fifteen],
        };

        let kernel_memory = self.kernel.memory();
        let memory = MemoryMetrics {
            total_bytes: self.sys.total_memory(),
            used_bytes: kernel_memory.map_or_else(|| self.sys.used_memory(), |m| m.used_bytes),
            breakdown: kernel_memory.map(|m| m.breakdown),
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

fn summarize(samples: Vec<Sample>, rates: Vec<Option<f32>>) -> ProcessSummary {
    let all: Vec<ProcessMetrics> = samples
        .into_iter()
        .zip(rates)
        .map(|(s, cpu_percent)| ProcessMetrics { pid: s.pid, name: s.name, cpu_percent, memory_bytes: s.footprint_bytes })
        .collect();
    ProcessSummary {
        total: all.len(),
        system: all.iter().filter(|p| p.memory_bytes.is_none()).count(),
        error: None,
        // Only measured processes are ranked; one without a rate is not idle.
        top_by_cpu: top_n(all.iter().filter(|p| p.cpu_percent.is_some()), |a, b| {
            b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap_or(Ordering::Equal).then(a.pid.cmp(&b.pid))
        }),
        top_by_memory: top_n(all.iter().filter(|p| p.memory_bytes.is_some()), |a, b| {
            b.memory_bytes.cmp(&a.memory_bytes).then(a.pid.cmp(&b.pid))
        }),
    }
}

fn top_n<'a>(
    candidates: impl Iterator<Item = &'a ProcessMetrics>,
    cmp: impl Fn(&ProcessMetrics, &ProcessMetrics) -> Ordering,
) -> Vec<ProcessMetrics> {
    let mut sorted: Vec<_> = candidates.collect();
    sorted.sort_by(|a, b| cmp(a, b));
    sorted.into_iter().take(TOP_PROCESSES).cloned().collect()
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
        let b = second.memory.breakdown.expect("kernel memory statistics");
        assert!(b.app_bytes + b.wired_bytes + b.compressed_bytes <= second.memory.used_bytes);
        assert!((0.0..=100.0).contains(&usage));
        assert_eq!(second.cpu.per_core_percent.len(), second.host.logical_cores);
        assert!(second.network.iter().all(|n| n.rx_bytes_per_sec.is_some()));

        let procs = &second.processes;
        assert!(!procs.top_by_cpu.is_empty() && procs.top_by_cpu.len() <= TOP_PROCESSES);
        assert!(procs.system > 0 && procs.system < procs.total);
        assert_eq!(procs.error, None);
        assert!(procs.top_by_memory.iter().all(|p| p.memory_bytes.is_some_and(|m| m > 0)));
        let footprints: Vec<u64> = procs.top_by_memory.iter().filter_map(|p| p.memory_bytes).collect();
        assert!(footprints.windows(2).all(|w| w[0] >= w[1]), "{footprints:?}");
        let cpus: Vec<f32> = procs.top_by_cpu.iter().map(|p| p.cpu_percent.unwrap()).collect();
        assert!(cpus.windows(2).all(|w| w[0] >= w[1]), "{cpus:?}");
    }

    #[test]
    fn summary_ranks_measured_processes_and_counts_system_ones() {
        let at = Instant::now();
        let sample = |pid, footprint_bytes| Sample { pid, uid: 0, name: format!("p{pid}"), cpu_secs: 0.0, at, footprint_bytes };
        let samples = vec![sample(1, None), sample(2, Some(10)), sample(3, Some(30)), sample(4, None)];
        let s = summarize(samples, vec![Some(5.0), None, Some(1.0), Some(9.0)]);
        assert_eq!((s.total, s.system, s.error), (4, 2, None));
        let pids = |rows: &[ProcessMetrics]| rows.iter().map(|p| p.pid).collect::<Vec<_>>();
        // A process without a rate yet is not ranked as idle.
        assert_eq!(pids(&s.top_by_cpu), [4, 1, 3]);
        // System processes have no readable memory, so they are not ranked by it.
        assert_eq!(pids(&s.top_by_memory), [3, 2]);
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
