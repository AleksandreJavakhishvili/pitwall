import { describe, expect, it } from "vitest";
import { shareAgents } from "./useAgents";
import type { AgentView } from "../types";

const agent = (id: string, status = "idle") => ({ id, name: id, status }) as unknown as AgentView;

describe("shareAgents", () => {
  it("keeps the previous list when nothing changed", () => {
    const prev = [agent("a"), agent("b")];
    expect(shareAgents(prev, [agent("a"), agent("b")])).toBe(prev);
  });

  it("reuses unchanged agents and takes changed ones", () => {
    const prev = [agent("a"), agent("b")];
    const next = shareAgents(prev, [agent("a"), agent("b", "working")]);
    expect(next).not.toBe(prev);
    expect(next[0]).toBe(prev[0]);
    expect(next[1].status).toBe("working");
  });

  it("notices order, additions and removals", () => {
    const prev = [agent("a"), agent("b")];
    const swapped = shareAgents(prev, [agent("b"), agent("a")]);
    expect(swapped).not.toBe(prev);
    expect(swapped[0]).toBe(prev[1]);
    expect(shareAgents(prev, [agent("a")])).toHaveLength(1);
    expect(shareAgents(prev, [agent("a"), agent("b"), agent("c")])).toHaveLength(3);
  });
});
