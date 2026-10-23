//! Kernel queries sysinfo does not expose: memory and thermal pressure, the
//! page counts behind Activity Monitor's Memory Used, and per-process paths,
//! CPU time, energy, footprint and working folder.

use std::ffi::CStr;
use std::mem::MaybeUninit;

use serde::Serialize;

unsafe extern "C" {
    /// "com.apple.system.thermalpressurelevel", from <libkern/OSThermalNotification.h>.
    static kOSThermalNotificationPressureLevelName: *const libc::c_char;
    fn notify_register_check(name: *const libc::c_char, out_token: *mut libc::c_int) -> u32;
    fn notify_get_state(token: libc::c_int, state: *mut u64) -> u32;
}

/// How hard macOS is working to cool the Mac, as `OSThermalPressureLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ThermalPressure {
    Nominal,
    Moderate,
    Heavy,
    Trapping,
    Sleeping,
}

/// The level Activity Monitor's Memory Pressure graph shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
}

/// Activity Monitor's "Memory Used": installed memory less free and
/// file-backed pages. It counts app, wired and compressed memory, purgeable
/// pages and the memory the kernel keeps outside its page lists.
pub fn memory_used(free_pages: u64, file_backed_pages: u64, page_size: u64, total_bytes: u64) -> u64 {
    total_bytes.saturating_sub((free_pages + file_backed_pages) * page_size)
}

/// The executable's path and file name, as `top` and Activity Monitor name
/// it. macOS gives them for processes of every user.
pub fn process_identity(pid: u32) -> Option<(String, Option<String>)> {
    let mut buf = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let pid = pid as libc::c_int;
    let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if let Some(path) = text(&buf, len) {
        if let Some(name) = path.rsplit('/').next().filter(|n| !n.is_empty()) {
            return Some((name.to_string(), Some(path.to_string())));
        }
    }
    // An app updated while it runs has no file at its path any more.
    let len = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    text(&buf, len).filter(|n| !n.is_empty()).map(|n| (n.to_string(), None))
}

fn text(buf: &[u8], len: libc::c_int) -> Option<&str> {
    let len = usize::try_from(len).ok().filter(|l| *l > 0 && *l <= buf.len())?;
    std::str::from_utf8(&buf[..len]).ok()
}

/// The working folder of one of this user's processes.
pub fn working_dir(pid: u32) -> Option<String> {
    let mut info = MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
    let size = size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    let n = unsafe { libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDVNODEPATHINFO, 0, info.as_mut_ptr().cast(), size) };
    if n != size {
        return None;
    }
    let info = unsafe { info.assume_init() };
    let bytes: Vec<u8> = info.pvi_cdir.vip_path.as_flattened().iter().map(|c| *c as u8).collect();
    let path = CStr::from_bytes_until_nul(&bytes).ok()?.to_str().ok()?;
    (!path.is_empty()).then(|| path.to_string())
}

/// What macOS reports about one of this user's processes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Usage {
    /// Accumulated CPU time; `None` if the kernel's tick length is unknown.
    pub cpu_secs: Option<f64>,
    /// Physical footprint, the measure in Activity Monitor's Memory column.
    pub footprint_bytes: u64,
    /// CPU energy the kernel has accounted to the process, in nanojoules;
    /// `None` on macOS versions without `RUSAGE_INFO_V6`.
    pub energy_nj: Option<u64>,
}

/// `rusage_info_v6` from <sys/resource.h>, with the fields not read here
/// kept as padding.
#[repr(C)]
struct RusageInfoV6 {
    uuid: [u8; 16],
    user_time: u64,
    system_time: u64,
    _wakeups_to_resident: [u64; 5],
    phys_footprint: u64,
    _to_energy: [u64; 32],
    energy_nj: u64,
    _rest: [u64; 15],
}

const RUSAGE_INFO_V6: libc::c_int = 6;

pub struct Kernel {
    port: libc::mach_port_t,
    page_size: u64,
    total_bytes: u64,
    /// rusage counts CPU time in Mach ticks: 1 ns on Intel, 125/3 ns on Apple Silicon.
    secs_per_tick: Option<f64>,
    thermal_token: Option<libc::c_int>,
}

impl Kernel {
    // libc points these Mach calls to the mach2 crate, which is not a dependency.
    #[allow(deprecated)]
    pub fn new() -> Self {
        // One send right kept for the app's lifetime, not one per query.
        let port = unsafe { libc::mach_host_self() };
        let page_size = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(0);
        let total_bytes = sysctl::<u64>(c"hw.memsize").unwrap_or(0);
        let mut timebase = MaybeUninit::<libc::mach_timebase_info>::zeroed();
        let secs_per_tick = (unsafe { libc::mach_timebase_info(timebase.as_mut_ptr()) } == libc::KERN_SUCCESS)
            .then(|| unsafe { timebase.assume_init() })
            .filter(|t| t.denom != 0)
            .map(|t| f64::from(t.numer) / f64::from(t.denom) / 1e9);
        let mut token = 0;
        let thermal_token = (unsafe { notify_register_check(kOSThermalNotificationPressureLevelName, &mut token) } == 0)
            .then_some(token);
        Self { port, page_size, total_bytes, secs_per_tick, thermal_token }
    }

    /// Memory Used, as Activity Monitor reports it.
    pub fn memory_used(&self) -> Option<u64> {
        let mut stats = MaybeUninit::<libc::vm_statistics64>::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let rc = unsafe {
            libc::host_statistics64(self.port, libc::HOST_VM_INFO64, stats.as_mut_ptr().cast(), &mut count)
        };
        if rc != libc::KERN_SUCCESS || self.page_size == 0 || self.total_bytes == 0 {
            return None;
        }
        let s = unsafe { stats.assume_init() };
        Some(memory_used(u64::from(s.free_count), u64::from(s.external_page_count), self.page_size, self.total_bytes))
    }

    pub fn thermal_pressure(&self) -> Option<ThermalPressure> {
        let mut state = 0u64;
        if unsafe { notify_get_state(self.thermal_token?, &mut state) } != 0 {
            return None;
        }
        Some(match state {
            0 => ThermalPressure::Nominal,
            1 => ThermalPressure::Moderate,
            2 => ThermalPressure::Heavy,
            3 => ThermalPressure::Trapping,
            4 => ThermalPressure::Sleeping,
            _ => return None,
        })
    }

    /// macOS answers only for processes owned by the same user.
    pub fn usage(&self, pid: u32) -> Option<Usage> {
        let mut info = MaybeUninit::<RusageInfoV6>::zeroed();
        if unsafe { libc::proc_pid_rusage(pid as libc::c_int, RUSAGE_INFO_V6, info.as_mut_ptr().cast()) } == 0 {
            let info = unsafe { info.assume_init() };
            return Some(self.to_usage(info.user_time, info.system_time, info.phys_footprint, Some(info.energy_nj)));
        }
        let mut info = MaybeUninit::<libc::rusage_info_v2>::zeroed();
        if unsafe { libc::proc_pid_rusage(pid as libc::c_int, libc::RUSAGE_INFO_V2, info.as_mut_ptr().cast()) } != 0 {
            return None;
        }
        let info = unsafe { info.assume_init() };
        Some(self.to_usage(info.ri_user_time, info.ri_system_time, info.ri_phys_footprint, None))
    }

    fn to_usage(&self, user_ticks: u64, system_ticks: u64, footprint_bytes: u64, energy_nj: Option<u64>) -> Usage {
        let ticks = user_ticks.saturating_add(system_ticks);
        Usage { cpu_secs: self.secs_per_tick.map(|s| ticks as f64 * s), footprint_bytes, energy_nj }
    }
}

pub fn memory_pressure() -> Option<MemoryPressure> {
    // Dispatch's memory-pressure levels: 1 normal, 2 warning, 4 critical.
    match sysctl::<i32>(c"kern.memorystatus_vm_pressure_level")? {
        1 => Some(MemoryPressure::Normal),
        2 => Some(MemoryPressure::Warning),
        4 => Some(MemoryPressure::Critical),
        _ => None,
    }
}

fn sysctl<T: Copy + Default>(name: &CStr) -> Option<T> {
    let mut value = T::default();
    let mut len = size_of::<T>();
    let rc = unsafe { libc::sysctlbyname(name.as_ptr(), (&raw mut value).cast(), &mut len, std::ptr::null_mut(), 0) };
    (rc == 0 && len == size_of::<T>()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `vm_stat` page counts (16 KB pages, 16 GB installed) read as Activity
    /// Monitor on macOS 26.6 showed Memory Used 13.85 GB.
    #[test]
    fn memory_used_matches_activity_monitor() {
        let used = memory_used(4_607, 136_363, 16_384, 16 << 30);
        assert_eq!((used as f64 / (1u64 << 30) as f64 * 100.0).round() / 100.0, 13.85);
    }

    #[test]
    fn rusage_v6_layout_matches_the_sdk() {
        // sizeof and offsetof from <sys/resource.h> in the macOS 26 SDK.
        assert_eq!(size_of::<RusageInfoV6>(), 464);
        assert_eq!(std::mem::offset_of!(RusageInfoV6, user_time), 16);
        assert_eq!(std::mem::offset_of!(RusageInfoV6, phys_footprint), 72);
        assert_eq!(std::mem::offset_of!(RusageInfoV6, energy_nj), 336);
    }

    #[test]
    fn live_memory_and_pressure_are_consistent() {
        let kernel = Kernel::new();
        let used = kernel.memory_used().expect("host statistics");
        assert!(used > 0 && used <= kernel.total_bytes);
        assert!(memory_pressure().is_some());
        assert!(kernel.thermal_pressure().is_some());
    }

    #[test]
    fn identities_are_readable_for_every_user() {
        let (own, path) = process_identity(std::process::id()).expect("own identity");
        assert!(own.starts_with("macpilot"), "{own}");
        assert!(path.is_some_and(|p| p.ends_with(&own)));
        // launchd belongs to root.
        assert_eq!(process_identity(1), Some(("launchd".into(), Some("/sbin/launchd".into()))));
        assert_eq!(process_identity(u32::MAX), None);
    }

    #[test]
    fn usage_and_working_folder_are_readable_for_own_processes_only() {
        let kernel = Kernel::new();
        let own = kernel.usage(std::process::id()).expect("own usage");
        assert!(own.footprint_bytes > 0 && own.cpu_secs.is_some_and(|c| c > 0.0));
        assert!(own.energy_nj.is_some());
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(working_dir(std::process::id()).as_deref(), cwd.to_str());
        // launchd belongs to root.
        assert_eq!(kernel.usage(1), None);
        assert_eq!(working_dir(1), None);
    }

    /// A busy loop's energy must be in the range of one Apple Silicon core.
    #[test]
    fn busy_thread_energy_is_plausible() {
        let kernel = Kernel::new();
        let pid = std::process::id();
        let before = kernel.usage(pid).unwrap();
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_millis(300) {
            std::hint::black_box(0u64);
        }
        let after = kernel.usage(pid).unwrap();
        let watts = (after.energy_nj.unwrap() - before.energy_nj.unwrap()) as f64 / 1e9 / started.elapsed().as_secs_f64();
        assert!((0.05..20.0).contains(&watts), "{watts} W");
    }
}
