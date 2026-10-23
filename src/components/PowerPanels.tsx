import type { ReactNode } from "react";
import { externalPowerRanges, isHealthyCondition, STATE_LABEL } from "../lib/battery";
import { formatClock, formatDayClock, formatMinutes, formatPercent, formatWatts } from "../lib/format";
import type { BatteryHistory, BatteryInfo } from "../lib/schemas";
import type { PollState } from "../hooks/usePolling";
import { LineChart } from "./LineChart";
import { Card, Dash, Loading, Meter, Row, Segmented, Stat, Status, Unavailable } from "./ui";

export const HISTORY_RANGES = [
  { value: 1, label: "1 h" },
  { value: 6, label: "6 h" },
  { value: 24, label: "24 h" },
  { value: 24 * 7, label: "7 d" },
];

/** Below this a flow is noise in the telemetry, not power moving. */
const IDLE_W = 0.05;

function timeLeft(b: BatteryInfo): string | null {
  if (b.timeRemainingMin !== null) {
    return b.timeRemainingKind === "untilFull"
      ? `Full in ${formatMinutes(b.timeRemainingMin)}`
      : `About ${formatMinutes(b.timeRemainingMin)} left`;
  }
  if (b.timeRemainingKind === "untilEmpty") return "macOS is still estimating the time left";
  if (b.timeRemainingKind === "untilFull") return "macOS is still estimating the time to full";
  return null;
}

function FlowNode({ label, value, main }: { label: string; value: ReactNode; main?: boolean }) {
  return (
    <div className={`flow-node${main ? " flow-main" : ""}`}>
      <span className="flow-label">{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function FlowArrow({ watts }: { watts: number | null }) {
  return (
    <div className="flow-arrow" aria-hidden>
      <span>{watts !== null ? formatWatts(watts) : ""}</span>
    </div>
  );
}

/** Where the power comes from and where it goes, from the battery's power telemetry. */
function PowerFlow({ b }: { b: BatteryInfo }) {
  const mac = b.systemPowerW;
  const battery = b.batteryPowerW;
  const pct = b.percent !== null ? `${b.percent}%` : "Battery";
  const macValue = mac !== null ? formatWatts(mac) : <Dash reason="The battery did not report the Mac's draw" />;
  const describe =
    b.externalPower === false
      ? `The battery supplies ${mac !== null ? formatWatts(mac) : "the Mac"}.`
      : `The charger supplies ${b.adapterPowerW !== null ? formatWatts(b.adapterPowerW) : "power"}${mac !== null ? `; the Mac uses ${formatWatts(mac)}` : ""}${battery !== null && battery > IDLE_W ? ` and ${formatWatts(battery)} goes into the battery` : ""}.`;
  return (
    <div className="flow" role="img" aria-label={describe} title={describe}>
      {b.externalPower === false ? (
        <>
          <FlowNode label="Battery" value={pct} />
          <FlowArrow watts={battery !== null ? -battery : mac} />
          <FlowNode label="Mac uses" value={macValue} main />
        </>
      ) : (
        <>
          <FlowNode label="Charger" value={b.adapterRatingW !== null ? `${b.adapterRatingW} W` : "Connected"} />
          <FlowArrow watts={b.adapterPowerW} />
          <FlowNode label="Mac uses" value={macValue} main />
          {battery !== null && battery > IDLE_W ? (
            <>
              <FlowArrow watts={battery} />
              <FlowNode label="Battery" value={pct} />
            </>
          ) : battery !== null && battery < -IDLE_W ? (
            <>
              <FlowArrow watts={-battery} />
              <FlowNode label="Battery also supplies" value={pct} />
            </>
          ) : (
            <FlowNode label="Battery, idle" value={pct} />
          )}
        </>
      )}
    </div>
  );
}

function comparedWithUsual(rate: number, usual: number): string {
  const diff = Math.round((rate / usual - 1) * 100);
  return Math.abs(diff) < 10 ? "about your usual" : `${Math.abs(diff)}% ${diff > 0 ? "above" : "below"} your usual`;
}

function DrainStats({ b, h }: { b: BatteryInfo; h?: BatteryHistory }) {
  const avg = h?.average;
  const usual = h?.usual;
  return (
    <div className="stats">
      <Stat
        label="Drain now"
        value={b.drainPercentPerHour !== null ? `${b.drainPercentPerHour.toFixed(1)}%/h` : <Unavailable reason="The battery gauge did not report a current" />}
        sub={
          !b.drainCalibrated
            ? "gauge's share, calibrating"
            : b.drainPercentPerHour !== null && usual && usual.percentPerHour > 0
              ? comparedWithUsual(b.drainPercentPerHour, usual.percentPerHour)
              : "changes with the load"
        }
        hint={
          b.drainCalibrated
            ? "The battery gauge's present current, in points of the percentage macOS shows per hour. It follows the load from moment to moment."
            : "The battery gauge's present current, as a share of its full charge per hour. Once MacPilot has read 11 different percentages on battery, it converts this to the percentage macOS shows, which can fall a few percent faster."
        }
      />
      <Stat
        label="Average"
        value={avg ? `${avg.percentPerHour.toFixed(1)}%/h` : <Dash reason="Not measured yet" />}
        sub={avg ? `last ${formatMinutes(avg.windowMinutes)} on battery` : (h?.averageUnavailableReason ?? "Reading history…")}
        hint={
          avg
            ? `${avg.confidence[0].toUpperCase()}${avg.confidence.slice(1)} confidence. ${avg.confidenceReason}${avg.projectedMinutesToEmpty !== null ? ` At this average the battery lasts about ${formatMinutes(avg.projectedMinutesToEmpty)} more.` : ""}`
            : undefined
        }
      />
      <Stat
        label="Your usual"
        value={usual ? `${usual.percentPerHour.toFixed(1)}%/h` : <Dash reason="Not measured yet" />}
        sub={usual ? `over ${usual.hours} h on battery this week` : "after 2 h on battery with MacPilot open"}
        hint="The drain while MacPilot was open on battery over the last 7 days, gaps left out."
      />
    </div>
  );
}

export function PowerCard({
  battery,
  history,
  hours,
  onHours,
  className,
}: {
  battery: PollState<BatteryInfo>;
  history: PollState<BatteryHistory>;
  hours: number;
  onHours: (hours: number) => void;
  className?: string;
}) {
  const b = battery.data;
  const h = history.data;
  const now = history.updatedAt ?? Date.now();
  const time = b ? timeLeft(b) : null;
  return (
    <Card
      title="Power"
      className={className}
      error={battery.error ?? history.error}
      action={<Segmented label="History range" options={HISTORY_RANGES} value={hours} onChange={onHours} />}
    >
      {!b ? (
        <Loading error={battery.error} />
      ) : !b.present ? (
        <p className="muted">This Mac has no internal battery.</p>
      ) : (
        <>
          <div className="power-head">
            <div className="hero">
              <span className="hero-value">{b.percent !== null ? formatPercent(b.percent) : <Unavailable />}</span>
              <span className="hero-label">{STATE_LABEL[b.state]}</span>
            </div>
            {time && (
              <span className="power-time" title="macOS's own estimate, from recent power use">
                {time}
              </span>
            )}
          </div>
          {b.percent !== null && <Meter value={b.percent / 100} label="Battery charge" />}
          <PowerFlow b={b} />
          {b.externalPower === false && <DrainStats b={b} h={h} />}
        </>
      )}
      {h && (
        <>
          <LineChart
            points={h.samples.map((s) => ({ t: s.ts * 1000, v: s.percent }))}
            xDomain={[now - hours * 3600_000, now]}
            maxGapMs={h.maxGapSecs * 1000}
            shaded={externalPowerRanges(h.samples, h.maxGapSecs)}
            formatValue={(v) => `${Math.round(v)}%`}
            formatTime={(t) => (hours >= 24 ? formatDayClock(t) : formatClock(t))}
            ariaLabel={`Battery percentage over the last ${hours} hours`}
            height={150}
          />
          <p className="legend">
            <span className="legend-swatch" aria-hidden /> Charger connected. Gaps: the Mac slept or MacPilot was closed.
            {!h.persistent && " History is not being saved to disk."}
          </p>
        </>
      )}
    </Card>
  );
}

export function HealthCard({ battery, className }: { battery: PollState<BatteryInfo>; className?: string }) {
  const b = battery.data;
  return (
    <Card title="Battery health" className={className} error={battery.error}>
      {!b ? (
        <Loading error={battery.error} />
      ) : !b.present ? (
        <p className="muted">No internal battery.</p>
      ) : (
        <>
          <dl className="rows">
            <Row label="Maximum capacity" hint="Charge the battery holds compared with when new, as System Settings > Battery shows it">
              {b.appleMaxCapacityPercent !== null ? formatPercent(b.appleMaxCapacityPercent) : <Unavailable />}
            </Row>
            <Row label="Condition">
              {b.condition ? (
                <Status label={b.condition} ok={isHealthyCondition(b.condition)} />
              ) : (
                <Unavailable />
              )}
            </Row>
            <Row label="Charge cycles">{b.cycleCount ?? <Unavailable />}</Row>
            <Row label="Temperature">{b.temperatureC !== null ? `${b.temperatureC.toFixed(1)} °C` : <Unavailable />}</Row>
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
