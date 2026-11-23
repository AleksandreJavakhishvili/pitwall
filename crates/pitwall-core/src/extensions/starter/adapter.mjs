#!/usr/bin/env node
// __NAME__'s adapter. It speaks Pitwall's extension adapter protocol,
// version 1: JSON lines over stdin (from Pitwall) and stdout (to Pitwall).
// Its stderr shows in Settings → Extensions → Log while you develop.
// No dependencies; Node 18 or later. Pitwall starts it while the extension
// is on, in this folder, and stops it by closing its stdin.
//
// It shows one thread in the extension's inbox and takes replies to it.
// Replace `threads` with your service's data: fetch it after `hello`,
// send the whole list whenever it changes.

import { createInterface } from "node:readline";

const PROTOCOL = 1;
const NAME = __NAME_JSON__;

const send = (msg) => process.stdout.write(JSON.stringify(msg) + "\n");
const log = (text) => process.stderr.write(`${text}\n`);
const now = () => new Date().toISOString();

const threads = new Map([
  [
    "welcome",
    {
      id: "welcome",
      title: `Hello from ${NAME}`,
      unread: 1,
      messages: [
        {
          id: "1",
          author: { kind: "person", name: NAME },
          time: now(),
          text: "This thread comes from **adapter.mjs**. Reply here: the adapter adds your message and Pitwall shows it.",
        },
      ],
    },
  ],
]);
let nextMessage = 2;

const summary = (t) => ({
  id: t.id,
  title: t.title,
  unread: t.unread,
  openAsks: 0,
  blocking: false,
  lastActivity: t.messages[t.messages.length - 1].time,
});
const sendThreads = () => send({ type: "threads", threads: [...threads.values()].map(summary) });
const sendThread = (t) => send({ type: "thread", thread: { id: t.id, title: t.title, unread: t.unread, canReply: true, messages: t.messages } });
const result = (req, ok, message) => send({ type: "result", req, ok, ...(message ? { message } : {}) });

function handle(msg) {
  const t = msg.thread !== undefined ? threads.get(msg.thread) : undefined;
  if (["open", "answer", "message", "markRead", "archive"].includes(msg.type) && !t) {
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
      sendThreads();
      break;
    case "message":
      t.messages.push({ id: String(nextMessage++), author: { kind: "you", name: "You" }, time: now(), text: msg.text });
      result(msg.req, true);
      sendThread(t);
      sendThreads();
      break;
    default:
      // A request it doesn't do (answer, archive): say so, never guess.
      if (msg.req !== undefined) result(msg.req, false, `${NAME} doesn't do “${msg.type}” yet.`);
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
    // Settings and secrets arrive only here (msg.settings, msg.secrets).
    // Never print a secret: Pitwall's log takes out the ones it knows,
    // but nothing else does.
    log(`${NAME}: started for Pitwall ${msg.pitwall}`);
    send({ type: "status", state: "ready" });
    sendThreads();
    return;
  }
  handle(msg);
});
lines.on("close", () => process.exit(0));
