import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getBattery, getBatteryHistory, getPorts, getSleepBlockers, getSystemSnapshot } from "./lib/api";
import { formatUptime } from "./lib/format";
import { analyze, checked, SUSTAINED_MS, type CpuReading, type DrainReading } from "./lib/insights";
import { loadChoice, saveChoice } from "./lib/prefs";
import { usePolling } from "./hooks/usePolling";
import type { ChartPoint } from "./components/LineChart";
import { AppsCard } from "./components/AppsCard";
import { AwakeCard, PortsCard } from "./components/ActivityPanels";
import { InsightsCard } from "./components/Insights";
import { HealthCard, HISTORY_RANGES, PowerCard } from "./components/PowerPanels";
import { Vitals } from "./components/SystemPanels";
import { Card, Loading } from "./components/ui";
import "./App.css";

const INTERVALS = [1000, 2000, 5000, 10000];
const INTERVAL_KEY = "macpilot.refreshMs";
const RANGE_KEY = "macpilot.historyHours";
const BATTERY_INTERVAL_MS = 10_000;
const HISTORY_INTERVAL_MS = 60_000;
const ACTIVITY_INTERVAL_MS = 15_000;
const CPU_HISTORY_POINTS = 90;
/** Readings kept for the analyzer, a little longer than the window it judges. */
const KEEP_READINGS_MS = SUSTAINED_MS + 30_000;

function App() {
  const [intervalMs, setIntervalMs] = useState(() => loadChoice(INTERVAL_KEY, INTERVALS, 2000));
  const [hours, setHours] = useState(() => loadChoice(RANGE_KEY, HISTORY_RANGES.map((r) => r.value), 6));
  const system = usePolling(getSystemSnapshot, intervalMs);
  const battery = usePolling(getBattery, BATTERY_INTERVAL_MS);
  const blockers = usePolling(getSleepBlockers, ACTIVITY_INTERVAL_MS);
  const ports = usePolling(getPorts, ACTIVITY_INTERVAL_MS);

  // The backend records an adapter change at once, so the history is fetched again then.
  const [reloads, setReloads] = useState(0);
  const lastPower = useRef<boolean | null | undefined>(undefined);
  const externalPower = battery.data?.externalPower;
  useEffect(() => {
    if (externalPower === undefined || externalPower === null) return;
    if (lastPower.current !== undefined && lastPower.current !== externalPower) setReloads((n) => n + 1);
    lastPower.current = externalPower;
  }, [externalPower]);
  // reloads is listed only so that an adapter change fetches again.
  const fetchHistory = useCallback(() => getBatteryHistory(hours), [hours, reloads]);
  const history = usePolling(fetchHistory, HISTORY_INTERVAL_MS);

  const s = system.data;
  const b = battery.data;
  const [cpuHistory, setCpuHistory] = useState<ChartPoint[]>([]);
  const [cpuReadings, setCpuReadings] = useState<CpuReading[]>([]);
  const [drainReadings, setDrainReadings] = useState<DrainReading[]>([]);

  useEffect(() => {
    if (!s) return;
    const usage = s.cpu.usagePercent;
    if (usage !== null) setCpuHistory((h) => [...h, { t: s.timestampMs, v: usage }].slice(-CPU_HISTORY_POINTS));
    const reading = {
      t: s.timestampMs,
      apps: s.apps.apps.map((a) => ({ name: a.name, cpu: a.cpuPercent, own: a.ownProcessCount > 0 })),
    };
    setCpuReadings((r) => [...r, reading].filter((x) => x.t >= s.timestampMs - KEEP_READINGS_MS));
  }, [s]);

  useEffect(() => {
    const at = battery.updatedAt;
    const v = b?.drainPercentPerHour;
    if (at === undefined) return;
    // Plugged in: no drain, and earlier readings no longer describe now.
    setDrainReadings((r) => (v == null ? [] : [...r, { t: at, v }].filter((x) => x.t >= at - KEEP_READINGS_MS)));
  }, [b, battery.updatedAt]);

  const inputs = {
    now: s?.timestampMs ?? Date.now(),
    snapshot: s,
    battery: b,
    history: history.data,
    blockers: blockers.data,
    ports: ports.data,
    cpuReadings,
    drainReadings,
  };
  const insights = useMemo(() => analyze(inputs), [s, b, history.data, blockers.data, ports.data, cpuReadings, drainReadings]);

  const changeInterval = (ms: number) => {
    setIntervalMs(ms);
    saveChoice(INTERVAL_KEY, ms);
  };
  const changeHours = (h: number) => {
    setHours(h);
    saveChoice(RANGE_KEY, h);
  };

  return (
    <div className="app">
      <header className="titlebar" data-tauri-drag-region>
        <div data-tauri-drag-region>
          <h1 data-tauri-drag-region>MacPilot</h1>
          <p className="muted small" data-tauri-drag-region>
            {s
              ? [s.host.hostname, s.host.osVersion, `up ${formatUptime(s.uptimeSecs)}`].filter(Boolean).join(" · ")
              : "Connecting to system monitor…"}
          </p>
        </div>
        <label className="interval">
          <span className={`live-dot ${system.error ? "stale" : ""}`} aria-hidden />
          {system.error ? "Stale" : "Live"} · refresh
          <select value={intervalMs} onChange={(e) => changeInterval(Number(e.target.value))}>
            {INTERVALS.map((ms) => (
              <option key={ms} value={ms}>
                {ms / 1000} s
              </option>
            ))}
          </select>
        </label>
      </header>

      <main className="grid">
        {system.error && s && (
          <p className="banner span-3" role="alert">
            Showing the last valid reading. {system.error}
          </p>
        )}
        <InsightsCard insights={insights} checked={checked(inputs)} className="span-3" />
        {s ? (
          <Vitals s={s} cpuHistory={cpuHistory} intervalMs={intervalMs} windowMs={intervalMs * CPU_HISTORY_POINTS} />
        ) : (
          <Card title="System" error={system.error} className="span-3">
            <Loading error={system.error} />
          </Card>
        )}
        <PowerCard battery={battery} history={history} hours={hours} onHours={changeHours} className="span-2" />
        <div className="stack">
          <HealthCard battery={battery} />
          <AwakeCard state={blockers} />
        </div>
        {s && <AppsCard s={s} className="span-2" />}
        <PortsCard state={ports} />
      </main>
    </div>
  );
}

export default App;
