// Drag & drop of agents (sidebar rows, rail items, pane headers, chips, Wall
// tiles) onto panes and space tabs.
//
// Pointer-driven rather than HTML5 DnD: in the Tauri WKWebView the native
// drag handler intercepts HTML5 drag events, WebKit hides dataTransfer data
// during dragover, dragleave flickers over child elements, and xterm's
// canvases swallow drag events. Pointer events plus elementFromPoint work the
// same everywhere, and give us Escape-to-cancel and a ghost for free.
//
// Drop targets are plain DOM attributes, hit-tested under the pointer:
//   data-drop="pane"  data-space-id  data-pane-id  data-agent-id? (empty pane: none)
//   data-drop="space" data-space-id   (a space tab or the space area: auto-place)
//   data-drop="new-space"             (the "+" tab)
import { useSyncExternalStore, type PointerEvent as RPointerEvent } from "react";
import { dropZone, type Side } from "../layout/tree";

export type AgentDropTarget =
  | { kind: "pane"; spaceId: string; paneId: string; zone: Side | "center" }
  | { kind: "space"; spaceId: string }
  | { kind: "new-space" };

interface DragState {
  agentId: string;
  target: AgentDropTarget | null;
  /** Hovering the agent's own pane: dropping there is a no-op, so no preview. */
  self: boolean;
}

/** Pixels the pointer must travel before a press becomes a drag (clicks stay clicks). */
export const DRAG_THRESHOLD = 5;

let state: DragState | null = null;
const subs = new Set<() => void>();
let handler: ((agentId: string, target: AgentDropTarget) => void) | null = null;

function emit(next: DragState | null) {
  state = next;
  subs.forEach((f) => f());
}
const subscribe = (f: () => void) => {
  subs.add(f);
  return () => {
    subs.delete(f);
  };
};

/** The app's drop handler (one per window). Returns an unregister function. */
export function setAgentDropHandler(fn: (agentId: string, target: AgentDropTarget) => void): () => void {
  handler = fn;
  return () => {
    if (handler === fn) handler = null;
  };
}

/** Resolve the drop target under a viewport point. */
export function hitTest(x: number, y: number, agentId: string): { target: AgentDropTarget | null; self: boolean } {
  const el = document.elementFromPoint(x, y);
  const t = el instanceof Element ? el.closest<HTMLElement>("[data-drop]") : null;
  if (!t) return { target: null, self: false };
  const d = t.dataset;
  if (d.drop === "pane" && d.spaceId && d.paneId) {
    const r = t.getBoundingClientRect();
    const zone = d.agentId ? dropZone((x - r.left) / r.width, (y - r.top) / r.height) : "center";
    return { target: { kind: "pane", spaceId: d.spaceId, paneId: d.paneId, zone }, self: d.agentId === agentId };
  }
  if (d.drop === "space" && d.spaceId) return { target: { kind: "space", spaceId: d.spaceId }, self: false };
  if (d.drop === "new-space") return { target: { kind: "new-space" }, self: false };
  return { target: null, self: false };
}

const sameTarget = (a: AgentDropTarget | null, b: AgentDropTarget | null) => JSON.stringify(a) === JSON.stringify(b);

function ghostFor(label: string): HTMLElement {
  const g = document.createElement("div");
  g.className = "drag-ghost";
  g.textContent = label;
  g.setAttribute("aria-hidden", "true");
  document.body.appendChild(g);
  return g;
}

/** Swallow the click that follows a drag's pointerup (so a row isn't also "selected"). */
function swallowNextClick() {
  const stop = (e: Event) => {
    e.stopPropagation();
    e.preventDefault();
  };
  window.addEventListener("click", stop, { capture: true, once: true });
  setTimeout(() => window.removeEventListener("click", stop, { capture: true }), 0);
}

function begin(e: RPointerEvent<HTMLElement>, agentId: string, label: string) {
  if (e.button !== 0 || state) return;
  // Buttons inside a source (pane header actions) keep their own clicks.
  const inner = (e.target as Element).closest("button, input, select, textarea, a");
  if (inner && inner !== e.currentTarget) return;
  const startX = e.clientX;
  const startY = e.clientY;
  let ghost: HTMLElement | null = null;

  const update = (x: number, y: number) => {
    if (ghost) ghost.style.transform = `translate(${x + 12}px, ${y + 10}px)`;
    const { target, self } = hitTest(x, y, agentId);
    if (!state || state.self !== self || !sameTarget(state.target, target)) emit({ agentId, target, self });
  };
  const move = (ev: PointerEvent) => {
    if (!ghost) {
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < DRAG_THRESHOLD) return;
      ghost = ghostFor(label);
      document.body.classList.add("dragging-agent");
      window.getSelection()?.removeAllRanges();
    }
    ev.preventDefault();
    update(ev.clientX, ev.clientY);
  };
  const finish = (drop: boolean, ev?: PointerEvent) => {
    window.removeEventListener("pointermove", move, true);
    window.removeEventListener("pointerup", up, true);
    window.removeEventListener("pointercancel", cancel, true);
    window.removeEventListener("keydown", key, true);
    window.removeEventListener("blur", cancel);
    if (!ghost) return; // just a click
    if (ev) update(ev.clientX, ev.clientY);
    const done = state;
    ghost.remove();
    document.body.classList.remove("dragging-agent");
    emit(null);
    swallowNextClick();
    if (drop && done?.target && !done.self) handler?.(agentId, done.target);
  };
  const up = (ev: PointerEvent) => finish(true, ev);
  const cancel = () => finish(false);
  const key = (ev: KeyboardEvent) => {
    if (ev.key !== "Escape" || !ghost) return;
    ev.preventDefault();
    ev.stopImmediatePropagation();
    finish(false);
  };
  window.addEventListener("pointermove", move, true);
  window.addEventListener("pointerup", up, true);
  window.addEventListener("pointercancel", cancel, true);
  window.addEventListener("keydown", key, true);
  window.addEventListener("blur", cancel);
}

/** Props that make an element a drag source for `agentId` (`label` shows in the ghost). */
export function agentDragSource(agentId: string, label: string) {
  return {
    onPointerDown: (e: RPointerEvent<HTMLElement>) => begin(e, agentId, label),
    "data-drag-agent": agentId,
  };
}

/** The agent being dragged right now, if any. */
export function useDraggedAgent(): string | null {
  return useSyncExternalStore(subscribe, () => state?.agentId ?? null);
}

/** The zone previewed over pane `paneId` (null when the pointer isn't there, or it's the agent's own pane). */
export function usePaneDropZone(paneId: string): Side | "center" | null {
  return useSyncExternalStore(subscribe, () =>
    state && !state.self && state.target?.kind === "pane" && state.target.paneId === paneId ? state.target.zone : null,
  );
}

/** Whether a space tab (or "new-space" for the + tab) is the current drop target. */
export function useSpaceDropOver(spaceId: string | "new-space"): boolean {
  return useSyncExternalStore(subscribe, () => {
    const t = state?.target;
    return !!t && (spaceId === "new-space" ? t.kind === "new-space" : t.kind === "space" && t.spaceId === spaceId);
  });
}
