import type { AgentView, MachineView, Status } from "../types";
import { sortAgents, STATUS_WORD } from "./status";

export interface ProjectGroup {
  /** Unique per machine + project: collapse state, React keys. The project
   * path itself on machines where agents can be started (so it matches the
   * project list and project spaces). */
  key: string;
  project: string;
  display: string;
  /** Where its agents run; null for a listed project without agents. */
  machine: MachineView | null;
  /** New agents and terminals can be started in it. */
  canCreate: boolean;
  agents: AgentView[]; // blocked → done → working → idle → others
  blocked: number;
}

const machineKey = (m: MachineView | null | undefined) => (m ? `${m.provider}:${m.id}` : "");

/**
 * Group agents by machine and project. Groups on machines where agents can
 * be started come first, then the other machines (agw VMs, …) by name;
 * within a machine, alphabetical by display.
 */
export function groupByProject(agents: AgentView[]): ProjectGroup[] {
  const map = new Map<string, AgentView[]>();
  for (const a of agents) {
    const canCreate = a.machine?.canCreate ?? true;
    const key = canCreate ? a.project : `${machineKey(a.machine)}|${a.project}`;
    const list = map.get(key);
    if (list) list.push(a);
    else map.set(key, [a]);
  }
  return sortGroups(
    [...map.entries()].map(([key, list]) => ({
      key,
      project: list[0].project,
      display: list[0].projectDisplay || list[0].project,
      machine: list[0].machine ?? null,
      canCreate: list[0].machine?.canCreate ?? true,
      agents: sortAgents(list),
      blocked: list.filter((a) => a.status === "blocked").length,
    })),
  );
}

/** The order of `groupByProject` (creatable machines first, then by machine, then display). */
export function sortGroups(groups: ProjectGroup[]): ProjectGroup[] {
  const label = (g: ProjectGroup) => (g.canCreate ? "" : (g.machine?.label ?? ""));
  return [...groups].sort(
    (a, b) => Number(b.canCreate) - Number(a.canCreate) || label(a).localeCompare(label(b)) || a.display.localeCompare(b.display),
  );
}

/**
 * The machine heading to show above `groups[i]`: when agents run on more
 * than one machine, each machine's first group names it.
 */
export function machineHeading(groups: ProjectGroup[], i: number): string | null {
  const keys = new Set(groups.filter((g) => g.machine).map((g) => machineKey(g.machine)));
  if (keys.size < 2) return null;
  const k = (g: ProjectGroup) => (g.machine ? machineKey(g.machine) : null);
  const mine = k(groups[i]);
  if (!mine) return null;
  // The first group of this machine (listed projects without agents don't count).
  const first = groups.findIndex((g) => k(g) === mine);
  return first === i ? (groups[i].machine?.label ?? null) : null;
}

/** "1 needs you · 2 idle" — for collapsed groups. */
export function summarize(agents: AgentView[]): string {
  const order: Status[] = ["blocked", "done", "working", "idle", "unknown", "exited", "stopped"];
  return order
    .map((s) => [s, agents.filter((a) => a.status === s).length] as const)
    .filter(([, n]) => n > 0)
    .map(([s, n]) => `${n} ${STATUS_WORD[s]}`)
    .join(" · ");
}

/** One account in the sidebar grouped by account: its project groups. */
export interface AccountSection {
  /** The account of its agents (`default`: the user's own logins). */
  account: string;
  /** Its collapse key (`acct:<account>`). */
  key: string;
  groups: ProjectGroup[];
  agents: number;
  blocked: number;
}

/**
 * The sidebar grouped by account, then project: Default first, then the
 * accounts in `order`, then any other. A terminal (no login of its own) and a
 * project without agents go with the project's account (`projectAccount`),
 * else Default. Group keys are prefixed with the section's, so each folds on
 * its own.
 */
export function byAccount(
  agents: AgentView[],
  listed: ProjectGroup[],
  order: string[],
  projectAccount: (project: string) => string | undefined = () => undefined,
): AccountSection[] {
  const keyOf = (a: AgentView) => a.account || (a.terminal ? projectAccount(a.project) : undefined) || "default";
  const keys: string[] = [];
  for (const a of agents) if (!keys.includes(keyOf(a))) keys.push(keyOf(a));
  const empties = listed.filter((g) => g.agents.length === 0).map((g) => ({ account: projectAccount(g.project) || "default", g }));
  for (const e of empties) if (!keys.includes(e.account)) keys.push(e.account);
  const rank = (k: string) => (k === "default" ? [0, 0, ""] : order.includes(k) ? [1, order.indexOf(k), ""] : [2, 0, k]) as [number, number, string];
  keys.sort((a, b) => {
    const [x, y] = [rank(a), rank(b)];
    return x[0] - y[0] || x[1] - y[1] || x[2].localeCompare(y[2]);
  });
  return keys.map((account) => {
    const mine = agents.filter((a) => keyOf(a) === account);
    let groups = groupByProject(mine);
    groups = [...groups, ...empties.filter((e) => e.account === account).map((e) => e.g)];
    const key = `acct:${account}`;
    groups = groups.map((g) => ({ ...g, key: `${key}|${g.key}` }));
    return { account, key, groups, agents: mine.length, blocked: groups.reduce((n, g) => n + g.blocked, 0) };
  });
}
