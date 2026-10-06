import type { AgentView } from "../types";
import { STATUS_WORD } from "../lib/status";
import { agentDragSource } from "../lib/dnd";

function initials(name: string): string {
  const parts = name.split(/[-_]/).filter(Boolean);
  return (parts.length > 1 ? parts[0][0] + parts[1][0] : name.slice(0, 2)).toUpperCase();
}

/** Icon-rail entry: initials with a status dot. */
export function RailItem({ agent: a, focused, onClick }: { agent: AgentView; focused: boolean; onClick(): void }) {
  return (
    <button
      className="rail-item"
      data-status={a.status}
      aria-current={focused ? "true" : undefined}
      onClick={onClick}
      {...agentDragSource(a.id, a.name)}
      title={`${a.name} · ${STATUS_WORD[a.status]}${a.statusDetail ? `: ${a.statusDetail}` : ""}`}
    >
      <span className="rail-initials">{initials(a.name)}</span>
      <span className="rail-dot" data-status={a.status} />
    </button>
  );
}
