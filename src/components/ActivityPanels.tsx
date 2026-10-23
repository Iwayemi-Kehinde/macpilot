import { byApp } from "../lib/awake";
import { formatList, formatMinutes } from "../lib/format";
import { describeServer, isAppleService, servers } from "../lib/ports";
import type { Listener, SleepBlocker } from "../lib/schemas";
import type { PollState } from "../hooks/usePolling";
import { Card, Loading } from "./ui";

const KEEPS: Record<SleepBlocker["keeps"], string> = {
  display: "Keeps the display on",
  system: "Keeps the Mac awake",
};

export function AwakeCard({ state, className }: { state: PollState<SleepBlocker[]>; className?: string }) {
  const apps = state.data ? byApp(state.data) : undefined;
  const lasting = apps?.filter((a) => !a.brief) ?? [];
  const brief = apps?.filter((a) => a.brief) ?? [];
  return (
    <Card title="Keeping your Mac awake" className={className} error={state.error} subtitle="Apps that stop the display or the Mac from sleeping">
      {!apps ? (
        <Loading error={state.error} />
      ) : (
        <>
          {lasting.length === 0 ? (
            <p className="muted">Nothing{brief.length > 0 ? " for long" : ""}. Your Mac and display can sleep as set in System Settings.</p>
          ) : (
            <ul className="list">
              {lasting.map((a) => (
                <li key={a.app}>
                  <div className="list-head">
                    <span className="list-title">{a.app}</span>
                    <span className={`tag${a.keeps === "display" ? " tag-strong" : ""}`}>{KEEPS[a.keeps]}</span>
                  </div>
                  <p className="small muted">
                    {a.reasons.join(" · ")}
                    {a.heldSecs !== null && ` · for ${formatMinutes(a.heldSecs / 60)}`}
                    {a.endsInSecs !== null && ` · ends in ${formatMinutes(Math.max(1, a.endsInSecs / 60))}`}
                  </p>
                </li>
              ))}
            </ul>
          )}
          {brief.length > 0 && (
            <p
              className="small muted footnote"
              title={brief.map((a) => `${a.app}: ${a.reasons.join(", ")}${a.endsInSecs !== null ? `, ends in ${formatMinutes(Math.max(1, a.endsInSecs / 60))}` : ""}`).join("\n")}
            >
              Also short-lived: {formatList(brief.map((a) => a.app))}.
            </p>
          )}
        </>
      )}
    </Card>
  );
}

export function PortsCard({ state, className }: { state: PollState<Listener[]>; className?: string }) {
  const list = state.data ? servers(state.data) : undefined;
  return (
    <Card title="Open ports" className={className} error={state.error} subtitle="Servers your apps are running, and who can reach them">
      {!list ? (
        <Loading error={state.error} />
      ) : list.length === 0 ? (
        <p className="muted">None of your apps is listening for connections.</p>
      ) : (
        <ul className="list">
          {list.map((s) => {
            const about = describeServer(s);
            return (
              <li key={`${s.pid}-${s.network}`}>
                <div className="list-head">
                  <span className="list-title" title={`${s.process}, pid ${s.pid}, on ${s.addresses.join(" and ")}`}>
                    <span className="port">{s.ports.join(", ")}</span> {s.app}
                  </span>
                  <span
                    className={`tag${s.network && !isAppleService(s) ? " tag-strong" : ""}`}
                    title={s.network ? "Bound to a network interface: other devices can reach it unless the firewall blocks it" : "Bound to 127.0.0.1: only this Mac can reach it"}
                  >
                    {s.network ? "Network" : "This Mac only"}
                  </span>
                </div>
                {about ? (
                  <p className="small muted">{about}</p>
                ) : (
                  s.folder && (
                    <p className="small muted truncate" title={s.folder}>
                      {s.folder}
                    </p>
                  )
                )}
              </li>
            );
          })}
        </ul>
      )}
    </Card>
  );
}
