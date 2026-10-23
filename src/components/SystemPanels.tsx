import { useState } from "react";
import { formatBytes, formatClockSeconds, formatPercent, formatRate } from "../lib/format";
import type { ProcessInfo, SystemSnapshot } from "../lib/schemas";
import { LineChart, type ChartPoint } from "./LineChart";
import { Card, Meter, Row, Unavailable } from "./ui";

type Snapshot = SystemSnapshot;

export function CpuCard({
  s,
  history,
  windowMs,
  intervalMs,
  className,
}: {
  s: Snapshot;
  history: ChartPoint[];
  windowMs: number;
  intervalMs: number;
  className?: string;
}) {
  const { cpu, host } = s;
  const end = history.length ? history[history.length - 1].t : s.timestampMs;
  return (
    <Card
      title="CPU"
      className={className}
      subtitle={[host.cpuBrand, `${host.logicalCores} cores`].filter(Boolean).join(" · ")}
    >
      <div className="hero">
        <span className="hero-value">
          {cpu.usagePercent !== null ? formatPercent(cpu.usagePercent) : <span className="muted">Measuring…</span>}
        </span>
        <span className="hero-label">
          Load {cpu.loadAverage.map((l) => l.toFixed(2)).join(" · ")} <span className="muted">(1, 5, 15 min)</span>
        </span>
      </div>
      <LineChart
        points={history}
        xDomain={[end - windowMs, end]}
        maxGapMs={intervalMs * 3}
        formatValue={(v) => `${Math.round(v)}%`}
        formatTime={formatClockSeconds}
        ariaLabel="Total CPU usage over recent minutes"
      />
      {cpu.perCorePercent.length > 0 && (
        <div className="cores" aria-label="Per-core usage">
          {cpu.perCorePercent.map((v, i) => (
            <div key={i} className="core" title={`Core ${i + 1}: ${formatPercent(v)}`}>
              <div className="core-fill" style={{ height: `${Math.min(v, 100)}%` }} />
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}

const KERNEL_UNANSWERED = "macOS did not return its memory statistics";
const USED_HINT =
  "Installed memory less free pages and file-backed cache, the figure Activity Monitor calls Memory Used. " +
  "App, wired and compressed memory add up to less, since purgeable pages and memory the kernel reserves are not in them.";

export function MemoryCard({ s }: { s: Snapshot }) {
  const m = s.memory;
  const b = m.breakdown;
  const part = (v: number | undefined) =>
    v !== undefined ? formatBytes(v, 1024) : <Unavailable reason={KERNEL_UNANSWERED} />;
  return (
    <Card
      title="Memory"
      subtitle={`${formatBytes(m.totalBytes, 1024)} installed${b ? " · the same measures as Activity Monitor" : ""}`}
    >
      <div className="hero">
        <span className="hero-value">{formatBytes(m.usedBytes, 1024)}</span>
        <span className="hero-label" title={b ? USED_HINT : undefined}>
          used of {formatBytes(m.totalBytes, 1024)}
        </span>
      </div>
      <Meter value={m.usedBytes / m.totalBytes} label="Memory used" />
      <dl className="rows">
        <Row label="App memory" hint="Memory apps are using, not counting caches they let macOS reclaim">
          {part(b?.appBytes)}
        </Row>
        <Row label="Wired memory" hint="Memory macOS keeps in place for itself; it cannot be compressed or swapped">
          {part(b?.wiredBytes)}
        </Row>
        <Row label="Compressed" hint="Space taken by memory macOS compressed to make room">
          {part(b?.compressedBytes)}
        </Row>
        <Row label="Cached files" hint="Recently used files kept in memory; macOS frees it whenever apps need room">
          {part(b?.cachedFilesBytes)}
        </Row>
        <Row label="Swap used">
          {m.swapTotalBytes > 0
            ? `${formatBytes(m.swapUsedBytes, 1024)} of ${formatBytes(m.swapTotalBytes, 1024)}`
            : "No swap in use"}
        </Row>
      </dl>
      {m.swapTotalBytes > 0 && <Meter value={m.swapUsedBytes / m.swapTotalBytes} label="Swap used" />}
    </Card>
  );
}

export function DiskCard({ s }: { s: Snapshot }) {
  return (
    <Card title="Disks" subtitle="Volumes sharing one APFS container are shown once">
      <ul className="list">
        {s.disks.map((d) => {
          const used = d.totalBytes - d.availableBytes;
          return (
            <li key={d.mountPoint}>
              <div className="list-head">
                <span className="list-title" title={d.mountPoint}>
                  {d.name || d.mountPoint}
                </span>
                {d.removable && <span className="tag">Removable or disk image</span>}
              </div>
              <Meter value={d.totalBytes ? used / d.totalBytes : 0} label={`${d.name} used`} />
              <p className="small muted">
                {formatBytes(d.availableBytes)} free of {formatBytes(d.totalBytes)} · {d.fileSystem} · {d.mountPoint}
              </p>
            </li>
          );
        })}
      </ul>
    </Card>
  );
}

const ETHERNET_OR_WIFI = /^en\d/;
const MAX_FALLBACK_INTERFACES = 6;

const IFACE_KIND: [RegExp, string][] = [
  [/^lo\d/, "Loopback"],
  [/^utun\d/, "Tunnel (VPN or system service)"],
  [/^awdl\d/, "Apple Wireless Direct Link (AirDrop)"],
  [/^llw\d/, "Low-latency WLAN"],
  [/^bridge\d/, "Bridge"],
  [ETHERNET_OR_WIFI, "Ethernet or Wi-Fi"],
];

export function NetworkCard({ s }: { s: Snapshot }) {
  const [showAll, setShowAll] = useState(false);
  const main = s.network.filter((n) => ETHERNET_OR_WIFI.test(n.interface));
  const others = s.network.filter((n) => !ETHERNET_OR_WIFI.test(n.interface));
  // Loopback, tunnels and AirDrop links are internal to macOS. A Mac with no
  // Ethernet or Wi-Fi traffic shows the busiest of them instead.
  const shown = showAll ? [...main, ...others] : main.length > 0 ? main : others.slice(0, MAX_FALLBACK_INTERFACES);
  return (
    <Card title="Network" subtitle="Interfaces with traffic since boot">
      <ul className="list">
        {shown.map((n) => (
          <li key={n.interface} className="net">
            <div>
              <span className="list-title">{n.interface}</span>
              <p className="small muted">{IFACE_KIND.find(([re]) => re.test(n.interface))?.[1] ?? "Interface"}</p>
            </div>
            <div className="net-rates">
              {n.rxBytesPerSec !== null && n.txBytesPerSec !== null ? (
                <>
                  <span title="Download">↓ {formatRate(n.rxBytesPerSec)}</span>
                  <span title="Upload">↑ {formatRate(n.txBytesPerSec)}</span>
                </>
              ) : (
                <span className="muted">Measuring…</span>
              )}
              <span className="small muted">
                {formatBytes(n.totalRxBytes)} in · {formatBytes(n.totalTxBytes)} out
              </span>
            </div>
          </li>
        ))}
      </ul>
      {main.length > 0 && others.length > 0 && (
        <button className="link" onClick={() => setShowAll((v) => !v)} aria-expanded={showAll}>
          {showAll ? "Hide" : "Show"} {others.length} system interfaces (loopback, tunnels, AirDrop)
        </button>
      )}
    </Card>
  );
}

export function ProcessCard({ s, className }: { s: Snapshot; className?: string }) {
  const [sortBy, setSortBy] = useState<"cpu" | "memory">("cpu");
  const { total, system, error, topByCpu, topByMemory } = s.processes;
  if (error) {
    return (
      <Card title="Processes" className={className} error={`Processes could not be read: ${error}`}>
        {null}
      </Card>
    );
  }
  const rows: ProcessInfo[] = sortBy === "cpu" ? topByCpu : topByMemory;
  return (
    <Card
      title="Processes"
      className={className}
      subtitle={
        <>
          {total} running. CPU is a percent of one core, so {s.host.logicalCores} cores allow up to{" "}
          {s.host.logicalCores * 100}%.
          {system > 0 && (
            <>
              {" "}
              {system} belong to macOS or other users: they are ranked by CPU, but macOS shows their memory only to
              administrator tools, so the Memory view lists your own processes.
            </>
          )}{" "}
          kernel_task is not visible to apps.
        </>
      }
      action={
        <div className="segmented" role="group" aria-label="Sort processes by">
          <button aria-pressed={sortBy === "cpu"} onClick={() => setSortBy("cpu")}>
            CPU
          </button>
          <button aria-pressed={sortBy === "memory"} onClick={() => setSortBy("memory")}>
            Memory
          </button>
        </div>
      }
    >
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th className="num">PID</th>
              <th className="num">CPU</th>
              <th className="num" title="Physical footprint, the same measure as Activity Monitor's Memory column">
                Memory
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((p) => (
              <tr key={p.pid}>
                <td className="truncate" title={p.name}>
                  {p.name}
                </td>
                <td className="num muted">{p.pid}</td>
                <td className="num">
                  {p.cpuPercent !== null ? formatPercent(p.cpuPercent, 1) : <Unavailable reason="Needs a second sample" />}
                </td>
                <td className="num">
                  {p.memoryBytes !== null ? (
                    formatBytes(p.memoryBytes, 1024)
                  ) : (
                    <Unavailable reason="macOS shows system processes' memory only to administrator tools" />
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 && (
          <p className="muted small">{sortBy === "cpu" ? "Measuring CPU per process…" : "No readable processes."}</p>
        )}
      </div>
    </Card>
  );
}

