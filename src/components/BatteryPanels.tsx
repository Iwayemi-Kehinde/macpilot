import { useCallback, useState } from "react";
import { getBatteryHistory } from "../lib/api";
import { externalPowerRanges, isHealthyCondition, STATE_LABEL, thinSamples } from "../lib/battery";
import { formatClock, formatDayClock, formatMinutes, formatPercent } from "../lib/format";
import type { BatteryInfo } from "../lib/schemas";
import { usePolling, type PollState } from "../hooks/usePolling";
import { LineChart } from "./LineChart";
import { Card, Loading, Meter, Row, Unavailable } from "./ui";

export function BatteryCard({ state }: { state: PollState<BatteryInfo> }) {
  const b = state.data;
  return (
    <Card title="Battery" subtitle={b?.powerSource ?? undefined} error={state.error}>
      {!b ? (
        <Loading error={state.error} />
      ) : !b.present ? (
        <p className="muted">This Mac has no internal battery.</p>
      ) : (
        <>
          <div className="hero">
            <span className="hero-value">{b.percent !== null ? formatPercent(b.percent) : <Unavailable />}</span>
            <span className="hero-label">{STATE_LABEL[b.state]}</span>
          </div>
          {b.percent !== null && <Meter value={b.percent / 100} label="Battery charge" />}
          <dl className="rows">
            <Row label={b.timeRemainingKind === "untilFull" ? "Time to full" : "Time remaining"} hint="Estimated by macOS (pmset)">
              {b.timeRemainingMin !== null ? (
                `${formatMinutes(b.timeRemainingMin)}`
              ) : (
                <Unavailable reason={b.timeRemainingKind ? "macOS has no estimate yet" : "Not applicable while on power"} />
              )}
            </Row>
            <Row label="Maximum capacity" hint="As shown in System Settings > Battery">
              {b.appleMaxCapacityPercent !== null ? formatPercent(b.appleMaxCapacityPercent) : <Unavailable />}
            </Row>
            <Row label="Condition">
              {b.condition ? (
                <span className={`status ${isHealthyCondition(b.condition) ? "status-good" : "status-warning"}`}>
                  <span aria-hidden>{isHealthyCondition(b.condition) ? "●" : "▲"}</span> {b.condition}
                </span>
              ) : (
                <Unavailable />
              )}
            </Row>
            <Row label="Cycle count">{b.cycleCount ?? <Unavailable />}</Row>
            <Row
              label="Gauge capacity"
              hint="Full-charge capacity over design capacity, read from the battery gauge. Apple's Maximum Capacity uses its own calculation and can differ."
            >
              {b.fullChargeCapacityMah !== null && b.designCapacityMah !== null ? (
                <>
                  {b.fullChargeCapacityMah} / {b.designCapacityMah} mAh
                  {b.rawCapacityPercent !== null && <span className="muted"> ({b.rawCapacityPercent}%)</span>}
                </>
              ) : (
                <Unavailable />
              )}
            </Row>
            <Row label="Charge held">{b.currentChargeMah !== null ? `${b.currentChargeMah} mAh` : <Unavailable />}</Row>
            <Row label="Power" hint="Instantaneous voltage x current from the gauge">
              {b.powerW !== null ? (
                `${b.powerW < 0 ? "Drawing" : b.powerW > 0 ? "Charging at" : "Idle,"} ${Math.abs(b.powerW).toFixed(1)} W`
              ) : (
                <Unavailable />
              )}
            </Row>
            <Row label="Temperature">
              {b.temperatureC !== null ? `${b.temperatureC.toFixed(1)} °C` : <Unavailable />}
            </Row>
          </dl>
          {b.notes.length > 0 && (
            <ul className="notes">
              {b.notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          )}
        </>
      )}
    </Card>
  );
}

const RANGES = [
  { hours: 6, label: "6 h" },
  { hours: 24, label: "24 h" },
  { hours: 24 * 7, label: "7 d" },
];

export function BatteryHistoryCard({ className }: { className?: string }) {
  const [hours, setHours] = useState(24);
  const [showTable, setShowTable] = useState(false);
  const fetcher = useCallback(() => getBatteryHistory(hours), [hours]);
  const state = usePolling(fetcher, 60_000);
  const h = state.data;
  const now = state.updatedAt ?? Date.now();
  const points = h?.samples.map((s) => ({ t: s.ts * 1000, v: s.percent })) ?? [];
  const e = h?.discharge;

  return (
    <Card
      title="Battery history"
      className={className}
      error={state.error}
      subtitle={
        h
          ? `Sampled every ${h.sampleIntervalSecs} s while MacPilot is open. Gaps mean the Mac slept or the app was closed.${h.persistent ? "" : " History is not being saved to disk."}`
          : undefined
      }
      action={
        <div className="segmented" role="group" aria-label="History range">
          {RANGES.map((r) => (
            <button key={r.hours} aria-pressed={hours === r.hours} onClick={() => setHours(r.hours)}>
              {r.label}
            </button>
          ))}
        </div>
      }
    >
      {!h ? (
        <Loading error={state.error} />
      ) : (
        <>
          <LineChart
            points={points}
            xDomain={[now - hours * 3600_000, now]}
            maxGapMs={h.maxGapSecs * 1000}
            shaded={externalPowerRanges(h.samples, h.maxGapSecs)}
            formatValue={(v) => `${Math.round(v)}%`}
            formatTime={(t) => (hours >= 24 ? formatDayClock(t) : formatClock(t))}
            ariaLabel={`Battery percentage over the last ${hours} hours`}
            height={170}
          />
          <p className="legend">
            <span className="legend-swatch" aria-hidden /> Shaded: power adapter connected
          </p>
          <div className="estimate">
            <h3>Discharge rate</h3>
            {e ? (
              <>
                <p>
                  <strong>{e.percentPerHour.toFixed(1)} %/h</strong>
                  {e.averageMa !== null && <span className="muted"> · average {e.averageMa} mA</span>}
                  {e.projectedMinutesToEmpty !== null && (
                    <span className="muted"> · about {formatMinutes(e.projectedMinutesToEmpty)} to empty at this rate</span>
                  )}
                </p>
                <p className="muted small">
                  <span className={`badge badge-${e.confidence}`}>{e.confidence} confidence</span> {e.confidenceReason}{" "}
                  Based on {e.sampleCount} samples over {formatMinutes(e.windowMinutes)} since the last unplug or gap.
                </p>
              </>
            ) : (
              <p className="muted">
                <Unavailable /> {h.dischargeUnavailableReason}
              </p>
            )}
          </div>
          <button className="link" onClick={() => setShowTable((v) => !v)} aria-expanded={showTable}>
            {showTable ? "Hide" : "Show"} data table
          </button>
          {showTable && (
            <div className="table-wrap">
              <table>
                <thead>
                  <tr>
                    <th>Time</th>
                    <th>Charge</th>
                    <th>Power</th>
                    <th>Current</th>
                  </tr>
                </thead>
                <tbody>
                  {thinSamples(h.samples, 15 * 60).map((s) => (
                    <tr key={s.ts}>
                      <td>{new Date(s.ts * 1000).toLocaleString([], { dateStyle: "short", timeStyle: "short" })}</td>
                      <td>{s.percent}%</td>
                      <td>{s.externalPower ? (s.charging ? "Adapter, charging" : "Adapter") : "Battery"}</td>
                      <td>{s.amperageMa !== null ? `${s.amperageMa} mA` : "Unavailable"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </>
      )}
    </Card>
  );
}
