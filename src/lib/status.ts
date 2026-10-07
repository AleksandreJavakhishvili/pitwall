import type { AgentView, Status } from "../types";

export const STATUS_ORDER: Record<Status, number> = {
  blocked: 0,
  done: 1,
  working: 2,
  idle: 3,
  unknown: 4,
  exited: 5,
  stopped: 6,
};

export const STATUS_WORD: Record<Status, string> = {
  blocked: "needs you",
  done: "done",
  working: "working",
  idle: "idle",
  unknown: "unknown",
  exited: "exited",
  stopped: "stopped",
};

export const STATUS_GLYPH: Record<Status, string> = {
  blocked: "▲",
  done: "⚑",
  working: "◐",
  idle: "●",
  unknown: "?",
  exited: "■",
  stopped: "■",
};

/** Sidebar order: blocked → done → working → idle → others; stable by creation time. */
export function sortAgents(agents: AgentView[]): AgentView[] {
  return [...agents].sort(
    (a, b) => STATUS_ORDER[a.status] - STATUS_ORDER[b.status] || a.createdAt - b.createdAt,
  );
}

/** Short race-radio line for an agent's state. */
export function radioLine(a: Pick<AgentView, "name" | "status">): string {
  switch (a.status) {
    case "blocked":
      return `${a.name} needs you`;
    case "done":
      return `${a.name} is done`;
    case "working":
      return `${a.name} is working`;
    case "exited":
      return `${a.name} has exited`;
    case "stopped":
      return `${a.name} is stopped`;
    default:
      return `${a.name} is ${a.status}`;
  }
}

export const NAME_RE = /^[a-z][a-z0-9_-]{0,31}$/;
