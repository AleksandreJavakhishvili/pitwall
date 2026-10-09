import { memo, type MouseEvent, type ReactNode } from "react";
import type { AgentView } from "../types";
import { STATUS_WORD } from "../lib/status";
import { agentDragSource } from "../lib/dnd";
import { StatusGlyph } from "./StatusGlyph";
import { DiffStat } from "./DiffStat";
import { Icon } from "./Icon";
import { countLabel } from "../lib/worktrees";
import { keys } from "../lib/host";

interface Props {
  agent: AgentView;
  index: number;
  selected: boolean;
  /** Space name where it's shown, if anywhere. */
  where: string | null;
  onSelect(): void;
  onContextMenu?(e: MouseEvent): void;
  /** Worktrees it has besides its own folder (docs/spec/worktrees-view.md): a chip that expands `children`. */
  worktrees?: number;
  worktreesOpen?: boolean;
  onToggleWorktrees?(): void;
  /** The expanded worktree list. */
  children?: ReactNode;
}

/**
 * Re-renders only when what it shows changes. The callbacks are left out on
 * purpose: they only call the window's stable actions with this agent.
 */
export const AgentRow = memo(
  AgentRowImpl,
  (a, b) =>
    a.agent === b.agent &&
    a.index === b.index &&
    a.selected === b.selected &&
    a.where === b.where &&
    a.worktrees === b.worktrees &&
    a.worktreesOpen === b.worktreesOpen &&
    a.children === b.children,
);

function AgentRowImpl({ agent: a, index, selected, where, onSelect, onContextMenu, worktrees = 0, worktreesOpen = false, onToggleWorktrees, children }: Props) {
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
            {a.engineer && <span className="chip">engineer</span>}
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
        {index < 9 && <span className="row-index">{keys(`⌘${index + 1}`, { compact: true })}</span>}
      </button>
      {worktrees > 0 && (
        <button
          className="wt-chip"
          aria-expanded={worktreesOpen}
          onClick={onToggleWorktrees}
          title={worktreesOpen ? "Hide its worktrees" : "Show its worktrees"}
        >
          <span className="chev" data-open={worktreesOpen}>
            <Icon name="chevron" size={10} />
          </span>
          <Icon name="branch" size={11} />
          {countLabel(worktrees)}
        </button>
      )}
      {worktreesOpen && children}
    </li>
  );
}
