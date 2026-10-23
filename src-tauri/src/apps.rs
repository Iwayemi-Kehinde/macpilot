//! Processes grouped into the apps they belong to. Every process inside an app
//! bundle counts for the outermost bundle, so a browser's helper processes add
//! up to the browser; a process outside any bundle is grouped with others of
//! the same name.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::macos;
use crate::processes::{Rate, Sample};

/// Apps sent to the window: the busiest by CPU, and the largest by power and memory.
const TOP_BY_CPU: usize = 25;
const TOP_BY_OTHER: usize = 12;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    pub name: String,
    pub process_count: usize,
    /// Processes of this user, the ones macOS reports memory and power for.
    pub own_process_count: usize,
    /// Sum over processes with a rate, in percent of one core.
    pub cpu_percent: Option<f32>,
    /// CPU power of this user's processes, in watts.
    pub power_w: Option<f32>,
    /// Physical footprint of this user's processes.
    pub memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSummary {
    /// Processes `ps` lists; kernel_task is hidden from apps.
    pub total_processes: usize,
    /// Processes of macOS and other users, whose memory and power macOS
    /// reports only to administrator tools.
    pub system_processes: usize,
    pub error: Option<String>,
    pub apps: Vec<AppUsage>,
}

/// The name of the outermost app bundle on `path`, for a program that runs as
/// part of the app: its executable, or a helper in a `Contents/MacOS` folder
/// or the app's frameworks. Tools only stored in a bundle, such as Xcode's
/// compilers, are programs of their own. Chrome runs updated copies of itself
/// from a clone named `Google Chrome.app.bundle`.
pub fn bundle_name(path: &str) -> Option<&str> {
    let end = [".app/", ".app.bundle/"].iter().filter_map(|suffix| path.find(suffix)).min()?;
    let inside = &path[end..];
    if !inside.contains("/Contents/MacOS/") && !inside.contains("/Contents/Frameworks/") {
        return None;
    }
    let name = path[..end].rsplit('/').next()?;
    (!name.is_empty()).then_some(name)
}

/// The name of the app a process belongs to. A tool installed as one binary
/// per version, as `~/.local/share/claude/versions/2.1.308`, takes the name of
/// its folder.
pub fn app_name<'a>(name: &'a str, path: Option<&'a str>) -> &'a str {
    if let Some(bundle) = path.and_then(bundle_name) {
        return bundle;
    }
    if is_version(name) {
        if let Some(folder) = path.and_then(tool_folder) {
            return folder;
        }
    }
    name
}

/// The app a running process belongs to, and its executable's path.
pub fn identify(pid: u32) -> Option<(String, Option<String>)> {
    let (name, path) = macos::process_identity(pid)?;
    Some((app_name(&name, path.as_deref()).to_string(), path))
}

fn is_version(name: &str) -> bool {
    let digits = name.strip_prefix('v').unwrap_or(name);
    digits.contains('.') && digits.split('.').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn tool_folder(path: &str) -> Option<&str> {
    path.rsplit('/')
        .skip(1)
        .find(|dir| !dir.is_empty() && !is_version(dir) && !matches!(*dir, "versions" | "bin" | "libexec" | "current"))
}

pub fn summarize(samples: &[Sample], rates: &[Rate], me: u32) -> AppSummary {
    let mut groups: HashMap<&str, AppUsage> = HashMap::new();
    for (s, rate) in samples.iter().zip(rates) {
        let name = app_name(&s.name, s.path.as_deref());
        let app = groups.entry(name).or_insert_with(|| AppUsage {
            name: name.to_string(),
            process_count: 0,
            own_process_count: 0,
            cpu_percent: None,
            power_w: None,
            memory_bytes: None,
        });
        app.process_count += 1;
        app.own_process_count += usize::from(s.uid == me);
        add(&mut app.cpu_percent, rate.cpu_percent);
        add(&mut app.power_w, rate.power_w);
        add(&mut app.memory_bytes, s.footprint_bytes);
    }
    AppSummary {
        total_processes: samples.len(),
        system_processes: samples.iter().filter(|s| s.uid != me).count(),
        error: None,
        apps: top(groups.into_values().collect()),
    }
}

fn add<T: std::ops::Add<Output = T> + Copy>(total: &mut Option<T>, value: Option<T>) {
    if let Some(v) = value {
        *total = Some(total.map_or(v, |t| t + v));
    }
}

/// Measured values first, largest first; an app without a reading is not ranked as idle.
fn desc<T: PartialOrd>(a: Option<T>, b: Option<T>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => b.partial_cmp(&a).unwrap_or(Ordering::Equal),
        (a, b) => b.is_some().cmp(&a.is_some()),
    }
}

/// The leaders by each measure, ordered by CPU.
fn top(mut apps: Vec<AppUsage>) -> Vec<AppUsage> {
    let mut keep = HashSet::new();
    let mut pick = |n: usize, measure: fn(&AppUsage) -> Option<f64>| {
        apps.sort_by(|a, b| desc(measure(a), measure(b)).then_with(|| a.name.cmp(&b.name)));
        keep.extend(apps.iter().take_while(|a| measure(a).is_some()).take(n).map(|a| a.name.clone()));
    };
    pick(TOP_BY_OTHER, |a| a.memory_bytes.map(|m| m as f64));
    pick(TOP_BY_OTHER, |a| a.power_w.map(f64::from));
    pick(TOP_BY_CPU, |a| a.cpu_percent.map(f64::from));
    apps.retain(|a| keep.contains(&a.name));
    apps
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn helpers_count_for_the_outermost_bundle() {
        let chrome = "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/141/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)";
        assert_eq!(bundle_name(chrome), Some("Google Chrome"));
        let clone = "/private/var/folders/9z/x/X/com.google.Chrome.code_sign_clone/code_sign_clone.mSx0oG/Google Chrome.app.bundle/Contents/Frameworks/Google Chrome Framework.framework/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper";
        assert_eq!(bundle_name(clone), Some("Google Chrome"));
        assert_eq!(app_name("Finder", Some("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder")), "Finder");
        assert_eq!(app_name("node", Some("/opt/homebrew/bin/node")), "node");
        assert_eq!(app_name("ghost", None), "ghost");
        let crashpad = "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/141/Helpers/chrome_crashpad_handler";
        assert_eq!(bundle_name(crashpad), Some("Google Chrome"));
        let clang = "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang";
        assert_eq!(app_name("clang", Some(clang)), "clang");
        assert_eq!(app_name("Xcode", Some("/Applications/Xcode.app/Contents/MacOS/Xcode")), "Xcode");
        assert_eq!(bundle_name("/Applications/.app/x"), None);
    }

    #[test]
    fn binaries_named_by_version_take_their_folder_name() {
        assert_eq!(app_name("2.1.308", Some("/Users/me/.local/share/claude/versions/2.1.308")), "claude");
        assert_eq!(app_name("v22.1.0", Some("/opt/tool/v22.1.0/bin/v22.1.0")), "tool");
        assert_eq!(app_name("2.1.308", None), "2.1.308");
        assert_eq!(app_name("python3.12", Some("/opt/homebrew/bin/python3.12")), "python3.12");
    }

    #[test]
    fn sums_rates_and_memory_per_app() {
        let at = Instant::now();
        let s = |pid, uid, name: &str, path: &str, footprint| Sample {
            pid,
            uid,
            name: name.into(),
            path: Some(path.into()),
            cpu_secs: 0.0,
            at,
            footprint_bytes: footprint,
            energy_nj: None,
        };
        let r = |cpu, power| Rate { cpu_percent: cpu, power_w: power };
        let samples = [
            s(1, 501, "Google Chrome", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", Some(300)),
            s(2, 501, "Google Chrome Helper", "/Applications/Google Chrome.app/Contents/Frameworks/H.app/Contents/MacOS/Google Chrome Helper", Some(200)),
            s(3, 0, "mds_stores", "/System/Library/Frameworks/CoreServices.framework/mds_stores", None),
            s(4, 501, "node", "/opt/homebrew/bin/node", Some(50)),
            s(5, 501, "node", "/Users/me/.nvm/versions/node/v22/bin/node", Some(70)),
        ];
        let rates = [r(Some(10.0), Some(0.5)), r(Some(5.0), Some(0.25)), r(Some(40.0), None), r(None, None), r(Some(1.0), Some(0.1))];
        let summary = summarize(&samples, &rates, 501);
        assert_eq!((summary.total_processes, summary.system_processes), (5, 1));
        let names: Vec<_> = summary.apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["mds_stores", "Google Chrome", "node"]);
        let chrome = &summary.apps[1];
        assert_eq!((chrome.process_count, chrome.own_process_count), (2, 2));
        assert_eq!((chrome.cpu_percent, chrome.power_w, chrome.memory_bytes), (Some(15.0), Some(0.75), Some(500)));
        let mds = &summary.apps[0];
        assert_eq!((mds.own_process_count, mds.power_w, mds.memory_bytes), (0, None, None));
        // A node process without a rate yet still counts its memory.
        assert_eq!((summary.apps[2].cpu_percent, summary.apps[2].memory_bytes), (Some(1.0), Some(120)));
    }

    #[test]
    fn keeps_the_top_apps_by_each_measure() {
        let app = |i: usize, cpu: f32, memory: u64| AppUsage {
            name: format!("app{i}"),
            process_count: 1,
            own_process_count: 1,
            cpu_percent: Some(cpu),
            power_w: None,
            memory_bytes: Some(memory),
        };
        // 40 apps; the biggest by memory are the idlest by CPU.
        let apps: Vec<_> = (0..40).map(|i| app(i, i as f32, 1000 - i as u64)).collect();
        let kept = top(apps);
        assert_eq!(kept.len(), TOP_BY_CPU + TOP_BY_OTHER);
        assert_eq!(kept[0].cpu_percent, Some(39.0));
        assert!(kept.iter().any(|a| a.memory_bytes == Some(1000)));
    }
}
