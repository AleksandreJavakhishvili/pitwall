import type { ProjectGroup } from "../lib/groups";
import { summarize } from "../lib/groups";
import type { UiState } from "../state/workspace";
import { locate } from "../state/workspace";
import { useActions } from "../lib/actions";
import { AgentRow } from "./AgentRow";
import { RailItem } from "./RailItem";
import { Icon } from "./Icon";
import { Kbd } from "./Kbd";
import { api } from "../api";

interface Props {
  mode: "full" | "rail" | "overlay";
  groups: ProjectGroup[];
  ui: UiState;
  me: string;
  focusedAgentId: string | null;
}

export function Sidebar({ mode, groups, ui, me, focusedAgentId }: Props) {
  const { showAgent, openNewAgent, openProjectSpace, toggleCollapsed, run } = useActions();
  let index = 0;

  if (mode === "rail") {
    return (
      <nav className="sidebar sidebar-rail" aria-label="Agents">
        {groups.map((g) => (
          <div key={g.project} className="rail-group" title={g.display}>
            {g.agents.map((a) => (
              <RailItem key={a.id} agent={a} focused={a.id === focusedAgentId} onClick={() => showAgent(a.id)} />
            ))}
          </div>
        ))}
        <button className="rail-new" onClick={openNewAgent} title="New agent (⌘N)">
          <Icon name="plus" />
        </button>
      </nav>
    );
  }

  return (
    <nav className={`sidebar ${mode === "overlay" ? "sidebar-overlay" : ""}`} aria-label="Agents">
      <div className="sidebar-scroll">
        {groups.map((g) => {
          const collapsed = ui.collapsed.includes(g.project);
          const start = index;
          index += g.agents.length;
          return (
            <section key={g.project} className="project-group" data-blocked={g.blocked > 0}>
              <header className="project-head">
                <button
                  className="project-toggle"
                  onClick={() => toggleCollapsed(g.project, "sidebar")}
                  aria-expanded={!collapsed}
                  title={g.project}
                >
                  <span className="chev" data-open={!collapsed}>
                    <Icon name="chevron" size={12} />
                  </span>
                  <span className="project-name">{g.display}</span>
                  {g.blocked > 0 && <span className="project-blocked">▲ {g.blocked}</span>}
                  <span className="label-count">{g.agents.length}</span>
                </button>
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
                  <div className="project-empty">
                    <span>No agents yet</span>
                    <button className="small-btn" onClick={() => openNewAgent(g.project)}>
                      <Icon name="plus" size={12} /> Agent
                    </button>
                  </div>
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
                      />
                    );
                  })}
                </ul>
              )}
            </section>
          );
        })}
      </div>
      <button className="new-agent-btn" onClick={openNewAgent}>
        <Icon name="plus" />
        <span>New agent</span>
        <Kbd>⌘N</Kbd>
      </button>
    </nav>
  );
}
