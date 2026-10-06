import type { AgentView } from "../types";
import { NextUp } from "./NextUp";
import { Changes } from "./Changes";
import { LastSent } from "./LastSent";
import { StatusGlyph } from "./StatusGlyph";

/** Details for the focused pane's agent; docked on wide windows, a drawer otherwise. */
export function RightPanel({ agent, drawer, onClose }: { agent: AgentView; drawer?: boolean; onClose?: () => void }) {
  return (
    <aside className={`right ${drawer ? "right-drawer" : ""}`} aria-label={`${agent.name} details`}>
      <div className="right-head">
        <StatusGlyph status={agent.status} size="sm" />
        <span className="right-name">{agent.name}</span>
        <span className="spacer" />
        {drawer && (
          <button className="icon-btn icon-btn-sm" onClick={onClose} aria-label="Close details">
            ✕
          </button>
        )}
      </div>
      <NextUp agent={agent} />
      <Changes agent={agent} />
      <LastSent agent={agent} />
    </aside>
  );
}
