#!/usr/bin/env node
// __NAME__'s adapter. It speaks Pitwall's extension adapter protocol,
// version 2: JSON lines over stdin (from Pitwall) and stdout (to Pitwall).
// Its stderr shows in Settings → Extensions → Log while you develop.
// No dependencies; Node 18 or later. Pitwall starts it while the extension
// is on, in this folder, and stops it by closing its stdin.
//
// It draws a section in the right panel for the focused agent, "about"
// (the "agents.read" permission: a section is about one agent). Its items
// carry buttons Pitwall runs itself, each behind its own permission the
// user grants or refuses: open a file in the agent's file viewer ("show")
// and focus an agent's tile ("agents.focus"). Without the grant the button
// is drawn disabled. Pitwall checks the grant when the button is pressed,
// the same as for a call this adapter makes.

import { createInterface } from "node:readline";

const NAME = __NAME_JSON__;
const PANEL = "about";

const send = (msg) => process.stdout.write(JSON.stringify(msg) + "\n");
const log = (text) => process.stderr.write(`${text}\n`);

// Calls: each has an id of ours; Pitwall answers with a reply carrying it.
let nextCall = 1;
const waiting = new Map();
function call(method, params = {}) {
  const id = `c${nextCall++}`;
  send({ type: "call", id, method, params });
  return new Promise((resolve, reject) => waiting.set(id, { resolve, reject }));
}

// The agents whose section is shown (Pitwall says which with show/hide).
const shown = new Set();

async function section(agentId) {
  let agent;
  try {
    agent = (await call("agent.list")).find((a) => a.id === agentId);
  } catch (e) {
    log(`${NAME}: can't read the agents: ${e.message}`);
  }
  if (!shown.has(agentId)) return;
  const items = agent
    ? [
        {
          id: "readme",
          title: "README.md",
          subtitle: agent.cwd || "its folder",
          icon: "file",
          agent: { agentId },
          // Pitwall opens it in this agent's file viewer (needs "show").
          actions: [{ id: "open", label: "Open", open: { file: "README.md", line: 1, agentId }, primary: true }],
        },
        {
          id: "agent",
          title: agent.name,
          subtitle: agent.status,
          icon: "terminal",
          agent: { agentId },
          // Pitwall focuses its tile while it is in front (needs "agents.focus").
          actions: [{ id: "focus", label: "Focus", focus: agentId }],
        },
      ]
    : [];
  send({ type: "view", view: PANEL, agentId, root: { kind: "list", empty: "Nothing about this agent", sections: [{ items }] } });
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
    case "show":
      if (msg.view === PANEL && msg.agentId) {
        shown.add(msg.agentId);
        section(msg.agentId);
      }
      return;
    case "hide":
      if (msg.view === PANEL) shown.delete(msg.agentId);
      return;
    case "grants":
      // A permission changed: draw each shown section again.
      for (const id of shown) section(id);
      return;
    default:
      // A request it doesn't do (Pitwall runs "open" and "focus" itself):
      // say so.
      if (msg.req !== undefined) send({ type: "result", req: msg.req, ok: false, message: `${NAME} doesn't do “${msg.action || msg.type}”.` });
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
    if (!(msg.protocols || []).includes(2) || !(msg.offers || []).includes("panels")) {
      send({ type: "hello", protocol: 1 });
      send({ type: "status", state: "error", message: "This extension needs a Pitwall that draws right-panel sections." });
      return;
    }
    send({ type: "hello", protocol: 2, uses: ["panels", "call"] });
    log(`${NAME}: started for Pitwall ${msg.pitwall}, granted: ${(msg.grants || []).join(", ") || "nothing"}`);
    send({ type: "status", state: "ready" });
    return;
  }
  handle(msg);
});
lines.on("close", () => process.exit(0));
