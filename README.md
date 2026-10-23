# MacPilot

A lightweight system monitor for Apple Silicon Macs, built with Tauri 2, React, TypeScript and Rust.

## Requirements

- macOS on Apple Silicon
- Xcode Command Line Tools (`xcode-select --install`)
- Rust stable, installed per user with rustup into `~/.cargo`
- Node.js 20 or newer with npm

Rust was installed with `--no-modify-path`. The npm scripts add `~/.cargo/bin` to `PATH` themselves, so
`npm run tauri dev` and `npm run test:rust` work without changing your shell profile.

## Commands

```sh
npm install            # project dependencies (local to this folder)
npm run tauri dev      # run the desktop app with hot reload
npm test               # frontend tests (Vitest)
npm run test:rust      # Rust tests (parsers, history, live metrics smoke test)
npm run tauri build    # release build, output in src-tauri/target/release/bundle/macos
```

## Architecture

```
src-tauri/src/
  metrics.rs           the snapshot: CPU, memory, disks and network from sysinfo
  macos.rs             kernel memory counts, per-process CPU time and footprint
  processes.rs         process list from `ps`, CPU rates from accumulated CPU time
  tool.rs              runs macOS tools with fixed arguments and a time limit
  battery/pmset.rs     parser for `pmset -g batt`
  battery/ioreg.rs     parser for `ioreg -rd1 -w0 -n AppleSmartBattery`
  battery/profiler.rs  parser for `system_profiler SPPowerDataType -json`
  battery/mod.rs       combines the three into one BatteryInfo
  history.rs           SQLite battery history and discharge-rate estimate
  lib.rs               Tauri commands and the battery sampler thread
src/
  lib/schemas.ts       zod schemas for every value the Rust layer returns
  lib/api.ts           typed command wrappers; invalid data throws, never renders
  hooks/usePolling.ts  polling that stops while the window is hidden
  components/          dashboard cards and the SVG line chart
```

The UI only talks to Rust through three read-only commands: `get_system_snapshot`, `get_battery` and
`get_battery_history`. MacPilot never terminates processes or changes system settings.

## Security

- Capabilities grant only `core:window:allow-start-dragging` and `core:window:allow-internal-toggle-maximize`,
  the two commands the title-bar drag region calls. No shell, file system, opener or event permissions.
- The production CSP allows only the app's own files and Tauri IPC. `tauri dev` loads the page from the Vite
  server, so the CSP is not enforced in development.
- System tools run by absolute path with fixed arguments, no shell, `LC_ALL=C`, and a timeout (5 s for `pmset`,
  `ioreg` and `ps`, 15 s for `system_profiler`) after which they are killed.
- No sudo, no privileged helpers, no network access, no telemetry.

## Data sources

| Value | Source |
|---|---|
| CPU, per-core usage, load average | sysinfo (Mach host statistics) |
| Memory used, app, wired, compressed, cached files | Kernel page counts (`host_statistics64`) with Activity Monitor's definitions: used = installed minus free and file-backed pages, app = anonymous minus purgeable, cached files = file-backed plus purgeable |
| Disks | sysinfo; free space is re-read every 30 s or when a volume is mounted or ejected, since reading it costs more than the rest of a refresh |
| Network | sysinfo |
| Processes | `ps` lists every process with its owner and CPU time, rounded to 10 ms; zombies are left out, as in `top`. Names come from the kernel (`proc_pidpath`). For your own processes the kernel (`proc_pid_rusage`) gives an exact CPU time and the physical footprint, the measure in Activity Monitor's Memory column |
| Battery percent, state, time remaining | `pmset -g batt` (the macOS estimate) |
| Cycle count, capacities, voltage, current, temperature | `ioreg` AppleSmartBattery |
| Maximum capacity and condition | `system_profiler SPPowerDataType`, cached for 15 minutes |

Checked against macOS 26.6 on an M1 Pro: the five memory figures match Activity Monitor's Memory tab to the
hundredth of a GB when read in the same second, and per-process CPU matches `top` within a point (a process
burning a steady 28% of one core read 28.1 to 28.5% in MacPilot and 27.9 to 28.9% in `top` over the same
seconds). Memory Used is about 0.8 GB more than App + Wired + Compressed, as in Activity Monitor, because
purgeable pages and the memory the kernel keeps outside its page lists count as used; Memory Used plus Cached
Files can therefore exceed the installed memory.

Battery history is stored at `~/Library/Application Support/com.macpilot.desktop/battery_history.sqlite`,
one sample a minute while the app is open, kept for 30 days. The battery card itself refreshes every 10 s while
the window is visible.

## Limitations

- About a third of processes (192 of 637 when measured) belong to macOS or other users. Their memory shows as
  Unavailable, because macOS reveals it only to administrator tools such as Activity Monitor and `top`, so the
  Memory view lists your own processes. They are still ranked by CPU, from `ps`, whose 10 ms rounding makes
  their rates move in steps of about 1 point at a 1 s refresh (5 points on a fresh 0.2 s reading). kernel_task
  is not visible to apps at all; its CPU is still part of the total.
- Per-process CPU is the change in accumulated CPU time between two readings. sysinfo's own per-process rate
  (0.39.6) keeps a process's last busy value after it goes idle, so it is not used.
- Polling stops whenever macOS reports the window hidden: minimized, app hidden, or fully covered by other
  windows. One reading is still taken at launch so the window has data when it is first shown.
- The battery sampler keeps running while the app is open, hidden or not: one `pmset` and one `ioreg` call a
  minute. If two copies run, samples closer than 45 s to an existing one in the same power state are skipped.
  Connecting or removing the adapter is recorded at once while the window is visible.
- Cost while visible: each refresh takes about 5 ms of CPU in the app (release build) and 19 ms in one `ps`
  call, so about 2.4% of one core at the 1 s refresh and 0.5% at 5 s. WebKit's content and GPU processes added
  roughly 5-10% more at 1 s when last measured. Hidden: about 0.1%.
- Rates (CPU usage, per-process CPU, network throughput) need two samples. The first reading, and the first
  after more than 15 s hidden or after sleep, is measured over a fresh 0.2 s window instead of averaging over
  the gap, so it is noisier than later readings.
- Per-process CPU is a percent of one core; a 10-core Mac can show up to 1000% in total.
- Battery history only covers time when MacPilot is running. Sleep and closed periods show as gaps, and
  the discharge rate is measured only over the latest unbroken stretch on battery.
- The drain rate comes from the battery gauge's charge counter (mAh), which still moves while macOS holds
  the displayed charge at 100%. Samples without gauge readings fall back to pmset's whole percents, where
  short windows give low-confidence rates. The confidence and its reason are shown next to every estimate.
- Gauge capacity (full charge over design capacity) can differ from Apple's "Maximum Capacity", which
  uses its own calculation. Both are shown and labelled.
- Per-process energy impact is not shown yet. The accurate source (`powermetrics`) needs root, which
  MacPilot does not use.
- Battery and system data come from macOS tools whose output format is not a public API. The parsers
  return "Unavailable" rather than guessing when the format changes.
