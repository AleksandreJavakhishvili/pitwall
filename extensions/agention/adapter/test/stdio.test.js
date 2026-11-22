// The adapter end to end: main.mjs spawned as Pitwall runs it (`node
// adapter/main.mjs` in the extension's folder), driven over stdin, its
// stdout read as protocol lines, against the fake server.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { after, before, describe, test } from "node:test";
import { fileURLToPath } from "node:url";
import { FakeAgention, KEYS } from "./fake-agention.js";

const extensionDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const everything = { stdout: "", stderr: "" }; // every run's output, checked for keys at the end

function startAdapter() {
  const child = spawn(process.execPath, ["adapter/main.mjs"], { cwd: extensionDir, stdio: ["pipe", "pipe", "pipe"] });
  const lines = [];
  const waiting = [];
  let buffered = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    everything.stdout += chunk;
    buffered += chunk;
    let at;
    while ((at = buffered.indexOf("\n")) >= 0) {
      const line = buffered.slice(0, at);
      buffered = buffered.slice(at + 1);
      const msg = JSON.parse(line); // every stdout line must be JSON
      lines.push(msg);
      for (const w of [...waiting]) w.check();
    }
  });
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
    everything.stderr += chunk;
  });
  const exited = new Promise((resolve) => child.on("exit", (code) => resolve(code)));
  let read = 0;
  return {
    child,
    lines,
    exited,
    stderr: () => stderr,
    send: (msg) => child.stdin.write(`${typeof msg === "string" ? msg : JSON.stringify(msg)}\n`),
    // The next line (after those already taken) matching pred.
    next(pred, what = "a line", ms = 5000) {
      return new Promise((resolve, reject) => {
        const w = {
          check: () => {
            for (let i = read; i < lines.length; i += 1) {
              if (pred(lines[i])) {
                read = i + 1;
                clearTimeout(w.timer);
                waiting.splice(waiting.indexOf(w), 1);
                resolve(lines[i]);
                return;
              }
            }
          },
        };
        w.timer = setTimeout(() => {
          waiting.splice(waiting.indexOf(w), 1);
          reject(new Error(`timed out waiting for ${what}; got ${JSON.stringify(lines)}; stderr ${stderr}`));
        }, ms);
        waiting.push(w);
        w.check();
      });
    },
    async stop() {
      child.stdin.end();
      return exited;
    },
  };
}

function hello(url, key, extra = {}) {
  const secrets = key === undefined ? {} : { apiKey: key };
  const settings = url === undefined ? {} : { server: url };
  return { type: "hello", protocol: 1, pitwall: "0.7.0", settings, secrets, ...extra };
}

const is = (type, more = () => true) => (m) => m.type === type && more(m);

describe("the adapter over stdio", () => {
  let fake;
  before(async () => {
    fake = await new FakeAgention().start();
  });
  after(async () => {
    await fake.close();
    // The key never reaches stdout or stderr, in any run.
    for (const key of Object.values(KEYS)) {
      assert.ok(!everything.stdout.includes(key), "a key in stdout");
      assert.ok(!everything.stderr.includes(key), "a key in stderr");
    }
    assert.ok(!/agention_[A-Za-z0-9_-]{8,}/.test(everything.stdout + everything.stderr));
  });

  test("lists, opens, answers, writes, hears news, marks read, archives, and exits with stdin", async () => {
    const id = fake.addThread({
      title: "Which store?",
      asks: [{ question: "Which store?", suggestions: [{ label: "Redis" }, { label: "SQLite", description: "a file" }] }],
    });
    const a = startAdapter();
    a.send("not json");
    a.send({ type: "open", thread: id }); // before hello: ignored
    a.send(hello(fake.url, KEYS.full));
    assert.deepEqual(await a.next(is("hello"), "hello"), { type: "hello", protocol: 1 });
    assert.equal((await a.next(is("status"))).state, "connecting");
    assert.equal((await a.next(is("status"))).state, "ready");
    const list = await a.next(is("threads"), "the list");
    assert.deepEqual(list.threads, [
      {
        id: "my-app/s1/1",
        title: "Which store?",
        from: { agent: "my-app", session: "s1" },
        unread: true,
        openAsks: 1,
        lastActivity: list.threads[0].lastActivity,
      },
    ]);

    a.send({ type: "open", thread: id });
    const opened = await a.next(is("thread"), "the thread");
    assert.equal(opened.thread.id, id);
    assert.equal(opened.thread.canReply, true);
    assert.deepEqual(opened.thread.messages[0].asks[0].suggestions[1], { label: "SQLite", description: "a file" });
    assert.equal(opened.thread.messages[0].asks[0].open, true);

    a.send({ type: "answer", req: 1, thread: id, ask: 1, picks: [1], text: "in the data dir" });
    assert.deepEqual(await a.next(is("result", (m) => m.req === 1)), { type: "result", req: 1, ok: true });
    const answered = await a.next(is("thread"), "the thread after the answer");
    const ask = answered.thread.messages[0].asks[0];
    assert.equal(ask.open, false);
    assert.deepEqual(ask.answer, { declined: false, picks: [1], text: "in the data dir" });
    assert.equal(answered.thread.messages[1].author.kind, "you");

    a.send({ type: "answer", req: 2, thread: id, ask: 1, declined: true });
    const again = await a.next(is("result", (m) => m.req === 2));
    assert.equal(again.ok, false);
    assert.match(again.message, /refused/);

    a.send({ type: "message", req: 3, thread: id, text: "Thanks!" });
    assert.equal((await a.next(is("result", (m) => m.req === 3))).ok, true);
    assert.equal(fake.threads.get(id).messages.at(-1).text, "Thanks!");

    // News from the agent arrives through the wait.
    fake.agentPost(id, { text: "Done, using SQLite." });
    const news = await a.next(is("threads", (m) => m.threads[0]?.unread === true), "the list with news");
    assert.equal(news.threads[0].id, id);
    const fresh = await a.next(is("thread", (m) => m.thread.messages.at(-1).text === "Done, using SQLite."), "the open thread, current");
    assert.equal(fresh.thread.unread, true);

    a.send({ type: "markRead", req: 4, thread: id });
    assert.equal((await a.next(is("result", (m) => m.req === 4))).ok, true);
    await a.next(is("threads", (m) => m.threads[0]?.unread === false), "the list, read");

    a.send({ type: "refresh" });
    await a.next(is("threads"), "the list again");

    a.send({ type: "archive", req: 5, thread: id });
    assert.equal((await a.next(is("result", (m) => m.req === 5))).ok, true);
    await a.next(is("threads", (m) => m.threads.length === 0), "the list without it");

    a.send({ type: "snooze", req: 6, thread: id });
    assert.equal((await a.next(is("result", (m) => m.req === 6))).ok, false);
    a.send({ type: "markRead", req: 7, thread: "../../etc" });
    assert.equal((await a.next(is("result", (m) => m.req === 7))).ok, false);
    a.send({ type: "close", thread: id });

    const started = Date.now();
    assert.equal(await a.stop(), 0);
    assert.ok(Date.now() - started < 3000, "exits soon after stdin closes");
  });

  test("a revoked key is signed out, in plain words", async () => {
    const a = startAdapter();
    a.send(hello(fake.url, KEYS.revoked));
    await a.next(is("hello"));
    const s = await a.next(is("status", (m) => m.state !== "connecting"));
    assert.equal(s.state, "signedOut");
    assert.match(s.message, /revoked or expired/);
    a.send({ type: "markRead", req: 1, thread: "my-app/s1/1" });
    assert.equal((await a.next(is("result"))).ok, false);
    assert.equal(await a.stop(), 0);
  });

  test("a key without threads:write is signed out, naming the scope", async () => {
    const a = startAdapter();
    a.send(hello(fake.url, KEYS.readOnly));
    const s = await a.next(is("status", (m) => m.state !== "connecting"));
    assert.equal(s.state, "signedOut");
    assert.match(s.message, /threads:write/);
    await a.stop();
  });

  test("no key, no server, a key of the wrong form", async () => {
    for (const [h, state, words] of [
      [hello(fake.url, undefined), "signedOut", /No Agention API key/],
      [hello(undefined, KEYS.full), "error", /No Agention server address/],
      [hello(fake.url, "agention_short"), "signedOut", /doesn't look like/],
      [hello("http://agention.example.ts.net", KEYS.full), "error", /https:\/\//],
    ]) {
      const a = startAdapter();
      a.send(h);
      const s = await a.next(is("status"));
      assert.equal(s.state, state);
      assert.match(s.message, words);
      assert.ok(s.message.length <= 500);
      await a.stop();
    }
  });

  test("a server from before the operator API", async () => {
    fake.legacy = true;
    try {
      const a = startAdapter();
      a.send(hello(fake.url, KEYS.full));
      const s = await a.next(is("status", (m) => m.state !== "connecting"));
      assert.equal(s.state, "error");
      assert.match(s.message, /Update Agention/);
      await a.stop();
    } finally {
      fake.legacy = false;
    }
  });

  test("another protocol is refused", async () => {
    const a = startAdapter();
    a.send(hello(fake.url, KEYS.full, { protocol: 2 }));
    await a.next(is("hello"));
    const s = await a.next(is("status"));
    assert.equal(s.state, "error");
    assert.match(s.message, /protocol 1/);
    await a.stop();
  });

  test("a server that can't be reached is retried, and says so", async () => {
    const a = startAdapter();
    a.send(hello("http://127.0.0.1:9", KEYS.full));
    await a.next(is("status", (m) => m.state === "connecting"));
    const s = await a.next(is("status", (m) => m.state === "error"), "the error", 15000);
    assert.match(s.message, /Can't reach the Agention server.*Trying again/);
    await a.stop();
  });
});
