/** Shortcuts the app owns; the terminal must not swallow these. */
export function OWNED_SHORTCUT(ev: KeyboardEvent): boolean {
  if (!ev.metaKey || ev.ctrlKey || ev.altKey) return false;
  const k = ev.key.toLowerCase();
  if (ev.shiftKey) return k === "n" || k === "t";
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
