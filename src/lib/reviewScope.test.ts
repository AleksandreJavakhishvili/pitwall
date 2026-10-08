// Review lists the active space's agents (docs/spec/review.md).
import { describe, expect, it } from "vitest";
import type { AgentView } from "../types";
import type { Space } from "../state/workspace";
import { reviewScope } from "./reviewScope";

const a = (id: string, project: string) => ({ id, project }) as AgentView;
const AGENTS = [a("api", "/code/orders"), a("docs", "/code/handbook"), a("tests", "/code/checkout"), a("fix", "/code/orders")];
const layout = { type: "pane", id: "p1", agentId: null } as unknown as Space["layout"];
const space = (p: Partial<Space>): Space => ({ id: "s", name: "S", kind: "custom", members: [], layout, focusedPaneId: "p1", maximizedPaneId: null, ...p });
const ids = (r: { agents: AgentView[] }) => r.agents.map((x) => x.id);

describe("reviewScope", () => {
  it("All space: everyone, no toggle", () => {
    const r = reviewScope(AGENTS, space({ kind: "all" }), false);
    expect(ids(r)).toEqual(["api", "docs", "tests", "fix"]);
    expect(r.canWiden).toBe(false);
    expect(reviewScope(AGENTS, null, false).canWiden).toBe(false);
  });

  it("project space: that project's agents", () => {
    const r = reviewScope(AGENTS, space({ kind: "project", project: "/code/orders" }), false);
    expect(ids(r)).toEqual(["api", "fix"]);
    expect(r.canWiden).toBe(true);
  });

  it("custom space: its members", () => {
    expect(ids(reviewScope(AGENTS, space({ members: ["docs", "tests"] }), false))).toEqual(["docs", "tests"]);
  });

  it("All projects widens; the agent Review was opened for is always there", () => {
    const sp = space({ kind: "project", project: "/code/orders" });
    expect(ids(reviewScope(AGENTS, sp, true))).toEqual(["api", "docs", "tests", "fix"]);
    expect(ids(reviewScope(AGENTS, sp, false, ["tests"]))).toEqual(["api", "tests", "fix"]);
  });
});
