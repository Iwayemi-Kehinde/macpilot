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
          <span aria-hidden>⚠︎</span> {error}
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

export function Loading() {
  return <p className="muted">Reading…</p>;
}
