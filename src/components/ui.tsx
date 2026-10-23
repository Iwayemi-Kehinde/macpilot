import type { ReactNode } from "react";

export function Card({
  title,
  subtitle,
  action,
  error,
  className = "",
  children,
}: {
  title: string;
  subtitle?: ReactNode;
  action?: ReactNode;
  error?: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section className={`card ${className}`}>
      <header className="card-head">
        <div>
          <h2>{title}</h2>
          {subtitle && <p className="card-sub">{subtitle}</p>}
        </div>
        {action}
      </header>
      {error && (
        <p className="card-error" role="alert">
          <span className="mark mark-warning" aria-hidden /> {error}
        </p>
      )}
      {children}
    </section>
  );
}

/** Shown wherever macOS does not expose a value. Never replaced by a guess. */
export function Unavailable({ reason }: { reason?: string }) {
  return (
    <span className="unavailable" title={reason}>
      Unavailable
    </span>
  );
}

export function Meter({ value, label }: { value: number; label: string }) {
  const clamped = Math.min(Math.max(value, 0), 1);
  return (
    <div
      className="meter"
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(clamped * 100)}
    >
      <div className="meter-fill" style={{ width: `${clamped * 100}%` }} />
    </div>
  );
}

export function Row({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <div className="row">
      <dt title={hint}>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/** Shown until the first reading arrives; an error replaces it rather than sitting beside it. */
export function Loading({ error }: { error?: string }) {
  return error ? null : <p className="muted">Reading…</p>;
}

export function Segmented<T extends string | number>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <div className="segmented" role="group" aria-label={label}>
      {options.map((o) => (
        <button key={o.value} aria-pressed={value === o.value} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Stat({ label, value, sub, hint }: { label: string; value: ReactNode; sub?: ReactNode; hint?: string }) {
  return (
    <div className="stat" title={hint}>
      <span className="stat-label">{label}</span>
      <span className="stat-value">{value}</span>
      {sub && <span className="stat-sub">{sub}</span>}
    </div>
  );
}

export function Status({ label, ok }: { label: string; ok: boolean }) {
  return (
    <span className="status">
      <span className={`mark${ok ? "" : " mark-warning"}`} aria-hidden /> {label}
    </span>
  );
}

/** An empty value, drawn rather than typed; `reason` says why it is empty. */
export function Dash({ reason }: { reason: string }) {
  return <span className="dash" role="img" aria-label={reason} title={reason} />;
}
