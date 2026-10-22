import type { BatterySample, ChargeState } from "./schemas";

export const STATE_LABEL: Record<ChargeState, string> = {
  charging: "Charging",
  discharging: "On battery",
  charged: "Charged",
  acAttached: "Plugged in, not charging",
  finishingCharge: "Finishing charge",
  unknown: "State unknown",
};

/** Time ranges (unix ms) during which the adapter was connected, for chart shading. */
export function externalPowerRanges(
  samples: BatterySample[],
  maxGapSecs: number,
): { start: number; end: number }[] {
  const ranges: { start: number; end: number }[] = [];
  samples.forEach((s, i) => {
    if (!s.externalPower) return;
    const next = samples[i + 1];
    // Extend to the next sample only when it follows without a gap.
    const endSecs = next && next.ts - s.ts <= maxGapSecs ? next.ts : s.ts;
    const last = ranges[ranges.length - 1];
    if (last && last.end === s.ts * 1000) last.end = endSecs * 1000;
    else ranges.push({ start: s.ts * 1000, end: endSecs * 1000 });
  });
  return ranges;
}

/** Keeps at most one sample per `stepSecs`, newest first, for the table view. */
export function thinSamples(samples: BatterySample[], stepSecs: number): BatterySample[] {
  const out: BatterySample[] = [];
  for (let i = samples.length - 1; i >= 0; i--) {
    const s = samples[i];
    if (out.length === 0 || out[out.length - 1].ts - s.ts >= stepSecs) out.push(s);
  }
  return out;
}

export function isHealthyCondition(condition: string): boolean {
  return ["good", "normal"].includes(condition.trim().toLowerCase());
}
