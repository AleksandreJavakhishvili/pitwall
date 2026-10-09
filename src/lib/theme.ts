// Appearance: System (follow macOS) / Dark / Light.
//
// The preference lives in the shared UI-state blob (every window follows it and
// it survives restarts). It is mirrored to localStorage so index.html's early
// script can set `data-theme` before first paint; the blob wins once loaded.
//
// `<html data-theme>`: "dark" | "light", absent for System (CSS falls back to
// prefers-color-scheme — see styles/tokens.css). Non-CSS consumers (xterm,
// the Review diff) read `currentScheme()` and subscribe with `onSchemeChange()`.

export type ThemePref = "system" | "dark" | "light";
export type Scheme = "dark" | "light";

export const THEME_PREFS: readonly ThemePref[] = ["system", "dark", "light"];
export const THEME_LABEL: Record<ThemePref, string> = { system: "System", dark: "Dark", light: "Light" };
/** localStorage mirror read by the inline script in index.html — keep the key in sync. */
export const THEME_KEY = "pitwall.theme";

export const isThemePref = (v: unknown): v is ThemePref => v === "system" || v === "dark" || v === "light";

/** The scheme actually shown for a preference, given whether the OS is dark. */
export function resolveScheme(pref: ThemePref, osDark: boolean): Scheme {
  if (pref === "system") return osDark ? "dark" : "light";
  return pref;
}

/** Value for `<html data-theme>`; null removes the attribute (System). */
export const themeAttr = (pref: ThemePref): Scheme | null => (pref === "system" ? null : pref);

// ── runtime (browser only; nothing runs at import time) ─────────────────────

let pref: ThemePref | null = null;
let scheme: Scheme | null = null;
let osQuery: MediaQueryList | null = null;
const listeners = new Set<(s: Scheme) => void>();

function query(): MediaQueryList | null {
  if (!osQuery && typeof window !== "undefined" && window.matchMedia) {
    osQuery = window.matchMedia("(prefers-color-scheme: dark)");
    // While on System, follow the OS. (When forced, Tauri's setTheme also flips
    // this query; recomputing from the preference keeps that harmless.)
    osQuery.addEventListener("change", () => recompute());
  }
  return osQuery;
}

function readMirror(): ThemePref {
  try {
    const v = localStorage.getItem(THEME_KEY);
    return isThemePref(v) ? v : "system";
  } catch {
    return "system";
  }
}

function currentPref(): ThemePref {
  if (pref === null) pref = readMirror();
  return pref;
}

function recompute() {
  const next = resolveScheme(currentPref(), query()?.matches ?? true);
  if (next === scheme) return;
  scheme = next;
  for (const fn of listeners) fn(next);
}

/** The scheme on screen now. */
export function currentScheme(): Scheme {
  if (scheme === null) scheme = resolveScheme(currentPref(), query()?.matches ?? true);
  return scheme;
}

/** Called with the new scheme whenever it changes (setting or OS). */
export function onSchemeChange(fn: (s: Scheme) => void): () => void {
  query();
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Apply a preference to this window: CSS attribute, mirror, listeners. */
export function applyTheme(p: ThemePref) {
  pref = p;
  const attr = themeAttr(p);
  const root = document.documentElement;
  if (attr) root.setAttribute("data-theme", attr);
  else root.removeAttribute("data-theme");
  try {
    localStorage.setItem(THEME_KEY, p);
  } catch {
    /* private mode etc. */
  }
  recompute();
}
