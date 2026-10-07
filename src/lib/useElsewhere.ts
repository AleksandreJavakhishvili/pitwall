import { useEffect, useState } from "react";
import { api } from "../api";
import type { RunningElsewhere } from "../types";

/** How often the sidebar's "Elsewhere" group refreshes (one cheap `ps` in the backend). */
export const ELSEWHERE_EVERY_MS = 10_000;

/** Agents running in other terminal apps, refreshed while `enabled` and the window is visible. */
export function useElsewhere(enabled: boolean): RunningElsewhere[] {
  const [rows, setRows] = useState<RunningElsewhere[]>([]);
  useEffect(() => {
    if (!enabled) {
      setRows([]);
      return;
    }
    let alive = true;
    const load = () => {
      if (document.visibilityState === "hidden") return;
      api
        .listElsewhere()
        .then((r) => alive && setRows(r))
        .catch(() => {}); // older backend or no process list: just no group
    };
    load();
    const t = window.setInterval(load, ELSEWHERE_EVERY_MS);
    document.addEventListener("visibilitychange", load);
    return () => {
      alive = false;
      window.clearInterval(t);
      document.removeEventListener("visibilitychange", load);
    };
  }, [enabled]);
  return rows;
}
