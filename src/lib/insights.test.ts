import { describe, expect, it } from "vitest";
import { analyze, checked, sustainedCpu, sustainedDrain, type CpuReading, type Inputs } from "./insights";
import type { BatteryHistory, BatteryInfo, Listener, SleepBlocker, SystemSnapshot } from "./schemas";
import snapshot from "./fixtures/snapshot.json";

const NOW = 10_000_000;
const snap = (): SystemSnapshot => structuredClone(snapshot) as SystemSnapshot;

const battery = (over: Partial<BatteryInfo> = {}): BatteryInfo => ({
  present: true,
  percent: 40,
  state: "discharging",
  externalPower: false,
  timeRemainingMin: 300,
  timeRemainingKind: "untilEmpty",
  cycleCount: 556,
  appleMaxCapacityPercent: 85,
  condition: "Normal",
  temperatureC: 31,
  systemPowerW: 6,
  adapterPowerW: 0,
  batteryPowerW: -6,
  adapterRatingW: null,
  drainPercentPerHour: 8,
  drainCalibrated: true,
  notes: [],
  ...over,
});

const history = (over: Partial<BatteryHistory> = {}): BatteryHistory => ({
  samples: [],
  average: null,
  averageUnavailableReason: "Shown after 10 min on battery; 2 min so far.",
  usual: null,
  sampleIntervalSecs: 60,
  maxGapSecs: 900,
  persistent: true,
  ...over,
});

/** Readings every 2 s over the last `secs`, each with the given apps. */
const readings = (secs: number, apps: CpuReading["apps"]): CpuReading[] =>
  Array.from({ length: secs / 2 + 1 }, (_, i) => ({ t: NOW - secs * 1000 + i * 2000, apps }));

const inputs = (over: Partial<Inputs> = {}): Inputs => ({
  now: NOW,
  snapshot: snap(),
  battery: battery(),
  history: history(),
  blockers: [],
  ports: [],
  cpuReadings: [],
  drainReadings: [],
  ...over,
});

const ids = (i: Inputs) => analyze(i).map((x) => `${x.level}:${x.id}`);

describe("a calm Mac", () => {
  it("needs no attention, and says what was checked", () => {
    expect(analyze(inputs())).toEqual([]);
    expect(checked(inputs())).toEqual(["CPU use", "memory pressure", "heat", "startup disk", "battery", "sleep", "open ports"]);
    expect(checked({ ...inputs(), ports: undefined, blockers: undefined })).not.toContain("open ports");
  });
});

describe("CPU", () => {
  it("judges use only over two minutes of readings", () => {
    const busy = [{ name: "node", cpu: 120, own: true }];
    expect(sustainedCpu(readings(60, busy), NOW)).toEqual([]);
    expect(sustainedCpu(readings(120, busy), NOW)).toEqual([{ name: "node", cpu: 120, own: true }]);
  });

  it("counts an app missing from a reading as idle", () => {
    const r = readings(120, [{ name: "node", cpu: 100, own: true }]);
    r.slice(0, 30).forEach((x) => (x.apps = []));
    expect(sustainedCpu(r, NOW)[0].cpu).toBeLessThan(80);
  });

  it("flags an app that keeps a core busy, with advice that fits who owns it", () => {
    const r = readings(150, [
      { name: "node", cpu: 95, own: true },
      { name: "mds_stores", cpu: 85, own: false },
      { name: "Terminal", cpu: 30, own: true },
    ]);
    const found = analyze(inputs({ cpuReadings: r }));
    expect(found.map((f) => f.id)).toEqual(["cpu:node", "cpu:mds_stores"]);
    expect(found[0].action).toContain("quit node");
    expect(found[1].action).toContain("belongs to macOS");
  });
});

describe("battery drain", () => {
  it("uses the run's average once it is trustworthy, else two minutes of gauge readings", () => {
    const avg = { percentPerHour: 18, averageMa: 1300, windowMinutes: 30, sampleCount: 31, projectedMinutesToEmpty: 120, confidence: "medium" as const, confidenceReason: "" };
    expect(sustainedDrain({ now: NOW, drainReadings: [], history: history({ average: avg }) })?.rate).toBe(18);
    expect(sustainedDrain({ now: NOW, drainReadings: [], history: history({ average: { ...avg, confidence: "low" } }) })).toBeNull();
    const gauge = Array.from({ length: 13 }, (_, i) => ({ t: NOW - 120_000 + i * 10_000, v: 20 }));
    expect(sustainedDrain({ now: NOW, drainReadings: gauge, history: history() })?.rate).toBe(20);
    expect(sustainedDrain({ now: NOW, drainReadings: gauge.slice(-3), history: history() })).toBeNull();
  });

  it("warns when the drain is fast, or much faster than usual", () => {
    const gauge = (v: number) => Array.from({ length: 13 }, (_, i) => ({ t: NOW - 120_000 + i * 10_000, v }));
    expect(ids(inputs({ drainReadings: gauge(16) }))).toEqual(["warning:drain"]);
    expect(ids(inputs({ drainReadings: gauge(12) }))).toEqual([]);
    const usual = history({ usual: { percentPerHour: 6, hours: 10 } });
    const found = analyze(inputs({ drainReadings: gauge(12), history: usual }));
    expect(found.map((f) => `${f.level}:${f.id}`)).toEqual(["info:drain"]);
    expect(found[0].title).toBe("Battery is draining faster than usual: 12.0% an hour");
    expect(found[0].detail).toContain("2.0 times your usual 6.0%");
    expect(analyze(inputs({ drainReadings: gauge(16), history: usual }))[0].title).toBe("Battery is draining fast: 16.0% an hour");
    expect(ids(inputs({ drainReadings: gauge(30), battery: battery({ externalPower: true }) }))).toEqual([]);
  });
});

describe("sleep", () => {
  const blocker = (over: Partial<SleepBlocker>): SleepBlocker => ({
    pid: 1,
    process: "zoom.us",
    forPid: null,
    app: "zoom.us",
    keeps: "display",
    reason: "Disabling display sleep",
    heldSecs: 3600,
    endsInSecs: null,
    ...over,
  });

  it("warns about a display kept on while on battery, and only notes it on power", () => {
    expect(ids(inputs({ blockers: [blocker({})] }))).toEqual(["warning:awake:zoom.us"]);
    expect(ids(inputs({ blockers: [blocker({})], battery: battery({ externalPower: true }) }))).toEqual(["info:awake:zoom.us"]);
  });

  it("ignores short or self-ending assertions and merges an app's assertions", () => {
    const blockers = [
      blocker({ heldSecs: 120 }),
      blocker({ app: "caffeinate", process: "caffeinate", keeps: "system", heldSecs: 7200, endsInSecs: 60 }),
      blocker({ app: "Google Chrome", process: "coreaudiod", keeps: "system", reason: "Audio in use (BuiltInSpeakerDevice)", heldSecs: 2400 }),
      blocker({ app: "Google Chrome", process: "Google Chrome", keeps: "system", reason: "Playing audio", heldSecs: 2700 }),
    ];
    const found = analyze(inputs({ blockers }));
    expect(found.map((f) => f.id)).toEqual(["awake:Google Chrome"]);
    expect(found[0].title).toBe("Google Chrome is keeping your Mac awake");
    expect(found[0].detail).toContain("45 min");
  });
});

describe("memory, heat and disk", () => {
  it("follows macOS's own pressure levels", () => {
    const s = snap();
    s.memory.pressure = "warning";
    expect(ids(inputs({ snapshot: s }))).toEqual(["warning:memory"]);
    s.memory.pressure = "critical";
    expect(ids(inputs({ snapshot: s }))).toEqual(["critical:memory"]);
    s.memory.pressure = "normal";
    s.cpu.thermal = "moderate";
    expect(ids(inputs({ snapshot: s }))).toEqual(["warning:thermal"]);
    s.cpu.thermal = "heavy";
    expect(ids(inputs({ snapshot: s }))).toEqual(["critical:thermal"]);
  });

  it("warns as the startup disk fills", () => {
    const s = snap();
    s.disk = { name: "Macintosh HD", totalBytes: 500e9, availableBytes: 15e9 };
    expect(ids(inputs({ snapshot: s }))).toEqual(["warning:disk"]);
    s.disk.availableBytes = 8e9;
    expect(ids(inputs({ snapshot: s }))).toEqual(["critical:disk"]);
  });
});

describe("battery health", () => {
  it("passes macOS's service recommendation on, and notes a worn battery", () => {
    expect(ids(inputs({ battery: battery({ condition: "Service Recommended" }) }))).toEqual(["warning:battery-condition"]);
    expect(ids(inputs({ battery: battery({ appleMaxCapacityPercent: 78 }) }))).toEqual(["info:battery-capacity"]);
  });
});

describe("open ports", () => {
  const port = (over: Partial<Listener>): Listener => ({
    port: 5173,
    pid: 4312,
    process: "node",
    app: "node",
    addresses: ["*"],
    network: true,
    folder: "~/Documents/app",
    path: "/opt/homebrew/bin/node",
    ...over,
  });

  it("notes servers other devices can reach, but not macOS's own or local ones", () => {
    const ports = [
      port({}),
      port({ port: 5174 }),
      port({ pid: 445, process: "ControlCenter", port: 5000, path: "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter" }),
      port({ pid: 9, port: 1420, network: false, addresses: ["127.0.0.1"] }),
    ];
    const found = analyze(inputs({ ports }));
    expect(found.map((f) => f.id)).toEqual(["port:4312"]);
    expect(found[0].title).toBe("node is reachable from your network on ports 5173 and 5174");
  });
});

it("orders critical before warning before notes", () => {
  const s = snap();
  s.cpu.thermal = "heavy";
  const found = ids(inputs({ snapshot: s, battery: battery({ appleMaxCapacityPercent: 70 }), drainReadings: Array.from({ length: 13 }, (_, i) => ({ t: NOW - 120_000 + i * 10_000, v: 30 })) }));
  expect(found).toEqual(["critical:thermal", "warning:drain", "info:battery-capacity"]);
});
