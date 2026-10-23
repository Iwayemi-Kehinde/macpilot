import { describe, expect, it } from "vitest";
import { externalPowerRanges, isHealthyCondition } from "./battery";
import type { BatterySample } from "./schemas";

const sample = (ts: number, externalPower: boolean): BatterySample => ({
  ts,
  percent: 50,
  charging: externalPower,
  externalPower,
});

describe("externalPowerRanges", () => {
  it("merges consecutive plugged-in samples into one range", () => {
    const samples = [sample(0, false), sample(60, true), sample(120, true), sample(180, false)];
    expect(externalPowerRanges(samples, 900)).toEqual([{ start: 60_000, end: 180_000 }]);
  });

  it("does not shade across a gap in sampling", () => {
    const samples = [sample(0, true), sample(5000, true)];
    expect(externalPowerRanges(samples, 900)).toEqual([
      { start: 0, end: 0 },
      { start: 5_000_000, end: 5_000_000 },
    ]);
  });

  it("returns nothing when always on battery", () => {
    expect(externalPowerRanges([sample(0, false), sample(60, false)], 900)).toEqual([]);
  });
});

describe("isHealthyCondition", () => {
  it("treats Normal as healthy", () => {
    expect(isHealthyCondition("Normal")).toBe(true);
    expect(isHealthyCondition("Service Recommended")).toBe(false);
  });
});
