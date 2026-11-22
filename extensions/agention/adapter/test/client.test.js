import assert from "node:assert/strict";
import { createServer } from "node:net";
import { after, before, beforeEach, describe, test } from "node:test";
import { AgentionClient, checkSettings, parseRetryAfter } from "../lib/client.js";
import { classifyFetchError } from "../lib/errors.js";
import { FakeAgention, KEYS } from "./fake-agention.js";

const noSleep = async () => {};

function client(fake, key = KEYS.full, options = {}) {
  return new AgentionClient({ url: fake.url, key }, { sleep: noSleep, random: () => 0.5, ...options });
}

async function rejects(promise, kind, check = () => {}) {
  await assert.rejects(promise, (error) => {
    assert.equal(error.kind, kind, `${error.kind}: ${error.message}`);
    assert.equal(typeof error.message, "string");
    check(error);
    return true;
  });
}

describe("settings", () => {
  test("no key, a key of the wrong form, and addresses", () => {
    assert.throws(() => checkSettings({ url: "https://a.example" }), { kind: "no_key" });
    assert.throws(() => checkSettings({ url: "https://a.example", key: "  " }), { kind: "no_key" });
    assert.throws(() => checkSettings({ url: "https://a.example", key: "sk-123" }), { kind: "no_key" });
    assert.throws(() => checkSettings({ key: KEYS.full }), { kind: "bad_url" });
    assert.throws(() => checkSettings({ url: "not a url", key: KEYS.full }), { kind: "bad_url" });
    assert.throws(() => checkSettings({ url: "ftp://a.example", key: KEYS.full }), { kind: "bad_url" });
    assert.throws(() => checkSettings({ url: "https://u:p@a.example", key: KEYS.full }), { kind: "bad_url" });
    assert.throws(() => checkSettings({ url: "http://agention.example.ts.net", key: KEYS.full }), {
      kind: "insecure_url",
    });
    assert.equal(checkSettings({ url: "https://a.example/", key: KEYS.full }).base, "https://a.example");
    assert.equal(checkSettings({ url: "http://127.0.0.1:8080", key: KEYS.full }).base, "http://127.0.0.1:8080");
  });

  test("the key isn't an enumerable field of the client", () => {
    const c = new AgentionClient({ url: "https://a.example", key: KEYS.full });
    assert.ok(!JSON.stringify(c).includes(KEYS.full));
    assert.ok(!Object.keys(c).includes("key"));
  });
});

describe("against the fake server", () => {
  let fake;
  before(async () => {
    fake = await new FakeAgention().start();
  });
  after(async () => {
    await fake.close();
  });
  beforeEach(() => {
    fake.threads.clear();
    fake.requests.length = 0;
    fake.failNext = [];
    fake.rateLimit = 0;
    fake.delayMs = 0;
    fake.dropNext = 0;
    fake.legacy = false;
    fake.notAgention = false;
    fake.redirect = false;
  });

  test("connect checks the version without the key, then the scopes", async () => {
    const out = await client(fake).connect();
    assert.deepEqual(out.versions, [1]);
    assert.equal(fake.requests[0].path, "/api/operator");
    assert.equal(fake.requests[0].authorization, null, "the key isn't sent before the server is known");
    assert.ok(fake.requests.slice(1).every((r) => r.authorization === `Bearer ${KEYS.full}`));
    // The probes change nothing.
    assert.equal(fake.threads.size, 0);
  });

  test("lists threads across pages, and counts", async () => {
    for (let i = 0; i < 7; i += 1) fake.addThread({ session: `s${i}`, title: `T${i}`, asks: [{ question: "Q?" }] });
    const c = client(fake);
    const list = await c.listThreads({ pageSize: 3 });
    assert.equal(list.threads.length, 7);
    assert.equal(list.total, 7);
    assert.equal(list.truncated, false);
    assert.equal(fake.requests.filter((r) => r.path === "/api/operator/v1/threads").length, 3);
    assert.deepEqual(list.threads[0].from, { agent: "my-app", session: "s6" });
    assert.equal(list.threads[0].id, "my-app/s6/1");
    assert.equal(list.threads[0].openAsks, 1);
    const capped = await c.listThreads({ pageSize: 3, max: 4 });
    assert.equal(capped.threads.length, 4);
    assert.equal(capped.truncated, true);
    const counts = await c.counts();
    assert.deepEqual(counts, { threads: 7, unread: 7, open: 7, blocking: 0, overdue: 0 });
  });

  test("reads a thread page by page", async () => {
    const id = fake.addThread({ text: "m1" });
    for (let i = 2; i <= 12; i += 1) fake.agentPost(id, { text: `m${i}` });
    const read = await client(fake).readThread(id, { pageSize: 5 });
    assert.equal(read.messages.length, 12);
    assert.deepEqual(read.messages.map((m) => m.text).slice(-2), ["m11", "m12"]);
    assert.equal(read.thread.lastSeq, 12);
    const latest = await client(fake).readThread(id, { pageSize: 5, after: 9 });
    assert.deepEqual(latest.messages.map((m) => m.seq), [10, 11, 12]);
    await rejects(client(fake).readThread("my-app/s1/99"), "not_found");
    await rejects(client(fake).readThread("../etc/1"), "invalid_input");
  });

  test("answers an ask with picks, own words, or a decline", async () => {
    const id = fake.addThread({
      asks: [
        { question: "Which store?", suggestions: [{ label: "Redis" }, { label: "SQLite" }] },
        { question: "Which tests?", multiple: true, suggestions: [{ label: "unit" }, { label: "e2e" }] },
        { question: "Ship it?" },
      ],
    });
    const c = client(fake);
    let seen = (await c.readThread(id)).thread.lastSeq;
    const w1 = await c.answer(id, [{ ask: 1, picks: [1] }], { seenThrough: seen });
    seen = w1.seq;
    const w2 = await c.answer(id, [{ ask: 2, picks: [0, 1], text: "and lint" }], { seenThrough: seen });
    seen = w2.seq;
    await c.answer(id, [{ ask: 3, declined: true }], { seenThrough: seen });
    const read = await c.readThread(id);
    const asks = read.messages[0].asks;
    assert.deepEqual(asks.map((a) => a.state), ["answered", "answered", "declined"]);
    assert.deepEqual(asks[0].answer.picks, [1]);
    assert.equal(asks[1].answer.text, "and lint");
    assert.equal(read.messages[1].author.kind, "operator");
    assert.equal(read.messages[1].answers[0].labels[0], "SQLite");
    // Refused before sending.
    await rejects(c.answer(id, [{ ask: 1, declined: true, text: "no" }], { seenThrough: 4 }), "invalid_input");
    await rejects(c.answer(id, [{ ask: 1, picks: [10] }], { seenThrough: 4 }), "invalid_input");
    await rejects(c.answer(id, [{ ask: 1 }], { seenThrough: 4 }), "invalid_input");
    await rejects(c.answer(id, [{ ask: 1, picks: [0] }], {}), "invalid_input");
    // Refused by the server: the ask isn't open.
    await rejects(c.answer(id, [{ ask: 1, picks: [0] }], { seenThrough: 4 }), "invalid_input");
  });

  test("a stale answer carries what was missed", async () => {
    const id = fake.addThread({ asks: [{ question: "Q?" }] });
    fake.agentPost(id, { text: "Actually, one more thing" });
    await rejects(client(fake).answer(id, [{ ask: 1, text: "yes" }], { seenThrough: 1 }), "stale", (e) => {
      assert.equal(e.missed.length, 1);
      assert.equal(e.missed[0].text, "Actually, one more thing");
    });
  });

  test("writes a message, marks read, archives", async () => {
    const id = fake.addThread();
    const c = client(fake);
    const w = await c.sendMessage(id, { text: "On it", detail: "## Plan\n- a" }, { seenThrough: 1 });
    assert.equal(w.seq, 2);
    await rejects(c.sendMessage(id, { text: "   " }, { seenThrough: 2 }), "invalid_input");
    await rejects(c.sendMessage(id, { text: "x".repeat(1001) }, { seenThrough: 2 }), "invalid_input");
    fake.agentPost(id, { text: "news" });
    assert.equal((await c.listThreads()).threads[0].unread, 1);
    await c.markRead(id, 3);
    assert.equal((await c.listThreads()).threads[0].unread, 0);
    await c.archive(id, 3);
    assert.equal((await c.listThreads()).threads.length, 0);
  });

  test("a retried write keeps its request id", async () => {
    const id = fake.addThread();
    fake.failNext = [{ status: 503, code: "internal" }];
    await client(fake).sendMessage(id, { text: "hi" }, { seenThrough: 1, requestId: "r-1" });
    const posts = fake.requests.filter((r) => r.method === "POST");
    assert.equal(posts.length, 2);
    assert.equal(JSON.parse(posts[0].body).request_id, "r-1");
    assert.equal(JSON.parse(posts[1].body).request_id, "r-1");
    assert.equal(fake.threads.get(id).messages.length, 2);
  });

  test("lists sessions", async () => {
    fake.addThread({ agent: "a", session: "s1" });
    fake.addThread({ agent: "b", session: "s2", asks: [{ question: "Q?" }] });
    const { sessions } = await client(fake).listSessions();
    assert.equal(sessions.length, 2);
    assert.deepEqual(
      sessions.map((s) => [s.agent, s.session, s.openAsks]),
      [
        ["a", "s1", 0],
        ["b", "s2", 1],
      ],
    );
  });

  test("the key is rejected (401)", async () => {
    await rejects(client(fake, KEYS.revoked).connect(), "key_rejected", (e) => {
      assert.equal(e.status, 401);
      assert.ok(!e.message.includes(KEYS.revoked));
    });
  });

  test("a missing scope is named (not_found as the API answers it)", async () => {
    await rejects(client(fake, KEYS.readOnly).connect(), "missing_scope", (e) => {
      assert.equal(e.scope, "threads:write");
      assert.match(e.message, /threads:write/);
    });
    await rejects(client(fake, KEYS.noSessions).connect(), "missing_scope", (e) => {
      assert.equal(e.scope, "sessions:read");
    });
  });

  test("a missing scope is named (403)", async () => {
    fake.failNext = [{ status: 403, code: "not_allowed" }];
    await rejects(client(fake).listSessions(), "missing_scope", (e) => {
      assert.equal(e.scope, "sessions:read");
      assert.equal(e.status, 403);
    });
  });

  test("a server from before the operator API is too old", async () => {
    fake.legacy = true;
    await rejects(client(fake).connect(), "too_old", (e) => assert.match(e.message, /API keys/));
  });

  test("another web server isn't Agention", async () => {
    fake.notAgention = true;
    await rejects(client(fake).connect(), "not_agention");
    fake.notAgention = false;
    fake.redirect = true;
    await rejects(client(fake).connect(), "not_agention");
  });

  test("a version this extension doesn't speak", async () => {
    const c = client(fake);
    c.fetch = async () => new Response(JSON.stringify({ versions: [2] }), { headers: { "content-type": "application/json" } });
    await rejects(c.checkVersion(), "unsupported_version", (e) => assert.match(e.message, /version 2/));
  });

  test("rate limited: waits as Retry-After says, then gives up", async () => {
    const slept = [];
    const c = client(fake, KEYS.full, { sleep: async (ms) => slept.push(ms) });
    fake.rateLimit = 1;
    fake.retryAfter = "3";
    await c.counts();
    assert.deepEqual(slept, [3000]);
    fake.rateLimit = 10;
    await rejects(c.counts(), "rate_limited", (e) => assert.equal(e.retryAfterMs, 3000));
    fake.rateLimit = 0;
    fake.failNext = [{ status: 429, code: "rate_limited", headers: { "retry-after": "120" } }];
    await rejects(c.counts(), "rate_limited", (e) => assert.equal(e.retryAfterMs, 120_000));
  });

  test("slow answers time out", async () => {
    fake.delayMs = 300;
    await rejects(client(fake, KEYS.full, { timeoutMs: 50, maxRetries: 0 }).counts(), "timeout");
    // and a retry gets through once the server is quick again
    const c = client(fake, KEYS.full, { timeoutMs: 100, maxRetries: 1 });
    c.sleep = async () => {
      fake.delayMs = 0;
    };
    await c.counts();
  });

  test("dropped connections are retried, then reported as the network", async () => {
    fake.dropNext = 1;
    await client(fake).counts();
    fake.dropNext = 5;
    await rejects(client(fake).counts(), "network");
  });

  test("an answer too large or of the wrong shape is refused", async () => {
    const c = client(fake);
    c.fetch = async () => new Response("x".repeat(9 * 1024 * 1024), { headers: { "content-type": "application/json" } });
    await rejects(c.counts(), "bad_response");
    c.fetch = async () =>
      new Response(JSON.stringify({ threads: [{ agent: "../x" }], total: 1 }), { headers: { "content-type": "application/json" } });
    await rejects(c.listThreads(), "bad_response");
  });

  test("cancelling stops a request", async () => {
    fake.delayMs = 1000;
    const stop = new AbortController();
    const p = client(fake).counts({ signal: stop.signal });
    setTimeout(() => stop.abort(), 20);
    await rejects(p, "cancelled");
  });
});

describe("the network and TLS", () => {
  test("nothing listening is the network", async () => {
    const s = createServer();
    await new Promise((r) => s.listen(0, "127.0.0.1", r));
    const port = s.address().port;
    await new Promise((r) => s.close(r));
    const c = new AgentionClient({ url: `http://127.0.0.1:${port}`, key: KEYS.full }, { sleep: noSleep, maxRetries: 0 });
    await rejects(c.connect(), "network", (e) => assert.equal(e.detail, "ECONNREFUSED"));
  });

  test("https to a server that doesn't speak TLS is a TLS failure", async () => {
    const fake = await new FakeAgention().start();
    try {
      const c = new AgentionClient({ url: `https://127.0.0.1:${fake.port}`, key: KEYS.full }, { sleep: noSleep, maxRetries: 0 });
      await rejects(c.connect(), "tls");
    } finally {
      await fake.close();
    }
  });

  test("certificate failures are TLS failures", () => {
    for (const code of ["DEPTH_ZERO_SELF_SIGNED_CERT", "CERT_HAS_EXPIRED", "ERR_TLS_CERT_ALTNAME_INVALID", "UNABLE_TO_VERIFY_LEAF_SIGNATURE"]) {
      const error = new TypeError("fetch failed", { cause: Object.assign(new Error("x"), { code }) });
      assert.equal(classifyFetchError(error).kind, "tls", code);
    }
    const dns = new TypeError("fetch failed", { cause: Object.assign(new Error("x"), { code: "ENOTFOUND" }) });
    assert.equal(classifyFetchError(dns).kind, "network");
  });

  test("Retry-After as seconds or a date, bounded", () => {
    assert.equal(parseRetryAfter("2"), 2000);
    assert.equal(parseRetryAfter("0"), 1000);
    assert.equal(parseRetryAfter("999999"), 3_600_000);
    assert.equal(parseRetryAfter(new Date(10_000 + 30_000).toUTCString(), 10_000), 30_000);
    assert.equal(parseRetryAfter(undefined), 5000);
  });
});
