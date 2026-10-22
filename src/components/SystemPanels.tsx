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

export function MemoryCard({ s }: { s: Snapshot }) {
  const m = s.memory;
  return (
    <Card title="Memory" subtitle={`${formatBytes(m.totalBytes, 1024)} installed`}>
      <div className="hero">
        <span className="hero-value">{formatBytes(m.usedBytes, 1024)}</span>
        <span className="hero-label">used of {formatBytes(m.totalBytes, 1024)}</span>
      </div>
      <Meter value={m.usedBytes / m.totalBytes} label="Memory used" />
      <dl className="rows">
        <Row label="Used" hint="App memory + wired + compressed, the same definition as Activity Monitor's Memory Used">
          {formatPercent((m.usedBytes / m.totalBytes) * 100)}
        </Row>
        <Row label="Cached files and free" hint="Memory macOS can reclaim or hand out right away">
          {formatBytes(m.totalBytes - m.usedBytes, 1024)}
        </Row>
        <Row label="Free pages">{formatBytes(m.freeBytes, 1024)}</Row>
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

const IFACE_KIND: [RegExp, string][] = [
  [/^lo\d/, "Loopback"],
  [/^utun\d/, "Tunnel (VPN or system service)"],
  [/^awdl\d/, "Apple Wireless Direct Link (AirDrop)"],
  [/^llw\d/, "Low-latency WLAN"],
  [/^bridge\d/, "Bridge"],
  [/^en\d/, "Ethernet or Wi-Fi"],
];

export function NetworkCard({ s }: { s: Snapshot }) {
  const shown = s.network.slice(0, 6);
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
    </Card>
  );
}

export function ProcessCard({ s, className }: { s: Snapshot; className?: string }) {
  const [sortBy, setSortBy] = useState<"cpu" | "memory">("cpu");
  const { total, unreadable, topByCpu, topByMemory } = s.processes;
  const rows: ProcessInfo[] = sortBy === "cpu" ? topByCpu : topByMemory;
  return (
    <Card
      title="Processes"
      className={className}
      subtitle={
        <>
          {total} running. CPU is a percent of one core, so {s.host.logicalCores} cores allow up to{" "}
          {s.host.logicalCores * 100}%.
          {unreadable > 0 && (
            <>
              {" "}
              {unreadable} belong to the system or other users; macOS only shows their usage to administrator tools,
              so they are not ranked here.
            </>
          )}
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
              <th className="num" title="Resident memory. Activity Monitor's Memory column shows a different measure (footprint).">
                Resident
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
                <td className="num">{formatBytes(p.memoryBytes, 1024)}</td>
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

