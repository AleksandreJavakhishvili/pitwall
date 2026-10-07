import { shortcutLabel } from "../lib/shortcuts";

/** A shortcut, written the way this OS spells it (⌘K / Ctrl+Shift+K). */
export function Kbd({ children }: { children: string }) {
  return <kbd className="kbd">{shortcutLabel(children)}</kbd>;
}
