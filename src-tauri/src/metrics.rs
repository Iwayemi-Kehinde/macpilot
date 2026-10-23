//! CPU, memory, disk, network, heat and per-app metrics. `sysinfo` reads the
//! native Mach and BSD interfaces; `macos` and `processes` cover what it does not.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use crate::apps::{self, AppSummary};
use crate::macos::{self, Kernel, MemoryPressure, ThermalPressure};
use crate::processes::{self, Rates, Sample};
use sysinfo::{DiskRefreshKind, Disks, Networks, System, MINIMUM_CPU_UPDATE_INTERVAL};

/// Rates averaged over a longer window than this describe the past, not now
/// (the window was hidden or the Mac slept), so they are withheld for a sample.
const MAX_RATE_WINDOW: Duration = Duration::from_secs(15);
/// Free space changes slowly, and reading it costs more than the rest of a
/// snapshot together, so it is re-read this often or when a volume comes or goes.
const DISK_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
/// Wi-Fi and Ethernet. Tunnels, loopback and AirDrop links carry traffic that
/// also crosses these, or never leaves the Mac.
const PHYSICAL_INTERFACE_PREFIX: &str = "en";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub hostname: Option<String>,
    pub os_version: Option<String>,
    pub cpu_brand: Option<String>,
    pub logical_cores: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuMetrics {
    /// A rate between two refreshes at least 0.2 s apart. Snapshots always
    /// measure one; `None` stays in the format so readers handle a missing rate.
    pub usage_percent: Option<f32>,
    pub thermal: Option<ThermalPressure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    /// Activity Monitor's "Memory Used", or sysinfo's figure if the kernel did not answer.
    pub used_bytes: u64,
    pub pressure: Option<MemoryPressure>,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

/// The startup volume.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskMetrics {
    pub name: String,
    pub total_bytes: u64,
    /// Finder's figure, which counts purgeable files.
    pub available_bytes: u64,
}

/// Traffic through Wi-Fi and Ethernet.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkMetrics {
    /// See `CpuMetrics::usage_percent`.
    pub rx_bytes_per_sec: Option<f64>,
    pub tx_bytes_per_sec: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    pub timestamp_ms: u64,
    pub uptime_secs: u64,
    pub host: HostInfo,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub disk: Option<DiskMetrics>,
    pub network: NetworkMetrics,
    pub apps: AppSummary,
}

pub struct Monitor {
    sys: System,
    kernel: Kernel,
    rates: Rates,
    disks: Disks,
    networks: Networks,
    host: HostInfo,
    /// When the counters behind every rate were last read.
    counters_read: Option<Stamp>,
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
            cpu_brand: sys.cpus().first().map(|c| c.brand().trim().to_string()).filter(|b| !b.is_empty()),
            logical_cores: sys.cpus().len(),
        };
        Self {
            sys,
            kernel: Kernel::new(),
            rates: Rates::new(MAX_RATE_WINDOW),
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            host,
            counters_read: None,
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

    /// Reads the counters behind every rate back to back, so CPU, network and
    /// process rates share one window.
    fn read_counters(&mut self) -> Result<Vec<Sample>, String> {
        self.counters_read = Some(Stamp::now());
        self.sys.refresh_cpu_usage();
        self.networks.refresh(true);
        processes::read(&self.kernel)
    }

    /// Blocks for up to `MINIMUM_CPU_UPDATE_INTERVAL` when it needs a fresh
    /// rate window, so call it off the async runtime.
    pub fn snapshot(&mut self) -> SystemSnapshot {
        match self.counters_read.map(|s| (s.mono.elapsed(), s.window())) {
            // Called again within the minimum interval: wait out the rest.
            Some((elapsed, _)) if elapsed < MINIMUM_CPU_UPDATE_INTERVAL => {
                std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL - elapsed)
            }
            Some((_, Some(_))) => {}
            // First call, or the last one was too long ago (window hidden, Mac
            // asleep) to describe now: take a fresh short sample instead.
            _ => {
                // Process rates must not span the gap either: forget the old
                // readings, then take a baseline. If ps fails here, the read
                // that follows reports it.
                self.rates.reset();
                if let Ok(samples) = self.read_counters() {
                    self.rates.update(&samples);
                }
                std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
            }
        }
        let window_secs = self.counters_read.map(|s| s.mono.elapsed().as_secs_f64());
        let apps = match self.read_counters() {
            Ok(samples) => {
                let rates = self.rates.update(&samples);
                apps::summarize(&samples, &rates, unsafe { libc::getuid() })
            }
            Err(error) => AppSummary { error: Some(error), ..AppSummary::default() },
        };
        self.sys.refresh_memory();
        self.refresh_disks();

        let cpu = CpuMetrics { usage_percent: Some(self.sys.global_cpu_usage()), thermal: self.kernel.thermal_pressure() };

        let memory = MemoryMetrics {
            total_bytes: self.sys.total_memory(),
            used_bytes: self.kernel.memory_used().unwrap_or_else(|| self.sys.used_memory()),
            pressure: macos::memory_pressure(),
            swap_total_bytes: self.sys.total_swap(),
            swap_used_bytes: self.sys.used_swap(),
        };

        // `/` and `/System/Volumes/Data` share one APFS container, and `/`
        // reports the space Finder shows for it.
        let disks = self.disks.list();
        let disk = ["/", "/System/Volumes/Data"]
            .iter()
            .find_map(|mount| disks.iter().find(|d| d.mount_point().as_os_str() == *mount))
            .map(|d| DiskMetrics {
                name: d.name().to_string_lossy().into_owned(),
                total_bytes: d.total_space(),
                available_bytes: d.available_space(),
            });

        let physical = || self.networks.list().iter().filter(|(name, _)| name.starts_with(PHYSICAL_INTERFACE_PREFIX)).map(|(_, n)| n);
        let network = NetworkMetrics {
            rx_bytes_per_sec: window_secs.map(|s| physical().map(|n| n.received()).sum::<u64>() as f64 / s),
            tx_bytes_per_sec: window_secs.map(|s| physical().map(|n| n.transmitted()).sum::<u64>() as f64 / s),
        };

        SystemSnapshot {
            timestamp_ms: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
            uptime_secs: System::uptime(),
            host: self.host.clone(),
            cpu,
            memory,
            disk,
            network,
            apps,
        }
    }
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
        assert!(first.network.rx_bytes_per_sec.is_some() && first.network.tx_bytes_per_sec.is_some());
        assert!(first.apps.apps.iter().any(|a| a.cpu_percent.is_some()));
        assert!(first.memory.total_bytes > 0);
        assert!(first.memory.used_bytes <= first.memory.total_bytes);
        assert!(first.memory.pressure.is_some() && first.cpu.thermal.is_some());
        let disk = first.disk.as_ref().expect("startup volume");
        assert!(disk.available_bytes <= disk.total_bytes && disk.total_bytes > 0);
        assert!(first.apps.total_processes > 0);
        assert!(first.host.logical_cores > 0);

        let started = Instant::now();
        let second = m.snapshot();
        assert!(started.elapsed() >= MINIMUM_CPU_UPDATE_INTERVAL / 2, "waited {:?}", started.elapsed());
        let usage = second.cpu.usage_percent.expect("cpu usage after the minimum interval");
        assert!((0.0..=100.0).contains(&usage));
        assert!(second.network.rx_bytes_per_sec.is_some());

        let apps = &second.apps;
        assert_eq!(apps.error, None);
        assert!(apps.system_processes > 0 && apps.system_processes < apps.total_processes);
        let cpus: Vec<f32> = apps.apps.iter().filter_map(|a| a.cpu_percent).collect();
        assert!(cpus.windows(2).all(|w| w[0] >= w[1]), "{cpus:?}");
        assert!(apps.apps.iter().any(|a| a.power_w.is_some()), "no app with a power reading");
        // This test binary is one of this user's processes.
        assert!(apps.apps.iter().filter(|a| a.own_process_count > 0).all(|a| a.memory_bytes.is_some()));
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
