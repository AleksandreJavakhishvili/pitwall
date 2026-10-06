import { useEffect, useRef, type ReactNode } from "react";

interface Props {
  title?: string;
  onClose(): void;
  children: ReactNode;
  width?: number;
  className?: string;
  /** Palette-style: anchored near the top, no header. */
  variant?: "dialog" | "palette" | "sheet";
}

export function Modal({ title, onClose, children, width = 480, className = "", variant = "dialog" }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const prevFocus = useRef<Element | null>(document.activeElement);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeRef.current();
      }
    };
    window.addEventListener("keydown", onKey, true);
    const el = ref.current?.querySelector<HTMLElement>("[data-autofocus], input, textarea, select, button");
    el?.focus();
    const prev = prevFocus.current;
    return () => {
      window.removeEventListener("keydown", onKey, true);
      if (prev instanceof HTMLElement) prev.focus();
    };
  }, []);

  return (
    <div className={`backdrop backdrop-${variant}`} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={ref} className={`modal modal-${variant} ${className}`} style={{ width }} role="dialog" aria-modal="true" aria-label={title}>
        {title && (
          <header className="modal-head">
            <h2 className="label-lg">{title}</h2>
            <button className="icon-btn" onClick={onClose} aria-label="Close" title="Close (esc)">
              ✕
            </button>
          </header>
        )}
        {children}
      </div>
    </div>
  );
}
