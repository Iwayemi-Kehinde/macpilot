import { describe, expect, it } from "vitest";
import { validate, ValidationError } from "./api";
import { BatteryHistorySchema, BatteryInfoSchema, ListenerSchema, SleepBlockerSchema, SystemSnapshotSchema } from "./schemas";
import snapshot from "./fixtures/snapshot.json";

// Mirrors the Rust combine test built from the real pmset/ioreg/system_profiler fixtures.
const battery = {
  present: true,
  percent: 86,
  state: "discharging",
  externalPower: false,
  timeRemainingMin: 726,
  timeRemainingKind: "untilEmpty",
  cycleCount: 555,
  appleMaxCapacityPercent: 85,
  condition: "Normal",
  temperatureC: 30.77,
  systemPowerW: 11.78,
  adapterPowerW: 0,
  batteryPowerW: -11.78,
  adapterRatingW: null,
  drainPercentPerHour: 14.38,
  drainCalibrated: true,
  notes: [],
};

const clone = <T>(v: T): T => structuredClone(v);

describe("SystemSnapshotSchema", () => {
  it("accepts a real snapshot from the Rust layer", () => {
    expect(SystemSnapshotSchema.safeParse(snapshot).success).toBe(true);
  });

  it("accepts a first sample and readings macOS did not give", () => {
    const s = clone(snapshot);
    s.cpu.usagePercent = null as unknown as number;
    s.cpu.thermal = null as unknown as string;
    s.network.rxBytesPerSec = null as unknown as number;
    s.memory.pressure = null as unknown as string;
    s.disk = null as unknown as typeof s.disk;
    s.apps.apps.forEach((a) => {
      a.cpuPercent = null as unknown as number;
    });
    expect(SystemSnapshotSchema.safeParse(s).success).toBe(true);
  });

  it("accepts a snapshot whose process list could not be read", () => {
    const s = { ...clone(snapshot), apps: { totalProcesses: 0, systemProcesses: 0, error: "/bin/ps did not finish within 5 s", apps: [] } };
    expect(SystemSnapshotSchema.safeParse(s).success).toBe(true);
  });

  it("rejects impossible values instead of displaying them", () => {
    const over = clone(snapshot);
    over.memory.usedBytes = over.memory.totalBytes + 1;
    expect(SystemSnapshotSchema.safeParse(over).success).toBe(false);

    const negative = clone(snapshot);
    negative.cpu.usagePercent = -3;
    expect(SystemSnapshotSchema.safeParse(negative).success).toBe(false);

    const system = clone(snapshot);
    system.apps.systemProcesses = system.apps.totalProcesses + 1;
    expect(SystemSnapshotSchema.safeParse(system).success).toBe(false);

    const own = clone(snapshot);
    own.apps.apps[0].ownProcessCount = own.apps.apps[0].processCount + 1;
    expect(SystemSnapshotSchema.safeParse(own).success).toBe(false);

    const heat = clone(snapshot);
    heat.cpu.thermal = "toasty";
    expect(SystemSnapshotSchema.safeParse(heat).success).toBe(false);

    const missing = clone(snapshot) as Record<string, unknown>;
    delete missing.memory;
    expect(SystemSnapshotSchema.safeParse(missing).success).toBe(false);
  });

  it("reports the failing path in ValidationError", () => {
    const bad = clone(snapshot);
    bad.disk.availableBytes = bad.disk.totalBytes + 1;
    expect(() => validate("get_system_snapshot", SystemSnapshotSchema, bad)).toThrow(ValidationError);
    expect(() => validate("get_system_snapshot", SystemSnapshotSchema, bad)).toThrow(/disk/);
  });
});

describe("BatteryInfoSchema", () => {
  it("accepts the real battery reading", () => {
    expect(BatteryInfoSchema.safeParse(battery).success).toBe(true);
  });

  it("accepts a Mac without a battery, with every value unavailable", () => {
    const none = {
      ...battery,
      present: false,
      percent: null,
      state: "unknown",
      timeRemainingMin: null,
      timeRemainingKind: null,
      cycleCount: null,
      appleMaxCapacityPercent: null,
      condition: null,
      temperatureC: null,
      systemPowerW: null,
      adapterPowerW: null,
      batteryPowerW: null,
      drainPercentPerHour: null,
      notes: ["No internal battery reported by macOS."],
    };
    expect(BatteryInfoSchema.safeParse(none).success).toBe(true);
  });

  it("rejects out-of-range or inconsistent readings", () => {
    expect(BatteryInfoSchema.safeParse({ ...battery, percent: 140 }).success).toBe(false);
    expect(BatteryInfoSchema.safeParse({ ...battery, state: "exploding" }).success).toBe(false);
    expect(BatteryInfoSchema.safeParse({ ...battery, timeRemainingKind: null }).success).toBe(false);
    expect(BatteryInfoSchema.safeParse({ ...battery, systemPowerW: -2 }).success).toBe(false);
    expect(BatteryInfoSchema.safeParse({ ...battery, drainPercentPerHour: -1 }).success).toBe(false);
  });
});

describe("BatteryHistorySchema", () => {
  const base = {
    samples: [
      { ts: 1000, percent: 90, charging: false, externalPower: false },
      { ts: 1060, percent: 90, charging: false, externalPower: false },
    ],
    average: null,
    averageUnavailableReason: "Shown after 10 min on battery; 1 min so far.",
    usual: { percentPerHour: 8.1, hours: 12.5 },
    sampleIntervalSecs: 60,
    maxGapSecs: 900,
    persistent: true,
  };

  it("accepts ordered samples", () => {
    expect(BatteryHistorySchema.safeParse(base).success).toBe(true);
  });

  it("rejects samples out of order", () => {
    const reversed = { ...base, samples: [...base.samples].reverse() };
    expect(BatteryHistorySchema.safeParse(reversed).success).toBe(false);
  });
});

describe("activity schemas", () => {
  it("accepts a sleep blocker held for an app and an open port", () => {
    const blocker = {
      pid: 172,
      process: "coreaudiod",
      forPid: 774,
      app: "Google Chrome",
      keeps: "system",
      reason: "Audio in use (BuiltInSpeakerDevice)",
      heldSecs: 165,
      endsInSecs: null,
    };
    expect(SleepBlockerSchema.safeParse(blocker).success).toBe(true);
    expect(SleepBlockerSchema.safeParse({ ...blocker, keeps: "everything" }).success).toBe(false);

    const port = {
      port: 5173,
      pid: 4312,
      process: "node",
      app: "node",
      addresses: ["127.0.0.1", "::1"],
      network: false,
      folder: "~/Documents/app",
      path: "/opt/homebrew/bin/node",
    };
    expect(ListenerSchema.safeParse(port).success).toBe(true);
    expect(ListenerSchema.safeParse({ ...port, port: 70000 }).success).toBe(false);
    expect(ListenerSchema.safeParse({ ...port, addresses: [] }).success).toBe(false);
  });
});
