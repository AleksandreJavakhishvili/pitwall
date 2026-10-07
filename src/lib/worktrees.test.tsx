// Worktrees in source control (docs/spec/worktrees-view.md): grouping for the
// sidebar and Review, what the UI offers by caps, and the mock's demo data.
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { AgentView, MachineView } from "../types";
import type { ProjectWorktrees, WorktreeView } from "../worktreesApi";
import { agentsSignature, countLabel, findWorktree, otherWorktrees, refKey, worktreesByAgent } from "./worktrees";
import { ActionsContext, type Actions } from "./actions";
import { AgentRow } from "../components/AgentRow";
import { WorktreeRows } from "../components/WorktreeRows";
import { WorktreeSection } from "../components/review/WorktreeSection";
import { createMockApi } from "../mock";

const here: MachineView = { provider: "p1", id: "m1", label: "This Mac", canCreate: true };
const caps = { diff: true, commit: true, merge: true, remove: true, terminal: true };
const wt = (name: string, p: Partial<WorktreeView>): WorktreeView => ({
  path: `/r/${name}`,
  pathDisplay: `~/r/${name}`,
  name,
  branch: `b-${name}`,
  head: "h",
  locked: false,
  lockReason: null,
  prunable: false,
  agentId: null,
  via: "other",
  caps,
  ...p,
});
const project = (id: string, agentIds: string[], worktrees: WorktreeView[]): ProjectWorktrees => ({
  id,
  repo: `/${id}`,
  repoDisplay: id,
  branch: "main",
  machine: here,
  agentIds,
  worktrees,
  error: null,
});

const projects = [
  project("api", ["a", "b"], [
    wt("zeta", { agentId: "a", via: "toolDir" }),
    wt("alpha", { agentId: "a", via: "process" }),
    wt("own", { agentId: "b", via: "own", caps: { ...caps, remove: false } }),
    wt("hotfix", {}),
  ]),
  project("web", ["c"], [wt("spare", {})]),
];

describe("worktree grouping", () => {
  it("lists an agent's own worktrees by name, never its working folder", () => {
    const by = worktreesByAgent(projects);
    expect(by.get("a")?.map((r) => [r.wt.name, r.projectId, r.target])).toEqual([
      ["alpha", "api", "main"],
      ["zeta", "api", "main"],
    ]);
    expect(by.has("b")).toBe(false);
  });

  it("puts the rest under the project group holding any of its agents", () => {
    expect(otherWorktrees(projects, ["b"]).map((r) => r.wt.name)).toEqual(["hotfix"]);
    expect(otherWorktrees(projects, ["c", "zz"]).map((r) => r.wt.name)).toEqual(["spare"]);
    expect(otherWorktrees(projects, ["nobody"])).toEqual([]);
  });

  it("finds a worktree again, or not once it left the list", () => {
    const r = findWorktree(projects, "api", "/r/hotfix")!;
    expect(refKey(r)).toBe(refKey({ projectId: "api", path: "/r/hotfix" }));
    expect(findWorktree(projects, "api", "/r/gone")).toBeNull();
    expect(findWorktree(projects, "nope", "/r/hotfix")).toBeNull();
  });

  it("only agents whose worktrees can be listed move the poller", () => {
    const a = { id: "a", cwd: "/r", added: 1, removed: 0, filesChanged: 1, branch: "main", status: "idle", caps: { worktrees: true } } as AgentView;
    const off = { ...a, id: "x", caps: { worktrees: false } } as AgentView;
    expect(agentsSignature([a, off])).toBe(agentsSignature([a]));
    expect(agentsSignature([{ ...a, added: 2 }])).not.toBe(agentsSignature([a]));
    expect(countLabel(1)).toBe("1 worktree");
    expect(countLabel(2)).toBe("2 worktrees");
  });
});

const actions = { openReview() {}, openTerminal() {}, openRemoveWorktree() {} } as unknown as Actions;
const withActions = (node: React.ReactNode) => renderToStaticMarkup(<ActionsContext.Provider value={actions}>{node}</ActionsContext.Provider>);

describe("worktree UI", () => {
  const agent = { id: "a", name: "refactor", status: "idle", kindName: "Claude Code", location: "local", added: 0, removed: 0 } as AgentView;

  it("an agent row shows a chip only when it has worktrees", () => {
    const row = (n: number) => renderToStaticMarkup(<AgentRow agent={agent} index={0} selected={false} where={null} onSelect={() => {}} worktrees={n} />);
    expect(row(2)).toContain("2 worktrees");
    expect(row(2)).toContain('aria-expanded="false"');
    expect(row(0)).not.toContain("worktree");
    const open = renderToStaticMarkup(
      <AgentRow agent={agent} index={0} selected={false} where={null} onSelect={() => {}} worktrees={1} worktreesOpen>
        <p>the list</p>
      </AgentRow>,
    );
    expect(open).toContain("the list");
  });

  it("rows show branch and locked; detached ones say so", () => {
    const refs = worktreesByAgent([
      project("api", ["a"], [
        wt("locked", { agentId: "a", via: "toolDir", locked: true, lockReason: "sub-agent running" }),
        wt("det", { agentId: "a", via: "toolDir", branch: null }),
      ]),
    ]).get("a")!;
    const h = withActions(<WorktreeRows refs={refs} label="Worktrees of refactor" />);
    expect(h).toContain("b-locked");
    expect(h).toContain("chip-warn");
    expect(h).toContain("sub-agent running");
    expect(h).toContain("det (detached)");
  });

  it("Review's section says when a worktree's folder is gone", () => {
    const r = worktreesByAgent([project("api", ["a"], [wt("gone", { agentId: "a", via: "toolDir", prunable: true, caps: { ...caps, diff: false, remove: false } })])]).get("a")![0];
    const h = renderToStaticMarkup(
      <WorktreeSection r={r} open current={false} selected={null} nonce={0} onToggle={() => {}} onShow={() => {}} onSelect={() => {}} isClosed={() => false} onToggleDir={() => {}} onFiles={() => {}} />,
    );
    expect(h).toContain("Its folder is gone.");
    expect(h).not.toContain("Reading git");
  });
});

describe("mock demo", () => {
  it("has an agent with two worktrees (one locked) and one other worktree", async () => {
    const api = createMockApi();
    const list = await api.listWorktrees();
    const agents = await api.listAgents();
    const by = worktreesByAgent(list);
    const owner = agents.find((a) => by.get(a.id)?.length === 2)!;
    expect(owner).toBeTruthy();
    const mine = by.get(owner.id)!;
    expect(mine.some((r) => r.wt.locked && !r.wt.caps.remove)).toBe(true);
    const others = otherWorktrees(list, [owner.id]);
    expect(others.map((r) => r.wt.branch)).toEqual(["hotfix/rate-limit"]);
    // Per-worktree calls work, and a locked one can't be removed.
    const files = await api.getWorktreeChanges(mine[0].projectId, mine[0].wt.path);
    expect(files.length).toBeGreaterThan(0);
    const locked = mine.find((r) => r.wt.locked)!;
    await expect(api.removeWorktree(locked.projectId, locked.wt.path)).rejects.toThrow(/locked/);
  });
});
