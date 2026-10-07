import { describe, expect, it } from "vitest";
import type { AgentView, MachineView } from "../types";
import { groupByProject, machineHeading } from "./groups";

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
