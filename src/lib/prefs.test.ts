import { afterEach, describe, expect, it, vi } from "vitest";
import { loadChoice, saveChoice } from "./prefs";

function fakeStorage(): Storage {
  const data = new Map<string, string>();
  return {
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => void data.set(k, v),
    removeItem: (k) => void data.delete(k),
    clear: () => data.clear(),
    key: () => null,
    get length() {
      return data.size;
    },
  };
}

describe("loadChoice", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("returns a saved choice that is still allowed", () => {
    vi.stubGlobal("localStorage", fakeStorage());
    saveChoice("k", 6);
    expect(loadChoice("k", [1, 6, 24], 24)).toBe(6);
  });

  it("falls back for missing, unknown or tampered values", () => {
    vi.stubGlobal("localStorage", fakeStorage());
    expect(loadChoice("k", [1, 6, 24], 24)).toBe(24);
    localStorage.setItem("k", "7");
    expect(loadChoice("k", [1, 6, 24], 24)).toBe(24);
    localStorage.setItem("k", "abc");
    expect(loadChoice("k", [1, 6, 24], 24)).toBe(24);
  });

  it("falls back when storage is blocked", () => {
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
    });
    expect(() => saveChoice("k", 6)).not.toThrow();
    expect(loadChoice("k", [1, 6, 24], 24)).toBe(24);
  });
});
