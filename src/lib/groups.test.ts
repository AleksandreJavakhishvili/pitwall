import { describe, expect, it } from "vitest";
import type { AgentView, MachineView } from "../types";
import { byAccount, groupByProject, machineHeading, type ProjectGroup } from "./groups";

const here: MachineView = { provider: "p1", id: "m1", label: "This Mac", canCreate: true };
const vm: MachineView = { provider: "p2", id: "vm", label: "my-vm", canCreate: false };
const agent = (id: string, project: string, machine: MachineView, status: AgentView["status"] = "idle") =>
  ({ id, name: id, project, projectDisplay: project.split("/").pop(), machine, status }) as AgentView;

describe("groupByProject", () => {
  it("groups by machine and project, creatable machines first", () => {
    const g = groupByProject([
      agent("a", "/w/api", vm),
      agent("b", "/c/zed", here),
      agent("c", "/w/api", here, "blocked"),
      agent("d", "/w/api", vm),
    ]);
    expect(g.map((x) => [x.key, x.agents.length, x.canCreate])).toEqual([
      ["/w/api", 1, true],
      ["/c/zed", 1, true],
      ["p2:vm|/w/api", 2, false],
    ]);
    expect(g[0].blocked).toBe(1);
    expect([0, 1, 2].map((i) => machineHeading(g, i))).toEqual(["This Mac", null, "my-vm"]);
  });

  it("shows no machine headings with one machine", () => {
    const g = groupByProject([agent("a", "/x", here), agent("b", "/y", here)]);
    expect(machineHeading(g, 0)).toBeNull();
  });
});

describe("byAccount", () => {
  const on = (a: AgentView, account: string | null) => ({ ...a, account }) as AgentView;
  it("groups by account then project, Default first, then the given order", () => {
    const empty: ProjectGroup = { key: "/w/empty", project: "/w/empty", display: "empty", machine: null, canCreate: true, agents: [], blocked: 0 };
    const s = byAccount(
      [
        on(agent("a", "/w/api", here), "claude-work"),
        on(agent("b", "/w/api", here, "blocked"), null),
        on(agent("c", "/w/web", here), "claude-work"),
        on(agent("d", "/w/docs", here), "codex-side"),
        on(agent("e", "/w/api", here), "claude-home"),
      ],
      [empty],
      ["claude-home", "claude-work"],
    );
    expect(s.map((x) => x.account)).toEqual(["default", "claude-home", "claude-work", "codex-side"]);
    expect(s[0].groups.map((g) => g.project)).toEqual(["/w/api", "/w/empty"]);
    expect([s[0].agents, s[0].blocked, s[2].agents]).toEqual([1, 1, 2]);
    expect(s[2].groups[0].key).toBe("acct:claude-work|/w/api");
    expect(byAccount([], [], [])).toEqual([]);
  });
  it("puts terminals and empty projects under their project's account", () => {
    const term = { ...agent("t", "/w/api", here), terminal: true } as AgentView;
    const quiet: ProjectGroup = { key: "/w/quiet", project: "/w/quiet", display: "quiet", machine: null, canCreate: true, agents: [], blocked: 0 };
    const of = (p: string) => ({ "/w/api": "claude-work", "/w/quiet": "claude-work" })[p];
    const s = byAccount([on(agent("a", "/w/api", here), "claude-work"), term, agent("b", "/w/web", here)], [quiet], ["claude-work"], of);
    expect(s.map((x) => x.account)).toEqual(["default", "claude-work"]);
    expect(s[0].groups.map((g) => g.project)).toEqual(["/w/web"]);
    expect(s[1].groups.map((g) => g.project)).toEqual(["/w/api", "/w/quiet"]);
    expect(s[1].agents).toBe(2);
  });
});
