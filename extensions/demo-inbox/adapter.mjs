#!/usr/bin/env node
// A fake adapter that speaks Pitwall's extension adapter protocol, version 1
// (docs/spec/extensions.md, "Adapter protocol"): JSON lines over stdin and
// stdout. It serves made-up threads from memory, answers every request, and
// exits when its stdin closes. No dependencies; Node 18 or later.
//
//   node adapter.mjs                 # as Pitwall runs it
//   node adapter.mjs --empty         # an inbox with nothing in it
//   node adapter.mjs --flood         # misbehaves: floods stdout (tests)
//   node adapter.mjs --crash         # misbehaves: exits after hello (tests)
//   node adapter.mjs --slow          # connects for a long time (the loading state)
//   node adapter.mjs --offline       # can't reach its server (the error state)
//
// DEMO_INBOX_AGENT / DEMO_INBOX_SESSION: the first thread comes from that
// agent (a Pitwall agent's project and id, to see it matched).
//
// Fit a real adapter to the same lines: read `hello` first (settings and
// secrets arrive only there), answer `hello`, send `status`, `threads` and
// `counts`, then serve `open`, `answer`, `message`, `markRead`, `archive`,
// `refresh` and `close`, each request with a `result` carrying its `req`.
//
// It also draws a view tile, "Demo runs" (made-up CI runs), answers the
// palette command "Start a made-up run" and puts a note on the first
// thread's agent's row: `show`, `hide`, `select`, `action` and `command`
// in, `view` and `badges` out (docs/extensions/views.md).

import { createInterface } from "node:readline";

const PROTOCOL = 1;
const send = (msg) => process.stdout.write(JSON.stringify(msg) + "\n");
const now = () => new Date().toISOString();
const ago = (min) => new Date(Date.now() - min * 60_000).toISOString();

const threads = new Map();
let nextMessage = 100;

const agent = (from) => ({ kind: "agent", name: from.agent });
const you = { kind: "you", name: "You" };

function seed(greeting) {
  const t = (id, title, from, unread, messages) => threads.set(id, { id, title, from, unread, archived: false, messages });

  const cache = { agent: process.env.DEMO_INBOX_AGENT || "my-app", session: process.env.DEMO_INBOX_SESSION || "demo-session" };
  t("demo/1", "Which database for the session cache?", cache, 2, [
    {
      id: "1",
      author: agent(cache),
      time: ago(14),
      text: "I'm adding a store for the **session cache** and need you to pick one before I write the migration.",
      detail:
        "## What I looked at\n\nRedis is already deployed for the job queue; SQLite needs no server at all.\n\n| | Redis | SQLite |\n|---|---|---|\n| Shared across instances | yes | no |\n| Extra ops | none (deployed) | none |\n| Latency | ~0.3 ms | ~0.05 ms |\n\n```toml\n[cache]\nbackend = \"redis\"\nttl = \"30m\"\n```\n\n- Redis keeps sessions when the app restarts.\n- SQLite is simpler to test locally." +
        (greeting ? `\n\nYou set: \`${greeting}\`` : ""),
      asks: [
        {
          number: 1,
          question: "Which store should the session cache use?",
          multiple: false,
          open: true,
          blocking: true,
          suggestions: [
            { label: "Redis", description: "Use the deployed instance; sessions survive restarts" },
            { label: "SQLite", description: "A file next to the app; nothing to run" },
            { label: "In memory", description: "Fastest, lost on restart" },
          ],
        },
      ],
    },
    { id: "2", author: agent(cache), time: ago(6), text: "I'll keep working on the API routes meanwhile; the cache layer waits for this.", asks: [] },
  ]);

  const docs = { agent: "docs-site", session: "s2" };
  t("demo/2", "Release notes wording", docs, 1, [
    {
      id: "1",
      author: agent(docs),
      time: ago(55),
      text: "Two quick questions about the 0.7 release notes.",
      asks: [
        { number: 1, question: "Lead with the new inbox, or with the speed work?", multiple: false, open: false, suggestions: [{ label: "Inbox" }, { label: "Speed" }], answer: { declined: false, picks: [0], text: "" } },
        {
          number: 2,
          question: "Which platforms should the download section list?",
          multiple: true,
          open: true,
          suggestions: [{ label: "macOS" }, { label: "Windows" }, { label: "Linux" }],
        },
      ],
    },
    { id: "2", author: you, time: ago(40), text: "", answers: [{ ask: 1, question: "Lead with the new inbox, or with the speed work?", declined: false, labels: ["Inbox"], text: "" }] },
  ]);

  const tests = { agent: "api", session: "fix-flaky" };
  t("demo/3", "Flaky test quarantined", tests, 0, [
    {
      id: "1",
      author: agent(tests),
      time: ago(180),
      text: "`orders::refund_twice` failed 3 times in 50 runs. I quarantined it and opened a fix.",
      asks: [
        { number: 1, question: "Should I also retry it in CI until the fix lands?", multiple: false, open: false, suggestions: [{ label: "Yes" }, { label: "No" }], answer: { declined: false, picks: [1], text: "Keep CI honest." } },
        { number: 2, question: "Disable the whole refunds suite?", multiple: false, open: false, suggestions: [{ label: "Yes" }, { label: "No" }], withdrawn: "Only one test was flaky." },
      ],
    },
    { id: "2", author: you, time: ago(170), text: "Thanks. Ping me when the fix is merged.", answers: [{ ask: 1, question: "Should I also retry it in CI until the fix lands?", declined: false, labels: ["No"], text: "Keep CI honest." }] },
    { id: "3", author: agent(tests), time: ago(120), text: "", withdrawals: [{ ask: 2, reason: "Only one test was flaky." }] },
  ]);

  const deps = { agent: "web-frontend", session: "deps" };
  t("demo/5", "Upgraded React to 19 across the dashboard, the settings pages and the marketing site", deps, 1, [
    {
      id: "1",
      author: agent(deps),
      time: ago(32),
      text: "Upgraded React to **19.1** in all three apps. Two components used the removed `defaultProps` on function components; I moved those to default parameters.",
      detail: "### Changed\n\n- `apps/dashboard`: 14 files\n- `apps/settings`: 6 files\n- `apps/marketing`: 3 files\n\nThe bundle is 4 KB smaller. Visual tests pass.",
      asks: [],
    },
    { id: "2", author: { kind: "person", name: "Maya (reviewer)" }, time: ago(20), text: "Looks good to me. Can you check the date picker on Safari before we merge?", asks: [] },
  ]);

  const infra = { agent: "infra", session: "dns" };
  t("demo/4", "All 412 tests pass", infra, 0, [
    { id: "1", author: agent(infra), time: ago(60 * 26), text: "All 412 tests pass after the DNS change. Nothing needed from you.", asks: [] },
  ]);
}

const openAsks = (t) => t.messages.flatMap((m) => m.asks || []).filter((a) => a.open);

function sendThreads() {
  const live = [...threads.values()].filter((t) => !t.archived);
  const list = live.map((t) => ({
    id: t.id,
    title: t.title,
    from: t.from,
    unread: t.unread,
    openAsks: openAsks(t).length,
    blocking: openAsks(t).some((a) => a.blocking),
    lastActivity: t.messages.at(-1).time,
  }));
  send({ type: "threads", threads: list });
  send({
    type: "counts",
    unread: live.filter((t) => t.unread > 0).length,
    openAsks: live.reduce((n, t) => n + openAsks(t).length, 0),
    blocking: live.reduce((n, t) => n + openAsks(t).filter((a) => a.blocking).length, 0),
  });
}

function sendThread(t) {
  send({ type: "thread", thread: { id: t.id, title: t.title, from: t.from, unread: t.unread, canReply: true, messages: t.messages } });
}

function result(req, ok, message) {
  send(ok ? { type: "result", req, ok } : { type: "result", req, ok, message });
}

// The view tile "Demo runs" (docs/extensions/views.md): made-up CI runs as
// a list, the selected one's detail beside it, a Retry button on a failed
// one; the palette command "Start a made-up run"; a note on the first
// thread's agent's row naming its failing run. Pitwall draws them from
// these lines; the whole view is sent each time.
const runs = [];
let nextRun = 41;
let shown = false;
let selectedRun = null;
let runsFrom = null;

function addRun(state, minutes) {
  const n = nextRun++;
  runs.unshift({ id: `run-${n}`, n, branch: n % 2 ? "main" : "session-cache", state, time: ago(minutes) });
}

function seedRuns(from) {
  runsFrom = from;
  addRun("passed", 95);
  addRun("passed", 50);
  addRun("failed", 12);
  addRun("running", 2);
}

const tone = { passed: "good", failed: "bad", running: "busy" };
const icon = { passed: "check", failed: "alert", running: "refresh" };

function runItem(r) {
  return {
    id: r.id,
    title: `Build #${r.n}`,
    subtitle: `${r.branch} · ${r.state}`,
    tone: tone[r.state],
    icon: icon[r.state],
    time: r.time,
    badges: r.state === "failed" ? [{ text: "failed", tone: "bad" }] : [],
    actions: r.state === "failed" ? [{ id: "retry", label: "Retry", primary: true }] : [],
  };
}

function runDetail() {
  const r = runs.find((x) => x.id === selectedRun);
  if (!r) return { kind: "status", state: "empty", icon: "list", title: "Pick a run", text: "Its steps and log show here." };
  return {
    kind: "detail",
    title: `Build #${r.n}`,
    fields: [
      { label: "Branch", value: r.branch },
      { label: "State", value: r.state },
      { label: "Started", value: new Date(r.time).toLocaleString() },
      { label: "Docs", value: "How views work", link: "https://github.com/AleksandreJavakhishvili/pitwall/blob/main/docs/extensions/views.md" },
    ],
    body:
      r.state === "failed"
        ? "**`npm test` failed** in `session-cache.test.ts`:\n\n```\nexpected 30m, got 0\n```\n\nRetry it, or ask the agent to look."
        : r.state === "running"
          ? "Running `npm test`…"
          : "All 214 tests passed.",
    actions: r.state === "failed" ? [{ id: "retry", label: "Retry" }] : [],
  };
}

function sendView() {
  if (!shown) return;
  const today = runs.filter((r) => Date.now() - Date.parse(r.time) < 60 * 60_000);
  const older = runs.filter((r) => !today.includes(r));
  const sections = [{ title: "Last hour", items: today.map(runItem) }];
  if (older.length) sections.push({ title: "Earlier", items: older.map(runItem) });
  send({ type: "view", view: "runs", root: { kind: "list", empty: "No runs yet", sections, detail: runDetail() } });
}

// A failed run on the agent's row, else that CI is fine.
function sendBadges() {
  if (!runsFrom) return send({ type: "badges", badges: [] });
  const failed = runs.find((r) => r.state === "failed");
  const badge = failed
    ? { note: `#${failed.n} failed`, tone: "bad", tooltip: "A made-up CI run failed", open: { view: "runs", item: failed.id } }
    : { note: "CI ok", tone: "good", tooltip: "The made-up CI runs pass", open: { view: "runs" } };
  send({ type: "badges", badges: [{ agent: { from: runsFrom }, ...badge }] });
}

// The view's lines; `true` when the message was one of them.
function handleView(msg) {
  switch (msg.type) {
    case "show":
      shown = msg.view === "runs" || shown;
      sendView();
      return true;
    case "hide":
      if (msg.view === "runs") shown = false;
      return true;
    case "select":
      selectedRun = msg.item;
      sendView();
      return true;
    case "action": {
      const r = runs.find((x) => x.id === msg.item);
      if (msg.action !== "retry" || !r || r.state !== "failed") {
        result(msg.req, false, "Only a failed run can be retried.");
        return true;
      }
      r.state = "running";
      result(msg.req, true);
      sendView();
      sendBadges();
      setTimeout(() => {
        r.state = "passed";
        sendView();
        sendBadges();
      }, 3000);
      return true;
    }
    case "command":
      if (msg.command !== "new-run") {
        result(msg.req, false, `This adapter has no command “${msg.command}”.`);
        return true;
      }
      addRun("running", 0);
      selectedRun = runs[0].id;
      send({ type: "result", req: msg.req, ok: true, message: `Build #${runs[0].n} started (made up).` });
      sendView();
      return true;
    default:
      return false;
  }
}

function handle(msg) {
  if (handleView(msg)) return;
  const t = msg.thread !== undefined ? threads.get(msg.thread) : undefined;
  const needThread = ["open", "answer", "message", "markRead", "archive"].includes(msg.type);
  if (needThread && !t) {
    if (msg.req !== undefined) result(msg.req, false, "That thread is gone.");
    return;
  }
  switch (msg.type) {
    case "open":
      sendThread(t);
      break;
    case "close":
      break;
    case "refresh":
      sendThreads();
      break;
    case "markRead":
      t.unread = 0;
      result(msg.req, true);
      sendThread(t);
      sendThreads();
      break;
    case "archive":
      t.archived = true;
      result(msg.req, true);
      sendThreads();
      break;
    case "answer": {
      const ask = t.messages.flatMap((m) => m.asks || []).find((a) => a.number === msg.ask);
      if (!ask || !ask.open) return result(msg.req, false, "That question isn't open any more.");
      if (msg.text === "fail") return result(msg.req, false, "The server refused that answer (a test failure).");
      ask.open = false;
      ask.answer = { declined: !!msg.declined, picks: msg.picks || [], text: msg.text || "" };
      const labels = (msg.picks || []).map((p) => ask.suggestions[p]?.label).filter(Boolean);
      t.messages.push({ id: String(nextMessage++), author: you, time: now(), text: "", answers: [{ ask: ask.number, question: ask.question, declined: !!msg.declined, labels, text: msg.text || "" }] });
      t.unread = 0;
      result(msg.req, true);
      sendThread(t);
      sendThreads();
      break;
    }
    case "message":
      t.messages.push({ id: String(nextMessage++), author: you, time: now(), text: msg.text, asks: [] });
      result(msg.req, true);
      sendThread(t);
      sendThreads();
      break;
    default:
      // A request this adapter doesn't know: say so, never guess.
      if (msg.req !== undefined) result(msg.req, false, `This adapter doesn't do “${msg.type}”.`);
  }
}

let greeted = false;
const lines = createInterface({ input: process.stdin });
lines.on("line", (line) => {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }
  if (!greeted) {
    if (msg.type !== "hello") return;
    greeted = true;
    send({ type: "hello", protocol: PROTOCOL });
    if (msg.protocol !== PROTOCOL) {
      send({ type: "status", state: "error", message: `This adapter speaks protocol ${PROTOCOL}; Pitwall asked for ${msg.protocol}.` });
      return;
    }
    if (process.argv.includes("--crash")) process.exit(3);
    if (process.argv.includes("--flood")) {
      const burst = () => {
        for (let i = 0; i < 1000; i++) send({ type: "status", state: "ready" });
        setImmediate(burst);
      };
      return burst();
    }
    send({ type: "status", state: "connecting" });
    if (process.argv.includes("--slow")) return;
    if (process.argv.includes("--offline")) {
      send({ type: "status", state: "error", message: "Can't reach https://inbox.example.com: the connection timed out. Retrying in 30 s." });
      return send({ type: "threads", threads: [] });
    }
    if (!process.argv.includes("--empty")) {
      seed(msg.settings && msg.settings.greeting);
      seedRuns(threads.get("demo/1").from);
    }
    send({ type: "status", state: "ready" });
    sendThreads();
    sendBadges();
    return;
  }
  handle(msg);
});
lines.on("close", () => process.exit(0));
