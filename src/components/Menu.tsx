import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

export interface MenuItem {
  id: string;
  label: ReactNode;
  icon?: ReactNode;
  hint?: ReactNode;
  run(): void;
}

interface Props {
  /** Viewport point the menu opens at (a click or the anchor's corner). */
  at: { x: number; y: number };
  items: MenuItem[];
  label: string;
  onClose(): void;
}

/** A small popup menu (project "+" button, right-click). Esc / click outside closes it. */
export function Menu({ at, items, label, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(at);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  // Keep it on screen.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(at.x, window.innerWidth - r.width - 4)),
      y: Math.max(4, Math.min(at.y, window.innerHeight - r.height - 4)),
    });
  }, [at.x, at.y]);

  useEffect(() => {
    ref.current?.querySelector<HTMLElement>("button")?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeRef.current();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const buttons = [...(ref.current?.querySelectorAll<HTMLElement>("button") ?? [])];
        const i = buttons.indexOf(document.activeElement as HTMLElement);
        const next = e.key === "ArrowDown" ? (i + 1) % buttons.length : (i - 1 + buttons.length) % buttons.length;
        buttons[next]?.focus();
      }
    };
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) closeRef.current();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("blur", closeRef.current);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("blur", closeRef.current);
    };
  }, []);

  return createPortal(
    <div ref={ref} className="menu" role="menu" aria-label={label} style={{ left: pos.x, top: pos.y }}>
      {items.map((it) => (
        <button
          key={it.id}
          role="menuitem"
          className="menu-item"
          onClick={() => {
            onClose();
            it.run();
          }}
        >
          <span className="menu-icon">{it.icon}</span>
          <span className="menu-label">{it.label}</span>
          {it.hint && <span className="menu-hint">{it.hint}</span>}
        </button>
      ))}
    </div>,
    document.body,
  );
}
