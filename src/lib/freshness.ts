// Source-control freshness: what a view shows, how old it is, and the
// user's "refresh now". A view reads on open with a forced refresh (the
// backend bypasses its polling back-off), then polls while the window is
// visible; ↻ / ⌘⇧R force again. Errors stay next to the data, with Retry.
import { useCallback, useEffect, useRef, useState } from "react";
import { errorText } from "../api";

export interface Fresh<T> {
  data: T | null;
  error: string | null;
  /** A forced refresh (on open, ↻, ⌘⇧R) is running. Polls are silent. */
  refreshing: boolean;
  /** When the data was last read (Date.now() ms), or null before the first read. */
  updatedAt: number | null;
}

export interface FreshSource<T> {
  /** A regular read (polling, and after the view's numbers move). */
  load(): Promise<T>;
  /** A forced refresh; defaults to `load`. */
  force?(): Promise<T>;
  /** Poll this often while the window is visible (0 / omitted: never). */
  everyMs?: number;
  /** Start with a forced refresh (default) or a plain read. */
  forceOnStart?: boolean;
}

/** Time, timers and window visibility (tests pass their own). */
export interface FreshEnv {
  now(): number;
  hidden(): boolean;
  every(ms: number, fn: () => void): () => void;
  /** Call `fn` when the window becomes visible again; returns an unsubscribe. */
  onVisible(fn: () => void): () => void;
}

export const browserEnv: FreshEnv = {
  now: () => Date.now(),
  hidden: () => typeof document !== "undefined" && document.visibilityState === "hidden",
  every(ms, fn) {
    const t = setInterval(fn, ms);
    return () => clearInterval(t);
  },
  onVisible(fn) {
    if (typeof document === "undefined") return () => {};
    const h = () => {
      if (document.visibilityState !== "hidden") fn();
    };
    document.addEventListener("visibilitychange", h);
    return () => document.removeEventListener("visibilitychange", h);
  },
};

export const EMPTY_FRESH: Fresh<never> = { data: null, error: null, refreshing: false, updatedAt: null };

const same = (a: unknown, b: unknown) => a === b || JSON.stringify(a) === JSON.stringify(b);

/**
 * One view's data and freshness. Forced refreshes coalesce (asked again while
 * one runs, the same one is awaited); polls are skipped while anything is in
 * flight and while the window is hidden; nothing lands after `stop`.
 */
export class FreshController<T> {
  private s: Fresh<T> = EMPTY_FRESH;
  private alive = false;
  private forcing: Promise<void> | null = null;
  private polling = false;
  private stops: (() => void)[] = [];

  constructor(
    private src: FreshSource<T>,
    private onChange: (s: Fresh<T>) => void,
    private env: FreshEnv = browserEnv,
  ) {}

  get state(): Fresh<T> {
    return this.s;
  }

  start(): void {
    if (this.alive) return;
    this.alive = true;
    if (this.src.forceOnStart === false) void this.poll();
    else void this.refresh();
    const every = this.src.everyMs ?? 0;
    if (every > 0) {
      this.stops.push(this.env.every(every, () => void this.poll()));
      this.stops.push(this.env.onVisible(() => void this.poll()));
    }
  }

  stop(): void {
    this.alive = false;
    this.stops.forEach((s) => s());
    this.stops = [];
  }

  /** Force a refresh now (`run`: this time's own forced read); one already running is awaited. */
  refresh(run?: () => Promise<T>): Promise<void> {
    if (this.forcing) return this.forcing;
    const go = run ?? this.src.force ?? this.src.load;
    this.set({ refreshing: true });
    const p = go
      .call(this.src)
      .then(
        (data) => this.landed(data),
        (e) => this.failed(e),
      )
      .finally(() => {
        this.forcing = null;
        this.set({ refreshing: false });
      });
    this.forcing = p;
    return p;
  }

  /** A regular read, unless the window is hidden or a read is already running. */
  poll(): Promise<void> {
    if (!this.alive || this.forcing || this.polling || this.env.hidden()) return Promise.resolve();
    this.polling = true;
    return this.src
      .load()
      .then(
        (data) => this.landed(data),
        (e) => this.failed(e),
      )
      .finally(() => {
        this.polling = false;
      });
  }

  private landed(data: T) {
    this.set({ data: same(this.s.data, data) ? this.s.data : data, error: null, updatedAt: this.env.now() });
  }

  private failed(e: unknown) {
    this.set({ error: errorText(e) });
  }

  private set(p: Partial<Fresh<T>>) {
    if (!this.alive) return;
    const next = { ...this.s, ...p };
    if (next.data === this.s.data && next.error === this.s.error && next.refreshing === this.s.refreshing && next.updatedAt === this.s.updatedAt) return;
    this.s = next;
    this.onChange(next);
  }
}

export interface FreshHandle<T> extends Fresh<T> {
  /** Force a refresh now (↻, Retry); `run` replaces the source's forced read this time. */
  refresh(run?: () => Promise<T>): Promise<void>;
  /** A regular read now (the view's numbers moved). */
  poll(): Promise<void>;
}

/**
 * A view's data while `key` is set: a forced refresh when it opens or `key`
 * changes, polling every `everyMs` while the window is visible, and the ⌘⇧R
 * shortcut (`requestRefresh`). `make` is read when `key` changes.
 */
export function useFresh<T>(key: string | null, make: () => FreshSource<T>, env: FreshEnv = browserEnv): FreshHandle<T> {
  const [state, setState] = useState<Fresh<T>>(EMPTY_FRESH);
  const ctl = useRef<FreshController<T> | null>(null);
  useEffect(() => {
    if (key === null) {
      setState(EMPTY_FRESH);
      return;
    }
    const c = new FreshController(make(), setState, env);
    ctl.current = c;
    setState(EMPTY_FRESH);
    c.start();
    const off = onRefreshRequest(() => void c.refresh());
    return () => {
      off();
      c.stop();
      if (ctl.current === c) ctl.current = null;
    };
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps
  const refresh = useCallback((run?: () => Promise<T>) => ctl.current?.refresh(run) ?? Promise.resolve(), []);
  const poll = useCallback(() => ctl.current?.poll() ?? Promise.resolve(), []);
  return { ...state, refresh, poll };
}

// ── ⌘⇧R ─────────────────────────────────────────────────────────────────

const refreshListeners = new Set<() => void>();

/** ⌘⇧R (Ctrl+Shift+Alt+R where the chord is Ctrl+Shift): every open source-control view refreshes. */
export function requestRefresh(): void {
  refreshListeners.forEach((l) => l());
}

export function onRefreshRequest(fn: () => void): () => void {
  refreshListeners.add(fn);
  return () => {
    refreshListeners.delete(fn);
  };
}

/** Run `fn` on ⌘⇧R while mounted. */
export function useRefreshRequest(fn: () => void): void {
  const ref = useRef(fn);
  ref.current = fn;
  useEffect(() => onRefreshRequest(() => ref.current()), []);
}

// ── labels ───────────────────────────────────────────────────────────────

/** "updated just now", "updated 12 s ago", "updated 3 min ago", "updated 2 h ago". */
export function updatedLabel(at: number, now: number): string {
  const s = Math.max(0, Math.round((now - at) / 1000));
  if (s < 5) return "updated just now";
  if (s < 60) return `updated ${s} s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `updated ${m} min ago`;
  return `updated ${Math.floor(m / 60)} h ago`;
}
