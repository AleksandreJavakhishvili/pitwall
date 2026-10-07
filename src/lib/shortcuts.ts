import { appChord } from "./host";

const PLAIN = new Set(["k", "j", "n", "t", "b", "e", "r", ".", ",", "=", "+", "-", "0", "enter", "1", "2", "3", "4", "5", "6", "7", "8", "9"]);
const SHIFTED = new Set(["n", "t", "r"]);

/**
 * Shortcuts the app owns; the terminal must not swallow these. The chord is
 * ⌘ or Ctrl+Shift, as this desktop says (`lib/host.ts`).
 */
export function OWNED_SHORTCUT(ev: KeyboardEvent): boolean {
  const c = appChord(ev);
  if (!c) return false;
  return c.shift ? SHIFTED.has(c.key) : PLAIN.has(c.key);
}
