import type { Toast } from "../lib/useToasts";

const GLYPH = { blocked: "▲", done: "⚑", error: "✕", info: "●" } as const;

export function Toasts({ toasts, onDismiss, onJump }: { toasts: Toast[]; onDismiss(id: number): void; onJump(agentId: string): void }) {
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className="toast" data-tone={t.tone}>
          <span className="toast-glyph">{GLYPH[t.tone]}</span>
          <button
            className="toast-body"
            onClick={() => {
              if (t.onClick) t.onClick();
              else if (t.agentId) onJump(t.agentId);
              onDismiss(t.id);
            }}
          >
            <span className="toast-title">{t.title}</span>
            {t.detail && <span className="toast-detail">{t.detail}</span>}
          </button>
          <button className="icon-btn icon-btn-sm" onClick={() => onDismiss(t.id)} aria-label="Dismiss">
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}
