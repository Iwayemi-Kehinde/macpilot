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
  metrics.rs           CPU, memory, disks, network, processes (sysinfo)
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
- System tools run by absolute path with fixed arguments, no shell, `LC_ALL=C`, and a timeout (5 s for `pmset`
  and `ioreg`, 15 s for `system_profiler`) after which they are killed.
- No sudo, no privileged helpers, no network access, no telemetry.

## Data sources

| Value | Source |
|---|---|
| CPU, per-core usage, load average | sysinfo (Mach host statistics) |
| Memory used | sysinfo: app + wired + compressed, the same definition as Activity Monitor |
| Disks | sysinfo; free space is re-read every 30 s or when a volume is mounted or ejected, since reading it costs more than the rest of a refresh |
| Network | sysinfo |
| Processes | sysinfo: CPU per process and resident memory, for processes this user can read |
| Battery percent, state, time remaining | `pmset -g batt` (the macOS estimate) |
| Cycle count, capacities, voltage, current, temperature | `ioreg` AppleSmartBattery |
| Maximum capacity and condition | `system_profiler SPPowerDataType`, cached for 15 minutes |

Battery history is stored at `~/Library/Application Support/com.macpilot.desktop/battery_history.sqlite`,
one sample a minute while the app is open, kept for 30 days. The battery card itself refreshes every 10 s while
the window is visible.

## Limitations

- About a third of processes (182 of 559 when measured) belong to root or system users. macOS only exposes
  their CPU and memory to privileged tools such as `top` and Activity Monitor, so MacPilot counts them but does
  not rank them. Total CPU and memory still include them.
- Process memory is resident size. Activity Monitor's Memory column shows the footprint, a different measure.
- Polling stops whenever macOS reports the window hidden: minimized, app hidden, or fully covered by other
  windows. One reading is still taken at launch so the window has data when it is first shown.
- The battery sampler keeps running while the app is open, hidden or not: one `pmset` and one `ioreg` call a
  minute. If two copies run, samples closer than 45 s to an existing one in the same power state are skipped.
  Connecting or removing the adapter is recorded at once while the window is visible.
- Cost while visible, measured at the 1 s refresh: about 4% of one core for the Rust backend (dev build).
  WebKit's content and GPU processes added roughly 5-10% more when last measured. Hidden: about 0.1%.
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
