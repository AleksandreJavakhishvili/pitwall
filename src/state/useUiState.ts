import { useCallback, useEffect, useRef, useState } from "react";
import { api, getApi } from "../api";
import { defaultUiState, sanitize, type UiState } from "./workspace";

/**
 * The shared UI blob: loaded once, written back (debounced) on every change,
 * and replaced when another window changes it.
 */
export function useUiState(label: string | null) {
  const [ui, setUi] = useState<UiState>(defaultUiState);
  const [ready, setReady] = useState(false);
  const ref = useRef(ui);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (!label) return;
    let alive = true;
    let off: (() => void) | null = null;
    (async () => {
      const a = await getApi();
      const u = await a.onUiStateChanged((e) => {
        if (e.sourceWindow === label) return;
        const next = sanitize(e.state);
        ref.current = next;
        setUi(next);
      });
      if (!alive) return u();
      off = u;
      const raw = await api.getUiState().catch(() => null);
      if (!alive) return;
      const s = sanitize(raw);
      ref.current = s;
      setUi(s);
      setReady(true);
    })();
    return () => {
      alive = false;
      off?.();
    };
  }, [label]);

  const flush = useCallback(() => {
    window.clearTimeout(timer.current);
    timer.current = undefined;
    return api.setUiState(ref.current).catch(() => {});
  }, []);

  /** Apply a pure update; persists ~150ms later (coalesced). */
  const update = useCallback(
    (fn: (s: UiState) => UiState) => {
      const next = fn(ref.current);
      if (next === ref.current) return;
      ref.current = next;
      setUi(next);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(flush, 150);
    },
    [flush],
  );

  return { ui, ready, update, flush, current: ref };
}
