#!/usr/bin/env node
// __NAME__'s adapter. It speaks Pitwall's extension adapter protocol,
// version 2: JSON lines over stdin (from Pitwall) and stdout (to Pitwall).
// Its stderr shows in Settings → Extensions → Log while you develop.
// No dependencies; Node 18 or later. Pitwall starts it while the extension
// is on, in this folder, and stops it by closing its stdin.
//
// It reads your agents (the "agents.read" permission, asked for in
// extension.json) and shows them in one inbox thread, kept up to date by
// the agents.changed event. Without the permission Pitwall refuses the
// call, and the thread says so.

import { createInterface } from "node:readline";

const NAME = __NAME_JSON__;

const send = (msg) => process.stdout.write(JSON.stringify(msg) + "\n");
const log = (text) => process.stderr.write(`${text}\n`);
const now = () => new Date().toISOString();

// Calls: each has an id of ours; Pitwall answers with a reply carrying it.
let nextCall = 1;
const waiting = new Map();
function call(method, params = {}) {
  const id = `c${nextCall++}`;
  send({ type: "call", id, method, params });
  return new Promise((resolve, reject) => waiting.set(id, { resolve, reject }));
}

let agents = null;
let refused = null;
let time = now();

const describe = () => {
  if (refused) return `Pitwall refused to list your agents: ${refused}`;
  if (agents === null) return "Reading your agents…";
  if (agents.length === 0) return "No agents yet.";
  return agents.map((a) => `- **${a.name}** (${a.kindName}): ${a.status}`).join("\n");
};
const thread = () => ({
  id: "agents",
  title: "Your agents",
  unread: 0,
  canReply: false,
  messages: [{ id: "1", author: { kind: "person", name: NAME }, time, text: describe() }],
});
const summary = () => ({ id: "agents", title: "Your agents", unread: 0, openAsks: 0, blocking: false, lastActivity: time });
let open = false;
const show = () => {
  time = now();
  send({ type: "threads", threads: [summary()] });
  if (open) send({ type: "thread", thread: thread() });
};

async function start() {
  try {
    agents = await call("agent.list");
    refused = null;
    await call("events.subscribe", { topics: ["agents"] });
  } catch (e) {
    refused = e.message;
  }
  show();
}

function handle(msg) {
  switch (msg.type) {
    case "reply": {
      const w = waiting.get(msg.id);
      if (!w) return;
      waiting.delete(msg.id);
      if (msg.error) w.reject(new Error(msg.error.message));
      else w.resolve(msg.result);
      return;
    }
    case "event":
      if (msg.event === "agents.changed") {
        agents = msg.data;
        show();
      }
      return;
    case "grants":
      // The user granted or revoked a permission: read again.
      start();
      return;
    case "open":
      open = msg.thread === "agents";
      if (open) send({ type: "thread", thread: thread() });
      return;
    case "close":
      open = false;
      return;
    case "refresh":
      start();
      return;
    default:
      // A request it doesn't do (answer, message, archive): say so.
      if (msg.req !== undefined) send({ type: "result", req: msg.req, ok: false, message: `${NAME} doesn't do “${msg.type}”.` });
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
    // Pitwall offers the protocols it speaks; answer with the one used.
    if (!(msg.protocols || []).includes(2)) {
      send({ type: "hello", protocol: 1 });
      send({ type: "status", state: "error", message: "This extension needs a Pitwall that speaks protocol 2." });
      return;
    }
    send({ type: "hello", protocol: 2, uses: ["inbox", "call", "events"] });
    log(`${NAME}: started for Pitwall ${msg.pitwall}, granted: ${(msg.grants || []).join(", ") || "nothing"}`);
    send({ type: "status", state: "ready" });
    start();
    return;
  }
  handle(msg);
});
lines.on("close", () => process.exit(0));
