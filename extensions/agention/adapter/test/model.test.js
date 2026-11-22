import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { BadData } from "../lib/bounds.js";
import { message, replyToMessage, threadDetail, threadList } from "../lib/model.js";
import {
  decodeCounts,
  decodeError,
  decodeMessage,
  decodeThread,
  decodeThreadPage,
  decodeThreadRead,
  encodeAnswer,
  encodeAnswers,
  parseThreadId,
  threadId,
} from "../lib/operator-v1.js";

// Wire shapes as the OpenAPI document gives them.
function wireThread(over = {}) {
  return {
    agent: "my-app",
    session: "s1",
    number: 3,
    title: "Which store?",
    created_at: "2026-11-20T10:00:00.000Z",
    updated_at: "2026-11-20T11:00:00.000Z",
    last_seq: 4,
    read_through: 1,
    unread: 2,
    message_count: 4,
    archived: false,
    open_asks: { asks: [{ number: 1, seq: 1 }], count: 1, blocking: true, overdue: false },
    status: "open",
    credential_active: true,
    ...over,
  };
}

function wireMessage(over = {}) {
  return {
    seq: 1,
    author: { kind: "session", agent: "my-app", session: "s1" },
    created_at: "2026-11-20T10:00:00Z",
    live: true,
    text: "I need a **store**.",
    detail: "Redis or SQLite",
    asks: [
      {
        number: 1,
        question: "Which store?",
        suggestions: [
          { label: "Redis", description: "deployed" },
          { label: "SQLite", description: "" },
        ],
        multiple: false,
        impact: "blocking",
        priority: "high",
        open: true,
      },
    ],
    ...over,
  };
}

describe("threads", () => {
  test("a thread into the model and Pitwall's list", () => {
    const t = decodeThread(wireThread());
    assert.deepEqual(threadList([t]), [
      {
        id: "my-app/s1/3",
        title: "Which store?",
        from: { agent: "my-app", session: "s1" },
        unread: true,
        openAsks: 1,
        lastActivity: "2026-11-20T11:00:00.000Z",
      },
    ]);
    assert.equal(t.lastSeq, 4);
    assert.equal(t.blocking, true);
  });

  test("archived threads leave the list; newest first", () => {
    const a = decodeThread(wireThread({ number: 1, updated_at: "2026-11-20T09:00:00Z" }));
    const b = decodeThread(wireThread({ number: 2, updated_at: "2026-11-20T12:00:00Z" }));
    const c = decodeThread(wireThread({ number: 4, archived: true }));
    assert.deepEqual(threadList([a, b, c]).map((t) => t.id), ["my-app/s1/2", "my-app/s1/1"]);
  });

  test("ids round-trip, and only well-formed ones parse", () => {
    assert.deepEqual(parseThreadId(threadId({ agent: "a", session: "b-c", number: 12 })), { agent: "a", session: "b-c", number: 12 });
    for (const bad of ["a/b", "a/b/c", "../b/1", "A/b/1", "a/b/1/2", "a/b/-1", 7, "a/b/1e3"]) {
      assert.equal(parseThreadId(bad), null, String(bad));
    }
  });

  test("names that could change a request path are refused", () => {
    for (const agent of ["../x", "a/b", "A", "", "a%2f", "x".repeat(64)]) {
      assert.throws(() => decodeThread(wireThread({ agent })), BadData, agent);
    }
  });

  test("wrong types are refused, long titles cut, controls removed", () => {
    assert.throws(() => decodeThread(wireThread({ unread: -1 })), BadData);
    assert.throws(() => decodeThread(wireThread({ unread: "2" })), BadData);
    assert.throws(() => decodeThread(wireThread({ number: 2 ** 60 })), BadData);
    assert.throws(() => decodeThread(wireThread({ updated_at: "yesterday" })), BadData);
    assert.throws(() => decodeThread(wireThread({ open_asks: null })), BadData);
    assert.throws(() => decodeThread("x"), BadData);
    const t = decodeThread(wireThread({ title: `a‮b\u0007\nc${"x".repeat(1000)}` }));
    assert.ok(t.title.startsWith("ab c"));
    assert.ok(t.title.length <= 501);
    assert.equal(threadList([t])[0].title.length, 300);
    // A status a later server adds reads as none.
    assert.equal(decodeThread(wireThread({ status: "snoozed" })).status, "none");
  });

  test("pages are bounded", () => {
    assert.throws(() => decodeThreadPage({ threads: Array(101).fill(wireThread()), total: 101 }), BadData);
    assert.throws(() => decodeThreadPage({ threads: [], total: 0, next: "x".repeat(1025) }), BadData);
    assert.equal(decodeThreadPage({ threads: [], total: 0, next: "abc" }).next, "abc");
  });

  test("counts", () => {
    assert.deepEqual(
      decodeCounts({
        status: { any: 5, open: 2, answered: 1, withdrawn: 0, none: 2 },
        unread: { any: 5, unread: 3 },
        impact: { any: 5, blocking: 1, assuming: 0 },
        overdue: { any: 5, overdue: 0 },
      }),
      { threads: 5, unread: 3, open: 2, blocking: 1, overdue: 0 },
    );
    assert.throws(() => decodeCounts({ status: {} }), BadData);
  });
});

describe("messages", () => {
  test("an agent's message with an open ask", () => {
    const m = message(decodeMessage(wireMessage()));
    assert.deepEqual(m, {
      id: "1",
      author: { kind: "agent", name: "my-app" },
      time: "2026-11-20T10:00:00.000Z",
      text: "I need a **store**.",
      detail: "Redis or SQLite",
      asks: [
        {
          number: 1,
          question: "Which store?",
          multiple: false,
          open: true,
          suggestions: [
            { label: "Redis", description: "deployed" },
            { label: "SQLite", description: "" },
          ],
        },
      ],
    });
  });

  test("answered, declined, withdrawn and replaced asks", () => {
    const ask = wireMessage().asks[0];
    const answered = decodeMessage(
      wireMessage({ asks: [{ ...ask, open: false, answer: { seq: 2, declined: false, picks: [1, 7], labels: ["SQLite"], text: "a file" } }] }),
    );
    assert.equal(answered.asks[0].state, "answered");
    assert.deepEqual(message(answered).asks[0].answer, { declined: false, picks: [1], text: "a file" });
    const declined = decodeMessage(wireMessage({ asks: [{ ...ask, open: false, answer: { seq: 2, declined: true } }] }));
    assert.equal(declined.asks[0].state, "declined");
    assert.deepEqual(message(declined).asks[0].answer, { declined: true, picks: [], text: "" });
    const withdrawn = decodeMessage(wireMessage({ asks: [{ ...ask, open: false, withdrawal: { seq: 3, reason: "settled" } }] }));
    assert.equal(message(withdrawn).asks[0].withdrawn, "settled");
    assert.equal(message(withdrawn).asks[0].open, false);
    const closed = decodeMessage(wireMessage({ asks: [{ ...ask, open: false }] }));
    assert.equal(closed.asks[0].state, "closed");
    assert.equal(decodeMessage(wireMessage({ live: false })), null, "a replaced message is left out");
  });

  test("the operator's messages are the user's; a bare answer is said in words", () => {
    const m = decodeMessage({
      seq: 2,
      author: { kind: "person", name: "Ada" },
      created_at: "2026-11-20T10:05:00Z",
      live: true,
      answers: [{ ask: 1, question: "Which store?", ask_seq: 1, ask_live: true, declined: false, picks: [1], labels: ["SQLite"] }],
    });
    const out = message(m);
    assert.deepEqual(out.author, { kind: "you", name: "Ada" });
    assert.equal(out.text, "- Answered 1 (Which store?): SQLite");
  });

  test("lists, sizes and authors are bounded", () => {
    const ask = wireMessage().asks[0];
    assert.throws(() => decodeMessage(wireMessage({ asks: Array(21).fill(ask) })), BadData);
    assert.throws(() => decodeMessage(wireMessage({ asks: [{ ...ask, suggestions: Array(21).fill({ label: "x", description: "" }) }] })), BadData);
    assert.throws(() => decodeMessage(wireMessage({ author: { kind: "robot" } })), BadData);
    assert.throws(() => decodeMessage(wireMessage({ seq: 1.5 })), BadData);
    const big = decodeMessage(wireMessage({ text: "t".repeat(5000), detail: "d".repeat(50_000) }));
    assert.equal(big.text.length, 2001);
    assert.equal(big.detail.length, 40_001);
    assert.equal(message(big).detail.length, 40_000);
    // Ten asks of eleven reach Pitwall.
    const many = decodeMessage(wireMessage({ asks: Array(11).fill(ask) }));
    assert.equal(message(many).asks.length, 10);
  });

  test("a thread read keeps the latest 200 messages for Pitwall", () => {
    const read = decodeThreadRead({
      thread: wireThread({ last_seq: 3 }),
      messages: [wireMessage({ seq: 1 }), wireMessage({ seq: 2, live: false }), wireMessage({ seq: 3, asks: [] })],
      cursors: { requester: 3, audience: 1 },
      marks: { read_through: 1, archived_through: 0 },
    });
    assert.equal(read.messages.length, 2);
    assert.equal(read.pageEnd, 3);
    const many = { thread: read.thread, messages: Array.from({ length: 250 }, (_, i) => decodeMessage(wireMessage({ seq: i + 1 }))) };
    const detail = threadDetail(many);
    assert.equal(detail.messages.length, 200);
    assert.equal(detail.messages[0].id, "51");
    assert.equal(detail.canReply, true);
  });

  test("error bodies are read leniently", () => {
    assert.equal(decodeError("<html>"), null);
    assert.equal(decodeError({ error: "x" }), null);
    assert.deepEqual(decodeError({ error: { code: "stale", message: "m", missed: [wireMessage()] } }).missed.length, 1);
    assert.equal(decodeError({ error: { code: "stale", message: "m", missed: [{ bad: 1 }] } }).missed.length, 0);
    assert.equal(decodeError({ error: { code: "x", message: "y".repeat(5000) } }).message.length, 301);
  });
});

describe("what is sent", () => {
  test("answers", () => {
    assert.deepEqual(encodeAnswer({ ask: 1, picks: [0, 2], text: " also " }), { ask: 1, picks: [0, 2], text: "also" });
    assert.deepEqual(encodeAnswer({ ask: 2, declined: true }), { ask: 2, declined: true });
    assert.deepEqual(encodeAnswer({ ask: 3, text: "my words" }), { ask: 3, text: "my words" });
    for (const bad of [
      { ask: 0, text: "x" },
      { ask: 1 },
      { ask: 1, declined: true, picks: [0] },
      { ask: 1, picks: [10] },
      { ask: 1, picks: [0, 0] },
      { ask: 1, text: "x".repeat(4001) },
      { ask: 1, picks: Array.from({ length: 11 }, (_, i) => i) },
    ]) {
      assert.throws(() => encodeAnswer(bad), TypeError, JSON.stringify(bad));
    }
    assert.throws(() => encodeAnswers("r", 1, [{ ask: 1, text: "a" }, { ask: 1, text: "b" }]), TypeError);
    assert.deepEqual(encodeAnswers("r", 4, [{ ask: 1, text: "a" }]), { request_id: "r", seen_through: 4, answers: [{ ask: 1, text: "a" }] });
  });

  test("a long reply keeps a summary and goes whole into detail", () => {
    assert.deepEqual(replyToMessage("short"), { text: "short" });
    const long = `First paragraph here.\n\n${"word ".repeat(400)}`;
    assert.deepEqual(replyToMessage(long), { text: "First paragraph here.", detail: long.trim() });
    const one = "word ".repeat(400).trim();
    const r = replyToMessage(one);
    assert.ok(r.text.length <= 1000);
    assert.ok(r.text.endsWith("…"));
    assert.equal(r.detail, one);
  });
});
