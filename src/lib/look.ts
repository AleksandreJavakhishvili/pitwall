// Appearance → Look (Flat / Glass) and Reduce motion (docs/spec/layout.md
// "Appearance").
//
// Like the theme, both live in the shared UI-state blob (every window follows
// them) and are mirrored to localStorage so index.html's early script can set
// the attributes before first paint.
//
// `<html data-look="glass" data-glass="native|mica|lite">`, absent for Flat
// (or when the OS asks for less transparency). The tier comes from what the
// desktop offers (`host().glass`, decided once in pitwall_core::host) and
// whether the native material actually went on for this window:
//   native — macOS vibrancy behind a transparent webview;
//   mica   — Windows 11 Mica behind it, frosted overlays in the webview;
//   lite   — no window material (Linux, Windows 10, the browser mock): the
//            UI paints an atmospheric gradient and tinted panels, no blur.
// `data-contrast="more"`: the OS asks for more contrast (glass goes nearly
// opaque). `data-motion="reduce"`: the Settings toggle; CSS treats it like
// prefers-reduced-motion. `data-hidden`: the page isn't visible; every
// animation is paused.

import type { GlassState, WindowGlass } from "../types";

export type Look = "flat" | "glass";
export type GlassTier = "native" | "mica" | "lite";

export const LOOKS: readonly Look[] = ["flat", "glass"];
export const LOOK_LABEL: Record<Look, string> = { flat: "Flat", glass: "Glass" };
/** localStorage mirrors read by the inline script in index.html — keep the keys in sync. */
export const LOOK_KEY = "pitwall.look";
export const MOTION_KEY = "pitwall.motion";

export const isLook = (v: unknown): v is Look => v === "flat" || v === "glass";

/** The tier a window shows, from what the desktop offers and what the native call did. */
export function glassTier(offer: WindowGlass, state: GlassState | null): GlassTier {
  if (!state?.native || offer === "none") return "lite";
  return offer === "mica" ? "mica" : "native";
}

/** How Settings names Glass on this desktop. */
export function glassLabel(offer: WindowGlass): string {
  return offer === "vibrancy" ? "Glass · native" : offer === "mica" ? "Glass · Mica" : "Glass lite";
}

/** Glass is shown unless the OS asks for less transparency. */
export const showsGlass = (look: Look, reduceTransparency: boolean) => look === "glass" && !reduceTransparency;

// ── runtime (browser only; nothing runs at import time) ─────────────────────

type SetNative = (on: boolean) => Promise<GlassState>;

let want: { look: Look; offer: WindowGlass; setNative: SetNative } | null = null;
let nativeOn = false;
let seq = 0;
let listening = false;

function media(q: string): boolean {
  try {
    return typeof window !== "undefined" && !!window.matchMedia && window.matchMedia(q).matches;
  } catch {
    return false;
  }
}

function mirror(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    /* private mode etc. */
  }
}

function setAttr(name: string, value: string | null) {
  const root = document.documentElement;
  if (value === null) root.removeAttribute(name);
  else root.setAttribute(name, value);
}

async function sync() {
  if (!want) return;
  const { look, offer, setNative } = want;
  const my = ++seq;
  const cssReduce = media("(prefers-reduced-transparency: reduce)");
  let state: GlassState | null = null;
  // Ask the window for its material when Glass is wanted, and to drop it
  // when it was on; Flat on a window that never had it costs no call.
  if (offer !== "none" && (look === "glass" || nativeOn)) {
    try {
      state = await setNative(look === "glass" && !cssReduce);
    } catch {
      state = null; // older backend / not a Tauri window: the lite tier
    }
    if (my !== seq) return;
    nativeOn = !!state?.native;
  }
  const reduce = cssReduce || !!state?.reduceTransparency;
  if (showsGlass(look, reduce)) {
    setAttr("data-look", "glass");
    setAttr("data-glass", glassTier(offer, state));
  } else {
    setAttr("data-look", null);
    setAttr("data-glass", null);
  }
  setAttr("data-contrast", state?.increaseContrast || media("(prefers-contrast: more)") ? "more" : null);
}

function listen() {
  if (listening || typeof window === "undefined") return;
  listening = true;
  // The OS settings can change while Pitwall runs: look again when the
  // window comes back (macOS has no media query WebKit reports for them all).
  window.addEventListener("focus", () => {
    if (want?.look === "glass") void sync();
  });
  for (const q of ["(prefers-reduced-transparency: reduce)", "(prefers-contrast: more)"]) {
    try {
      window.matchMedia?.(q).addEventListener("change", () => void sync());
    } catch {
      /* unsupported query */
    }
  }
  // Nothing animates while the page can't be seen (hidden, minimised, occluded).
  const vis = () => setAttr("data-hidden", document.visibilityState === "hidden" ? "" : null);
  document.addEventListener("visibilitychange", vis);
  vis();
}

/** Apply the look to this window: attributes, mirror, native material. */
export function applyLook(look: Look, offer: WindowGlass, setNative: SetNative) {
  mirror(LOOK_KEY, look);
  want = { look, offer, setNative };
  listen();
  void sync();
}

/** Settings → Appearance → Reduce motion (prefers-reduced-motion applies regardless). */
export function applyMotion(reduce: boolean) {
  mirror(MOTION_KEY, reduce ? "reduce" : null);
  setAttr("data-motion", reduce ? "reduce" : null);
}
