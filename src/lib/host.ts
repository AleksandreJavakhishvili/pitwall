// What this desktop offers (`host_info`, pitwall_core::host::HostInfo): the
// shortcut modifier, how this machine is named, Pitwall's data folder, Dock /
// tray, menu bar, badge, local sockets and the Glass look's window material. The UI
// decides by these capabilities, never by which OS it runs on. Loaded once
// before the first render (main.tsx); until then, and when the backend is too
// old to say, the macOS values apply.
import type { HostInfo, ShortcutModifier } from "../types";

export const DEFAULT_HOST: HostInfo = {
  machineLabel: "This Mac",
  shortcuts: "meta",
  dataDir: "~/.pitwall",
  dock: true,
  tray: false,
  menu: "app",
  badge: "dock",
  localSockets: "unix",
  glass: "vibrancy",
};

let current: HostInfo = DEFAULT_HOST;

export function host(): HostInfo {
  return current;
}

export function setHost(info: HostInfo | null | undefined): void {
  current = info ? { ...DEFAULT_HOST, ...info } : DEFAULT_HOST;
}

/** Ask the backend once; never blocks the app for long or fails it. */
export async function loadHost(get: () => Promise<HostInfo>, timeoutMs = 1500): Promise<void> {
  const timeout = new Promise<null>((resolve) => setTimeout(() => resolve(null), timeoutMs));
  setHost(await Promise.race([get().catch(() => null), timeout]));
}

// ── shortcuts ────────────────────────────────────────────────────────────

/**
 * An app shortcut as pressed: `key` (lower-case letter or digit, ".", ",",
 * "=", "+", "-", "enter") and whether it is the shifted variant (⌘⇧N). `null`:
 * not an app chord.
 *
 * - "meta": ⌘ + key, ⌘⇧ + key for the variant.
 * - "ctrlShift": Ctrl+Shift + key, Ctrl+Shift+Alt + key for the variant
 *   (Ctrl alone belongs to the terminal). Shift changes `key` ("!" for 1),
 *   so the physical key (`code`) is read instead.
 */
export function appChord(ev: Pick<KeyboardEvent, "key" | "code" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">, mod: ShortcutModifier = current.shortcuts): { key: string; shift: boolean } | null {
  if (mod === "meta") {
    if (!ev.metaKey || ev.ctrlKey || ev.altKey) return null;
    return { key: ev.key.toLowerCase(), shift: ev.shiftKey };
  }
  if (!ev.ctrlKey || !ev.shiftKey || ev.metaKey) return null;
  const key = keyOfCode(ev.code);
  return key ? { key, shift: ev.altKey } : null;
}

/**
 * Copy / paste in a terminal where Ctrl+C is the terminal's own
 * ("ctrlShift": Ctrl+Shift+C / Ctrl+Shift+V, as in other terminal apps).
 * With ⌘ the native Edit menu already does it: `null`.
 */
export function terminalClipboardChord(ev: Pick<KeyboardEvent, "code" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">, mod: ShortcutModifier = current.shortcuts): "copy" | "paste" | null {
  if (mod === "meta" || !ev.ctrlKey || !ev.shiftKey || ev.altKey || ev.metaKey) return null;
  return ev.code === "KeyC" ? "copy" : ev.code === "KeyV" ? "paste" : null;
}

const CODE_KEYS: Record<string, string> = {
  Period: ".",
  Comma: ",",
  Equal: "=",
  Minus: "-",
  NumpadAdd: "+",
  NumpadSubtract: "-",
  Enter: "enter",
  NumpadEnter: "enter",
};

function keyOfCode(code: string | undefined): string | null {
  if (!code) return null;
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1].toLowerCase();
  const digit = /^(?:Digit|Numpad)([0-9])$/.exec(code);
  if (digit) return digit[1];
  return CODE_KEYS[code] ?? null;
}

/**
 * A shortcut label written the macOS way ("⌘K", "⌘⇧N", "⌘+ / ⌘−") for this
 * desktop: unchanged with ⌘, "Ctrl+Shift+K" / "Ctrl+Shift+Alt+N" otherwise.
 * `submit`: a form's ⌘↵, which is plain Ctrl+↵ there.
 */
export function keys(label: string, opts: { submit?: boolean; compact?: boolean; mod?: ShortcutModifier } = {}): string {
  const mod = opts.mod ?? current.shortcuts;
  if (mod === "meta") return label;
  if (opts.submit) return label.replace(/⌘/g, opts.compact ? "⌃" : "Ctrl+");
  if (opts.compact) return label.replace(/⌘(⇧)?/g, (_m, shift?: string) => (shift ? "⌃⇧⌥" : "⌃⇧"));
  return label.replace(/⌘(⇧)?/g, (_m, shift?: string) => (shift ? "Ctrl+Shift+Alt+" : "Ctrl+Shift+"));
}
