# MacPilot

MacPilot shows where your Mac's battery and speed are going, and what you can safely do about it. It is a
desktop app for Apple Silicon Macs, built with Tauri 2, React, TypeScript and Rust.

## What it shows

- **Needs attention.** A short list of problems found in the readings below. Each one gives the evidence,
  where it was measured and a safe next step. MacPilot only suggests: it never quits apps or changes settings.
- **Vitals.** CPU use with the Mac's heat (thermal pressure), memory with pressure and swap, free space on the
  startup disk, and network throughput.
- **Power.** The battery percentage and macOS's time estimate, where the power goes in watts (charger, Mac,
  battery), the drain right now, the average over this run on battery, your usual drain, and a history chart
  from 1 hour to 7 days.
- **Battery health.** Maximum capacity, condition, charge cycles and temperature.
- **Apps.** Processes grouped by app, with CPU, power and memory, sortable by each.
- **Keeping your Mac awake.** Apps that stop the display or the Mac from sleeping, and for how long.
- **Open ports.** Servers your apps are running, the folder they were started from, and whether other devices
  on the network can reach them.

## What the analyzer checks

| Check | Flags | Source |
|---|---|---|
| Heat | Moderate thermal pressure as a warning, heavy or worse as critical | Thermal state macOS publishes to apps |
| Memory | Pressure at warning or critical, naming your largest apps | Kernel memory pressure level |
| Startup disk | Under 25 GB free as a warning, under 10 GB as critical | Finder's available space |
| CPU | An app averaging 80% of a core or more over 2 minutes | CPU time from `ps` and the kernel |
| Battery drain | 15% an hour or more, or 1.6 times your usual, naming the biggest power users | The average over this run when its confidence is medium or high, else 2 minutes of the gauge's current |
| Sleep | An app keeping the display on for 10 minutes (a warning on battery) or the Mac awake for 30 minutes | `pmset -g assertions`; assertions that end on their own are left out |
| Battery health | A condition other than Normal, or under 80% maximum capacity | `system_profiler` |
| Open ports | A server outside macOS that other devices on the network can reach | `lsof` |

When nothing is flagged, the card says so and lists the checks that had data.

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
npm run test:rust      # Rust tests (parsers, history, live checks against macOS tools)
npm run tauri build    # release build, output in src-tauri/target/release/bundle/macos
```

## Architecture

```
src-tauri/src/
  metrics.rs           the snapshot: CPU, heat, memory, startup disk, network and apps
  macos.rs             kernel calls: memory, thermal and memory pressure, and per-process CPU time,
                       footprint, energy and working folder
  processes.rs         process list from `ps`; CPU and power from accumulated CPU time and energy
  apps.rs              groups processes by app
  awake.rs             parser for `pmset -g assertions`
  ports.rs             parser for the listening TCP sockets `lsof` reports
  tool.rs              runs macOS tools with fixed arguments and a time limit
  battery/pmset.rs     parser for `pmset -g batt`
  battery/ioreg.rs     parser for `ioreg -rd1 -w0 -n AppleSmartBattery`, including its power telemetry
  battery/profiler.rs  parser for `system_profiler SPPowerDataType -json`
  battery/mod.rs       combines the three into one BatteryInfo
  history.rs           SQLite battery history, the average and usual drain, and the percentage calibration
  lib.rs               Tauri commands and the battery sampler thread
src/
  lib/schemas.ts       zod schemas for every value the Rust layer returns
  lib/api.ts           typed command wrappers; invalid data throws, never renders
  lib/insights.ts      the analyzer: readings in, things that need attention out
  lib/awake.ts         groups sleep assertions by app
  lib/ports.ts         groups listening ports by process, notes on well-known ones
  hooks/usePolling.ts  polling that stops while the window is hidden
  components/          dashboard cards and the SVG charts
```

The UI talks to Rust only through five read-only commands: `get_system_snapshot`, `get_battery`,
`get_battery_history`, `get_sleep_blockers` and `get_ports`. MacPilot never terminates processes or changes
system settings.

## Security

- Capabilities grant only `core:window:allow-start-dragging` and `core:window:allow-internal-toggle-maximize`,
  the two commands the title-bar drag region calls. No shell, file system, opener or event permissions.
- The production CSP allows only the app's own files and Tauri IPC. `tauri dev` loads the page from the Vite
  server, so the CSP is not enforced in development.
- System tools run by absolute path with fixed arguments, no shell, `LC_ALL=C`, and a timeout (5 s for `pmset`,
  `ioreg`, `ps` and `lsof`, 15 s for `system_profiler`) after which they are killed.
- No sudo, no privileged helpers, no network access, no telemetry.

## Data sources

| Value | Source |
|---|---|
| CPU use | sysinfo (Mach host statistics) |
| Heat | The thermal pressure level macOS posts through `notify` (`kOSThermalNotificationPressureLevelName`), the same state `ProcessInfo.thermalState` reports |
| Memory used | Kernel page counts (`host_statistics64`) with Activity Monitor's definition: installed memory minus free and file-backed pages |
| Memory pressure, swap | `kern.memorystatus_vm_pressure_level` and `vm.swapusage` |
| Startup disk | sysinfo, for the volume mounted at `/`. Available space is Finder's figure, which counts purgeable files macOS removes when it needs room (Disk Utility's "free" leaves them out). It is re-read every 30 s or when a volume is mounted or ejected, since reading it costs more than the rest of a refresh |
| Network | sysinfo, summed over the Wi-Fi and Ethernet interfaces |
| Apps | `ps` lists every process with its owner and CPU time; names and app bundles come from the kernel (`proc_pidpath`). For your own processes the kernel (`proc_pid_rusage`) gives an exact CPU time, the physical footprint (Activity Monitor's Memory column) and the CPU energy used, from which power is the energy between two readings over the time between them |
| Battery percent, state, time remaining | `pmset -g batt` (the macOS estimate) |
| Watts | The battery's power telemetry in `ioreg` AppleSmartBattery: the Mac's total draw, the power from the charger and the power into or out of the battery. Voltage times current where the telemetry is missing |
| Charger rating | `ioreg` AppleSmartBattery adapter details |
| Cycle count, current, temperature | `ioreg` AppleSmartBattery |
| Maximum capacity and condition | `system_profiler SPPowerDataType`, cached for 15 minutes. Its JSON names the condition internally ("Good"); MacPilot shows macOS's wording ("Normal", or "Service Recommended") from the report's own string table |
| Drain now | The gauge's present current over its full charge, converted to the percentage macOS shows (see Limitations) |
| Average and usual drain | The battery history below |
| Sleep | `pmset -g assertions`: each app's assertions, merged with those `coreaudiod` holds on its behalf |
| Open ports | `lsof -nP -iTCP -sTCP:LISTEN`; the folder a server was started from comes from the kernel (`proc_pidinfo`) |

Checked against macOS 26.6 on an M1 Pro:

- Memory Used matched Activity Monitor's Memory tab to the hundredth of a GB when read in the same second.
  Swap, available disk space, cycle count, maximum capacity and condition equal what `vm.swapusage`, Finder and
  `system_profiler` report, and the heat level matches `ProcessInfo.thermalState`.
- Per-process CPU matches `top` within a point (a process burning a steady 28% of one core read 28.1 to 28.5%
  in MacPilot and 27.9 to 28.9% in `top` over the same seconds). Total CPU under a steady load matched `top`
  second by second within about a point.
- On battery, the Mac's draw from the telemetry equals voltage times current (9.70 W at 11.001 V and 882 mA).
  Drain now agrees with macOS's own estimate: 16% at 12.3% an hour lasts 78 minutes, and `pmset` showed 1:18.
- A busy loop on one performance core read 3.67 W of CPU power, while the Mac's total draw rose about 5.5 W.
- Over a 6.5-hour run on battery the percentage fell from 92% to 39%, 8.18 points an hour, and MacPilot's
  average read 8.12.
- The sleep list and open ports matched `pmset -g assertions` and `lsof` run at the same time.

Battery history is stored at `~/Library/Application Support/com.macpilot.desktop/battery_history.sqlite`,
one sample a minute while the app is open, kept for 30 days.

## Limitations

- About a third of processes (192 of about 620 here) belong to macOS or other users. macOS reveals their
  memory and energy only to administrator tools such as Activity Monitor and `top`, so the Apps table shows a
  dash for them and ranks them by CPU from `ps` alone, whose 10 ms rounding makes their rates move in steps of
  about 1 point at a 1 s refresh. kernel_task is not visible to apps at all; its CPU is still part of the total.
- Power per app is the CPU energy macOS accounts to each of your processes. It leaves out the display, the
  graphics and the rest of the chip, so apps add up to much less than the Mac's total draw (about 0.4 W of apps
  against 5 to 6 W here). Whole-app figures such as Activity Monitor's Energy Impact come from `powermetrics`,
  which needs root, and MacPilot does not use root.
- Apps are grouped by the outermost app bundle on their path, else by process name. Tools only stored in a
  bundle, such as Xcode's compilers, count as programs of their own, and a tool installed as one binary per
  version takes its folder's name. WebKit's helper processes serve every app that shows web pages,
  MacPilot included, so they are listed under their own names.
- Open ports come from `lsof` without root, which sees only the sockets of processes running as you. System
  daemons' listeners are not listed. "Network" means bound to an interface other devices can reach; the macOS
  firewall may still block them, and MacPilot does not read its settings.
- Sleep assertions that end on their own, or that started less than a minute ago, are folded into one
  "short-lived" line.
- Drain now follows the load from moment to moment. The average needs 10 minutes on battery in one unbroken
  run. The usual drain needs 2 hours on battery over the last 7 days, and like all history it only counts
  time when MacPilot was open.
- The drain rates are given in points of the percentage macOS shows. That percentage falls about 7% faster
  than the gauge's share of its full charge on the M1 Pro it was checked on, so MacPilot fits one against the
  other over the last 7 days on battery and converts with that line. The share is fitted, not mAh, because
  the gauge keeps relearning its full charge (7588 to 7701 mAh within a few hours here); the fitted line is
  within about half a point of every reading. Until MacPilot has read 11 different percentages on battery,
  the rates are the gauge's share, and the Power card says so. Under the line's 0% point the time to empty
  uses the percentage macOS shows.
- The average gives no rate when the newest reading is more than 15 minutes old. Samples without gauge
  readings fall back to pmset's whole percents, where short windows give low-confidence rates. The
  confidence and its reason are shown with the average.
- The time left at the top of the Power card is macOS's own estimate from recent power use; the average's
  time to empty uses the average over this run on battery, so the two differ when the load changes.
- Polling stops whenever macOS reports the window hidden: minimized, app hidden, or fully covered by other
  windows. One reading is still taken at launch so the window has data when it is first shown. While visible,
  the snapshot refreshes at the chosen interval (2 s by default), the battery every 10 s, sleep assertions and
  ports every 15 s, and the history every minute.
- The battery sampler keeps running while the app is open, hidden or not: one `pmset` and one `ioreg` call a
  minute. If two copies run, samples closer than 45 s to an existing one in the same power state are skipped.
  Connecting or removing the adapter is recorded at once while the window is visible.
- Rates (CPU use, per-app CPU and power, network throughput) need two samples. The first reading, and the
  first after more than 15 s hidden or after sleep, is measured over a fresh 0.2 s window instead of averaging
  over the gap, so it is noisier than later readings.
- Per-app CPU is a percent of one core; a 10-core Mac can show up to 1000% in total.
- Battery and system data come from macOS tools whose output format is not a public API. The parsers
  return "Unavailable" rather than guessing when the format changes.
