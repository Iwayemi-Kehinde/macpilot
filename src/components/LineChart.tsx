import { useEffect, useRef, useState, type PointerEvent } from "react";

export interface ChartPoint {
  t: number;
  v: number;
}

interface Props {
  points: ChartPoint[];
  xDomain: [number, number];
  yMax?: number;
  height?: number;
  /** Points further apart than this are not joined, so no line is drawn through missing data. */
  maxGapMs: number;
  shaded?: { start: number; end: number }[];
  formatValue: (v: number) => string;
  formatTime: (t: number) => string;
  ariaLabel: string;
}

const PAD = { top: 10, right: 10, bottom: 22, left: 40 };

export function LineChart({
  points,
  xDomain,
  yMax = 100,
  height = 160,
  maxGapMs,
  shaded = [],
  formatValue,
  formatTime,
  ariaLabel,
}: Props) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [hover, setHover] = useState<ChartPoint | null>(null);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const [t0, t1] = xDomain;
  const innerW = Math.max(0, width - PAD.left - PAD.right);
  const innerH = height - PAD.top - PAD.bottom;
  const x = (t: number) => PAD.left + ((t - t0) / Math.max(1, t1 - t0)) * innerW;
  const y = (v: number) => PAD.top + innerH - (Math.min(Math.max(v, 0), yMax) / yMax) * innerH;

  let d = "";
  // Points joined to neither neighbour would draw nothing, so they get a marker,
  // as does the newest point so a short recent run stays visible on long ranges.
  const markers: ChartPoint[] = [];
  points.forEach((p, i) => {
    const joinPrev = i > 0 && p.t - points[i - 1].t <= maxGapMs;
    const joinNext = i < points.length - 1 && points[i + 1].t - p.t <= maxGapMs;
    if ((!joinPrev && !joinNext) || i === points.length - 1) markers.push(p);
    d += `${joinPrev ? "L" : "M"}${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`;
  });

  const onMove = (e: PointerEvent<SVGSVGElement>) => {
    if (points.length === 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const t = t0 + ((e.clientX - rect.left - PAD.left) / Math.max(1, innerW)) * (t1 - t0);
    let best = points[0];
    for (const p of points) if (Math.abs(p.t - t) < Math.abs(best.t - t)) best = p;
    setHover(best);
  };

  const yTicks = [0, yMax / 2, yMax];
  const xTicks = [t0, (t0 + t1) / 2, t1];

  return (
    <div className="chart" ref={wrapRef} style={{ height }}>
      {width > 0 && (
        <svg
          width={width}
          height={height}
          role="img"
          aria-label={ariaLabel}
          onPointerMove={onMove}
          onPointerLeave={() => setHover(null)}
        >
          {shaded.map((s, i) => (
            <rect
              key={i}
              className="chart-shade"
              x={x(Math.max(s.start, t0))}
              y={PAD.top}
              width={Math.max(1, x(Math.min(s.end, t1)) - x(Math.max(s.start, t0)))}
              height={innerH}
            />
          ))}
          {yTicks.map((v) => (
            <g key={v}>
              <line className="chart-grid" x1={PAD.left} x2={PAD.left + innerW} y1={y(v)} y2={y(v)} />
              <text className="chart-axis" x={PAD.left - 8} y={y(v)} dy="0.32em" textAnchor="end">
                {formatValue(v)}
              </text>
            </g>
          ))}
          {xTicks.map((t, i) => (
            <text
              key={i}
              className="chart-axis"
              x={x(t)}
              y={height - 6}
              textAnchor={i === 0 ? "start" : i === 2 ? "end" : "middle"}
            >
              {formatTime(t)}
            </text>
          ))}
          <path className="chart-line" d={d} />
          {markers.map((p) => (
            <circle key={p.t} className="chart-point" cx={x(p.t)} cy={y(p.v)} r={3} />
          ))}
          {hover && (
            <g>
              <line className="chart-crosshair" x1={x(hover.t)} x2={x(hover.t)} y1={PAD.top} y2={PAD.top + innerH} />
              <circle className="chart-dot" cx={x(hover.t)} cy={y(hover.v)} r={4} />
            </g>
          )}
        </svg>
      )}
      {hover && (
        <div
          className="chart-tooltip"
          style={{
            left: Math.min(Math.max(x(hover.t), 60), width - 60),
            top: Math.max(y(hover.v) - 44, 0),
          }}
        >
          <strong>{formatValue(hover.v)}</strong>
          <span>{formatTime(hover.t)}</span>
        </div>
      )}
      {points.length === 0 && <div className="chart-empty">No samples yet</div>}
    </div>
  );
}
