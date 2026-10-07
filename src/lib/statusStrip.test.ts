import { describe, expect, it } from "vitest";
import type { AgentView } from "../types";
import { stripState } from "./statusStrip";

const a = (id: string, status: AgentView["status"]) => ({ id, name: id, status }) as AgentView;

describe("status strip", () => {
  it("is calm with counts when nothing needs you", () => {
    const s = stripState([a("x", "working"), a("y", "working"), a("z", "idle")]);
    expect(s).toMatchObject({ tone: "calm", lead: null, more: 0, working: 2, done: 0, blocked: 0, total: 3 });
    expect(stripState([])).toMatchObject({ tone: "calm", total: 0 });
  });

  it("names the first blocked agent, before any finished one", () => {
    const s = stripState([a("d", "done"), a("b1", "blocked"), a("w", "working"), a("b2", "blocked")]);
    expect([s.tone, s.lead?.id, s.more, s.done, s.blocked]).toEqual(["blocked", "b1", 1, 1, 2]);
  });

  it("is mint when only finished agents wait", () => {
    const s = stripState([a("w", "working"), a("d1", "done"), a("d2", "done")]);
    expect([s.tone, s.lead?.id, s.more]).toEqual(["done", "d1", 1]);
  });
});
