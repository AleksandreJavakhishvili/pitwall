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

function handle(msg) {
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
    if (!process.argv.includes("--empty")) seed(msg.settings && msg.settings.greeting);
    send({ type: "status", state: "ready" });
    sendThreads();
    return;
  }
  handle(msg);
});
lines.on("close", () => process.exit(0));
