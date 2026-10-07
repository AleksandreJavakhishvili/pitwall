import { describe, expect, it } from "vitest";
import type { ApprovalView } from "../types";
import { requesterLine, timeLeft } from "./ApprovalDialog";

const base: ApprovalView = {
  id: "a",
  action: "session.add",
  summary: 'start the session "work" on vm-1 (agw)',
  details: [],
  requester: { kind: "agent", agentId: "a1", name: "Race Engineer", pid: 42, process: "pitwall" },
  risk: "low",
  rememberable: true,
  createdAt: 0,
  expiresAt: 120_000,
};

describe("approval dialog", () => {
  it("counts down to the automatic denial", () => {
    expect(timeLeft(120_000, 0)).toBe("2:00");
    expect(timeLeft(120_000, 114_500)).toBe("0:06");
    expect(timeLeft(120_000, 200_000)).toBe("0:00");
  });

  it("names who asked, as Pitwall established it", () => {
    expect(requesterLine(base)).toBe("agent “Race Engineer” in Pitwall (pitwall, pid 42)");
    const outside = { ...base, requester: { kind: "outside" as const, agentId: null, name: "x", pid: null, process: null } };
    expect(requesterLine(outside)).toBe("a process outside Pitwall's agents");
  });
});
