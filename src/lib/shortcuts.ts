/**
 * The app's own shortcuts. macOS: ⌘ + key (⌘⇧ for the shifted ones).
 * Windows / Linux: Ctrl+Shift + key (Ctrl+Shift+Alt for the shifted ones),
 * because plain Ctrl belongs to the terminal (Ctrl+K, Ctrl+R, … in shells).
 */
export const IS_MAC: boolean = (() => {
  if (typeof navigator === "undefined") return true;
  const p = `${navigator.platform ?? ""} ${navigator.userAgent ?? ""}`;
  return !/Win|Linux|X11|CrOS/i.test(p) || /Mac|iPhone|iPad/i.test(navigator.platform ?? "");
})();

/** The key an app shortcut uses, with Shift taken by the modifier off macOS. */
function keyOf(ev: KeyboardEvent, mac: boolean): string {
  if (mac) return ev.key.toLowerCase();
  // With Shift held, `key` is the shifted character ("!" for 1): use the
  // physical key instead.
  const code = ev.code ?? "";
  if (code.startsWith("Key")) return code.slice(3).toLowerCase();
  if (code.startsWith("Digit")) return code.slice(5);
  const named: Record<string, string> = { Comma: ",", Period: ".", Equal: "=", Minus: "-", Enter: "enter", NumpadEnter: "enter", NumpadAdd: "+", NumpadSubtract: "-" };
  return named[code] ?? ev.key.toLowerCase();
}

/** `{ key, shift }` when `ev` is an app shortcut chord, else null. */
export function chord(ev: KeyboardEvent, mac: boolean = IS_MAC): { key: string; shift: boolean } | null {
  if (mac) {
    if (!ev.metaKey || ev.ctrlKey || ev.altKey) return null;
    return { key: keyOf(ev, true), shift: ev.shiftKey };
  }
  if (!ev.ctrlKey || !ev.shiftKey || ev.metaKey) return null;
  return { key: keyOf(ev, false), shift: ev.altKey };
}

/** Shortcuts the app owns; the terminal must not swallow these. */
export function OWNED_SHORTCUT(ev: KeyboardEvent, mac: boolean = IS_MAC): boolean {
  const c = chord(ev, mac);
  if (!c) return false;
  const k = c.key;
  if (c.shift) return k === "n" || k === "t";
  return (
    k === "k" ||
    k === "j" ||
    k === "n" ||
    k === "t" ||
    k === "b" ||
    k === "e" ||
    k === "r" ||
    k === "." ||
    k === "," ||
    k === "=" ||
    k === "+" ||
    k === "-" ||
    k === "0" ||
    k === "enter" ||
    /^[1-9]$/.test(k)
  );
}

/** A shortcut label for this OS: "⌘⇧T" → "Ctrl+Shift+Alt+T" off macOS. */
export function shortcutLabel(label: string, mac: boolean = IS_MAC): string {
  if (mac || !label.includes("⌘")) return label;
  return label.replace(/⌘(⇧)?/g, (_, shift) => (shift ? "Ctrl+Shift+Alt+" : "Ctrl+Shift+"));
}
