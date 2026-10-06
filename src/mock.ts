// Mock backend for running the UI in a plain browser (`pnpm dev`).
// Behaves like the real contract: commands mutate state and emit agents-changed.
import type { Api, Unlisten } from "./api";
import { createOnboardingMock } from "./components/onboarding/mockOnboarding";
import { createReviewMock } from "./mockReview";
import type {
  AgentView,
  AttentionEvent,
  FileChange,
  HooksStatus,
  KindView,
  RecentProject,
  Status,
} from "./types";

const enc = new TextEncoder();
const now = Date.now();
const min = 60_000;

// ── ANSI helpers ───────────────────────────────────────────────────────────
const E = "\x1b[";
const reset = `${E}0m`;
const bold = (s: string) => `${E}1m${s}${reset}`;
const fg = (n: number, s: string) => `${E}38;5;${n}m${s}${reset}`;
const green = (s: string) => fg(114, s);
const red = (s: string) => fg(203, s);
const orange = (s: string) => fg(209, s);
const blue = (s: string) => fg(111, s);
const grey = (s: string) => fg(245, s);
const yellow = (s: string) => fg(221, s);
const NL = "\r\n";

function box(lines: string[], width = 64, color = (s: string) => grey(s)): string {
  const strip = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "");
  const top = color("╭" + "─".repeat(width - 2) + "╮");
  const bot = color("╰" + "─".repeat(width - 2) + "╯");
  const body = lines.map((l) => {
    const pad = Math.max(0, width - 4 - [...strip(l)].length);
    return color("│") + " " + l + " ".repeat(pad) + " " + color("│");
  });
  return [top, ...body, bot].join(NL) + NL;
}

function claudeBanner(cwd: string): string {
  return (
    box([`${orange("✻")} ${bold("Welcome to Claude Code")}`, "", grey(`  cwd: ${cwd}`)], 56, (s) => orange(s)) + NL
  );
}

function userTurn(text: string): string {
  return `${grey(">")} ${text}${NL}${NL}`;
}

const screens: Record<string, string> = {
  tests:
    claudeBanner("~/code/checkout-web") +
    userTurn("Run the test suite and fix the flaky cart spec. Don't touch the snapshot files.") +
    `${green("⏺")} I'll run the cart specs first to reproduce the flake.${NL}${NL}` +
    `${green("⏺")} ${bold("Read")}(src/cart/cart.spec.ts)${NL}` +
    `  ${grey("⎿")}  Read ${bold("212")} lines${NL}${NL}` +
    `${green("⏺")} ${bold("Read")}(src/cart/useCart.ts)${NL}` +
    `  ${grey("⎿")}  Read ${bold("88")} lines${NL}${NL}` +
    `${green("⏺")} The spec relies on ${blue("Date.now()")} without a fake timer, so the${NL}` +
    `  "expires after 30 minutes" case races the clock. I'll pin the timer${NL}` +
    `  and re-run the suite.${NL}${NL}` +
    `${green("⏺")} ${bold("Update")}(src/cart/cart.spec.ts)${NL}` +
    `  ${grey("⎿")}  Updated src/cart/cart.spec.ts with ${green("9 additions")} and ${red("2 removals")}${NL}` +
    `     ${grey("41")}   describe("cart expiry", () => {${NL}` +
    `     ${grey("42")} ${green("+   beforeEach(() => vi.useFakeTimers());")}${NL}` +
    `     ${grey("43")} ${green("+   afterEach(() => vi.useRealTimers());")}${NL}${NL}` +
    box(
      [
        bold("Bash command"),
        "",
        `  ${blue("pnpm test --run src/cart")}`,
        grey("  Run cart specs"),
        "",
        "Do you want to proceed?",
        `${blue("❯")} ${blue("1. Yes")}`,
        "  2. Yes, and don't ask again for pnpm test commands",
        "  3. No, and tell Claude what to do differently (esc)",
      ],
      68,
      (s) => blue(s),
    ),
  "api-fix":
    `${bold(">_ codex")} ${grey("· model gpt-5-codex · ~/code/orders-api")}${NL}${NL}` +
    `${bold("user")}${NL}Fix the 500 on /v2/orders when the cart is empty. Add a regression test.${NL}${NL}` +
    `${fg(176, "thinking")}${NL}${grey("Tracing the handler: createOrder() assumes items[0] exists.")}${NL}${NL}` +
    `${yellow("exec")} ${bold("rg -n \"items\\[0\\]\" src/")}${NL}` +
    `${grey("src/orders/handler.ts:57:  const first = items[0].sku;")}${NL}` +
    `${grey("src/orders/pricing.ts:19:  return items[0].currency;")}${NL}${NL}` +
    `${yellow("apply_patch")} ${grey("src/orders/handler.ts")}${NL}` +
    `${red("-  const first = items[0].sku;")}${NL}` +
    `${green("+  if (items.length === 0) {")}${NL}` +
    `${green("+    return reply.code(422).send({ error: \"cart_empty\" });")}${NL}` +
    `${green("+  }")}${NL}${NL}` +
    `${yellow("exec")} ${bold("pnpm vitest run orders")}${NL}`,
  docs:
    claudeBanner("~/code/handbook/.claude/worktrees/docs") +
    userTurn("Rewrite the onboarding page so a new hire can get a dev env running in 15 minutes.") +
    `${green("⏺")} ${bold("Read")}(docs/onboarding.md)${NL}` +
    `  ${grey("⎿")}  Read ${bold("140")} lines${NL}${NL}` +
    `${green("⏺")} ${bold("Write")}(docs/onboarding.md)${NL}` +
    `  ${grey("⎿")}  Wrote ${bold("96")} lines to docs/onboarding.md${NL}${NL}` +
    `${green("⏺")} ${bold("Write")}(docs/dev-env/checklist.md)${NL}` +
    `  ${grey("⎿")}  Wrote ${bold("34")} lines to docs/dev-env/checklist.md${NL}${NL}` +
    `${green("⏺")} Done. The onboarding page now opens with a 6-step checklist,${NL}` +
    `  moves troubleshooting to the end, and links the new${NL}` +
    `  ${blue("docs/dev-env/checklist.md")}. I removed the outdated Vagrant section.${NL}${NL}` +
    `${grey("─".repeat(60))}${NL}${grey(">")} ${NL}${grey("─".repeat(60))}${NL}` +
    `  ${grey("? for shortcuts")}${NL}`,
  refactor:
    claudeBanner("~/code/orders-api") +
    `${grey("─".repeat(60))}${NL}${grey(">")} ${NL}${grey("─".repeat(60))}${NL}` +
    `  ${grey("? for shortcuts")}${NL}`,
  scratch:
    `${green("ako@mbp")} ${blue("~/code/orders-api")} ${grey("(main)")} $ git log --oneline -3${NL}` +
    `${yellow("a41c9e2")} orders: reject empty carts${NL}` +
    `${yellow("77b0d13")} pricing: currency from store config${NL}` +
    `${yellow("e09f5aa")} ci: cache pnpm store${NL}` +
    `${green("ako@mbp")} ${blue("~/code/orders-api")} ${grey("(main)")} $ exit${NL}`,
};

const workingTicks = [
  `${grey(" RUN  v2.1.4 ~/code/orders-api")}${NL}`,
  ` ${green("✓")} test/orders/create.test.ts ${grey("(14 tests) 212ms")}${NL}`,
  ` ${green("✓")} test/orders/pricing.test.ts ${grey("(9 tests) 48ms")}${NL}`,
  ` ${green("✓")} test/orders/empty-cart.test.ts ${grey("(2 tests) 11ms")}${NL}`,
  `${NL} ${bold("Test Files")}  ${green("3 passed")} (3)${NL}      ${bold("Tests")}  ${green("25 passed")} (25)${NL}${NL}`,
  `${fg(176, "thinking")}${NL}${grey("Same pattern in pricing.ts; guarding currency lookup too.")}${NL}${NL}`,
  `${yellow("apply_patch")} ${grey("src/orders/pricing.ts")}${NL}${red("-  return items[0].currency;")}${NL}${green("+  return items[0]?.currency ?? store.currency;")}${NL}${NL}`,
  `${yellow("exec")} ${bold("pnpm tsc --noEmit")}${NL}`,
];

// ── Fake diffs ─────────────────────────────────────────────────────────────
const changes: Record<string, FileChange[]> = {
  tests: [
    { path: "src/cart/cart.spec.ts", added: 9, removed: 2, untracked: false, binary: false },
    { path: "src/cart/useCart.ts", added: 3, removed: 1, untracked: false, binary: false },
  ],
  "api-fix": [
    { path: "src/orders/handler.ts", added: 18, removed: 6, untracked: false, binary: false },
    { path: "src/orders/pricing.ts", added: 4, removed: 2, untracked: false, binary: false },
    { path: "src/orders/errors.ts", added: 22, removed: 0, untracked: false, binary: false },
    { path: "test/orders/empty-cart.test.ts", added: 41, removed: 0, untracked: true, binary: false },
    { path: "src/orders/schema.ts", added: 63, removed: 29, untracked: false, binary: false },
  ],
  docs: [
    { path: "docs/onboarding.md", added: 30, removed: 8, untracked: false, binary: false },
    { path: "docs/dev-env/checklist.md", added: 34, removed: 0, untracked: true, binary: false },
    { path: "docs/img/setup-flow.png", added: 0, removed: 0, untracked: true, binary: true },
  ],
  refactor: [],
  scratch: [],
};

function fakeDiff(path: string, untracked: boolean): string {
  if (path.endsWith(".png")) {
    return `diff --git a/${path} b/${path}\nnew file mode 100644\nBinary files /dev/null and b/${path} differ\n`;
  }
  if (untracked) {
    const body = [
      'import { describe, expect, it } from "vitest";',
      'import { buildApp } from "../helpers";',
      "",
      'describe("POST /v2/orders with an empty cart", () => {',
      '  it("returns 422 instead of 500", async () => {',
      "    const app = await buildApp();",
      '    const res = await app.inject({ method: "POST", url: "/v2/orders", payload: { items: [] } });',
      "    expect(res.statusCode).toBe(422);",
      '    expect(res.json()).toEqual({ error: "cart_empty" });',
      "  });",
      "});",
    ];
    return (
      `diff --git a/${path} b/${path}\nnew file mode 100644\n--- /dev/null\n+++ b/${path}\n@@ -0,0 +1,${body.length} @@\n` +
      body.map((l) => "+" + l).join("\n") +
      "\n"
    );
  }
  return `diff --git a/${path} b/${path}
index 3f1a2b9..8c7d0e4 100644
--- a/${path}
+++ b/${path}
@@ -52,12 +52,18 @@ export async function createOrder(req: OrderRequest, reply: Reply) {
   const { items, customerId } = req.body;
   const store = await loadStore(req.storeId);

-  const first = items[0].sku;
-  const currency = items[0].currency;
+  if (items.length === 0) {
+    return reply.code(422).send({ error: "cart_empty" });
+  }
+
+  const first = items[0].sku;
+  const currency = items[0]?.currency ?? store.currency;
   const total = priceItems(items, currency);

-  log.info("order", first);
+  log.info({ sku: first, customerId }, "order.create");
   const order = await db.orders.insert({
     customerId,
     total,
+    currency,
     status: "pending",
   });
@@ -88,7 +94,7 @@ export async function cancelOrder(req: CancelRequest, reply: Reply) {
   const order = await db.orders.get(req.params.id);
   if (!order) return reply.code(404).send();
-  if (order.status !== "pending") throw new Error("cannot cancel");
+  if (order.status !== "pending") return reply.code(409).send({ error: "not_cancellable" });
   await db.orders.update(order.id, { status: "cancelled" });
   return reply.code(204).send();
 }
`;
}

// ── State ──────────────────────────────────────────────────────────────────
function agent(p: Partial<AgentView> & Pick<AgentView, "id" | "name" | "status">): AgentView {
  return {
    kind: "claude",
    kindName: "Claude Code",
    cwd: "/Users/ako/code/orders-api",
    cwdDisplay: "~/code/orders-api",
    project: "/Users/ako/code/orders-api",
    projectDisplay: "orders-api",
    branch: "main",
    worktree: false,
    location: "local",
    statusSource: "screen",
    statusDetail: null,
    running: true,
    added: 0,
    removed: 0,
    filesChanged: 0,
    queue: [],
    autoSend: true,
    lastSent: null,
    lastSentAt: null,
    createdAt: now - 60 * min,
    cols: 100,
    rows: 30,
    ...p,
  };
}

function totals(id: string) {
  const c = changes[id] ?? [];
  return {
    added: c.reduce((s, f) => s + f.added, 0),
    removed: c.reduce((s, f) => s + f.removed, 0),
    filesChanged: c.length,
  };
}

let agents: AgentView[] = [
  agent({
    id: "a-tests",
    name: "tests",
    status: "blocked",
    statusSource: "hooks",
    statusDetail: "Bash: pnpm test --run src/cart",
    cwd: "/Users/ako/code/checkout-web/.claude/worktrees/tests",
    cwdDisplay: "~/code/checkout-web/.claude/worktrees/tests",
    project: "/Users/ako/code/checkout-web",
    projectDisplay: "checkout-web",
    branch: "worktree-tests",
    worktree: true,
    lastSent: "Run the test suite and fix the flaky cart spec. Don't touch the snapshot files.",
    lastSentAt: now - 4 * min,
    createdAt: now - 50 * min,
    ...totals("tests"),
  }),
  agent({
    id: "a-api-fix",
    name: "api-fix",
    kind: "codex",
    kindName: "Codex",
    status: "working",
    statusSource: "screen",
    branch: "codex/api-fix",
    worktree: true,
    cwd: "/Users/ako/.codex/worktrees/a1b2/orders-api",
    cwdDisplay: "~/.codex/worktrees/a1b2/orders-api",
    queue: [
      { id: "q1", text: "Now add the same guard to PATCH /v2/orders/:id and cover it with a test." },
      { id: "q2", text: "Summarise the change in 3 bullets for the PR description." },
    ],
    lastSent: "Fix the 500 on /v2/orders when the cart is empty. Add a regression test.",
    lastSentAt: now - 11 * min,
    createdAt: now - 40 * min,
    ...totals("api-fix"),
  }),
  agent({
    id: "a-docs",
    name: "docs",
    status: "done",
    statusSource: "hooks",
    cwd: "/Users/ako/code/handbook/.claude/worktrees/docs",
    cwdDisplay: "~/code/handbook/.claude/worktrees/docs",
    project: "/Users/ako/code/handbook",
    projectDisplay: "handbook",
    branch: "worktree-docs",
    worktree: true,
    lastSent: "Rewrite the onboarding page so a new hire can get a dev env running in 15 minutes.",
    lastSentAt: now - 26 * min,
    createdAt: now - 30 * min,
    ...totals("docs"),
  }),
  agent({
    id: "a-refactor",
    name: "refactor",
    status: "idle",
    statusSource: "screen",
    createdAt: now - 20 * min,
  }),
  agent({
    id: "a-scratch",
    name: "scratch",
    kind: "shell",
    kindName: "Shell",
    status: "stopped",
    statusSource: "activity",
    running: false,
    createdAt: now - 10 * min,
  }),
];

const screenFor = (a: AgentView) => screens[a.name] ?? "";
const outBuf = new Map<string, string>(agents.map((a) => [a.id, screenFor(a)]));
const outSubs = new Map<string, Set<(b: Uint8Array) => void>>();
const agentSubs = new Set<(a: AgentView[]) => void>();
const attnSubs = new Set<(e: AttentionEvent) => void>();

function emitAgents() {
  const snap = agents.map((a) => ({ ...a, queue: [...a.queue] }));
  agentSubs.forEach((cb) => cb(snap));
}
function out(id: string, s: string) {
  outBuf.set(id, (outBuf.get(id) ?? "") + s);
  const bytes = enc.encode(s);
  outSubs.get(id)?.forEach((cb) => cb(bytes));
}
function find(id: string): AgentView {
  const a = agents.find((x) => x.id === id);
  if (!a) throw `no agent with id ${id}`;
  return a;
}
function update(id: string, patch: Partial<AgentView>): AgentView {
  agents = agents.map((a) => (a.id === id ? { ...a, ...patch } : a));
  emitAgents();
  return find(id);
}
function setStatus(id: string, status: Status, detail: string | null = null) {
  const a = find(id);
  update(id, { status, statusDetail: detail });
  if (status === "blocked" || status === "done") {
    const e: AttentionEvent = { agentId: id, name: a.name, reason: status };
    if (detail) e.detail = detail;
    attnSubs.forEach((cb) => cb(e));
  }
  if ((status === "done" || status === "idle") && a.autoSend) {
    setTimeout(() => {
      const cur = find(id);
      if ((cur.status === "done" || cur.status === "idle") && cur.autoSend && cur.queue.length) {
        deliver(id, cur.queue[0].text);
        update(id, { queue: cur.queue.slice(1) });
      }
    }, 900);
  }
}
const delay = <T>(v: T, ms = 60) => new Promise<T>((r) => setTimeout(() => r(v), ms));

function deliver(id: string, text: string) {
  const a = find(id);
  out(id, `${NL}${grey(">")} ${text}${NL}${NL}`);
  update(id, { lastSent: text, lastSentAt: Date.now() });
  setStatus(id, "working");
  if (a.kind !== "shell") {
    setTimeout(() => out(id, `${green("⏺")} On it.${NL}`), 700);
    setTimeout(() => {
      out(id, `${green("⏺")} Finished.${NL}${NL}`);
      if (find(id).status === "working") setStatus(id, "done");
    }, 6000);
  }
}

// Keep api-fix visibly busy, and finish it once so the attention toast shows.
let tick = 0;
setInterval(() => {
  const a = agents.find((x) => x.id === "a-api-fix");
  if (!a || a.status !== "working" || tick >= workingTicks.length) return;
  out(a.id, workingTicks[tick++]);
  if (tick === workingTicks.length) {
    setTimeout(() => {
      out(a.id, `${NL}${bold("codex")}${NL}Empty carts now return 422 with ${blue("cart_empty")}. Added a regression test.${NL}${NL}`);
      setStatus(a.id, "done");
    }, 2500);
  }
}, 2200);

// ── Api ────────────────────────────────────────────────────────────────────
const kinds: KindView[] = [
  { id: "claude", name: "Claude Code", installed: true, path: "/opt/homebrew/bin/claude", worktree: true },
  { id: "codex", name: "Codex", installed: true, path: "/opt/homebrew/bin/codex", worktree: true },
  { id: "aider", name: "Aider", installed: false },
  { id: "shell", name: "Shell", installed: true, path: "/bin/zsh" },
];
const recents: RecentProject[] = [
  { path: "/Users/ako/code/orders-api", display: "~/code/orders-api", lastUsed: now - 10 * min },
  { path: "/Users/ako/code/checkout-web", display: "~/code/checkout-web", lastUsed: now - 50 * min },
  { path: "/Users/ako/code/handbook", display: "~/code/handbook", lastUsed: now - 3 * 60 * min },
];
let hooks: HooksStatus = { installed: false, path: "~/.codex/hooks.json" };
let seq = 0;

export function createMockApi(): Api {
  const mock: Api = {
    isMock: true,
    ...createReviewMock({ find, update, changes: (id) => (changes[find(id).name] ??= []) }),
    listKinds: () => delay(kinds),
    recentProjects: () => delay(recents),
    listAgents: () => delay(agents),
    async createAgent(req) {
      await delay(null, 300);
      if (agents.some((a) => a.name === req.name)) throw `an agent named "${req.name}" already exists`;
      const kind = kinds.find((k) => k.id === req.kind);
      const proj = recents.find((r) => r.path === req.projectPath);
      const project = (!req.worktree && req.displayProject) || req.projectPath;
      const projectDisplay = project.split("/").filter(Boolean).pop() ?? project;
      // The agent makes its own worktree; it "appears" a moment after launch.
      const cwd = req.projectPath;
      const a = agent({
        id: `a-new-${++seq}`,
        name: req.name,
        status: "working",
        statusSource: "activity",
        kind: req.kind,
        kindName: kind?.name ?? (req.kind === "custom" ? "Custom" : req.kind),
        cwd,
        cwdDisplay: cwd.replace("/Users/ako", "~"),
        project,
        projectDisplay,
        branch: "main",
        worktree: false,
        worktreePending: req.worktree,
        createdAt: Date.now(),
        ...(req.cols && req.rows ? { cols: req.cols, rows: req.rows } : {}),
      });
      agents = [...agents, a];
      outBuf.set(a.id, "");
      changes[a.name] = [];
      if (!proj) recents.unshift({ path: req.projectPath, display: req.projectPath, lastUsed: Date.now() });
      emitAgents();
      setTimeout(() => {
        out(a.id, req.kind === "claude" ? claudeBanner(a.cwdDisplay) : `${bold(req.customCommand ?? a.kindName)} ${grey("· " + a.cwdDisplay)}${NL}${NL}`);
        out(a.id, `${grey("─".repeat(60))}${NL}${grey(">")} `);
      }, 400);
      setTimeout(() => setStatus(a.id, "idle"), 1800);
      if (req.worktree) {
        const wt = req.kind === "codex" ? `/Users/ako/.codex/worktrees/c0de/${projectDisplay}` : `${project}/.claude/worktrees/${req.name}`;
        setTimeout(() => {
          update(a.id, {
            cwd: wt,
            cwdDisplay: wt.replace("/Users/ako", "~"),
            branch: req.kind === "codex" ? null : `worktree-${req.name}`,
            worktree: true,
            worktreePending: false,
          });
        }, 1200);
      }
      return a;
    },
    async attachOutput(agentId, onData) {
      if (!find(agentId).running) throw "agent is not running";
      let set = outSubs.get(agentId);
      if (!set) outSubs.set(agentId, (set = new Set()));
      const buf = outBuf.get(agentId) ?? "";
      // Replay in a few chunks, like a real PTY buffer would arrive.
      const bytes = enc.encode(buf);
      for (let i = 0; i < bytes.length; i += 4096) onData(bytes.slice(i, i + 4096));
      set.add(onData);
      return () => set!.delete(onData);
    },
    async writeInput(agentId, data) {
      const a = find(agentId);
      if (!a.running) return;
      // Local echo stands in for the PTY.
      const echo = data.replace(/\r/g, NL).replace(/\x7f/g, "\b \b");
      out(agentId, echo);
      if (a.status === "blocked" && /[1-3\r]/.test(data)) {
        out(agentId, `${NL}${green("⏺")} ${bold("Bash")}(pnpm test --run src/cart)${NL}  ${grey("⎿")}  ${green("✓ 31 passed")}${NL}${NL}`);
        setStatus(agentId, "working");
        setTimeout(() => setStatus(agentId, "done"), 5000);
      }
    },
    async resize(agentId, cols, rows) {
      const a = agents.find((x) => x.id === agentId);
      if (a && (a.cols !== cols || a.rows !== rows)) update(agentId, { cols, rows });
    },
    async sendPrompt(agentId, text) {
      deliver(agentId, text);
    },
    async queueAdd(agentId, text) {
      const a = find(agentId);
      return update(agentId, { queue: [...a.queue, { id: `q-${++seq}`, text }] });
    },
    async queueRemove(agentId, itemId) {
      const a = find(agentId);
      return update(agentId, { queue: a.queue.filter((q) => q.id !== itemId) });
    },
    async queueSendNow(agentId, itemId) {
      const a = find(agentId);
      const item = a.queue.find((q) => q.id === itemId);
      if (!item) throw "queue item not found";
      update(agentId, { queue: a.queue.filter((q) => q.id !== itemId) });
      deliver(agentId, item.text);
      return find(agentId);
    },
    async setAutoSend(agentId, enabled) {
      return update(agentId, { autoSend: enabled });
    },
    async markSeen(agentId) {
      const a = find(agentId);
      if (a.status === "done") update(agentId, { status: "idle", statusDetail: null });
    },
    getChanges: (agentId) => delay(changes[find(agentId).name] ?? [], 120),
    getFileDiff: (_agentId, path, untracked) => delay(fakeDiff(path, untracked), 120),
    async stopAgent(agentId) {
      out(agentId, `${NL}${grey("[process exited with code 0]")}${NL}`);
      update(agentId, { running: false, status: "exited", statusDetail: null });
    },
    async restartAgent(agentId, size) {
      await delay(null, 250);
      if (size) update(agentId, { cols: size.cols, rows: size.rows });
      const a = find(agentId);
      outBuf.set(agentId, "");
      setTimeout(() => {
        out(agentId, a.kind === "shell" ? `${green("ako@mbp")} ${blue(a.cwdDisplay)} $ ` : claudeBanner(a.cwdDisplay) + grey("Resumed session.") + NL);
      }, 300);
      return update(agentId, { running: true, status: "idle", statusSource: "screen" });
    },
    async removeAgent(agentId) {
      agents = agents.filter((a) => a.id !== agentId);
      outBuf.delete(agentId);
      outSubs.delete(agentId);
      emitAgents();
    },
    codexHooksStatus: () => delay(hooks),
    async installCodexHooks() {
      await delay(null, 400);
      hooks = { ...hooks, installed: true };
      return hooks;
    },
    async getUiState() {
      try {
        const s = localStorage.getItem("pitwall.mock.ui");
        return s ? JSON.parse(s) : null;
      } catch {
        return null;
      }
    },
    async setUiState(state) {
      try {
        localStorage.setItem("pitwall.mock.ui", JSON.stringify(state));
      } catch {
        /* ignore */
      }
    },
    async openWindow(spaceId) {
      const label = `pitwall-${++seq}`;
      console.info(`[mock] open_window(${spaceId}) → ${label} (no real window in the browser mock)`);
      return label;
    },
    async focusWindow(label) {
      console.info(`[mock] focus_window(${label})`);
    },
    listWindows: () => delay(["main"]),
    windowLabel: () => "main",
    async onUiStateChanged(): Promise<Unlisten> {
      return () => {};
    },
    async onWindowClosed(): Promise<Unlisten> {
      return () => {};
    },
    async onOpenSettings(): Promise<Unlisten> {
      return () => {}; // no native menu in the browser; ⌘, still works
    },
    async onAgentsChanged(cb): Promise<Unlisten> {
      agentSubs.add(cb);
      return () => agentSubs.delete(cb);
    },
    async onAttention(cb): Promise<Unlisten> {
      attnSubs.add(cb);
      return () => attnSubs.delete(cb);
    },
    ...createOnboardingMock((req) => mock.createAgent(req)),
  };
  return mock;
}

