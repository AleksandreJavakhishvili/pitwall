// Projects as a first-class list: helpers shared by the sidebar and the
// welcome screen. Pure functions (unit-tested).
import type { AgentView, Project, RunningElsewhere, ScannedConversation, ScannedProject } from "../../types";
import { sortGroups, type ProjectGroup } from "../../lib/groups";
import { NAME_RE } from "../../lib/status";

/**
 * Sidebar groups = agent groups + listed projects that have no agents yet.
 * Projects whose agents run in a sub-folder or worktree are matched by the
 * agent's `project` (repo root). Order stays alphabetical by display.
 */
export function withProjects(groups: ProjectGroup[], projects: Project[]): ProjectGroup[] {
  const have = new Set(groups.filter((g) => g.canCreate).map((g) => g.project));
  const empty: ProjectGroup[] = projects
    .filter((p) => !have.has(p.path))
    .map((p) => ({ key: p.path, project: p.path, display: p.display, machine: null, canCreate: true, agents: [], blocked: 0 }));
  if (!empty.length) return groups;
  return sortGroups([...groups, ...empty]);
}

/** Default selection on the welcome screen: recent agent projects (≤ 30 days), max 12. */
export function defaultSelection(projects: ScannedProject[], now = Date.now()): Set<string> {
  const month = 30 * 24 * 3600_000;
  return new Set(
    projects
      .filter((p) => !p.added)
      .filter((p) => p.lastUsed !== null && now - p.lastUsed < month)
      .filter((p) => p.agentHistory)
      .slice(0, 12)
      .map((p) => p.path),
  );
}

/** A valid, unused agent name for continuing a conversation in `projectPath`. */
export function agentNameFor(projectPath: string, taken: Iterable<string>): string {
  const used = new Set(taken);
  let base = (projectPath.split("/").filter(Boolean).pop() ?? "agent")
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^[^a-z]+/, "")
    .replace(/-+$/, "")
    .slice(0, 26);
  if (!base) base = "agent";
  if (!NAME_RE.test(base)) base = "agent";
  if (!used.has(base)) return base;
  for (let i = 2; ; i++) if (!used.has(`${base}-${i}`)) return `${base}-${i}`;
}

/** Initial "Show under project…" choices: what the user picked last time. */
export function rememberedShowUnder(
  conversations: Pick<ScannedConversation, "kind" | "sessionId" | "outsideProject" | "displayProject">[],
  running: Pick<RunningElsewhere, "kind" | "sessionId" | "outsideProject" | "displayProject">[],
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const c of conversations) if (c.outsideProject && c.displayProject) out[convKey(c)] = c.displayProject;
  for (const r of running)
    if (r.outsideProject && r.sessionId && r.displayProject) out[convKey({ kind: r.kind, sessionId: r.sessionId })] = r.displayProject;
  return out;
}

/** Conversations used this recently are ticked by default. */
export const RECENT_CONVERSATION_MS = 3 * 24 * 3600_000;

export const convKey = (c: { kind: string; sessionId: string }) => `${c.kind}:${c.sessionId}`;

/** Does a Pitwall agent already work in `path` (its folder or its repo)? */
export function hasAgentIn(path: string, agents: Pick<AgentView, "cwd" | "project">[]): boolean {
  return agents.some((a) => a.cwd === path || a.project === path);
}

/**
 * Default ticked conversations: for each ticked project without a Pitwall
 * agent, its most recent conversation if used in the last 3 days. Sessions
 * already in Pitwall, started outside a project, running elsewhere, or of
 * kinds that aren't installed are never ticked.
 */
export function defaultConversations(
  conversations: ScannedConversation[],
  tickedProjects: Set<string>,
  opts: { agents: Pick<AgentView, "cwd" | "project">[]; installed: Set<string>; now?: number },
): Set<string> {
  const now = opts.now ?? Date.now();
  const newest = new Map<string, ScannedConversation>();
  for (const c of conversations) {
    // Started in ~ (no project) or open in another terminal: never ticked for you.
    if (c.inPitwall || c.outsideProject || c.runningElsewhere || !opts.installed.has(c.kind)) continue;
    const cur = newest.get(c.projectPath);
    if (!cur || c.lastUsed > cur.lastUsed) newest.set(c.projectPath, c);
  }
  const out = new Set<string>();
  for (const [path, c] of newest) {
    if (!tickedProjects.has(path) || hasAgentIn(path, opts.agents)) continue;
    if (now - c.lastUsed < RECENT_CONVERSATION_MS) out.add(convKey(c));
  }
  return out;
}

export interface PlannedAgent {
  name: string;
  kind: string;
  /** Where it runs (a conversation's own cwd). */
  projectPath: string;
  /** Resume this session (continue_conversation); none → a fresh agent (create_agent). */
  sessionId?: string;
  /** Sidebar project when it differs from `projectPath` ("Show under project…"). */
  displayProject?: string;
}

/**
 * What "Start Pitwall" creates, in order: ticked conversations, ticked
 * running-elsewhere sessions (same session only once), then a fresh agent for
 * each project whose "Start a new agent" toggle is on and has nothing else.
 * Names come from the project folder and are unique against `taken`.
 */
export function planAgents(input: {
  conversations: ScannedConversation[];
  convSel: Set<string>;
  running: RunningElsewhere[];
  runSel: Set<number>;
  /** project path → kind, for projects with "Start a new agent" on. */
  fresh: Record<string, string>;
  taken: Iterable<string>;
  /** convKey → project chosen with "Show under project…" (outside-project rows). */
  showUnder?: Record<string, string>;
}): PlannedAgent[] {
  const used = new Set(input.taken);
  const sessions = new Set<string>();
  const busy = new Set<string>();
  const out: PlannedAgent[] = [];
  const shown = (kind: string, sessionId: string, outside: boolean) =>
    outside ? input.showUnder?.[convKey({ kind, sessionId })] || undefined : undefined;
  const push = (kind: string, projectPath: string, sessionId?: string, displayProject?: string) => {
    if (sessionId) {
      if (sessions.has(sessionId)) return;
      sessions.add(sessionId);
    }
    const name = agentNameFor(displayProject ?? projectPath, used);
    used.add(name);
    busy.add(displayProject ?? projectPath);
    out.push({ name, kind, projectPath, ...(sessionId ? { sessionId } : {}), ...(displayProject ? { displayProject } : {}) });
  };
  for (const c of input.conversations) {
    if (input.convSel.has(convKey(c)) && !c.inPitwall)
      push(c.kind, c.projectPath, c.sessionId, shown(c.kind, c.sessionId, c.outsideProject));
  }
  for (const r of input.running) {
    if (input.runSel.has(r.pid) && r.sessionId && r.cwd && !r.inPitwall)
      push(r.kind, r.cwd, r.sessionId, shown(r.kind, r.sessionId, r.outsideProject));
  }
  for (const [path, kind] of Object.entries(input.fresh)) {
    if (!busy.has(path)) push(kind, path);
  }
  return out;
}

/**
 * Start planned agents (or add sessions) one by one. A failure is reported
 * (`onError`) and the rest still start. Returns what started, in plan order.
 */
export async function startAll<P, T>(
  plan: P[],
  start: (p: P) => Promise<T>,
  hooks: { onProgress?(done: number, total: number): void; onError?(p: P, e: unknown): void } = {},
): Promise<T[]> {
  const out: T[] = [];
  for (let i = 0; i < plan.length; i++) {
    hooks.onProgress?.(i, plan.length);
    try {
      out.push(await start(plan[i]));
    } catch (e) {
      hooks.onError?.(plan[i], e);
    }
  }
  return out;
}

/** Identity of a session found on another machine (what "Add to Pitwall" takes). */
export const sessionKey = (s: { provider: string; machine: string; native: string }) => `${s.provider}:${s.machine}/${s.native}`;
