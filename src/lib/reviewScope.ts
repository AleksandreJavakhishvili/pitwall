// Which agents Review lists (docs/spec/review.md): the active space's — a
// project space's agents, a custom space's members; in "All", the focused
// agent's project (everyone when no agent is focused) — or everyone when
// widened with "All projects". Pure, tested.
import type { AgentView } from "../types";
import { spaceMembers, type Space } from "../state/workspace";

export interface ReviewScope {
  agents: AgentView[];
  /** Review is narrowed: it offers the "All projects" toggle. */
  canWiden: boolean;
  /** What it's narrowed to (the space's or the focused agent's project name). */
  label: string;
}

/**
 * `all`: the "All projects" toggle. `extra`: agents shown however the space
 * is scoped (the one Review was opened for, a focused worktree's agents).
 */
export function reviewScope(
  agents: AgentView[],
  space: Space | null,
  all: boolean,
  extra: readonly string[] = [],
  focused: AgentView | null = null,
): ReviewScope {
  const inAll = !space || space.kind === "all";
  if (inAll && !focused) return { agents, canWiden: false, label: "" };
  const label = inAll ? focused!.projectDisplay : space!.name;
  if (all) return { agents, canWiden: true, label };
  const members = inAll ? agents.filter((a) => a.project === focused!.project) : spaceMembers(space!, agents);
  const ids = new Set(members.map((a) => a.id));
  for (const id of extra) ids.add(id);
  return { agents: agents.filter((a) => ids.has(a.id)), canWiden: true, label };
}
