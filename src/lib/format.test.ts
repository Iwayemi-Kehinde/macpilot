import { describe, expect, it } from "vitest";
import { formatBytes, formatList, formatMinutes, formatRate, formatUptime, formatWatts } from "./format";

describe("formatBytes", () => {
  it("uses decimal units by default, like Finder", () => {
    expect(formatBytes(494_384_795_648)).toBe("494 GB");
    expect(formatBytes(1_500)).toBe("1.5 KB");
    expect(formatBytes(999)).toBe("999 B");
  });

  it("uses binary units for memory, like Activity Monitor", () => {
    expect(formatBytes(17_179_869_184, 1024)).toBe("16.0 GB");
  });

  it("refuses invalid input rather than printing a number", () => {
    expect(formatBytes(-1)).toBe("-");
    expect(formatBytes(Number.NaN)).toBe("-");
  });
});

describe("durations", () => {
  it("formats minutes", () => {
    expect(formatMinutes(726)).toBe("12 h 6 min");
    expect(formatMinutes(45)).toBe("45 min");
    expect(formatMinutes(120)).toBe("2 h");
  });

  it("formats uptime", () => {
    expect(formatUptime(229_155)).toBe("2 d 15 h");
    expect(formatUptime(3_700)).toBe("1 h 1 min");
    expect(formatUptime(59)).toBe("0 min");
  });

  it("formats rates", () => {
    expect(formatRate(80_102.5)).toBe("80.1 KB/s");
  });
});

describe("formatWatts", () => {
  it("keeps two digits for small draws and one above a watt", () => {
    expect(formatWatts(0.03194)).toBe("0.03 W");
    expect(formatWatts(3.67)).toBe("3.7 W");
    expect(formatWatts(13.24)).toBe("13.2 W");
    expect(formatWatts(0.004)).toBe("<0.01 W");
    expect(formatWatts(0)).toBe("0 W");
    expect(formatWatts(-5.26)).toBe("5.3 W");
    expect(formatWatts(Number.NaN)).toBe("-");
  });
});

describe("formatList", () => {
  it("joins with commas and a final and", () => {
    expect(formatList([])).toBe("");
    expect(formatList(["a"])).toBe("a");
    expect(formatList(["a", "b"])).toBe("a and b");
    expect(formatList(["a", "b", "c"])).toBe("a, b and c");
  });
});
