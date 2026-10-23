//! Kernel queries sysinfo does not expose: the page counts behind Activity
//! Monitor's memory breakdown, and per-process names, CPU time and footprint.

use std::mem::MaybeUninit;

use serde::Serialize;

/// Memory as Activity Monitor accounts for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    /// "Memory Used": everything but free and file-backed pages. Beyond app,
    /// wired and compressed memory it counts purgeable pages and the memory
    /// the kernel keeps outside its page lists, about 0.8 GB on an M1 Pro.
    pub used_bytes: u64,
    pub breakdown: MemoryBreakdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBreakdown {
    /// Anonymous memory in use by apps, less purgeable caches.
    pub app_bytes: u64,
    pub wired_bytes: u64,
    /// Space the compressor occupies, not the larger amount it holds.
    pub compressed_bytes: u64,
    /// File-backed and purgeable pages, which macOS frees whenever apps need memory.
    pub cached_files_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PageCounts {
    pub free: u64,
    pub internal: u64,
    pub purgeable: u64,
    pub wired: u64,
    pub compressor: u64,
    pub external: u64,
}

/// Activity Monitor's definitions, applied to `vm_statistics64` page counts.
pub fn account(pages: &PageCounts, page_size: u64, total_bytes: u64) -> Memory {
    Memory {
        used_bytes: total_bytes.saturating_sub((pages.free + pages.external) * page_size),
        breakdown: MemoryBreakdown {
            app_bytes: pages.internal.saturating_sub(pages.purgeable) * page_size,
            wired_bytes: pages.wired * page_size,
            compressed_bytes: pages.compressor * page_size,
            cached_files_bytes: (pages.external + pages.purgeable) * page_size,
        },
    }
}

/// The executable's file name, as `top` and Activity Monitor show it. macOS
/// gives it for processes of every user.
pub fn process_name(pid: u32) -> Option<String> {
    let mut buf = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let pid = pid as libc::c_int;
    let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if let Some(name) = text(&buf, len).and_then(|path| path.rsplit('/').next().map(str::to_string)).filter(|n| !n.is_empty()) {
        return Some(name);
    }
    // An app updated while it runs has no file at its path any more.
    let len = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    text(&buf, len).map(str::to_string).filter(|n| !n.is_empty())
}

fn text(buf: &[u8], len: libc::c_int) -> Option<&str> {
    let len = usize::try_from(len).ok().filter(|l| *l > 0 && *l <= buf.len())?;
    std::str::from_utf8(&buf[..len]).ok()
}

/// What macOS reports about one of this user's processes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Usage {
    /// Accumulated CPU time; `None` if the kernel's tick length is unknown.
    pub cpu_secs: Option<f64>,
    /// Physical footprint, the measure in Activity Monitor's Memory column.
    pub footprint_bytes: u64,
}

pub struct Kernel {
    port: libc::mach_port_t,
    page_size: u64,
    total_bytes: u64,
    /// rusage counts CPU time in Mach ticks: 1 ns on Intel, 125/3 ns on Apple Silicon.
    secs_per_tick: Option<f64>,
}

impl Kernel {
    // libc points these Mach calls to the mach2 crate, which is not a dependency.
    #[allow(deprecated)]
    pub fn new() -> Self {
        // One send right kept for the app's lifetime, not one per query.
        let port = unsafe { libc::mach_host_self() };
        let page_size = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(0);
        let mut total_bytes = 0u64;
        let mut len = std::mem::size_of::<u64>();
        let rc = unsafe {
            libc::sysctlbyname(c"hw.memsize".as_ptr(), (&mut total_bytes as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0)
        };
        if rc != 0 {
            total_bytes = 0;
        }
        let mut timebase = MaybeUninit::<libc::mach_timebase_info>::zeroed();
        let secs_per_tick = (unsafe { libc::mach_timebase_info(timebase.as_mut_ptr()) } == libc::KERN_SUCCESS)
            .then(|| unsafe { timebase.assume_init() })
            .filter(|t| t.denom != 0)
            .map(|t| f64::from(t.numer) / f64::from(t.denom) / 1e9);
        Self { port, page_size, total_bytes, secs_per_tick }
    }

    pub fn memory(&self) -> Option<Memory> {
        let mut stats = MaybeUninit::<libc::vm_statistics64>::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let rc = unsafe {
            libc::host_statistics64(self.port, libc::HOST_VM_INFO64, stats.as_mut_ptr().cast(), &mut count)
        };
        if rc != libc::KERN_SUCCESS || self.page_size == 0 || self.total_bytes == 0 {
            return None;
        }
        let s = unsafe { stats.assume_init() };
        let pages = PageCounts {
            free: u64::from(s.free_count),
            internal: u64::from(s.internal_page_count),
            purgeable: u64::from(s.purgeable_count),
            wired: u64::from(s.wire_count),
            compressor: u64::from(s.compressor_page_count),
            external: u64::from(s.external_page_count),
        };
        Some(account(&pages, self.page_size, self.total_bytes))
    }

    /// macOS answers only for processes owned by the same user.
    pub fn usage(&self, pid: u32) -> Option<Usage> {
        let mut info = MaybeUninit::<libc::rusage_info_v2>::zeroed();
        let rc = unsafe { libc::proc_pid_rusage(pid as libc::c_int, libc::RUSAGE_INFO_V2, info.as_mut_ptr().cast()) };
        if rc != 0 {
            return None;
        }
        let info = unsafe { info.assume_init() };
        let ticks = info.ri_user_time.saturating_add(info.ri_system_time);
        Some(Usage { cpu_secs: self.secs_per_tick.map(|s| ticks as f64 * s), footprint_bytes: info.ri_phys_footprint })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `vm_stat` page counts (16 KB pages, 16 GB installed) read as Activity
    /// Monitor on macOS 26.6 showed Memory Used 13.85 GB, App Memory 5.93,
    /// Wired 2.61, Compressed 4.52 and Cached Files 2.24.
    #[test]
    fn accounts_like_activity_monitor() {
        let pages = PageCounts { free: 4_607, internal: 399_073, purgeable: 10_656, wired: 170_771, compressor: 296_354, external: 136_363 };
        let m = account(&pages, 16_384, 16 << 30);
        let gb = |bytes: u64| (bytes as f64 / (1u64 << 30) as f64 * 100.0).round() / 100.0;
        let b = m.breakdown;
        assert_eq!(gb(m.used_bytes), 13.85);
        assert_eq!((gb(b.app_bytes), gb(b.wired_bytes), gb(b.compressed_bytes), gb(b.cached_files_bytes)), (5.93, 2.61, 4.52, 2.24));
    }

    #[test]
    fn live_memory_is_consistent() {
        let m = Kernel::new().memory().expect("host statistics");
        let total = sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::nothing().with_memory(sysinfo::MemoryRefreshKind::nothing().with_ram()),
        )
        .total_memory();
        let b = m.breakdown;
        assert!(b.app_bytes + b.wired_bytes + b.compressed_bytes <= m.used_bytes && m.used_bytes <= total);
    }

    #[test]
    fn names_are_readable_for_every_user() {
        let own = process_name(std::process::id()).expect("own name");
        assert!(own.starts_with("macpilot"), "{own}");
        // launchd belongs to root.
        assert_eq!(process_name(1).as_deref(), Some("launchd"));
        assert_eq!(process_name(u32::MAX), None);
    }

    #[test]
    fn usage_is_readable_for_own_processes_only() {
        let kernel = Kernel::new();
        let own = kernel.usage(std::process::id()).expect("own usage");
        assert!(own.footprint_bytes > 0 && own.cpu_secs.is_some_and(|c| c > 0.0));
        // launchd belongs to root.
        assert_eq!(kernel.usage(1), None);
    }
}
