import { useMemo, useState, type MouseEvent } from "react";
import type { ProjectGroup } from "../lib/groups";
import type { RunningElsewhere } from "../types";
import { machineHeading, summarize } from "../lib/groups";
import type { UiState } from "../state/workspace";
import { locate } from "../state/workspace";
import { useActions } from "../lib/actions";
import { AgentRow } from "./AgentRow";
import { RailItem } from "./RailItem";
import { Icon } from "./Icon";
import { Kbd } from "./Kbd";
import { api } from "../api";
import { Menu, type MenuItem } from "./Menu";
import { ElsewhereGroup } from "./ElsewhereGroup";
import { forceRefreshWorktrees, useWorktreeList } from "../lib/useWorktrees";
import { otherWorktrees, worktreesByAgent } from "../lib/worktrees";
import { WorktreeRows } from "./WorktreeRows";
import { keys } from "../lib/host";

/** Collapse key of the "Elsewhere" group (shares `ui.collapsed` with projects). */
const ELSEWHERE = "\u0000elsewhere";

interface Props {
  mode: "full" | "rail" | "overlay";
  groups: ProjectGroup[];
  ui: UiState;
  me: string;
  focusedAgentId: string | null;
  /** Agents running in other terminal apps (already filtered). */
  elsewhere?: RunningElsewhere[];
}

export function Sidebar({ mode, groups, ui, me, focusedAgentId, elsewhere = [] }: Props) {
  const { showAgent, openNewAgent, openProjectSpace, toggleCollapsed, run, openTerminal, openRemove } = useActions();
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; label: string; items: MenuItem[] } | null>(null);
  // Worktrees (docs/spec/worktrees-view.md): expanded lists, per agent id / project key.
  const projectsWt = useWorktreeList();
  const byAgent = useMemo(() => worktreesByAgent(projectsWt), [projectsWt]);
  const [openWt, setOpenWt] = useState<Set<string>>(() => new Set());
  const toggleWt = (key: string) => {
    // Expanding lists them again now (the backend's pace is bypassed).
    if (!openWt.has(key)) void forceRefreshWorktrees().catch(() => {});
    setOpenWt((s) => {
      const n = new Set(s);
      if (!n.delete(key)) n.add(key);
      return n;
    });
  };
  let index = 0;

  const terminalItem = (path: string): MenuItem => ({
    id: "terminal",
    icon: <Icon name="terminal" size={14} />,
    label: "New terminal here",
    run: () => openTerminal(path),
  });
  const projectMenu = (project: string, at: { x: number; y: number }) =>
    setMenu({
      at,
      label: "Project",
      items: [
        terminalItem(project),
        { id: "agent", icon: <Icon name="plus" size={14} />, label: "New agent…", run: () => openNewAgent(project) },
      ],
    });
  const below = (e: MouseEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    return { x: r.left, y: r.bottom + 4 };
  };

  if (mode === "rail") {
    return (
      <nav className="sidebar sidebar-rail" aria-label="Agents">
        {groups.map((g) => (
          <div key={g.key} className="rail-group" title={g.machine && !g.canCreate ? `${g.display} · ${g.machine.label}` : g.display}>
            {g.agents.map((a) => (
              <RailItem key={a.id} agent={a} focused={a.id === focusedAgentId} onClick={() => showAgent(a.id)} />
            ))}
          </div>
        ))}
        <button className="rail-new" onClick={openNewAgent} title={keys("New agent (⌘N)")}>
          <Icon name="plus" />
        </button>
      </nav>
    );
  }

  return (
    <nav className={`sidebar ${mode === "overlay" ? "sidebar-overlay" : ""}`} aria-label="Agents">
      <div className="sidebar-scroll">
        {groups.map((g, gi) => {
          const collapsed = ui.collapsed.includes(g.key);
          const start = index;
          index += g.agents.length;
          const heading = machineHeading(groups, gi);
          const others = otherWorktrees(
            projectsWt,
            g.agents.map((a) => a.id),
          );
          const othersKey = `\u0000wt:${g.key}`;
          return (
            <section key={g.key} className="project-group" data-blocked={g.blocked > 0}>
              {heading && <div className="machine-head">{heading}</div>}
              <header
                className="project-head"
                onContextMenu={(e) => {
                  e.preventDefault();
                  if (g.canCreate) projectMenu(g.project, { x: e.clientX, y: e.clientY });
                }}
              >
                <button
                  className="project-toggle"
                  onClick={() => toggleCollapsed(g.key, "sidebar")}
                  aria-expanded={!collapsed}
                  title={g.machine && !g.canCreate ? `${g.project} on ${g.machine.label}` : g.project}
                >
                  <span className="chev" data-open={!collapsed}>
                    <Icon name="chevron" size={12} />
                  </span>
                  <span className="project-name">{g.display}</span>
                  {g.blocked > 0 && <span className="project-blocked">▲ {g.blocked}</span>}
                  <span className="label-count">{g.agents.length}</span>
                </button>
                {g.canCreate && (
                  <button
                    className="icon-btn icon-btn-sm project-add-btn"
                    onClick={(e) => projectMenu(g.project, below(e))}
                    title="New terminal or agent here"
                    aria-haspopup="menu"
                  >
                    <Icon name="plus" size={14} />
                  </button>
                )}
                {g.agents.length > 0 ? (
                  <button
                    className="icon-btn icon-btn-sm project-space-btn"
                    onClick={() => openProjectSpace(g.project, g.display)}
                    title="Open as space"
                  >
                    <Icon name="grid" size={14} />
                  </button>
                ) : (
                  <button
                    className="icon-btn icon-btn-sm project-remove"
                    onClick={() => run(api.removeProject(g.project), "remove the project")}
                    title="Remove from Pitwall (files are not touched)"
                  >
                    <Icon name="x" size={12} />
                  </button>
                )}
              </header>
              {g.agents.length === 0 ? (
                !collapsed && (
                  <button className="project-empty" onClick={() => openNewAgent(g.project)} title="New agent in this project">
                    <Icon name="plus" size={12} />
                    <span>New agent</span>
                    <span className="project-empty-hint">No agents yet</span>
                  </button>
                )
              ) : collapsed ? (
                <div className="project-summary">{summarize(g.agents)}</div>
              ) : (
                <ul className="agent-list">
                  {g.agents.map((a, i) => {
                    const loc = locate(ui, a.id);
                    const where = loc ? (loc.window !== me ? "other window" : loc.space.name) : null;
                    return (
                      <AgentRow
                        key={a.id}
                        agent={a}
                        index={start + i}
                        selected={a.id === focusedAgentId}
                        where={where}
                        onSelect={() => showAgent(a.id)}
                        worktrees={byAgent.get(a.id)?.length ?? 0}
                        worktreesOpen={openWt.has(a.id)}
                        onToggleWorktrees={() => toggleWt(a.id)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setMenu({
                            at: { x: e.clientX, y: e.clientY },
                            label: a.name,
                            items: [
                              // A terminal only where Pitwall can start one (not on read-only machines).
                              ...(g.canCreate ? [{ ...terminalItem(a.cwd), label: "Open terminal here" }] : []),
                              // Same confirm dialog as the tile's trash button (worktree choice included;
                              // for remote agents it only detaches Pitwall).
                              { id: "remove", icon: <Icon name="trash" size={14} />, label: "Remove agent…", run: () => openRemove(a.id) },
                            ],
                          });
                        }}
                      >
                        {openWt.has(a.id) && <WorktreeRows refs={byAgent.get(a.id) ?? []} label={`Worktrees of ${a.name}`} />}
                      </AgentRow>
                    );
                  })}
                  {others.length > 0 && (
                    <li className="wt-others">
                      <button className="wt-chip" aria-expanded={openWt.has(othersKey)} onClick={() => toggleWt(othersKey)} title="Worktrees of this project no agent works in">
                        <span className="chev" data-open={openWt.has(othersKey)}>
                          <Icon name="chevron" size={10} />
                        </span>
                        <Icon name="branch" size={11} />
                        {others.length === 1 ? "1 other worktree" : `${others.length} other worktrees`}
                      </button>
                      {openWt.has(othersKey) && <WorktreeRows refs={others} label={`Other worktrees of ${g.display}`} />}
                    </li>
                  )}
                </ul>
              )}
            </section>
          );
        })}
        {!ui.hideElsewhere && (
          <ElsewhereGroup rows={elsewhere} collapsed={ui.collapsed.includes(ELSEWHERE)} onToggle={() => toggleCollapsed(ELSEWHERE, "sidebar")} />
        )}
      </div>
      <div className="sidebar-foot">
        <button className="new-agent-btn" onClick={openNewAgent}>
          <Icon name="plus" />
          <span>New agent</span>
          <Kbd>⌘N</Kbd>
        </button>
        <button className="icon-btn new-terminal-btn" onClick={() => openTerminal()} title={keys("New terminal here (⌘T) · ⌘⇧T to choose a folder")}>
          <Icon name="terminal" />
        </button>
      </div>
      {menu && <Menu at={menu.at} label={menu.label} items={menu.items} onClose={() => setMenu(null)} />}
    </nav>
  );
}
