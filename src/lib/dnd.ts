// Drag & drop of agents (sidebar rows, chips) onto panes and tabs.
import type { DragEvent } from "react";

export const AGENT_MIME = "application/x-pitwall-agent";

let dragging: string | null = null;

export function startAgentDrag(e: DragEvent, agentId: string) {
  e.dataTransfer.setData(AGENT_MIME, agentId);
  e.dataTransfer.setData("text/plain", agentId);
  e.dataTransfer.effectAllowed = "move";
  dragging = agentId;
  document.body.classList.add("dragging-agent");
}

export function endAgentDrag() {
  dragging = null;
  document.body.classList.remove("dragging-agent");
}

export function draggedAgent(e: DragEvent): string | null {
  if (!e.dataTransfer.types.includes(AGENT_MIME)) return null;
  return dragging ?? (e.dataTransfer.getData(AGENT_MIME) || null);
}
