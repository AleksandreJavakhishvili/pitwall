import type { AgentView } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { STATUS_WORD } from "../lib/status";
import { agentDragSource } from "../lib/dnd";
import { StatusGlyph } from "./StatusGlyph";
import { DiffStat } from "./DiffStat";
import { Icon } from "./Icon";
import { RulesStaleButton } from "./rules/RulesStaleButton";

interface Props {
  agent: AgentView;
  paneId: string;
  maximized: boolean;
  canClose: boolean;
}

export function PaneHeader({ agent: a, paneId, maximized, canClose }: Props) {
  const { run, openRemove, toggleMaximize, closePane } = useActions();
  return (
    <header
      className="pane-head"
      data-status={a.status}
      onDoubleClick={(e) => {
        if ((e.target as HTMLElement).closest("button")) return;
        toggleMaximize(paneId);
      }}
      {...agentDragSource(a.id, a.name)}
      title="Double-click to maximise · drag to move"
    >
      <StatusGlyph status={a.status} />
      <span className="pane-name">{a.name}</span>
      <span className="status-word" data-status={a.status}>
        {STATUS_WORD[a.status]}
      </span>
      {a.status === "blocked" && a.statusDetail && <span className="pane-detail">{a.statusDetail}</span>}
      <span className="pane-meta">
        <span className="chip" title={a.machine ? `${a.machine.label} (${a.machine.provider})` : a.location}>
          {a.machine?.label ?? a.location}
        </span>
        <span className="meta-item" title={a.cwd}>
          {a.projectDisplay}
        </span>
        <span className="meta-item">{a.kindName}</span>
        {a.branch && (
          <span className="meta-item mono" title={a.worktree ? `Separate worktree: ${a.cwdDisplay}` : a.cwdDisplay}>
            <Icon name="branch" size={12} />
            {a.branch}
          </span>
        )}
        <DiffStat added={a.added} removed={a.removed} />
      </span>
      <span className="spacer" />
      <RulesStaleButton agentId={a.id} />
      <span className="pane-actions">
        {a.caps.stop && (
          <button className="icon-btn icon-btn-sm" onClick={() => run(api.stopAgent(a.id), "stop agent")} title="Stop process">
            <Icon name="stop" size={13} />
          </button>
        )}
        <button className="icon-btn icon-btn-sm" onClick={() => openRemove(a.id)} title="Remove agent">
          <Icon name="trash" size={13} />
        </button>
        <button
          className="icon-btn icon-btn-sm"
          onClick={() => toggleMaximize(paneId)}
          title={maximized ? "Restore (⌘⏎)" : "Maximise (⌘⏎)"}
        >
          <Icon name={maximized ? "restore" : "maximize"} size={13} />
        </button>
        {canClose && (
          <button className="icon-btn icon-btn-sm" onClick={() => closePane(paneId)} title="Close pane (agent keeps running)">
            <Icon name="x" size={13} />
          </button>
        )}
      </span>
    </header>
  );
}
