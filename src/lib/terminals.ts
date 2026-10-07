// Plain terminals anywhere + agents started by hand (docs/spec/terminals.md).
// Pure helpers (unit-tested); App and the sidebar wire them to the API.
import type { AgentView, ContinueConversationRequest, CreateAgentRequest, RunningElsewhere } from "../types";
import { agentNameFor } from "../components/onboarding/projects";

/** The kind a plain terminal is created with. */
export const SHELL_KIND = "shell";

/** ⌘T: the focused agent's working folder, else the selected project, else home. */
export function terminalFolder(focused: Pick<AgentView, "cwd"> | null, selectedProject: string | null): string {
  return focused?.cwd || selectedProject || "~";
}

/** Default terminal name: the folder's name (+ "-2", "-3"… when taken); "home" for `~`. */
export function terminalName(path: string, taken: Iterable<string>): string {
  const p = path.trim().replace(/\/+$/, "");
  return agentNameFor(p === "~" || p === "" ? "home" : p, taken);
}

export function terminalRequest(path: string, agents: Pick<AgentView, "name">[]): CreateAgentRequest {
  const projectPath = path.trim() || "~";
  return {
    name: terminalName(projectPath, agents.map((a) => a.name)),
    kind: SHELL_KIND,
    projectPath,
    worktree: false,
  };
}

/** The Restart/Resume button of a stopped agent, from its caps (and, for a terminal, the agent it restarts as). */
export function restartAction(a: Pick<AgentView, "caps" | "restartAs">): { label: string; busy: string; note: string } {
  const resume = a.caps.resume;
  const verb = resume ? "Resume" : "Restart";
  const as = a.restartAs ?? null;
  const busy = `${resume ? "Resuming" : "Restarting"}…`;
  if (as) {
    return {
      label: `${verb} ${as}`,
      busy,
      note: resume ? ` Starts the shell and picks up ${as}'s previous session in it.` : ` Starts the shell and ${as} in it.`,
    };
  }
  return { label: verb, busy, note: resume ? " Resume picks up the previous session." : "" };
}

/** ⌘K: a typed path ("/…" or "~/…") offers "Terminal at <path>". */
export function looksLikePath(q: string): boolean {
  const t = q.trim();
  return t === "~" || t.startsWith("/") || t.startsWith("~/");
}

/** "Elsewhere" rows worth showing: not a conversation a Pitwall agent already has. */
export function visibleElsewhere(rows: RunningElsewhere[], agents: Pick<AgentView, "sessionId">[]): RunningElsewhere[] {
  const owned = new Set(agents.map((a) => a.sessionId).filter((s): s is string => !!s));
  return rows.filter((r) => !r.inPitwall && !(r.sessionId && owned.has(r.sessionId)));
}

/** Only a known conversation in a known folder can be resumed here. */
export function canBringIn(r: RunningElsewhere): boolean {
  return !!r.sessionId && !!r.cwd;
}

/** "Bring into Pitwall": resume the conversation where it runs (continue_conversation). */
export function bringInRequest(r: RunningElsewhere, agents: Pick<AgentView, "name">[]): ContinueConversationRequest | null {
  if (!r.sessionId || !r.cwd) return null;
  return {
    kind: r.kind,
    sessionId: r.sessionId,
    projectPath: r.cwd,
    name: agentNameFor(r.displayProject ?? r.cwd, agents.map((a) => a.name)),
    ...(r.displayProject ? { displayProject: r.displayProject } : {}),
  };
}
