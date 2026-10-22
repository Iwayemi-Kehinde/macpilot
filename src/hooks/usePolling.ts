import { useEffect, useState } from "react";

export interface PollState<T> {
  data?: T;
  error?: string;
  /** When `data` was last fetched successfully. */
  updatedAt?: number;
}

/**
 * Calls `fetcher` every `intervalMs`. Stops while the window is hidden so the
 * app costs nothing in the background, and fetches at once when it returns.
 * On failure the last good data is kept, and `error` marks it as stale.
 */
export function usePolling<T>(fetcher: () => Promise<T>, intervalMs: number): PollState<T> {
  const [state, setState] = useState<PollState<T>>({});

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    let inFlight = false;

    const tick = async () => {
      window.clearTimeout(timer);
      if (document.hidden || inFlight) return;
      inFlight = true;
      try {
        const data = await fetcher();
        if (!cancelled) setState({ data, updatedAt: Date.now() });
      } catch (e) {
        if (!cancelled) setState((s) => ({ ...s, error: e instanceof Error ? e.message : String(e) }));
      } finally {
        inFlight = false;
      }
      if (!cancelled && !document.hidden) timer = window.setTimeout(tick, intervalMs);
    };

    const onVisibility = () => {
      if (!document.hidden) void tick();
    };

    void tick();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [fetcher, intervalMs]);

  return state;
}
