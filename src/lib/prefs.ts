/** Reads a remembered choice, falling back when storage is unavailable or holds something unexpected. */
export function loadChoice(key: string, allowed: number[], fallback: number): number {
  try {
    const v = Number(localStorage.getItem(key));
    return allowed.includes(v) ? v : fallback;
  } catch {
    return fallback;
  }
}

export function saveChoice(key: string, value: number): void {
  try {
    localStorage.setItem(key, String(value));
  } catch {
    // The choice is only kept for this session.
  }
}
