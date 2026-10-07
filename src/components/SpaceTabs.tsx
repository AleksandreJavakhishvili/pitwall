import { useState } from "react";
import type { AgentView } from "../types";
import { spaceMembers, type Space } from "../state/workspace";
import { agentsIn } from "../layout/tree";
import { useSpaceDropOver } from "../lib/dnd";
import { Icon } from "./Icon";
import { keys } from "../lib/host";

interface Props {
  spaces: Space[];
  activeSpaceId: string | null;
  agents: AgentView[];
  onSelect(id: string): void;
  onNew(): void;
  onClose(id: string): void;
  onRename(id: string, name: string): void;
  onMove(id: string): void;
}

export function SpaceTabs(p: Props) {
  const [editing, setEditing] = useState<string | null>(null);

  return (
    <div className="tabs" role="tablist" aria-label="Spaces">
      {p.spaces.map((s) => {
        // A space tab shows ▲ when an agent it shows (or holds) is blocked.
        const shown = new Set(agentsIn(s.layout));
        const members = s.kind === "all" ? p.agents.filter((a) => shown.has(a.id)) : spaceMembers(s, p.agents);
        const blocked = members.some((a) => a.status === "blocked");
        // …and ⚑ when one finished and hasn't been looked at yet (blocked wins).
        const done = !blocked && members.some((a) => a.status === "done");
        const active = s.id === p.activeSpaceId;
        return (
          <div
            key={s.id}
            className="tab"
            role="tab"
            aria-selected={active}
            data-drop="space"
            data-space-id={s.id}
            onClick={() => p.onSelect(s.id)}
            onDoubleClick={() => s.kind !== "all" && setEditing(s.id)}
            title={s.kind === "project" ? s.project : s.kind === "all" ? "Every agent" : "Custom space — drag agents here"}
          >
            <DropHint spaceId={s.id} />
            {blocked && <span className="tab-blocked">▲</span>}
            {done && (
              <span className="tab-done" title="An agent here finished">
                ⚑
              </span>
            )}
            {editing === s.id ? (
              <input
                className="tab-input"
                defaultValue={s.name}
                autoFocus
                onClick={(e) => e.stopPropagation()}
                onBlur={(e) => {
                  p.onRename(s.id, e.target.value);
                  setEditing(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                  if (e.key === "Escape") setEditing(null);
                }}
              />
            ) : (
              <span className="tab-name">{s.name}</span>
            )}
            {s.kind !== "all" && (
              <span className="tab-actions">
                <button
                  className="tab-btn"
                  title={keys("Move to new window (⌘⇧N)")}
                  onClick={(e) => {
                    e.stopPropagation();
                    p.onMove(s.id);
                  }}
                >
                  <Icon name="window" size={12} />
                </button>
                <button
                  className="tab-btn"
                  title="Close space"
                  onClick={(e) => {
                    e.stopPropagation();
                    p.onClose(s.id);
                  }}
                >
                  <Icon name="x" size={12} />
                </button>
              </span>
            )}
          </div>
        );
      })}
      <button className="tab tab-new" data-drop="new-space" onClick={() => p.onNew()} title="New space (or drop an agent here)">
        <DropHint spaceId="new-space" />
        <Icon name="plus" size={14} />
      </button>
    </div>
  );
}

/** Highlights its tab while an agent is dragged over it (a CSS :has() hook on the tab). */
function DropHint({ spaceId }: { spaceId: string }) {
  return useSpaceDropOver(spaceId) ? <span className="tab-drop-hint" aria-hidden /> : null;
}
