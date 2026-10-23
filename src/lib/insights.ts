// Turns measured readings into a short list of things that need attention,
// each with its evidence, where it was measured and a safe next step.
// Nothing here acts on the system; suggestions are for the user to follow.
import { formatBytes, formatList as list, formatMinutes, formatWatts } from "./format";
import { stronger } from "./awake";
import { isHealthyCondition } from "./battery";
import { describeServer, isAppleService, servers } from "./ports";
import type { BatteryHistory, BatteryInfo, Listener, SleepBlocker, SystemSnapshot } from "./schemas";

export type Level = "critical" | "warning" | "info";

export interface Insight {
  id: string;
  level: Level;
  title: string;
  detail: string;
  action?: string;
  /** Where the numbers come from, and so how far to trust them. */
  source: string;
}

/** Per-app CPU from one snapshot, kept to judge use over minutes rather than one spike. */
export interface CpuReading {
  t: number;
  apps: { name: string; cpu: number | null; own: boolean }[];
}

export interface DrainReading {
  t: number;
  v: number;
}

export interface Inputs {
  now: number;
  snapshot?: SystemSnapshot;
  battery?: BatteryInfo;
  history?: BatteryHistory;
  blockers?: SleepBlocker[];
  ports?: Listener[];
  cpuReadings: CpuReading[];
  drainReadings: DrainReading[];
}

/** Use is judged over this window, and only once readings span it. */
export const SUSTAINED_MS = 2 * 60_000;
/** Most of one core, held for the whole window. */
export const BUSY_CPU_PERCENT = 80;
/** About 6 h 40 min from full: a light workload drains under half that. */
export const FAST_DRAIN_PERCENT_PER_HOUR = 15;
/** Draining this many times faster than usual is worth a look, even if the rate itself is moderate. */
export const FASTER_THAN_USUAL = 1.6;
const USUAL_MIN_HOURS = 2;
const DISPLAY_AWAKE_SECS = 10 * 60;
const SYSTEM_AWAKE_SECS = 30 * 60;
/** macOS needs about this much free for a major update, and far less leaves swap and saves at risk. */
const DISK_WARNING_BYTES = 25e9;
const DISK_CRITICAL_BYTES = 10e9;
const WORN_CAPACITY_PERCENT = 80;

const ORDER: Record<Level, number> = { critical: 0, warning: 1, info: 2 };

/** Average CPU per app over the last `SUSTAINED_MS`, once the readings span it. */
/** The readings of the last `SUSTAINED_MS`, once they span most of it. */
function spanning<T extends { t: number }>(readings: T[], now: number): T[] | null {
  const recent = readings.filter((r) => r.t >= now - SUSTAINED_MS - 5_000);
  return recent.length >= 3 && recent[recent.length - 1].t - recent[0].t >= SUSTAINED_MS * 0.9 ? recent : null;
}

export function sustainedCpu(readings: CpuReading[], now: number): { name: string; cpu: number; own: boolean }[] {
  const recent = spanning(readings, now);
  if (!recent) return [];
  const totals = new Map<string, { sum: number; own: boolean }>();
  for (const r of recent) {
    for (const a of r.apps) {
      const t = totals.get(a.name) ?? { sum: 0, own: a.own };
      // An app missing from a reading was not among the busiest then, so it counts as idle.
      t.sum += a.cpu ?? 0;
      totals.set(a.name, t);
    }
  }
  return [...totals].map(([name, t]) => ({ name, cpu: t.sum / recent.length, own: t.own }));
}

/** The drain to judge: the gauge average over this run on battery, else the last minutes of the gauge's current. */
export function sustainedDrain(i: Pick<Inputs, "history" | "drainReadings" | "now">): { rate: number; basis: string } | null {
  const avg = i.history?.average;
  if (avg && avg.confidence !== "low") {
    return { rate: avg.percentPerHour, basis: `battery gauge, averaged over ${formatMinutes(avg.windowMinutes)} on battery` };
  }
  const recent = spanning(i.drainReadings, i.now);
  if (!recent) return null;
  const rate = recent.reduce((s, r) => s + r.v, 0) / recent.length;
  return { rate, basis: "battery gauge's current, over the last 2 min" };
}

function heavyApps(s: SystemSnapshot | undefined, by: "cpu" | "power" | "memory", n = 3): string[] {
  if (!s) return [];
  const value = (a: SystemSnapshot["apps"]["apps"][number]) =>
    by === "cpu" ? a.cpuPercent : by === "power" ? a.powerW : a.memoryBytes;
  const floor = by === "cpu" ? 20 : by === "power" ? 0.3 : 500 * 1024 ** 2;
  return [...s.apps.apps]
    .filter((a) => (value(a) ?? 0) >= floor)
    .sort((a, b) => (value(b) ?? 0) - (value(a) ?? 0))
    .slice(0, n)
    .map((a) => {
      const v = value(a) ?? 0;
      const shown = by === "cpu" ? `${Math.round(v)}% CPU` : by === "power" ? formatWatts(v) : formatBytes(v, 1024);
      return `${a.name} (${shown})`;
    });
}

function cpuInsights(i: Inputs): Insight[] {
  return sustainedCpu(i.cpuReadings, i.now)
    .filter((a) => a.cpu >= BUSY_CPU_PERCENT)
    .sort((a, b) => b.cpu - a.cpu)
    .map((a) => ({
      id: `cpu:${a.name}`,
      level: "warning" as const,
      title: `${a.name} has kept the CPU busy for 2 minutes`,
      detail: `It averaged ${Math.round(a.cpu)}% of a core${a.cpu >= 150 ? ` (about ${(a.cpu / 100).toFixed(1)} cores)` : ""}, which drains the battery and warms the Mac.`,
      action: a.own
        ? `If you are not using it right now, quit ${a.name}. MacPilot never quits apps for you.`
        : "It belongs to macOS or another user, so it cannot be quit like an app. Such work usually finishes on its own.",
      source: a.own ? "CPU time from the kernel, read at every refresh." : "CPU time from ps, read at every refresh.",
    }));
}

function drainInsights(i: Inputs): Insight[] {
  const b = i.battery;
  if (!b?.present || b.externalPower !== false) return [];
  const drain = sustainedDrain(i);
  if (!drain) return [];
  const usual = i.history?.usual;
  const ratio = usual && usual.hours >= USUAL_MIN_HOURS && usual.percentPerHour > 0 ? drain.rate / usual.percentPerHour : null;
  const fast = drain.rate >= FAST_DRAIN_PERCENT_PER_HOUR;
  const faster = ratio !== null && ratio >= FASTER_THAN_USUAL;
  if (!fast && !faster) return [];
  const left =
    b.timeRemainingMin !== null
      ? `macOS expects about ${formatMinutes(b.timeRemainingMin)} left.`
      : b.percent !== null
        ? `At this rate ${b.percent}% lasts about ${formatMinutes((b.percent / drain.rate) * 60)}.`
        : "";
  const users = heavyApps(i.snapshot, "power");
  const busy = heavyApps(i.snapshot, "cpu");
  const culprits = users.length > 0 ? `Biggest power users: ${list(users)}.` : busy.length > 0 ? `Busiest apps: ${list(busy)}.` : "";
  return [
    {
      id: "drain",
      level: fast ? "warning" : "info",
      title: `Battery is draining ${fast ? "fast" : "faster than usual"}: ${drain.rate.toFixed(1)}% an hour`,
      detail: [
        faster ? `That is ${ratio!.toFixed(1)} times your usual ${usual!.percentPerHour.toFixed(1)}% an hour.` : "",
        left,
        b.systemPowerW !== null ? `The Mac is drawing ${formatWatts(b.systemPowerW)}.` : "",
        culprits,
      ]
        .filter(Boolean)
        .join(" "),
      action: "Lower the screen brightness, which is usually the largest single draw, and quit apps you are not using.",
      source: `Measured by the ${drain.basis}.`,
    },
  ];
}

function awakeInsights(i: Inputs): Insight[] {
  const onBattery = i.battery?.externalPower === false;
  // Assertions with a timeout release themselves, as caffeinate -t does.
  const lasting = (i.blockers ?? []).filter((b) => b.endsInSecs === null && b.heldSecs !== null).sort(stronger);
  const out: Insight[] = [];
  for (const app of new Set(lasting.map((b) => b.app))) {
    const longest = (keeps: SleepBlocker["keeps"]) => lasting.find((b) => b.app === app && b.keeps === keeps);
    const display = longest("display");
    const system = longest("system");
    if (display && display.heldSecs! >= DISPLAY_AWAKE_SECS) {
      out.push({
        id: `awake:${app}`,
        level: onBattery ? "warning" : "info",
        title: `${app} is keeping your display on`,
        detail: `For ${formatMinutes(display.heldSecs! / 60)} ("${display.reason}"). The display is one of the largest drains on a MacBook.`,
        action: `Quit ${app}, or end what it is doing, when you no longer need it.`,
        source: "Power assertions macOS lists in pmset -g assertions.",
      });
    } else if (system && system.heldSecs! >= SYSTEM_AWAKE_SECS) {
      out.push({
        id: `awake:${app}`,
        level: "info",
        title: `${app} is keeping your Mac awake`,
        detail: `For ${formatMinutes(system.heldSecs! / 60)} ("${system.reason}"). The display can still turn off, but the Mac will not sleep on its own.`,
        action: `Before you leave the Mac, stop what ${app} is doing.`,
        source: "Power assertions macOS lists in pmset -g assertions.",
      });
    }
  }
  return out;
}

function memoryInsights(i: Inputs): Insight[] {
  const m = i.snapshot?.memory;
  if (!m || (m.pressure !== "warning" && m.pressure !== "critical")) return [];
  const largest = heavyApps(i.snapshot, "memory");
  return [
    {
      id: "memory",
      level: m.pressure === "critical" ? "critical" : "warning",
      title: m.pressure === "critical" ? "Your Mac is out of memory" : "Memory is under pressure",
      detail: [
        `${formatBytes(m.usedBytes, 1024)} of ${formatBytes(m.totalBytes, 1024)} is in use${m.swapUsedBytes > 0 ? ` and ${formatBytes(m.swapUsedBytes, 1024)} is swapped to disk` : ""}, so apps slow down while macOS compresses and swaps.`,
        largest.length > 0 ? `Largest of your apps: ${list(largest)}.` : "",
      ]
        .filter(Boolean)
        .join(" "),
      action: "Quit apps you are not using, starting with the largest.",
      source: "Memory pressure from the kernel, the level Activity Monitor graphs.",
    },
  ];
}

function thermalInsights(i: Inputs): Insight[] {
  const t = i.snapshot?.cpu.thermal;
  if (!t || t === "nominal") return [];
  const busy = heavyApps(i.snapshot, "cpu");
  const hot = t !== "moderate";
  return [
    {
      id: "thermal",
      level: hot ? "critical" : "warning",
      title: hot ? "Your Mac is hot and macOS is slowing it down" : "Your Mac is warm and may slow down",
      detail: `macOS reports ${t} thermal pressure.${busy.length > 0 ? ` Busiest apps: ${list(busy)}.` : ""}`,
      action: "Keep the vents clear and the Mac on a hard surface, and quit heavy apps you do not need.",
      source: "Thermal pressure macOS publishes to apps.",
    },
  ];
}

function diskInsights(i: Inputs): Insight[] {
  const d = i.snapshot?.disk;
  if (!d || d.availableBytes >= DISK_WARNING_BYTES) return [];
  const critical = d.availableBytes < DISK_CRITICAL_BYTES;
  return [
    {
      id: "disk",
      level: critical ? "critical" : "warning",
      title: `Startup disk is almost full: ${formatBytes(d.availableBytes)} available`,
      detail: `macOS needs free space for updates, swap and caches; with little left, apps slow down or fail to save.`,
      action: "Empty the Trash and review large files in System Settings > General > Storage.",
      source: "Available space as Finder counts it.",
    },
  ];
}

function healthInsights(i: Inputs): Insight[] {
  const b = i.battery;
  if (!b?.present) return [];
  if (b.condition && !isHealthyCondition(b.condition)) {
    return [
      {
        id: "battery-condition",
        level: "warning",
        title: "macOS recommends servicing the battery",
        detail: `macOS rates the battery's condition as "${b.condition}"${b.appleMaxCapacityPercent !== null ? `, with ${b.appleMaxCapacityPercent}% of its original capacity` : ""}.`,
        action: "See System Settings > Battery > Battery Health, and an Apple service provider if runtime matters to you.",
        source: "Battery condition from system_profiler.",
      },
    ];
  }
  if (b.appleMaxCapacityPercent !== null && b.appleMaxCapacityPercent < WORN_CAPACITY_PERCENT) {
    return [
      {
        id: "battery-capacity",
        level: "info",
        title: `Battery holds ${b.appleMaxCapacityPercent}% of its original charge`,
        detail: `Below ${WORN_CAPACITY_PERCENT}%, a full charge lasts noticeably less than when new.${b.cycleCount !== null ? ` It has ${b.cycleCount} charge cycles.` : ""}`,
        source: "Maximum capacity as System Settings shows it.",
      },
    ];
  }
  return [];
}

function portInsights(i: Inputs): Insight[] {
  return servers(i.ports ?? [])
    .filter((s) => s.network && !isAppleService(s))
    .map((s) => {
      const known = describeServer(s);
      return {
        id: `port:${s.pid}`,
        level: "info" as const,
        title: `${s.app} is reachable from your network on port${s.ports.length > 1 ? "s" : ""} ${list(s.ports.map(String))}`,
        detail: `${known ? `${known} ` : ""}Other devices on your network can connect to it unless the macOS firewall blocks it.`,
        action: known ? "Stop it when you do not need it." : "If only this Mac needs it, start it bound to 127.0.0.1.",
        source: "Listening sockets from lsof.",
      };
    });
}

export function analyze(i: Inputs): Insight[] {
  return [
    ...thermalInsights(i),
    ...memoryInsights(i),
    ...diskInsights(i),
    ...cpuInsights(i),
    ...drainInsights(i),
    ...awakeInsights(i),
    ...healthInsights(i),
    ...portInsights(i),
  ].sort((a, b) => ORDER[a.level] - ORDER[b.level]);
}

/** What was checked, for the all-clear line; a check without data is not claimed. */
export function checked(i: Inputs): string[] {
  const s = i.snapshot;
  return [
    s && "CPU use",
    s?.memory.pressure && "memory pressure",
    s?.cpu.thermal && "heat",
    s?.disk && "startup disk",
    i.battery?.present && "battery",
    i.blockers && "sleep",
    i.ports && "open ports",
  ].filter((c): c is string => typeof c === "string");
}
