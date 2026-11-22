import assert from "node:assert/strict";
import { after, before, beforeEach, describe, test } from "node:test";
import { AgentionClient, backoffMs } from "../lib/client.js";
import { FakeAgention, KEYS } from "./fake-agention.js";

describe("backoff", () => {
  test("doubles to its cap, half of each step random", () => {
    assert.equal(backoffMs(0, { random: () => 0 }), 500);
    assert.equal(backoffMs(0, { random: () => 1 }), 1000);
    assert.equal(backoffMs(3, { random: () => 0 }), 4000);
    assert.equal(backoffMs(20, { random: () => 1 }), 60_000);
    assert.equal(backoffMs(1000, { random: () => 0.5 }), 45_000);
  });
});

describe("the wait loop", () => {
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
    fake.dropNext = 0;
    fake.failNext = [];
    fake.rateLimit = 0;
  });

  // A client whose sleeps are recorded and return at once, with a short
  // server-side wait so a quiet loop turns quickly.
  function client(key = KEYS.full) {
    const slept = [];
    const c = new AgentionClient(
      { url: fake.url, key },
      { sleep: async (ms, signal) => {
        slept.push(ms);
        if (signal?.aborted) throw Object.assign(new Error("cancelled"), { kind: "cancelled" });
      }, random: () => 0, waitSeconds: 1 },
    );
    return { c, slept };
  }

  test("hears of new messages once each, following the cursor", async () => {
    const { c } = client();
    const stop = new AbortController();
    const heard = [];
    const id = fake.addThread({ title: "first" });
    const done = c.waitLoop({
      signal: stop.signal,
      onThreads: (threads) => {
        heard.push(threads.map((t) => `${t.title}:${t.unread}`));
        if (heard.length === 1) setTimeout(() => fake.agentPost(id, { text: "second" }), 20);
        if (heard.length === 2) stop.abort();
      },
    });
    assert.equal(await done, undefined);
    assert.deepEqual(heard, [["first:1"], ["first:2"]]);
    const waits = fake.requests.filter((r) => r.path === "/api/operator/v1/wait");
    assert.equal(new URLSearchParams(waits[0].search).get("since"), null);
    assert.ok(new URLSearchParams(waits[1].search).get("since").startsWith("w"));
    assert.equal(new URLSearchParams(waits[1].search).get("wait"), "1");
  });

  test("reconnects after dropped connections and server errors, backing off", async () => {
    const { c, slept } = client();
    fake.dropNext = 2;
    fake.failNext = [{ status: 503, code: "internal" }, { status: 502, code: "internal" }];
    const states = [];
    const stop = new AbortController();
    fake.addThread();
    await c.waitLoop({
      signal: stop.signal,
      onState: (s) => states.push(s.state + (s.error ? `:${s.error.kind}` : "")),
      onThreads: () => stop.abort(),
    });
    // Drops are checked before failures, so: two drops,
    // then two server errors, then the answer.
    assert.deepEqual(states, [
      "retrying:network",
      "retrying:network",
      "retrying:server",
      "retrying:server",
      "live",
      "stopped",
    ]);
    // random() = 0: half of 1, 2, 4, 8 seconds.
    assert.deepEqual(slept, [500, 1000, 2000, 4000]);
  });

  test("a rate limit waits as Retry-After says", async () => {
    const { c, slept } = client();
    fake.rateLimit = 1;
    fake.retryAfter = "7";
    const stop = new AbortController();
    fake.addThread();
    await c.waitLoop({ signal: stop.signal, onThreads: () => stop.abort() });
    assert.deepEqual(slept, [7000]);
  });

  test("stops at an error a retry can't fix", async () => {
    const { c, slept } = client(KEYS.revoked);
    const states = [];
    const error = await c.waitLoop({ onState: (s) => states.push(s.state) });
    assert.equal(error.kind, "key_rejected");
    assert.deepEqual(states, ["stopped"]);
    assert.deepEqual(slept, []);
  });

  test("cancelling ends a wait the server is holding, at once", async () => {
    const slow = new AgentionClient({ url: fake.url, key: KEYS.full }, { waitSeconds: 20 });
    const stop = new AbortController();
    const started = Date.now();
    setTimeout(() => stop.abort(), 50);
    assert.equal(await slow.waitLoop({ signal: stop.signal }), undefined);
    assert.ok(Date.now() - started < 2000);
  });

  test("cancelling during a backoff ends it", async () => {
    const stop = new AbortController();
    const c = new AgentionClient(
      { url: fake.url, key: KEYS.full },
      {
        random: () => 0,
        // A real wait on the signal, as the default sleep does, aborted here.
        sleep: (ms, signal) =>
          new Promise((_, reject) => {
            signal.addEventListener("abort", () => reject(Object.assign(new Error("x"), { kind: "cancelled" })));
            stop.abort();
          }),
      },
    );
    fake.failNext = [{ status: 503, code: "internal" }];
    assert.equal(await c.waitLoop({ signal: stop.signal }), undefined);
  });
});
