import { useEffect, useRef } from "react";
import { OWNED_SHORTCUT } from "./shortcuts";

export interface ShortcutHandlers {
  palette(): void;
  newAgent(): void;
  /** ⌘T: a terminal where you are. */
  newTerminal?(): void;
  /** ⌘⇧T: a terminal in a folder you choose. */
  newTerminalAt?(): void;
  nextBlocked(): void;
  toggleSidebar(): void;
  toggleRight(): void;
  toggleWall(): void;
  /** ⌘R (also stops the webview from reloading). */
  toggleReview?(): void;
  toggleMaximize(): void;
  /** ⌘, (the native menu's Settings… item sends the same request). */
  settings?(): void;
  moveSpaceToWindow(): void;
  fontSize(delta: number | null): void;
  selectIndex(i: number): void;
}

/** Global ⌘ shortcuts, captured before the terminal sees them. */
export function useShortcuts(h: ShortcutHandlers) {
  const ref = useRef(h);
  ref.current = h;
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (!OWNED_SHORTCUT(ev)) return;
      const k = ev.key.toLowerCase();
      const x = ref.current;
      ev.preventDefault();
      ev.stopPropagation();
      if (ev.shiftKey && k === "n") x.moveSpaceToWindow();
      else if (ev.shiftKey && k === "t") x.newTerminalAt?.();
      else if (k === "t") x.newTerminal?.();
      else if (k === "k") x.palette();
      else if (k === "n") x.newAgent();
      else if (k === "j") x.nextBlocked();
      else if (k === "b") x.toggleSidebar();
      else if (k === "e") x.toggleWall();
      else if (k === "r") x.toggleReview?.();
      else if (k === ".") x.toggleRight();
      else if (k === ",") x.settings?.();
      else if (k === "enter") x.toggleMaximize();
      else if (k === "=" || k === "+") x.fontSize(1);
      else if (k === "-") x.fontSize(-1);
      else if (k === "0") x.fontSize(null);
      else x.selectIndex(Number(k) - 1);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
}
