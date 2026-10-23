import { describe, expect, it } from "vitest";
import { validate, ValidationError } from "./api";
import { BatteryHistorySchema, BatteryInfoSchema, SystemSnapshotSchema } from "./schemas";
import snapshot from "./fixtures/snapshot.json";

// Mirrors the Rust combine test built from the real pmset/ioreg/system_profiler fixtures.
const battery = {
  present: true,
  powerSource: "Battery Power",
  percent: 86,
  state: "discharging",
  externalPower: false,
  timeRemainingMin: 726,
  timeRemainingKind: "untilEmpty",
  cycleCount: 555,
  designCapacityMah: 8694,
  fullChargeCapacityMah: 7627,
  currentChargeMah: 6230,
  rawCapacityPercent: 87.7,
  appleMaxCapacityPercent: 85,
  condition: "Good",
  voltageMv: 12106,
  amperageMa: -1024,
  powerW: -12.4,
  temperatureC: 30.77,
  notes: [],
};

const clone = <T>(v: T): T => structuredClone(v);

describe("SystemSnapshotSchema", () => {
  it("accepts a real snapshot from the Rust layer", () => {
    expect(SystemSnapshotSchema.safeParse(snapshot).success).toBe(true);
  });

  it("accepts the first sample, where rates are not measured yet", () => {
    const s = clone(snapshot);
    s.cpu.usagePercent = null as unknown as number;
    s.cpu.perCorePercent = [];
    s.network.forEach((n) => {
      n.rxBytesPerSec = null as unknown as number;
      n.txBytesPerSec = null as unknown as number;
    });
    expect(SystemSnapshotSchema.safeParse(s).success).toBe(true);
  });

  it("accepts a snapshot whose process list or memory breakdown could not be read", () => {
    const s = {
      ...clone(snapshot),
      processes: { total: 0, system: 0, error: "/bin/ps did not finish within 5 s", topByCpu: [], topByMemory: [] },
    };
    s.memory.breakdown = null as unknown as typeof s.memory.breakdown;
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
    system.processes.system = system.processes.total + 1;
    expect(SystemSnapshotSchema.safeParse(system).success).toBe(false);


    const missing = clone(snapshot) as Record<string, unknown>;
    delete missing.memory;
    expect(SystemSnapshotSchema.safeParse(missing).success).toBe(false);
  });

  it("reports the failing path in ValidationError", () => {
    const bad = clone(snapshot);
    bad.disks[0].availableBytes = bad.disks[0].totalBytes + 1;
    expect(() => validate("get_system_snapshot", SystemSnapshotSchema, bad)).toThrow(ValidationError);
    expect(() => validate("get_system_snapshot", SystemSnapshotSchema, bad)).toThrow(/disks\.0/);
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
      designCapacityMah: null,
      fullChargeCapacityMah: null,
      currentChargeMah: null,
      rawCapacityPercent: null,
      appleMaxCapacityPercent: null,
      condition: null,
      voltageMv: null,
      amperageMa: null,
      powerW: null,
      temperatureC: null,
      notes: ["No internal battery reported by macOS."],
    };
    expect(BatteryInfoSchema.safeParse(none).success).toBe(true);
  });

  it("rejects out-of-range or inconsistent readings", () => {
    expect(BatteryInfoSchema.safeParse({ ...battery, percent: 140 }).success).toBe(false);
    expect(BatteryInfoSchema.safeParse({ ...battery, state: "exploding" }).success).toBe(false);
    expect(BatteryInfoSchema.safeParse({ ...battery, timeRemainingKind: null }).success).toBe(false);
  });
});

describe("BatteryHistorySchema", () => {
  const base = {
    samples: [
      { ts: 1000, percent: 90, charging: false, externalPower: false, chargeMah: 7000, fullChargeMah: 7620, amperageMa: -900, voltageMv: 12300 },
      { ts: 1060, percent: 90, charging: false, externalPower: false, chargeMah: null, fullChargeMah: null, amperageMa: null, voltageMv: null },
    ],
    discharge: null,
    dischargeUnavailableReason: "Collecting data",
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
