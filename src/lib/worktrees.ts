// Worktrees in source control (docs/spec/worktrees-view.md): grouping the
// backend's per-project lists for the sidebar and Review. Pure, tested.
import type { AgentView } from "../types";
import type { ProjectWorktrees, WorktreeView } from "../worktreesApi";

/** One worktree with the project it belongs to (what the per-worktree calls take). */
export interface WorktreeRef {
  projectId: string;
  /** The project's main checkout (for display) and its current branch (merge target). */
  repoDisplay: string;
  target: string | null;
  wt: WorktreeView;
}

export const refKey = (r: { projectId: string; wt: { path: string } } | { projectId: string; path: string }) =>
  `${r.projectId}\u0000${"wt" in r ? r.wt.path : r.path}`;

const ref = (p: ProjectWorktrees, wt: WorktreeView): WorktreeRef => ({ projectId: p.id, repoDisplay: p.repoDisplay, target: p.branch, wt });

/** Worktrees an agent has besides its own folder (tool-managed or where its processes work), by agent id. */
export function worktreesByAgent(projects: ProjectWorktrees[]): Map<string, WorktreeRef[]> {
  const out = new Map<string, WorktreeRef[]>();
  for (const p of projects)
    for (const wt of p.worktrees) {
      if (!wt.agentId || wt.via === "own" || wt.via === "other") continue;
      const list = out.get(wt.agentId);
      if (list) list.push(ref(p, wt));
      else out.set(wt.agentId, [ref(p, wt)]);
    }
  for (const list of out.values()) list.sort((a, b) => a.wt.name.localeCompare(b.wt.name));
  return out;
}

/** A project's worktrees that belong to no agent, for the group holding any of `agentIds`. */
export function otherWorktrees(projects: ProjectWorktrees[], agentIds: string[]): WorktreeRef[] {
  const ids = new Set(agentIds);
  return projects
    .filter((p) => p.agentIds.some((id) => ids.has(id)))
    .flatMap((p) => p.worktrees.filter((w) => w.via === "other").map((w) => ref(p, w)))
    .sort((a, b) => a.wt.name.localeCompare(b.wt.name));
}

/** Find a listed worktree again (after a refresh), or null when it left the list. */
export function findWorktree(projects: ProjectWorktrees[], projectId: string, path: string): WorktreeRef | null {
  const p = projects.find((x) => x.id === projectId);
  const wt = p?.worktrees.find((w) => w.path === path);
  return p && wt ? ref(p, wt) : null;
}

/** "2 worktrees" */
export const countLabel = (n: number) => `${n} worktree${n === 1 ? "" : "s"}`;

/** What the poller compares: something changed in a project when one of its agents' numbers moved. */
export function agentsSignature(agents: AgentView[]): string {
  return agents
    .filter((a) => a.caps.worktrees)
    .map((a) => `${a.id}:${a.cwd}:${a.added}:${a.removed}:${a.filesChanged}:${a.branch ?? ""}:${a.status}`)
    .join("|");
}
