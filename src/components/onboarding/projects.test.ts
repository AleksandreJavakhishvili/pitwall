import { describe, expect, it, vi } from "vitest";
import type { Project, RunningElsewhere, ScannedConversation, ScannedProject } from "../../types";
import type { ProjectGroup } from "../../lib/groups";
import {
  agentNameFor,
  defaultConversations,
  defaultSelection,
  planAgents,
  rememberedShowUnder,
  startAll,
  withProjects,
} from "./projects";

const group = (project: string, display: string): ProjectGroup => ({
  key: project,
  project,
  display,
  machine: null,
  canCreate: true,
  agents: [],
  blocked: 0,
});
const project = (path: string, display: string): Project => ({ path, display, isGit: true, addedAt: 1 });
const scanned = (path: string, p: Partial<ScannedProject> = {}): ScannedProject => ({
  path,
  display: path,
  isGit: true,
  lastUsed: Date.now(),
  sources: ["claude"],
  agentHistory: true,
  added: false,
  rules: { rulesync: false, claudeMd: false, agentsMd: false },
  ...p,
});

describe("withProjects", () => {
  it("adds projects without agents, alphabetically, without duplicates", () => {
    const groups = [group("/c/orders", "~/c/orders")];
    const out = withProjects(groups, [project("/c/orders", "~/c/orders"), project("/c/alpha", "~/c/alpha")]);
    expect(out.map((g) => g.project)).toEqual(["/c/alpha", "/c/orders"]);
    expect(out[0].agents).toEqual([]);
  });
  it("returns the same groups when nothing is added", () => {
    const groups = [group("/x", "x")];
    expect(withProjects(groups, [])).toBe(groups);
  });
});

describe("defaultSelection", () => {
  it("picks recent agent projects that aren't added yet", () => {
    const now = Date.now();
    const sel = defaultSelection(
      [
        scanned("/a"),
        scanned("/old", { lastUsed: now - 90 * 24 * 3600_000 }),
        scanned("/editor", { sources: ["vscode"], agentHistory: false, lastUsed: null }),
        scanned("/recent-editor", { sources: ["vscode"], agentHistory: false }),
        scanned("/added", { added: true }),
        scanned("/codex", { sources: ["codex", "folder"] }),
      ],
      now,
    );
    expect([...sel]).toEqual(["/a", "/codex"]);
  });
});

describe("agentNameFor", () => {
  it("makes valid unique names", () => {
    expect(agentNameFor("/Users/me/code/Orders API", [])).toBe("orders-api");
    expect(agentNameFor("/Users/me/code/orders-api", ["orders-api", "orders-api-2"])).toBe("orders-api-3");
    expect(agentNameFor("/Users/me/123", [])).toBe("agent");
    expect(agentNameFor("/", [])).toBe("agent");
  });
});

const conv = (projectPath: string, sessionId: string, ago: number, p: Partial<ScannedConversation> = {}): ScannedConversation => ({
  kind: "claude",
  kindName: "Claude Code",
  sessionId,
  projectPath,
  projectDisplay: projectPath,
  title: sessionId,
  lastUsed: NOW - ago,
  inPitwall: false,
  outsideProject: false,
  displayProject: null,
  runningElsewhere: false,
  ...p,
});
const NOW = 1_800_000_000_000;
const H = 3600_000;
const running = (pid: number, p: Partial<RunningElsewhere> = {}): RunningElsewhere => ({
  pid,
  kind: "claude",
  kindName: "Claude Code",
  cwd: "/r",
  cwdDisplay: "/r",
  sessionId: `run-${pid}`,
  title: null,
  inPitwall: false,
  outsideProject: false,
  displayProject: null,
  ...p,
});

describe("defaultConversations", () => {
  const installed = new Set(["claude", "codex"]);
  it("ticks the newest recent conversation of each ticked project", () => {
    const convs = [
      conv("/a", "a-new", 1 * H),
      conv("/a", "a-old", 2 * H),
      conv("/b", "b-stale", 80 * H), // older than 3 days
      conv("/c", "c-1", 1 * H), // project not ticked
      conv("/d", "d-codex", 1 * H, { kind: "codex" }),
      conv("/d", "d-claude", 3 * H),
    ];
    const sel = defaultConversations(convs, new Set(["/a", "/b", "/d"]), { agents: [], installed, now: NOW });
    expect([...sel].sort()).toEqual(["claude:a-new", "codex:d-codex"]);
  });
  it("skips sessions already in Pitwall, missing kinds and projects with agents", () => {
    const convs = [
      conv("/a", "in", 1 * H, { inPitwall: true }),
      conv("/a", "next", 2 * H),
      conv("/b", "b", 1 * H, { kind: "aider" }),
      conv("/c", "c", 1 * H),
    ];
    const sel = defaultConversations(convs, new Set(["/a", "/b", "/c"]), {
      agents: [{ cwd: "/c", project: "/c" }],
      installed,
      now: NOW,
    });
    expect([...sel]).toEqual(["claude:next"]);
  });
});

describe("planAgents", () => {
  it("plans conversations, running sessions and fresh agents with unique names", () => {
    const conversations = [conv("/x/orders", "s1", H), conv("/x/orders", "s2", 2 * H), conv("/x/web", "s3", H)];
    const plan = planAgents({
      conversations,
      convSel: new Set(["claude:s1", "claude:s2"]),
      running: [running(7, { cwd: "/x/web", sessionId: "s1" }), running(8, { cwd: "/x/infra" }), running(9, { sessionId: null })],
      runSel: new Set([7, 8, 9]),
      fresh: { "/x/orders": "codex", "/x/docs": "codex" },
      taken: ["orders"],
    });
    expect(plan).toEqual([
      { name: "orders-2", kind: "claude", projectPath: "/x/orders", sessionId: "s1" },
      { name: "orders-3", kind: "claude", projectPath: "/x/orders", sessionId: "s2" },
      // pid 7 resumes s1 again → skipped; pid 9 has no session → skipped
      { name: "infra", kind: "claude", projectPath: "/x/infra", sessionId: "run-8" },
      // "/x/orders" already gets agents, so only docs gets a fresh one
      { name: "docs", kind: "codex", projectPath: "/x/docs" },
    ]);
    for (const p of plan) expect(p.name).toMatch(/^[a-z][a-z0-9_-]{0,31}$/);
  });
  it("never plans sessions already in Pitwall", () => {
    const plan = planAgents({
      conversations: [conv("/a", "s", H, { inPitwall: true })],
      convSel: new Set(["claude:s"]),
      running: [running(1, { inPitwall: true })],
      runSel: new Set([1]),
      fresh: {},
      taken: [],
    });
    expect(plan).toEqual([]);
  });
});

describe("conversations started in ~", () => {
  const HOME = "/Users/me";
  it("are never ticked by default, nor are sessions running elsewhere", () => {
    const convs = [
      conv(HOME, "home", H, { outsideProject: true }),
      conv("/a", "busy", H, { runningElsewhere: true }),
      conv("/a", "older", 2 * H),
    ];
    const sel = defaultConversations(convs, new Set(["/a", HOME]), { agents: [], installed: new Set(["claude"]), now: NOW });
    expect([...sel]).toEqual(["claude:older"]);
  });
  it("run in their own cwd but show under the chosen project", () => {
    const plan = planAgents({
      conversations: [
        conv(HOME, "h1", H, { outsideProject: true }),
        conv(HOME, "h2", H, { outsideProject: true }),
        // A choice for a normal conversation is ignored.
        conv("/x/web", "w", H),
      ],
      convSel: new Set(["claude:h1", "claude:h2", "claude:w"]),
      running: [running(5, { cwd: HOME, sessionId: "r5", outsideProject: true })],
      runSel: new Set([5]),
      fresh: { "/x/pitwall": "claude" },
      taken: [],
      showUnder: { "claude:h1": "/x/pitwall", "claude:w": "/x/pitwall", "claude:r5": "/x/infra" },
    });
    expect(plan).toEqual([
      { name: "pitwall", kind: "claude", projectPath: HOME, sessionId: "h1", displayProject: "/x/pitwall" },
      { name: "me", kind: "claude", projectPath: HOME, sessionId: "h2" },
      { name: "web", kind: "claude", projectPath: "/x/web", sessionId: "w" },
      { name: "infra", kind: "claude", projectPath: HOME, sessionId: "r5", displayProject: "/x/infra" },
      // "/x/pitwall" already gets the h1 agent → no fresh one
    ]);
  });
  it("remembers the project picked last time", () => {
    const got = rememberedShowUnder(
      [conv(HOME, "h", H, { outsideProject: true, displayProject: "/x/p" }), conv("/a", "a", H, { displayProject: "/x/q" })],
      [running(1, { sessionId: "r", outsideProject: true, displayProject: "/x/r" }), running(2, { sessionId: null, outsideProject: true, displayProject: "/x/s" })],
    );
    expect(got).toEqual({ "claude:h": "/x/p", "claude:r": "/x/r" });
  });
});

describe("startAll", () => {
  it("starts one by one, reports failures and keeps going", async () => {
    const plan = [
      { name: "a", kind: "claude", projectPath: "/a", sessionId: "1" },
      { name: "b", kind: "claude", projectPath: "/b" },
      { name: "c", kind: "codex", projectPath: "/c" },
    ];
    const calls: string[] = [];
    const progress: string[] = [];
    const failed: string[] = [];
    const create = vi.fn(async (p: (typeof plan)[number]) => {
      calls.push(p.name);
      if (p.name === "b") throw "boom";
      return { id: p.name };
    });
    const out = await startAll(plan, create, {
      onProgress: (d, t) => progress.push(`${d}/${t}`),
      onError: (p, e) => failed.push(`${p.name}:${e}`),
    });
    expect(out).toEqual([{ id: "a" }, { id: "c" }]);
    expect(calls).toEqual(["a", "b", "c"]);
    expect(progress).toEqual(["0/3", "1/3", "2/3"]);
    expect(failed).toEqual(["b:boom"]);
  });
});
