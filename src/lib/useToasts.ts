import { useCallback, useRef, useState } from "react";

export type ToastTone = "blocked" | "done" | "error" | "info";
export interface Toast {
  id: number;
  tone: ToastTone;
  title: string;
  detail?: string;
  agentId?: string;
  /** Clicking the toast does this (instead of jumping to `agentId`). */
  onClick?(): void;
}

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const seq = useRef(0);

  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  const push = useCallback(
    (t: Omit<Toast, "id">) => {
      const id = ++seq.current;
      setToasts((list) => [...list.filter((x) => !(t.agentId && x.agentId === t.agentId)), { ...t, id }].slice(-4));
      const ttl = t.tone === "blocked" ? 9000 : t.tone === "error" ? 8000 : 5000;
      setTimeout(() => dismiss(id), ttl);
    },
    [dismiss],
  );

  return { toasts, push, dismiss };
}
