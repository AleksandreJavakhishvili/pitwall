// Releasing Monaco when Review closes (docs/spec/perf.md). Kept apart from
// monaco.ts so Review can call it without pulling Monaco in when it was
// never loaded.

let release: (() => void) | null = null;

/** monaco.ts registers how to dispose what Monaco still holds. */
export function onMonacoRelease(fn: () => void) {
  release = fn;
}

/**
 * Dispose every editor and model left over. With no models, Monaco stops its
 * editor worker right away (instead of after ~5 idle minutes). The code
 * itself stays loaded; the next Review reuses it.
 */
export function releaseMonaco() {
  try {
    release?.();
  } catch {
    // already gone
  }
}
