import { useMemo, useState, type MouseEvent } from "react";
import type { AccountSection, ProjectGroup } from "../lib/groups";
import type { AgentView, RunningElsewhere } from "../types";
import { byAccount, machineHeading, summarize } from "../lib/groups";
import { accountOf, accountsOfKind, useAccounts } from "../lib/useAccounts";
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
  const {
    showAgent,
    openNewAgent,
    openProjectSpace,
    openAccountSpace,
    toggleCollapsed,
    run,
    openTerminal,
    openRemove,
    sidebarGroup,
    setSidebarGroup,
    openProjectAccounts,
    switchAccount,
  } = useActions();
  const accounts = useAccounts();
  const byAcct = sidebarGroup === "account";
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
        // Which login its new agents use (agents with more than one).
        ...(accounts?.kinds.length
          ? [
              { id: "accounts", label: "Accounts…", run: () => openProjectAccounts(project) },
              { id: "group", label: "Group by account", run: () => setSidebarGroup("account") },
            ]
          : []),
      ],
    });
  /** An agent's right-click menu: a terminal there, which login it runs on, remove. */
  const agentMenu = (a: AgentView, canCreate: boolean, at: { x: number; y: number }) => {
    const list = accountsOfKind(accounts, a.kind);
    // Only where Pitwall starts the process itself (it sets the login folder).
    const local = a.machine?.canCreate ?? true;
    const current = a.account ?? null;
    const run = (id: string | null, name: string): MenuItem => ({
      id: `acct-${id ?? "default"}`,
      icon: current === id ? <Icon name="check" size={14} /> : undefined,
      label: name,
      run: () => current !== id && switchAccount(a.id, id, name),
    });
    setMenu({
      at,
      label: a.name,
      items: [
        // A terminal only where Pitwall can start one (not on read-only machines).
        ...(canCreate ? [{ ...terminalItem(a.cwd), label: "Open terminal here" }] : []),
        ...(local && list.length && !a.engineer
          ? [{ id: "acct-head", label: "Run on account", header: true, run: () => {} }, run(null, "Default"), ...list.map((x) => run(x.id, x.name))]
          : []),
        // Same confirm dialog as the tile's trash button (worktree choice included;
        // for remote agents it only detaches Pitwall).
        { id: "remove", icon: <Icon name="trash" size={14} />, label: "Remove agent…", run: () => openRemove(a.id) },
      ],
    });
  };
  /** An account heading's right-click menu. */
  const accountMenu = (account: string, name: string, at: { x: number; y: number }) =>
    setMenu({
      at,
      label: name,
      items: [
        { id: "space", icon: <Icon name="grid" size={14} />, label: "Open as space", run: () => openAccountSpace(account, name) },
        { id: "group", label: "Group by project", run: () => setSidebarGroup("project") },
      ],
    });
  const below = (e: MouseEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    return { x: r.left, y: r.bottom + 4 };
  };

  /** One project group and its agent rows (`list`: the groups it is among). */
  const renderGroup = (g: ProjectGroup, gi: number, list: ProjectGroup[]) => {
          const collapsed = ui.collapsed.includes(g.key);
          const start = index;
          index += g.agents.length;
          const heading = machineHeading(list, gi);
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
                        account={byAcct ? null : (accountOf(accounts, a.account)?.name ?? null)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          agentMenu(a, g.canCreate, { x: e.clientX, y: e.clientY });
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
        {byAcct
          ? byAccount(
              groups.flatMap((g) => g.agents),
              groups,
              accounts?.accounts.map((x) => x.id) ?? [],
            ).map((sec) => {
              const collapsed = ui.collapsed.includes(sec.key);
              return (
                <section key={sec.key} className="account-section">
                  <AccountHead
                    sec={sec}
                    collapsed={collapsed}
                    onToggle={() => toggleCollapsed(sec.key, "sidebar")}
                    onSpace={(name) => openAccountSpace(sec.account, name)}
                    onMenu={(name, at) => accountMenu(sec.account, name, at)}
                  />
                  {!collapsed && <div className="account-groups">{sec.groups.map((g, gi) => renderGroup(g, gi, sec.groups))}</div>}
                </section>
              );
            })
          : groups.map((g, gi) => renderGroup(g, gi, groups))}
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

/** An account's heading when the sidebar groups by account: its name and login, needs-you and
 * agent counts, "Open as space"; folds its projects; right-click for its menu. */
function AccountHead({
  sec,
  collapsed,
  onToggle,
  onSpace,
  onMenu,
}: {
  sec: AccountSection;
  collapsed: boolean;
  onToggle(): void;
  onSpace(name: string): void;
  onMenu(name: string, at: { x: number; y: number }): void;
}) {
  const v = useAccounts();
  const acc = accountOf(v, sec.account);
  const name = sec.account === "default" ? "Default" : (acc?.name ?? sec.account);
  const kind = v?.kinds.find((k) => k.id === acc?.kind)?.name ?? acc?.kind ?? "";
  const sub = sec.account === "default" ? "your own logins" : acc?.login ? `${kind} · ${acc.login}` : kind;
  return (
    <header
      className="account-head"
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu(name, { x: e.clientX, y: e.clientY });
      }}
    >
      <button className="account-toggle" onClick={onToggle} aria-expanded={!collapsed} title={`${name} — ${sub}`}>
        <span className="chev" data-open={!collapsed}>
          <Icon name="chevron" size={12} />
        </span>
        <Icon name="user" size={12} />
        <span className="account-name">{name}</span>
        <span className="account-sub">{sub}</span>
        <span className="spacer" />
        {sec.blocked > 0 && <span className="project-blocked">▲ {sec.blocked}</span>}
        <span className="label-count">{sec.agents}</span>
      </button>
      {sec.agents > 0 && (
        <button className="icon-btn icon-btn-sm project-space-btn" onClick={() => onSpace(name)} title="Open as space">
          <Icon name="grid" size={14} />
        </button>
      )}
    </header>
  );
}
