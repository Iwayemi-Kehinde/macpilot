const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** Disk and network sizes use decimal units, as Finder does; memory uses binary (base 1024), as Activity Monitor does. */
export function formatBytes(value: number, base: 1000 | 1024 = 1000): string {
  if (!Number.isFinite(value) || value < 0) return "-";
  let v = value;
  let unit = 0;
  while (v >= base && unit < UNITS.length - 1) {
    v /= base;
    unit++;
  }
  const digits = unit === 0 || v >= 100 ? 0 : 1;
  return `${v.toFixed(digits)} ${UNITS[unit]}`;
}

export function formatRate(bytesPerSec: number): string {
  return `${formatBytes(bytesPerSec)}/s`;
}

export function formatPercent(value: number, digits = 0): string {
  return `${value.toFixed(digits)}%`;
}

export function formatMinutes(totalMinutes: number): string {
  const m = Math.round(totalMinutes);
  const h = Math.floor(m / 60);
  const rest = m % 60;
  if (h === 0) return `${rest} min`;
  return rest === 0 ? `${h} h` : `${h} h ${rest} min`;
}

export function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days} d ${hours} h`;
  if (hours > 0) return `${hours} h ${minutes} min`;
  return `${minutes} min`;
}

export function formatClock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function formatClockSeconds(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function formatDayClock(ms: number): string {
  return new Date(ms).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" });
}

export function formatWatts(w: number): string {
  if (!Number.isFinite(w)) return "-";
  const a = Math.abs(w);
  if (a < 0.01) return a === 0 ? "0 W" : "<0.01 W";
  return `${a < 1 ? a.toFixed(2) : a < 100 ? a.toFixed(1) : a.toFixed(0)} W`;
}

/** "a", "a and b", "a, b and c". */
export function formatList(items: string[]): string {
  return items.length <= 1 ? (items[0] ?? "") : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}
