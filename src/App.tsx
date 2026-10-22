import { useEffect, useState } from "react";
import { getBattery, getSystemSnapshot } from "./lib/api";
import { formatUptime } from "./lib/format";
import { loadChoice, saveChoice } from "./lib/prefs";
import { usePolling } from "./hooks/usePolling";
import type { ChartPoint } from "./components/LineChart";
import { BatteryCard, BatteryHistoryCard } from "./components/BatteryPanels";
import { CpuCard, DiskCard, MemoryCard, NetworkCard, ProcessCard } from "./components/SystemPanels";
import { Card, Loading } from "./components/ui";
import "./App.css";

const INTERVALS = [1000, 2000, 5000, 10000];
const INTERVAL_KEY = "macpilot.refreshMs";
const BATTERY_INTERVAL_MS = 10_000;
const CPU_HISTORY_POINTS = 90;

function App() {
  const [intervalMs, setIntervalMs] = useState(() => loadChoice(INTERVAL_KEY, INTERVALS, 2000));
  const system = usePolling(getSystemSnapshot, intervalMs);
  const battery = usePolling(getBattery, BATTERY_INTERVAL_MS);
  const [cpuHistory, setCpuHistory] = useState<ChartPoint[]>([]);

  const s = system.data;
  const b = battery.data;

  useEffect(() => {
    const usage = s?.cpu.usagePercent;
    if (s == null || usage == null) return;
    setCpuHistory((h) => [...h, { t: s.timestampMs, v: usage }].slice(-CPU_HISTORY_POINTS));
  }, [s]);

  const changeInterval = (ms: number) => {
    setIntervalMs(ms);
    saveChoice(INTERVAL_KEY, ms);
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
        {!s ? (
          <Card title="System" error={system.error} className="span-3">
            <Loading error={system.error} />
          </Card>
        ) : (
          <>
            {system.error && (
              <p className="banner span-3" role="alert">
                Showing the last valid reading. {system.error}
              </p>
            )}
            <CpuCard
              s={s}
              history={cpuHistory}
              windowMs={intervalMs * CPU_HISTORY_POINTS}
              intervalMs={intervalMs}
              className="span-2"
            />
            <MemoryCard s={s} />
          </>
        )}
        <BatteryCard state={battery} />
        <BatteryHistoryCard className="span-2" externalPower={b?.externalPower ?? undefined} />
        {s && (
          <>
            <ProcessCard s={s} className="span-2 row-span-2" />
            <NetworkCard s={s} />
            <DiskCard s={s} />
          </>
        )}
      </main>
    </div>
  );
}

export default App;
