import { describe, expect, it } from "vitest";
import { byApp } from "./awake";
import type { SleepBlocker } from "./schemas";

const blocker = (over: Partial<SleepBlocker>): SleepBlocker => ({
  pid: 1,
  process: "zoom.us",
  forPid: null,
  app: "zoom.us",
  keeps: "system",
  reason: "In a meeting",
  heldSecs: 600,
  endsInSecs: null,
  ...over,
});

describe("byApp", () => {
  it("merges an app's assertions under the strongest and longest", () => {
    const [zoom] = byApp([
      blocker({ process: "coreaudiod", reason: "Audio in use (BuiltInSpeakerDevice)", heldSecs: 300 }),
      blocker({ keeps: "display", reason: "Disabling display sleep", heldSecs: 900 }),
      blocker({ heldSecs: null }),
    ]);
    expect(zoom).toEqual({
      app: "zoom.us",
      keeps: "display",
      reasons: ["Disabling display sleep", "Audio in use (BuiltInSpeakerDevice)", "In a meeting"],
      heldSecs: 900,
      endsInSecs: null,
      brief: false,
    });
  });

  it("marks apps whose assertions end on their own or just started as brief, and lists them last", () => {
    const apps = byApp([
      blocker({ app: "caffeinate", endsInSecs: 240 }),
      blocker({ app: "akd", heldSecs: 20 }),
      blocker({ app: "Google Chrome", reason: "Playing audio", heldSecs: 3540 }),
      blocker({ app: "zoom.us", keeps: "display", heldSecs: 30, endsInSecs: 60 }),
    ]);
    expect(apps.map((a) => [a.app, a.brief])).toEqual([
      ["zoom.us", false],
      ["Google Chrome", false],
      ["caffeinate", true],
      ["akd", true],
    ]);
    expect(apps.find((a) => a.app === "caffeinate")?.endsInSecs).toBe(240);
  });
});
