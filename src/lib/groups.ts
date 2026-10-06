import type { AgentView, Status } from "../types";
import { sortAgents, STATUS_WORD } from "./status";

export interface ProjectGroup {
  project: string;
  display: string;
  agents: AgentView[]; // blocked → done → working → idle → others
  blocked: number;
}

/** Group agents by project; groups in stable alphabetical order. */
export function groupByProject(agents: AgentView[]): ProjectGroup[] {
  const map = new Map<string, AgentView[]>();
  for (const a of agents) {
    const list = map.get(a.project);
    if (list) list.push(a);
    else map.set(a.project, [a]);
  }
  return [...map.entries()]
    .map(([project, list]) => ({
      project,
      display: list[0].projectDisplay || project,
      agents: sortAgents(list),
      blocked: list.filter((a) => a.status === "blocked").length,
    }))
    .sort((a, b) => a.display.localeCompare(b.display));
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
