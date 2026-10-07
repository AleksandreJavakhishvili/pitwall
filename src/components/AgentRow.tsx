import { memo, type MouseEvent } from "react";
import type { AgentView } from "../types";
import { STATUS_WORD } from "../lib/status";
import { agentDragSource } from "../lib/dnd";
import { StatusGlyph } from "./StatusGlyph";
import { DiffStat } from "./DiffStat";

interface Props {
  agent: AgentView;
  index: number;
  selected: boolean;
  /** Space name where it's shown, if anywhere. */
  where: string | null;
  onSelect(): void;
  onContextMenu?(e: MouseEvent): void;
}

/**
 * Re-renders only when what it shows changes. The callbacks are left out on
 * purpose: they only call the window's stable actions with this agent.
 */
export const AgentRow = memo(
  AgentRowImpl,
  (a, b) => a.agent === b.agent && a.index === b.index && a.selected === b.selected && a.where === b.where,
);

function AgentRowImpl({ agent: a, index, selected, where, onSelect, onContextMenu }: Props) {
  return (
    <li>
      <button
        className="agent-row"
        data-status={a.status}
        aria-current={selected ? "true" : undefined}
        onClick={onSelect}
        onContextMenu={onContextMenu}
        {...agentDragSource(a.id, a.name)}
        title={`${a.name}${where ? ` — in ${where}` : ""}${a.statusDetail ? `\n${a.statusDetail}` : ""}`}
      >
        <StatusGlyph status={a.status} />
        <span className="agent-row-main">
          <span className="agent-row-top">
            <span className="agent-name">{a.name}</span>
            <span className="status-word" data-status={a.status}>
              {STATUS_WORD[a.status]}
            </span>
          </span>
          <span className="agent-row-sub">
            <span className="chip">{a.location}</span>
            <span className="agent-kind">
              {a.kindName}
              {a.agentInTerminal && <span className="in-terminal"> · in terminal</span>}
            </span>
            <DiffStat added={a.added} removed={a.removed} />
          </span>
          {a.status === "blocked" && a.statusDetail && <span className="agent-row-detail">{a.statusDetail}</span>}
        </span>
        {index < 9 && <span className="row-index">⌘{index + 1}</span>}
      </button>
    </li>
  );
}
