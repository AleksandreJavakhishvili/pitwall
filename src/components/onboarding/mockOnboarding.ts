// Browser-mock side of the first-launch scan: staggered scan-progress events,
// a persisted (localStorage) project list and the onboarded flag.
import type { Api, Unlisten } from "../../api";
import type { AgentView, AgwSession, CreateAgentRequest, Project, ScanProgress, ScanResult } from "../../types";

type OnboardingApi = Pick<
  Api,
  | "scanEnvironment"
  | "getOnboarded"
  | "listProjects"
  | "addProject"
  | "removeProject"
  | "completeOnboarding"
  | "continueConversation"
  | "onScanProgress"
  | "onProjectsChanged"
>;

const KEY = "pitwall.mock.projects";
const HOME = "/Users/ako";
const min = 60_000;
const hour = 60 * min;

interface Stored {
  onboarded: boolean;
  projects: Project[];
  /** session id → "Show under project…" choice. */
  shownUnder?: Record<string, string>;
}

function load(): Stored {
  try {
    const s = localStorage.getItem(KEY);
    if (s) return JSON.parse(s) as Stored;
  } catch {
    /* ignore */
  }
  // `?onboarded` in the URL skips the welcome screen in the mock.
  const skip = typeof location !== "undefined" && new URLSearchParams(location.search).has("onboarded");
  return { onboarded: skip, projects: [] };
}

function save(s: Stored) {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* ignore */
  }
}

const tilde = (p: string) => (p.startsWith(HOME) ? "~" + p.slice(HOME.length) : p);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const AGW_KINDS: Record<string, [string, string]> = {
  claude: ["claude-code", "Claude Code"],
  codex: ["codex", "Codex"],
  shell: ["login-shell", "Shell"],
};

function agwSession(
  machine: string,
  name: string,
  kind: "claude" | "codex" | "shell",
  workspace: string,
  agent: string | null,
  status: AgwSession["status"],
): AgwSession {
  const [harness, kindName] = AGW_KINDS[kind];
  return { machine, name, kind, kindName, harness, workspace, agent, status };
}

function fakeScan(known: Set<string>): ScanResult {
  const now = Date.now();
  const proj = (
    path: string,
    sources: ScanResult["projects"][number]["sources"],
    lastUsed: number | null,
    rules: Partial<ScanResult["projects"][number]["rules"]> = {},
    isGit = true,
  ) => ({
    path,
    display: tilde(path),
    isGit,
    lastUsed,
    sources,
    agentHistory: sources.some((s) => s !== "vscode" && s !== "cursor" && s !== "folder"),
    added: known.has(path),
    rules: { rulesync: false, claudeMd: false, agentsMd: false, ...rules },
  });
  return {
    agents: [
      { kind: "claude", name: "Claude Code", installed: true, path: "/opt/homebrew/bin/claude", version: "2.1.280" },
      { kind: "codex", name: "Codex", installed: true, path: "/opt/homebrew/bin/codex", version: "0.160.1" },
      { kind: "aider", name: "Aider", installed: false, path: null, version: null },
    ],
    projects: [
      proj(`${HOME}/code/orders-api`, ["claude", "codex", "vscode"], now - 10 * min, { claudeMd: true, agentsMd: true }),
      proj(`${HOME}/code/checkout-web`, ["claude", "folder"], now - 50 * min, { rulesync: true }),
      proj(`${HOME}/code/handbook`, ["claude"], now - 3 * hour, {}, true),
      proj(`${HOME}/Desktop/Projects/combat-sim`, ["codex"], now - 26 * hour, { agentsMd: true }),
      proj(`${HOME}/code/infra`, ["folder"], now - 9 * 24 * hour),
      proj(`${HOME}/Desktop/sparks-api`, ["vscode"], null),
      proj(`${HOME}/Desktop/sparks-web`, ["vscode", "cursor"], null, {}, false),
    ],
    conversations: [
      {
        kind: "claude",
        kindName: "Claude Code",
        sessionId: "985e7068-10dd-4ed3-a8fa-4d6b402c38e6",
        projectPath: `${HOME}/code/orders-api`,
        projectDisplay: "~/code/orders-api",
        title: "Fix the 500 on /v2/orders when the cart is empty. Add a regression test.",
        lastUsed: now - 10 * min,
        inPitwall: false,
      },
      {
        kind: "codex",
        kindName: "Codex",
        sessionId: "019efa39-c007-7e73-85bc-c56d596129ad",
        projectPath: `${HOME}/code/orders-api`,
        projectDisplay: "~/code/orders-api",
        title: "Rename createOrder() to placeOrder() everywhere and update the docs",
        lastUsed: now - 2 * hour,
        inPitwall: false,
      },
      {
        kind: "claude",
        kindName: "Claude Code",
        sessionId: "1919cf60-564d-4dff-8aae-6d9c26d58726",
        projectPath: `${HOME}/code/checkout-web`,
        projectDisplay: "~/code/checkout-web",
        title: "Run the test suite and fix the flaky cart spec. Don't touch the snapshot files.",
        lastUsed: now - 50 * min,
        inPitwall: false,
      },
      {
        kind: "claude",
        kindName: "Claude Code",
        sessionId: "f1d7cc90-148a-44a6-893b-c3629d391872",
        projectPath: `${HOME}/code/handbook`,
        projectDisplay: "~/code/handbook",
        title: "Rewrite the onboarding page so a new hire can get a dev env running in 15 minutes.",
        lastUsed: now - 3 * hour,
        inPitwall: false,
      },
      {
        kind: "claude",
        kindName: "Claude Code",
        sessionId: "7d1a5e0c-3f4b-4a8e-9c2d-6b0e1f2a3c4d",
        projectPath: HOME,
        projectDisplay: "~",
        title: "Add a Settings → Scan again button to Pitwall and keep the project choice per session",
        lastUsed: now - 40 * min,
        inPitwall: false,
        outsideProject: true,
        displayProject: null,
        runningElsewhere: true,
      },
      {
        kind: "codex",
        kindName: "Codex",
        sessionId: "019f0b2c-aaaa-7bbb-8ccc-1234567890ab",
        projectPath: HOME,
        projectDisplay: "~",
        title: "Why is my zsh startup slow?",
        lastUsed: now - 30 * hour,
        inPitwall: false,
        outsideProject: true,
        displayProject: null,
        runningElsewhere: false,
      },
    ].map((c) => ({ outsideProject: false, displayProject: null, runningElsewhere: false, ...c })),
    running: [
      {
        pid: 29306,
        kind: "claude",
        kindName: "Claude Code",
        cwd: `${HOME}/code/infra`,
        cwdDisplay: "~/code/infra",
        sessionId: "5c0ffee0-2b1e-4c55-9d1a-0d5f3e7a9b21",
        title: "Bump the Terraform AWS provider and fix the plan diff",
        inPitwall: false,
        outsideProject: false,
        displayProject: null,
      },
      {
        pid: 41022,
        kind: "claude",
        kindName: "Claude Code",
        cwd: HOME,
        cwdDisplay: "~",
        sessionId: "7d1a5e0c-3f4b-4a8e-9c2d-6b0e1f2a3c4d",
        title: "Add a Settings → Scan again button to Pitwall and keep the project choice per session",
        inPitwall: false,
        outsideProject: true,
        displayProject: null,
      },
      {
        pid: 31479,
        kind: "codex",
        kindName: "Codex",
        cwd: null,
        cwdDisplay: null,
        sessionId: null,
        title: null,
        inPitwall: false,
        outsideProject: false,
        displayProject: null,
      },
    ],
    agw: {
      path: `${HOME}/.local/bin/agw`,
      version: "0.19.0",
      machines: [
        {
          id: "shakedown",
          site: "lima-local",
          status: "running",
          sessions: [
            agwSession("shakedown", "gwlive", "claude", "gwlive", null, "running"),
            agwSession("shakedown", "orders-fix", "codex", "orders-api", "codex-bot", "running"),
            agwSession("shakedown", "work", "claude", "work", null, "stopped"),
          ],
        },
        {
          id: "gpu-box",
          site: "aws-us-east",
          status: "stopped",
          sessions: [agwSession("gpu-box", "train-eval", "shell", "ml-evals", "evals", "stopped")],
        },
      ],
    },
    codexHooks: { installed: false, path: "~/.codex/hooks.json" },
  };
}

/** `create` is the mock's own createAgent (continuing = creating, in the mock). */
export function createOnboardingMock(create: (req: CreateAgentRequest) => Promise<AgentView>): OnboardingApi {
  const progressSubs = new Set<(e: ScanProgress) => void>();
  const projectSubs = new Set<(p: Project[]) => void>();
  let state = load();
  /** Sessions the mock's agents resumed (so "Scan again" skips them). */
  const continued = new Set<string>();

  const sorted = () => [...state.projects].sort((a, b) => a.display.localeCompare(b.display));
  const commit = () => {
    save(state);
    const list = sorted();
    projectSubs.forEach((cb) => cb(list));
    return list;
  };
  const add = (path: string) => {
    const p = path.trim().replace(/\/+$/, "").replace(/^~(?=\/|$)/, HOME);
    if (!p.startsWith("/")) throw `${path} is not a folder`;
    if (!state.projects.some((x) => x.path === p)) {
      state = {
        ...state,
        projects: [...state.projects, { path: p, display: tilde(p), isGit: !/notes|scratch/.test(p), addedAt: Date.now() }],
      };
    }
  };
  const emit = (e: ScanProgress) => progressSubs.forEach((cb) => cb(e));

  return {
    async scanEnvironment() {
      const result = fakeScan(new Set(state.projects.map((p) => p.path)));
      const chosen = state.shownUnder ?? {};
      result.conversations = result.conversations.map((c) => ({
        ...c,
        inPitwall: continued.has(c.sessionId),
        displayProject: c.outsideProject ? (chosen[c.sessionId] ?? null) : null,
      }));
      result.running = result.running.map((r) => ({
        ...r,
        inPitwall: !!r.sessionId && continued.has(r.sessionId),
        displayProject: r.outsideProject && r.sessionId ? (chosen[r.sessionId] ?? null) : null,
      }));
      const steps: [ScanProgress["step"], ScanProgress["status"], string, number][] = [
        ["agents", "done", "Claude Code 2.1.280 · Codex 0.160.1 · agw 0.19.0 (2 VMs)", 900],
        ["projects", "done", `${result.projects.length} projects`, 900],
        ["conversations", "done", `${result.conversations.length} you can continue`, 600],
        ["running", "done", `${result.running.length} on this Mac · 4 sessions on agw`, 500],
        ["rules", "done", "3 projects with rule files", 250],
        ["hooks", "done", "Codex hooks not installed", 200],
      ];
      for (const [step, status, summary, ms] of steps) {
        emit({ step, status: "running" });
        await sleep(ms);
        emit({ step, status, summary });
      }
      return result;
    },
    getOnboarded: async () => state.onboarded,
    listProjects: async () => sorted(),
    async addProject(path) {
      await sleep(80);
      add(path);
      return commit();
    },
    async removeProject(path) {
      state = { ...state, projects: state.projects.filter((p) => p.path !== path) };
      return commit();
    },
    async completeOnboarding(projects, installCodexHooks) {
      await sleep(300);
      projects.forEach(add);
      state = { ...state, onboarded: true };
      if (installCodexHooks) console.info("[mock] would merge Pitwall hooks into ~/.codex/hooks.json (backup first)");
      return commit();
    },
    async continueConversation(req) {
      const a = await create({
        name: req.name,
        kind: req.kind,
        projectPath: req.projectPath,
        worktree: false,
        resumeSessionId: req.sessionId,
        displayProject: req.displayProject,
      });
      continued.add(req.sessionId);
      if (req.displayProject) {
        state = { ...state, shownUnder: { ...state.shownUnder, [req.sessionId]: req.displayProject } };
        add(req.displayProject);
      } else if (req.projectPath !== HOME) add(req.projectPath);
      commit();
      return a;
    },
    async onScanProgress(cb): Promise<Unlisten> {
      progressSubs.add(cb);
      return () => progressSubs.delete(cb);
    },
    async onProjectsChanged(cb): Promise<Unlisten> {
      projectSubs.add(cb);
      return () => projectSubs.delete(cb);
    },
  };
}
