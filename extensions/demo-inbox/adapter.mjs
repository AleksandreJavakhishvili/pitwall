#!/usr/bin/env node
// A fake adapter that speaks Pitwall's extension adapter protocol, version 1
// (docs/spec/extensions.md, "Adapter protocol"): JSON lines over stdin and
// stdout. It serves made-up threads from memory, answers every request, and
// exits when its stdin closes. No dependencies; Node 18 or later.
//
//   node adapter.mjs                 # as Pitwall runs it
//   node adapter.mjs --flood         # misbehaves: floods stdout (tests)
//   node adapter.mjs --crash         # misbehaves: exits after hello (tests)
//
// Fit a real adapter to the same lines: read `hello` first (settings and
// secrets arrive only there), answer `hello`, send `status` and `threads`,
// then serve `open`, `answer`, `message`, `markRead`, `archive`, `refresh`
// and `close`, each request with a `result` carrying its `req`.

import { createInterface } from "node:readline";

const PROTOCOL = 1;
const send = (msg) => process.stdout.write(JSON.stringify(msg) + "\n");
const now = () => new Date().toISOString();

const threads = new Map();
let nextMessage = 100;

function seed(greeting) {
  const t = (id, title, from, messages) => threads.set(id, { id, title, from, unread: true, archived: false, messages });
  t("demo/1", "Which database for the cache?", { agent: "my-app", session: "demo-session" }, [
    {
      id: "1",
      author: { kind: "agent", name: "my-app/demo-session" },
      time: now(),
      text: "I need a store for the **session cache**. Which one should I use?",
      detail: "Redis is already deployed; SQLite needs no server.\n\n- Redis: shared, fast\n- SQLite: local, simple" + (greeting ? `\n\nYou set: \`${greeting}\`` : ""),
      asks: [
        {
          number: 1,
          question: "Which store?",
          multiple: false,
          open: true,
          suggestions: [
            { label: "Redis", description: "Use the deployed instance" },
            { label: "SQLite", description: "A file next to the app" },
          ],
        },
      ],
    },
  ]);
  t("demo/2", "Tests are green", { agent: "docs", session: "s2" }, [
    { id: "1", author: { kind: "agent", name: "docs/s2" }, time: now(), text: "All 412 tests pass. Nothing needed from you.", asks: [] },
  ]);
}

function openAsks(t) {
  return t.messages.flatMap((m) => m.asks || []).filter((a) => a.open).length;
}

function sendThreads() {
  const list = [...threads.values()]
    .filter((t) => !t.archived)
    .map((t) => ({ id: t.id, title: t.title, from: t.from, unread: t.unread, openAsks: openAsks(t), lastActivity: t.messages.at(-1).time }));
  send({ type: "threads", threads: list });
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
      t.unread = false;
      result(msg.req, true);
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
      ask.open = false;
      ask.answer = { declined: !!msg.declined, picks: msg.picks || [], text: msg.text || "" };
      t.unread = false;
      result(msg.req, true);
      sendThread(t);
      sendThreads();
      break;
    }
    case "message":
      t.messages.push({ id: String(nextMessage++), author: { kind: "you", name: "You" }, time: now(), text: msg.text, asks: [] });
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
    seed(msg.settings && msg.settings.greeting);
    send({ type: "status", state: "ready" });
    sendThreads();
    return;
  }
  handle(msg);
});
lines.on("close", () => process.exit(0));
