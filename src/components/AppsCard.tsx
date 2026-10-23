import { useState } from "react";
import { formatBytes, formatPercent, formatWatts } from "../lib/format";
import type { AppUsage, SystemSnapshot } from "../lib/schemas";
import { Card, Dash, Segmented } from "./ui";

type SortBy = "cpu" | "power" | "memory";

const SORTS: { value: SortBy; label: string }[] = [
  { value: "cpu", label: "CPU" },
  { value: "power", label: "Power" },
  { value: "memory", label: "Memory" },
];

const ROWS = 12;
const HIDDEN = "macOS reports power and memory only for your own apps, not for its own or other users' processes";
const MEASURING = "Needs a second reading";

const measure: Record<SortBy, (a: AppUsage) => number | null> = {
  cpu: (a) => a.cpuPercent,
  power: (a) => a.powerW,
  memory: (a) => a.memoryBytes,
};

/** Unmeasured values sort last: an app without a reading is not shown as idle. */
export function rankApps(apps: AppUsage[], by: SortBy): AppUsage[] {
  const m = measure[by];
  return [...apps]
    .filter((a) => m(a) !== null)
    .sort((a, b) => m(b)! - m(a)! || a.name.localeCompare(b.name))
    .slice(0, ROWS);
}

export function AppsCard({ s, className }: { s: SystemSnapshot; className?: string }) {
  const [sortBy, setSortBy] = useState<SortBy>("cpu");
  const { totalProcesses, systemProcesses, error, apps } = s.apps;
  if (error) {
    return (
      <Card title="Apps" className={className} error={`Processes could not be read: ${error}`}>
        {null}
      </Card>
    );
  }
  const rows = rankApps(apps, sortBy);
  return (
    <Card
      title="Apps"
      className={className}
      subtitle={`${totalProcesses} processes, grouped by app. CPU is a percent of one core (${s.host.logicalCores} cores allow ${s.host.logicalCores * 100}%).`}
      action={<Segmented label="Sort apps by" options={SORTS} value={sortBy} onChange={setSortBy} />}
    >
      <div className="table-wrap">
        <table className="apps">
          <thead>
            <tr>
              <th>App</th>
              <th className="num">CPU</th>
              <th className="num" title="CPU energy the kernel accounts to the app, averaged since the last refresh">
                Power
              </th>
              <th className="num" title="Physical footprint, as in Activity Monitor's Memory column">
                Memory
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((a) => (
              <tr key={a.name}>
                <td className="truncate" title={`${a.name}: ${a.processCount} process${a.processCount === 1 ? "" : "es"}`}>
                  {a.name}
                  {a.processCount > 1 && <span className="muted small"> · {a.processCount}</span>}
                </td>
                <td className="num">{a.cpuPercent !== null ? formatPercent(a.cpuPercent, 1) : <Dash reason={MEASURING} />}</td>
                <td className="num">
                  {a.powerW !== null ? formatWatts(a.powerW) : <Dash reason={a.ownProcessCount > 0 ? MEASURING : HIDDEN} />}
                </td>
                <td className="num">
                  {a.memoryBytes !== null ? formatBytes(a.memoryBytes, 1024) : <Dash reason={HIDDEN} />}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 && <p className="muted small">Measuring…</p>}
      </div>
      <p className="footnote">
        Power is each app's CPU energy as macOS measures it; the screen, graphics and the rest of the Mac draw the remainder of
        its total.{" "}
        {systemProcesses > 0 && `A dash marks a value macOS keeps to itself, as for its ${systemProcesses} system processes, or one that needs a second reading.`}
      </p>
    </Card>
  );
}
