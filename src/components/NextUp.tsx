import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import type { AgentView } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { relTime } from "../lib/time";
import { useNow } from "../lib/useNow";
import { chipLabel, closeNextUp, openNextUp, useNextUpOpen } from "../lib/nextUp";
import { Icon } from "./Icon";

/**
 * Next up on an agent's tile header: "2 queued" while something waits, and
 * the popover it opens (the queue, Auto-send, the last prompt sent). Nothing
 * is typed here: prompts are queued from the palette ("name: prompt"),
 * Review, `pitwall queue` or an extension.
 */
export function NextUpChip({ agent: a }: { agent: AgentView }) {
  const open = useNextUpOpen() === a.id;
  const anchor = useRef<HTMLSpanElement>(null);
  const label = chipLabel(a.queue.length);
  if (!open && !label) return null;
  const fromExtension = a.queue.some((q) => q.origin);
  return (
    <span className="next-up-anchor" ref={anchor} onPointerDown={(e) => e.stopPropagation()}>
      {label && (
        <button
          className="small-btn next-up-chip"
          data-ext={fromExtension || undefined}
          aria-pressed={open}
          aria-haspopup="dialog"
          title="Next up: what is sent to this agent next"
          onMouseDown={(e) => e.stopPropagation()}
          onClick={() => (open ? closeNextUp() : openNextUp(a.id))}
        >
          <Icon name="send" size={11} />
          {label}
        </button>
      )}
      {open && <NextUp agent={a} anchor={anchor} />}
    </span>
  );
}

/** Popover width (px), as in Pitwall. */
const WIDTH = 340;

/**
 * The popover, under its chip with their right edges lined up, kept on
 * screen (in a portal: the tile clips what overflows it). Esc or a press
 * outside it and its chip closes it.
 */
function NextUp({ agent: a, anchor }: { agent: AgentView; anchor: RefObject<HTMLSpanElement | null> }) {
  const { run, patch } = useActions();
  const ref = useRef<HTMLDivElement>(null);
  const now = useNow();
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);

  useLayoutEffect(() => {
    const place = () => {
      const r = anchor.current?.getBoundingClientRect();
      if (!r) return;
      const h = ref.current?.getBoundingClientRect().height ?? 0;
      setPos({
        x: Math.max(4, Math.min(r.right - WIDTH, window.innerWidth - WIDTH - 4)),
        y: Math.max(4, Math.min(r.bottom + 4, window.innerHeight - h - 4)),
      });
    };
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [anchor, a.queue.length]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      closeNextUp();
    };
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!ref.current?.contains(t) && !anchor.current?.contains(t)) closeNextUp();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("mousedown", onDown, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("mousedown", onDown, true);
    };
  }, [anchor]);

  const act = async (p: Promise<AgentView>, what: string) => {
    const r = await run(p, what);
    if (r) patch(r);
  };

  return createPortal(
    <div
      ref={ref}
      className="next-up"
      role="dialog"
      aria-label={`Next up for ${a.name}`}
      style={{ left: pos?.x ?? 0, top: pos?.y ?? 0, width: WIDTH, visibility: pos ? undefined : "hidden" }}
    >
      <div className="next-up-head">
        <span className="label">Next up</span>
        {a.queue.length > 0 && <span className="label-count">{a.queue.length}</span>}
        <span className="spacer" />
        <label className="toggle" title="Send the next item when the agent is idle or done and you're not typing">
          <input type="checkbox" checked={a.autoSend} onChange={(e) => act(api.setAutoSend(a.id, e.target.checked), "change auto-send")} />
          <span className="toggle-track" aria-hidden />
          <span className="toggle-label">Auto-send</span>
        </label>
      </div>

      {a.queue.length === 0 ? (
        <p className="hint next-up-empty">Nothing queued. Type “name: prompt” in the palette (⌘K) to add one.</p>
      ) : (
        <ol className="queue">
          {a.queue.map((q, i) => (
            <li key={q.id} className="queue-item" data-ext={q.origin ? true : undefined}>
              <span className="queue-n">{i + 1}</span>
              <span className="queue-body">
                {q.origin && (
                  <span className="queue-from">
                    <b>From {q.origin.name}</b> · waits for you: press Send
                  </span>
                )}
                <span className="queue-text">{q.text}</span>
              </span>
              <span className="queue-actions">
                {a.running && (
                  <button className="icon-btn icon-btn-sm" title="Send now" onClick={() => act(api.queueSendNow(a.id, q.id), "send prompt")}>
                    <Icon name="send" size={14} />
                  </button>
                )}
                <button className="icon-btn icon-btn-sm" title="Remove from queue" onClick={() => act(api.queueRemove(a.id, q.id), "remove queue item")}>
                  <Icon name="x" size={14} />
                </button>
              </span>
            </li>
          ))}
        </ol>
      )}

      {a.lastSent && (
        <div className="next-up-last" title={a.lastSent}>
          <b>Last sent</b>
          {a.lastSentAt && (
            <time className="mono" dateTime={new Date(a.lastSentAt).toISOString()} title={new Date(a.lastSentAt).toLocaleString()}>
              {relTime(a.lastSentAt, now)}
            </time>
          )}
          <span className="next-up-text">{a.lastSent.split(/\s+/).join(" ")}</span>
        </div>
      )}
    </div>,
    document.body,
  );
}
