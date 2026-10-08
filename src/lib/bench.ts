// Benchmark hook (scripts/bench.sh, docs/spec/perf.md). The backend only
// sends `bench` events when the app runs with PITWALL_BENCH=1 (src-tauri
// bench.rs); otherwise nothing here ever fires. Never used in the browser mock.

export interface BenchHandlers {
  wall(on: boolean): void;
  review(on: boolean): void;
  /** Show every agent once (so each gets its terminal), then tile with Auto grid. */
  visitAll(): Promise<void>;
  /** Open the palette or Settings (screenshots of overlays); null closes. */
  modal(which: "palette" | "settings" | null): void;
}

/** Subscribe to bench commands; tells the backend once the UI is up. */
export async function installBench(h: () => BenchHandlers): Promise<() => void> {
  const { listen, emit } = await import("@tauri-apps/api/event");
  const off = await listen<string>("bench", (e) => {
    const [cmd, arg] = e.payload.trim().split(/\s+/);
    const on = arg !== "off";
    if (cmd === "wall") h().wall(on);
    else if (cmd === "review") h().review(on);
    else if (cmd === "visit-all") void h().visitAll();
    else if (cmd === "palette" || cmd === "settings") h().modal(on ? cmd : null);
  });
  await emit("bench-ready");
  return off;
}
