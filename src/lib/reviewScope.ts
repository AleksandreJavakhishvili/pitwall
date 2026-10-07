// Which agents Review lists (docs/spec/review.md): the active space's — a
// project space's agents, a custom space's members, everyone in "All" — or
// everyone when widened with "All projects". Pure, tested.
import type { AgentView } from "../types";
import { spaceMembers, type Space } from "../state/workspace";

export interface ReviewScope {
  agents: AgentView[];
  /** The space isn't "All": Review offers the "All projects" toggle. */
  canWiden: boolean;
}

/**
 * `all`: the "All projects" toggle. `extra`: agents shown however the space
 * is scoped (the one Review was opened for, a focused worktree's agents).
 */
export function reviewScope(agents: AgentView[], space: Space | null, all: boolean, extra: readonly string[] = []): ReviewScope {
  if (!space || space.kind === "all") return { agents, canWiden: false };
  if (all) return { agents, canWiden: true };
  const ids = new Set(spaceMembers(space, agents).map((a) => a.id));
  for (const id of extra) ids.add(id);
  return { agents: agents.filter((a) => ids.has(a.id)), canWiden: true };
}
