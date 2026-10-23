import type { SleepBlocker } from "./schemas";

export interface AwakeApp {
  app: string;
  /** The display if any of the app's assertions keeps it on, else the Mac. */
  keeps: SleepBlocker["keeps"];
  reasons: string[];
  heldSecs: number | null;
  /** Set when every assertion releases itself: the time until the last one does. */
  endsInSecs: number | null;
  /** Only Mac-awake assertions that release themselves or were taken under a minute ago. */
  brief: boolean;
}

const BRIEF_SECS = 60;

type Hold = Pick<SleepBlocker, "keeps" | "heldSecs">;

/** Display before Mac-awake, then the longest held. */
export const stronger = (a: Hold, b: Hold) =>
  Number(a.keeps === "system") - Number(b.keeps === "system") || (b.heldSecs ?? 0) - (a.heldSecs ?? 0);

const isBrief = (b: SleepBlocker) => b.keeps === "system" && (b.endsInSecs !== null || (b.heldSecs ?? 0) < BRIEF_SECS);

/** Assertions grouped by the app they keep the Mac awake for, lasting ones first. */
export function byApp(blockers: SleepBlocker[]): AwakeApp[] {
  const groups = new Map<string, SleepBlocker[]>();
  for (const b of blockers) groups.set(b.app, [...(groups.get(b.app) ?? []), b]);
  return [...groups]
    .map(([app, list]) => {
      const sorted = [...list].sort(stronger);
      const held = list.flatMap((b) => (b.heldSecs === null ? [] : [b.heldSecs]));
      const timed = list.every((b) => b.endsInSecs !== null);
      return {
        app,
        keeps: sorted[0].keeps,
        reasons: [...new Set(sorted.map((b) => b.reason))],
        heldSecs: held.length > 0 ? Math.max(...held) : null,
        endsInSecs: timed ? Math.max(...list.map((b) => b.endsInSecs!)) : null,
        brief: list.every(isBrief),
      };
    })
    .sort((a, b) => Number(a.brief) - Number(b.brief) || stronger(a, b));
}
