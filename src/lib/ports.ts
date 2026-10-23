import type { Listener } from "./schemas";

/** macOS's own programs live in these folders, which only macOS can write to; /usr/local holds what users install. */
const APPLE_PREFIXES = ["/System/", "/usr/", "/sbin/", "/bin/", "/Library/Apple/"];

export function isAppleService({ path }: Pick<Listener, "path">): boolean {
  return path !== null && !path.startsWith("/usr/local/") && APPLE_PREFIXES.some((p) => path.startsWith(p));
}

export interface Server extends Omit<Listener, "port"> {
  ports: number[];
}

/** One entry per process and reach, since a server often listens on several ports. */
export function servers(listeners: Listener[]): Server[] {
  const groups = new Map<string, Server>();
  for (const { port, ...l } of listeners) {
    const key = `${l.pid}:${l.network}`;
    const g = groups.get(key);
    if (g) {
      g.ports.push(port);
      g.addresses = [...new Set([...g.addresses, ...l.addresses])];
    } else {
      groups.set(key, { ...l, addresses: [...l.addresses], ports: [port] });
    }
  }
  return [...groups.values()];
}

/** What a few well-known listeners are, keyed by process name. */
const KNOWN: Record<string, (port: number) => string | null> = {
  ControlCenter: (port) =>
    port === 5000 || port === 7000
      ? "AirPlay Receiver. To free the port, turn it off in System Settings > General > AirDrop & Handoff."
      : null,
  rapportd: () => "Continuity: Handoff, Universal Control and nearby Apple devices.",
  sharingd: () => "AirDrop and sharing.",
  limactl: () => "Lima's virtual machine (used by Colima), forwarding a port.",
  "com.docker.backend": () => "Docker Desktop, forwarding a container port.",
};

export function describeServer(s: Pick<Server, "process" | "ports">): string | null {
  const known = KNOWN[s.process];
  return (known && s.ports.map(known).find((d) => d !== null)) ?? null;
}
