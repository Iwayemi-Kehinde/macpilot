import { useEffect, useState } from "react";
import { getBattery, getSystemSnapshot } from "./lib/api";
import { formatUptime } from "./lib/format";
import { usePolling } from "./hooks/usePolling";
import type { ChartPoint } from "./components/LineChart";
import { BatteryCard, BatteryHistoryCard } from "./components/BatteryPanels";
import { CpuCard, DiskCard, MemoryCard, NetworkCard, ProcessCard } from "./components/SystemPanels";
import { Card, Loading } from "./components/ui";
import "./App.css";

const INTERVALS = [1000, 2000, 5000, 10000];
const INTERVAL_KEY = "macpilot.refreshMs";
const BATTERY_INTERVAL_MS = 30_000;
const CPU_HISTORY_POINTS = 90;

function loadInterval(): number {
  try {
    const v = Number(localStorage.getItem(INTERVAL_KEY));
    return INTERVALS.includes(v) ? v : 2000;
  } catch {
    return 2000;
  }
}

function App() {
  const [intervalMs, setIntervalMs] = useState(loadInterval);
  const system = usePolling(getSystemSnapshot, intervalMs);
  const battery = usePolling(getBattery, BATTERY_INTERVAL_MS);
  const [cpuHistory, setCpuHistory] = useState<ChartPoint[]>([]);

  const s = system.data;

  useEffect(() => {
    const usage = s?.cpu.usagePercent;
    if (s == null || usage == null) return;
    setCpuHistory((h) => [...h, { t: s.timestampMs, v: usage }].slice(-CPU_HISTORY_POINTS));
  }, [s]);

  const changeInterval = (ms: number) => {
    setIntervalMs(ms);
    try {
      localStorage.setItem(INTERVAL_KEY, String(ms));
    } catch {
      // Preference is only kept for this session.
    }
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
            <Loading />
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
        <BatteryHistoryCard className="span-2" />
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
