import type { ReactNode } from "react";
import { formatBytes, formatPercent, formatRate } from "../lib/format";
import type { SystemSnapshot } from "../lib/schemas";
import type { ChartPoint } from "./LineChart";
import { Meter, Status, Unavailable } from "./ui";

type Snapshot = SystemSnapshot;

/** Apple's names for the thermal states apps are told about. */
const HEAT: Record<NonNullable<Snapshot["cpu"]["thermal"]>, { label: string; ok: boolean }> = {
  nominal: { label: "Normal", ok: true },
  moderate: { label: "Fair", ok: false },
  heavy: { label: "Serious", ok: false },
  trapping: { label: "Critical", ok: false },
  sleeping: { label: "Critical", ok: false },
};

const PRESSURE: Record<NonNullable<Snapshot["memory"]["pressure"]>, { label: string; ok: boolean }> = {
  normal: { label: "Normal", ok: true },
  warning: { label: "Warning", ok: false },
  critical: { label: "Critical", ok: false },
};

function Arrow({ up }: { up?: boolean }) {
  return (
    <svg className="net-arrow" viewBox="0 0 10 12" aria-hidden>
      <path d={up ? "M5 11V1M1 5l4-4 4 4" : "M5 1v10M1 7l4 4 4-4"} />
    </svg>
  );
}

function Tile({ label, value, sub, children }: { label: string; value: ReactNode; sub?: ReactNode; children?: ReactNode }) {
  return (
    <section className="card tile">
      <h2>{label}</h2>
      <div className="tile-value">{value}</div>
      {children}
      {sub && <p className="tile-sub">{sub}</p>}
    </section>
  );
}

/** Recent total CPU use, 0 to 100%; points further apart than `maxGapMs` are not joined. */
function Sparkline({ points, maxGapMs, windowMs }: { points: ChartPoint[]; maxGapMs: number; windowMs: number }) {
  const end = points.length ? points[points.length - 1].t : 0;
  const x = (t: number) => 100 - ((end - t) / windowMs) * 100;
  const d = points
    .map((p, i) => `${i > 0 && p.t - points[i - 1].t <= maxGapMs ? "L" : "M"}${x(p.t).toFixed(2)},${(100 - Math.min(p.v, 100)).toFixed(2)}`)
    .join("");
  return (
    <svg className="sparkline" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden>
      <line x1={0} x2={100} y1={100} y2={100} vectorEffect="non-scaling-stroke" />
      <path d={d} vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

export function Vitals({ s, cpuHistory, intervalMs, windowMs }: { s: Snapshot; cpuHistory: ChartPoint[]; intervalMs: number; windowMs: number }) {
  const { cpu, memory: m, disk, network: n } = s;
  return (
    <div className="vitals">
      <Tile
        label="CPU"
        value={cpu.usagePercent !== null ? formatPercent(cpu.usagePercent) : <span className="muted">Measuring…</span>}
        sub={
          <>
            Heat {cpu.thermal ? <Status {...HEAT[cpu.thermal]} /> : <Unavailable reason="macOS did not report its thermal state" />}
          </>
        }
      >
        <Sparkline points={cpuHistory} maxGapMs={intervalMs * 3} windowMs={windowMs} />
      </Tile>
      <Tile
        label="Memory"
        value={
          <>
            {formatBytes(m.usedBytes, 1024)} <span className="tile-of">of {formatBytes(m.totalBytes, 1024)}</span>
          </>
        }
        sub={
          <>
            Pressure {m.pressure ? <Status {...PRESSURE[m.pressure]} /> : <Unavailable />}
            {m.swapUsedBytes > 0 && <span className="muted"> · {formatBytes(m.swapUsedBytes, 1024)} swap</span>}
          </>
        }
      >
        <Meter value={m.usedBytes / m.totalBytes} label="Memory used" />
      </Tile>
      <Tile
        label="Startup disk"
        value={
          disk ? (
            <>
              {formatBytes(disk.availableBytes)} <span className="tile-of">free</span>
            </>
          ) : (
            <Unavailable reason="The startup volume was not found" />
          )
        }
        sub={
          disk ? (
            <span title="Counts purgeable files macOS removes when it needs room, as Finder does">
              of {formatBytes(disk.totalBytes)} · {disk.name}
            </span>
          ) : undefined
        }
      >
        {disk && <Meter value={disk.totalBytes ? 1 - disk.availableBytes / disk.totalBytes : 0} label="Startup disk used" />}
      </Tile>
      <Tile
        label="Network"
        value={
          n.rxBytesPerSec !== null && n.txBytesPerSec !== null ? (
            <span className="net-rates">
              <span title="Download">
                <Arrow />
                <span className="sr-only">Download </span>
                {formatRate(n.rxBytesPerSec)}
              </span>
              <span title="Upload">
                <Arrow up />
                <span className="sr-only">Upload </span>
                {formatRate(n.txBytesPerSec)}
              </span>
            </span>
          ) : (
            <span className="muted">Measuring…</span>
          )
        }
        sub="Wi-Fi and Ethernet"
      />
    </div>
  );
}
