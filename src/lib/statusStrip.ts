// The bottom status strip (docs/spec/layout.md "Needs-you strip"): always the
// same height, so its changes never resize the terminals. Pure (unit-tested).
import type { AgentView } from "../types";

export type StripTone = "calm" | "blocked" | "done";

export interface StripState {
  /** blocked: someone needs you (amber) · done: finished, not looked at (mint) · calm: neither. */
  tone: StripTone;
  /** The agent the strip names (first blocked, else first finished), in the given order. */
  lead: AgentView | null;
  /** Others in the same state as `lead`. */
  more: number;
  working: number;
  done: number;
  blocked: number;
  total: number;
}

/** `agents` in sidebar order (the first blocked one is the one ⌘J jumps to first). */
export function stripState(agents: AgentView[]): StripState {
  const of = (s: AgentView["status"]) => agents.filter((a) => a.status === s);
  const blocked = of("blocked");
  const done = of("done");
  const lead = blocked[0] ?? done[0] ?? null;
  const tone: StripTone = blocked.length ? "blocked" : done.length ? "done" : "calm";
  const same = tone === "blocked" ? blocked.length : tone === "done" ? done.length : 0;
  return {
    tone,
    lead,
    more: Math.max(0, same - 1),
    working: of("working").length,
    done: done.length,
    blocked: blocked.length,
    total: agents.length,
  };
}
