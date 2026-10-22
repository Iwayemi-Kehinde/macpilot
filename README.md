# MacPilot

A lightweight system monitor for Apple Silicon Macs, built with Tauri 2, React, TypeScript and Rust.

## Requirements

- macOS on Apple Silicon
- Xcode Command Line Tools (`xcode-select --install`)
- Rust stable, installed per user with rustup into `~/.cargo`
- Node.js 20 or newer with npm

Rust was installed with `--no-modify-path`, so add Cargo to your shell once:

```sh
echo '. "$HOME/.cargo/env"' >> ~/.zprofile
```

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
  battery/ioreg.rs     parser for `ioreg -rn AppleSmartBattery`
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

## Data sources

| Value | Source |
|---|---|
| CPU, per-core usage, load average | sysinfo (Mach host statistics) |
| Memory used | sysinfo: app + wired + compressed, the same definition as Activity Monitor |
| Disks, network, processes | sysinfo |
| Battery percent, state, time remaining | `pmset -g batt` (the macOS estimate) |
| Cycle count, capacities, voltage, current, temperature | `ioreg` AppleSmartBattery |
| Maximum capacity and condition | `system_profiler SPPowerDataType`, cached for 15 minutes |

Battery history is stored at `~/Library/Application Support/com.macpilot.app/battery_history.sqlite`,
one sample a minute while the app is open, kept for 30 days.

## Limitations

- Rates (CPU usage, per-process CPU, network throughput) need two samples, so the first reading shows
  "Measuring" instead of a number.
- Per-process CPU is a percent of one core; a 10-core Mac can show up to 1000% in total.
- Battery history only covers time when MacPilot is running. Sleep and closed periods show as gaps, and
  the discharge rate is measured only over the latest unbroken stretch on battery.
- pmset reports whole percents, so short windows give low-confidence drain rates. The confidence and its
  reason are shown next to every estimate.
- Gauge capacity (full charge over design capacity) can differ from Apple's "Maximum Capacity", which
  uses its own calculation. Both are shown and labelled.
- Per-process energy impact is not shown yet. The accurate source (`powermetrics`) needs root, which
  MacPilot does not use.
- Battery and system data come from macOS tools whose output format is not a public API. The parsers
  return "Unavailable" rather than guessing when the format changes.
