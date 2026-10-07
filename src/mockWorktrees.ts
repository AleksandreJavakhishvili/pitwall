// Mock backend for worktrees in source control (browser `pnpm dev`): the
// "refactor" agent has two worktrees of its own (Claude sub-agents under
// .claude/worktrees, one locked) and orders-api has one other worktree.
import type { AgentView, FileChange, MachineView } from "./types";
import type { MergeStatus } from "./reviewTypes";
import type { ProjectWorktrees, WorktreeView, WorktreesApi } from "./worktreesApi";
import { versionsFor } from "./mockReview";

const wait = <T>(v: T, ms = 120) => new Promise<T>((r) => setTimeout(() => r(v), ms));
const LOCAL: MachineView = { provider: "local", id: "this-mac", label: "This Mac", canCreate: true };
const HOME = "/Users/dev";
const tilde = (p: string) => (p.startsWith(HOME) ? "~" + p.slice(HOME.length) : p);

interface Wt {
  repo: string;
  path: string;
  branch: string | null;
  /** Agent name it belongs to and how. */
  agent: string | null;
  via: WorktreeView["via"];
  locked?: string;
  files: FileChange[];
}

const ORDERS = "/Users/dev/code/orders-api";
const worktrees: Wt[] = [
  { repo: ORDERS, path: "/Users/dev/.codex/worktrees/a1b2/orders-api", branch: "codex/api-fix", agent: "api-fix", via: "own", files: [] },
  {
    repo: ORDERS,
    path: `${ORDERS}/.claude/worktrees/agent-a1f3`,
    branch: "worktree-agent-a1f3",
    agent: "refactor",
    via: "toolDir",
    files: [
      { path: "src/orders/handler.ts", added: 12, removed: 4, untracked: false, binary: false, status: "M" },
      { path: "src/orders/money.ts", added: 31, removed: 0, untracked: true, binary: false, status: "U" },
    ],
  },
  {
    repo: ORDERS,
    path: `${ORDERS}/.claude/worktrees/agent-77c2`,
    branch: "worktree-agent-77c2",
    agent: "refactor",
    via: "toolDir",
    locked: "claude agent agent-77c2 (pid 4242)",
    files: [{ path: "docs/api/orders.md", added: 8, removed: 2, untracked: false, binary: false, status: "M" }],
  },
  {
    repo: ORDERS,
    path: "/Users/dev/code/orders-api-hotfix",
    branch: "hotfix/rate-limit",
    agent: null,
    via: "other",
    files: [{ path: "src/middleware/rateLimit.ts", added: 5, removed: 1, untracked: false, binary: false, status: "M" }],
  },
  { repo: "/Users/dev/code/checkout-web", path: "/Users/dev/code/checkout-web/.claude/worktrees/tests", branch: "worktree-tests", agent: "tests", via: "own", files: [] },
  { repo: "/Users/dev/code/handbook", path: "/Users/dev/code/handbook/.claude/worktrees/docs", branch: "worktree-docs", agent: "docs", via: "own", files: [] },
];

const key = (repo: string) => `local:this-mac:${repo}`;

export function createWorktreesMock(agents: () => AgentView[]): WorktreesApi {
  const find = (projectId: string, path: string) => {
    const w = worktrees.find((x) => key(x.repo) === projectId && x.path === path);
    if (!w) throw new Error("That worktree isn't listed any more.");
    return w;
  };
  const view = (w: Wt, list: AgentView[]): WorktreeView => {
    const owner = w.agent ? (list.find((a) => a.name === w.agent) ?? null) : null;
    const via = owner ? w.via : "other";
    return {
      path: w.path,
      pathDisplay: tilde(w.path),
      name: w.path.split("/").pop() ?? w.path,
      branch: w.branch,
      head: "4f2c9e1",
      locked: !!w.locked,
      lockReason: w.locked ?? null,
      prunable: false,
      agentId: owner?.id ?? null,
      via,
      caps: { diff: true, commit: true, merge: !!w.branch, remove: !w.locked && via !== "own", terminal: true },
    };
  };
  const status = (w: Wt): MergeStatus => ({
    worktree: true,
    branch: w.branch,
    target: "main",
    targetDirty: false,
    uncommitted: w.files.length,
    ahead: w.files.length ? 1 : 0,
  });

  const listWorktrees = async (): Promise<ProjectWorktrees[]> => {
    const list = agents().filter((a) => a.caps.worktrees);
    const repos = [...new Set(worktrees.map((w) => w.repo))];
    return wait(
      repos
        .map((repo) => ({ repo, members: list.filter((a) => a.project === repo) }))
        .filter((r) => r.members.length > 0)
        .map(({ repo, members }) => ({
          id: key(repo),
          repo,
          repoDisplay: tilde(repo),
          branch: "main",
          machine: LOCAL,
          agentIds: members.map((a) => a.id),
          worktrees: worktrees.filter((w) => w.repo === repo).map((w) => view(w, list)),
          error: null,
        })),
      80,
    );
  };

  return {
    listWorktrees,
    async refreshWorktrees() {
      await wait(null, 300);
      return listWorktrees();
    },
    getWorktreeChanges: async (projectId, path) => wait(find(projectId, path).files.map((f) => ({ ...f }))),
    getWorktreeFileVersions: async (projectId, path, file) =>
      wait(versionsFor(find(projectId, path).files.find((f) => f.path === file), file)),
    getWorktreeMergeStatus: async (projectId, path) => wait(status(find(projectId, path))),
    async commitWorktree(projectId, path, message) {
      const w = find(projectId, path);
      if (!message.trim()) throw new Error("commit message is empty");
      if (!w.files.length) throw new Error("nothing to commit");
      return wait("9b1e0d3", 300);
    },
    async mergeWorktree(projectId, path) {
      const w = find(projectId, path);
      if (!w.branch) throw new Error("This worktree isn't on a branch (detached HEAD). Create a branch there, then merge.");
      w.files = [];
      return wait({ merged: true, conflict: false, message: `Merged ${w.branch} into main.`, branch: "main" }, 400);
    },
    async removeWorktree(projectId, path) {
      const w = find(projectId, path);
      if (w.locked) throw new Error(`${tilde(w.path)} is locked (${w.locked}). Pitwall doesn't remove locked worktrees.`);
      if (w.via === "own") throw new Error(`${tilde(w.path)} is where an agent works: remove the agent (with its worktree) instead.`);
      worktrees.splice(worktrees.indexOf(w), 1);
      return wait(undefined, 300);
    },
  };
}
