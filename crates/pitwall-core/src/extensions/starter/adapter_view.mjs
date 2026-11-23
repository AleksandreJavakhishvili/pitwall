#!/usr/bin/env node
// __NAME__'s adapter. It speaks Pitwall's extension adapter protocol,
// version 2: JSON lines over stdin (from Pitwall) and stdout (to Pitwall).
// Its stderr shows in Settings → Extensions → Log while you develop.
// No dependencies; Node 18 or later. Pitwall starts it while the extension
// is on, in this folder, and stops it by closing its stdin.
//
// It draws one view tile, "items" (a list, with the selected item's detail
// beside it), answers a palette command, "refresh", and puts a short note
// on your agents' rows in the sidebar (the "agents.read" permission, asked
// for in extension.json). Pitwall draws all of it from the data sent here:
// the whole view each time, never a part of it.

import { createInterface } from "node:readline";

const NAME = __NAME_JSON__;
const VIEW = "items";

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

// What the view shows: replace these with your service's data.
const items = [
  { id: "welcome", title: `Welcome to ${NAME}`, subtitle: "Select me: my detail shows beside the list", done: false, time: new Date().toISOString() },
  { id: "components", title: "A list, a detail, Markdown and a status", subtitle: "Pitwall draws them from the JSON this adapter sends", done: false, time: new Date().toISOString() },
];
let shown = false;
let selected = null;
let refreshed = 0;

const item = (i) => ({
  id: i.id,
  title: i.title,
  subtitle: i.subtitle,
  tone: i.done ? "good" : "none",
  icon: i.done ? "check" : "list",
  time: i.time,
  badges: i.done ? [{ text: "done", tone: "good" }] : [],
  actions: i.done ? [] : [{ id: "done", label: "Mark done", primary: true }],
});

const detail = () => {
  const i = items.find((x) => x.id === selected);
  if (!i) return { kind: "markdown", text: "Select an item to see it here." };
  return {
    kind: "detail",
    title: i.title,
    fields: [
      { label: "Id", value: i.id },
      { label: "State", value: i.done ? "done" : "to do" },
      { label: "Docs", value: "Components", link: "https://github.com/AleksandreJavakhishvili/pitwall/blob/main/docs/extensions/views.md" },
    ],
    body: `**${i.subtitle}.**\n\nRefreshed ${refreshed} time(s) from the command palette.`,
    actions: i.done ? [] : [{ id: "done", label: "Mark done" }],
  };
};

const view = () => {
  if (!shown) return;
  const open = items.filter((i) => !i.done).map(item);
  const done = items.filter((i) => i.done).map(item);
  const sections = [{ title: "To do", items: open }];
  if (done.length) sections.push({ title: "Done", items: done });
  send({ type: "view", view: VIEW, root: { kind: "list", empty: "Nothing here", sections, detail: detail() } });
};

// A note on each agent's row (agents by Pitwall's id need agents.read).
async function badges() {
  try {
    const agents = await call("agent.list");
    send({ type: "badges", badges: agents.slice(0, 20).map((a) => ({ agent: { agentId: a.id }, note: "hello", tooltip: `${NAME} sees ${a.name}`, open: { view: VIEW } })) });
  } catch (e) {
    log(`${NAME}: no badges: ${e.message}`);
    send({ type: "badges", badges: [] });
  }
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
    case "grants":
      // The user granted or revoked a permission: badge again.
      badges();
      return;
    case "show":
      if (msg.view === VIEW) {
        shown = true;
        view();
      }
      return;
    case "hide":
      if (msg.view === VIEW) shown = false;
      return;
    case "select":
      selected = msg.item;
      view();
      return;
    case "action": {
      const i = items.find((x) => x.id === msg.item);
      if (msg.action === "done" && i) {
        i.done = true;
        send({ type: "result", req: msg.req, ok: true });
        view();
      } else {
        send({ type: "result", req: msg.req, ok: false, message: `${NAME} can't “${msg.action}” that.` });
      }
      return;
    }
    case "command":
      if (msg.command === "refresh") {
        refreshed += 1;
        send({ type: "result", req: msg.req, ok: true, message: "Refreshed." });
        view();
        badges();
      } else {
        send({ type: "result", req: msg.req, ok: false, message: `${NAME} has no command “${msg.command}”.` });
      }
      return;
    default:
      // A request it doesn't do: say so.
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
    if (!(msg.protocols || []).includes(2) || !(msg.offers || []).includes("views")) {
      send({ type: "hello", protocol: 1 });
      send({ type: "status", state: "error", message: "This extension needs a Pitwall that draws views." });
      return;
    }
    send({ type: "hello", protocol: 2, uses: ["views", "commands", "badges", "call"] });
    log(`${NAME}: started for Pitwall ${msg.pitwall}, granted: ${(msg.grants || []).join(", ") || "nothing"}`);
    send({ type: "status", state: "ready" });
    badges();
    return;
  }
  handle(msg);
});
lines.on("close", () => process.exit(0));
