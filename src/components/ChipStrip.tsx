import type { AgentView } from "../types";
import { locate, type UiState } from "../state/workspace";
import { useActions } from "../lib/actions";
import { agentDragSource } from "../lib/dnd";
import { StatusGlyph } from "./StatusGlyph";

/** Agents of this space that aren't in a visible pane. Click to swap in. */
export function ChipStrip({ agents, ui, me, spaceId }: { agents: AgentView[]; ui: UiState; me: string; spaceId: string }) {
  const { showAgent } = useActions();
  return (
    <div className="chip-strip" aria-label="More agents">
      {agents.map((a) => {
        const loc = locate(ui, a.id);
        const elsewhere = loc && loc.space.id !== spaceId ? (loc.window === me ? loc.space.name : "other window") : null;
        return (
          <button
            key={a.id}
            className="agent-chip"
            data-status={a.status}
            onClick={() => showAgent(a.id)}
            {...agentDragSource(a.id, a.name)}
            title={elsewhere ? `${a.name} is shown in ${elsewhere}` : `Show ${a.name}`}
          >
            <StatusGlyph status={a.status} size="sm" />
            <span>{a.name}</span>
            {elsewhere && <span className="chip-where">↗ {elsewhere}</span>}
          </button>
        );
      })}
    </div>
  );
}
