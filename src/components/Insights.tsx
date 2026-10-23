import { formatList } from "../lib/format";
import type { Insight, Level } from "../lib/insights";
import { Card } from "./ui";

const LEVEL: Record<Level, { label: string; mark: string }> = {
  critical: { label: "Critical", mark: "!!" },
  warning: { label: "Warning", mark: "!" },
  info: { label: "Note", mark: "i" },
};

export function InsightsCard({ insights, checked, className }: { insights: Insight[]; checked: string[]; className?: string }) {
  const urgent = insights.filter((i) => i.level !== "info").length;
  return (
    <Card
      title={insights.length === 0 ? "All clear" : "Needs attention"}
      className={className}
      subtitle={
        insights.length === 0
          ? undefined
          : `${urgent > 0 ? `${urgent} to act on` : "Nothing urgent"}${insights.length > urgent ? `, ${insights.length - urgent} to know` : ""}. MacPilot only suggests; it never changes anything for you.`
      }
    >
      {insights.length === 0 ? (
        <p className="all-clear">
          <svg className="all-clear-mark" viewBox="0 0 12 12" aria-hidden>
            <path d="M2 6.5l2.5 2.5L10 3.5" />
          </svg>
          Nothing needs your attention.
          {checked.length > 0 && <span className="muted"> Checked {formatList(checked)}.</span>}
        </p>
      ) : (
        <ul className="insights">
          {insights.map((i) => (
            <li key={i.id} className={`insight insight-${i.level}`}>
              <span className="insight-mark" aria-hidden>
                {LEVEL[i.level].mark}
              </span>
              <div className="insight-body">
                <p className="insight-title">
                  <span className="sr-only">{LEVEL[i.level].label}: </span>
                  {i.title}
                </p>
                <p className="insight-detail">{i.detail}</p>
                {i.action && <p className="insight-action">{i.action}</p>}
                <p className="insight-source">{i.source}</p>
              </div>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
