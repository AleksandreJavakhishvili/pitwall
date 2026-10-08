// Benchmark hook of the old Tauri shell (scripts/bench.sh, docs/spec/perf.md).
// This UI now runs only as the website's demo (src/README.md), which has no
// backend to send `bench` commands: the hook is inert and kept so App.tsx's
// bench handlers stay type-checked.

export interface BenchHandlers {
  wall(on: boolean): void;
  review(on: boolean): void;
  /** Show every agent once (so each gets its terminal), then tile with Auto grid. */
  visitAll(): Promise<void>;
  /** Open the palette or Settings (screenshots of overlays); null closes. */
  modal(which: "palette" | "settings" | null): void;
}

/** Nothing to subscribe to in the browser; resolves to a no-op unsubscribe. */
export async function installBench(_h: () => BenchHandlers): Promise<() => void> {
  return () => {};
}
