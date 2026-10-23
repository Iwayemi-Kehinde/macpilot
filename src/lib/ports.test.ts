import { describe, expect, it } from "vitest";
import { describeServer, isAppleService, servers } from "./ports";
import type { Listener } from "./schemas";

const listener = (over: Partial<Listener>): Listener => ({
  port: 5000,
  pid: 445,
  process: "ControlCenter",
  app: "ControlCenter",
  addresses: ["*"],
  network: true,
  folder: null,
  path: "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter",
  ...over,
});

describe("servers", () => {
  it("groups a process's ports, keeping local and network listeners apart", () => {
    const list = servers([
      listener({}),
      listener({ port: 7000, addresses: ["[::]", "*"] }),
      listener({ port: 53, pid: 900, process: "limactl", app: "limactl", path: "/opt/homebrew/bin/limactl" }),
      listener({ port: 49272, pid: 900, process: "limactl", app: "limactl", network: false, addresses: ["127.0.0.1"], path: "/opt/homebrew/bin/limactl" }),
    ]);
    expect(list.map((s) => [s.process, s.ports, s.network])).toEqual([
      ["ControlCenter", [5000, 7000], true],
      ["limactl", [53], true],
      ["limactl", [49272], false],
    ]);
    expect(list[0].addresses).toEqual(["*", "[::]"]);
  });
});

describe("describeServer", () => {
  it("names well-known listeners by process and port", () => {
    expect(describeServer({ process: "ControlCenter", ports: [5000, 7000] })).toMatch(/^AirPlay Receiver/);
    expect(describeServer({ process: "ControlCenter", ports: [8080] })).toBeNull();
    expect(describeServer({ process: "node", ports: [5173] })).toBeNull();
  });

  it("treats only programs in macOS's own folders as Apple's", () => {
    expect(isAppleService(listener({}))).toBe(true);
    expect(isAppleService(listener({ path: "/opt/homebrew/bin/node" }))).toBe(false);
    expect(isAppleService(listener({ path: "/usr/local/bin/node" }))).toBe(false);
    expect(isAppleService(listener({ path: "/usr/libexec/rapportd" }))).toBe(true);
    expect(isAppleService(listener({ path: null }))).toBe(false);
  });
});
