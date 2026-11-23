// A fake Agention server for tests, written from the operator API's OpenAPI
// document (docs/api/openapi.json in Agention's repository):
// in-memory threads, messages and asks; bearer keys with scopes; the wait
// long poll; and switches that make it misbehave. It listens on 127.0.0.1,
// on a port the system picks, and nowhere else.

import { createServer } from "node:http";

export const KEYS = {
  full: `agention_${"A".repeat(42)}A`,
  readOnly: `agention_${"B".repeat(42)}A`,
  noSessions: `agention_${"C".repeat(42)}A`,
  revoked: `agention_${"D".repeat(42)}A`,
};

const INCLUDED = {
  "threads:read": [],
  "threads:write": ["threads:read"],
  "sessions:read": [],
  "sessions:write": ["sessions:read"],
  "sessions:approve": ["sessions:read"],
};

function holds(scopes, needed) {
  return scopes.some((s) => s === needed || INCLUDED[s]?.includes(needed));
}

const NAME = /^[a-z0-9][a-z0-9_-]{0,62}$/;

class Refusal extends Error {
  constructor(status, code, message, extra = {}) {
    super(message);
    this.status = status;
    this.body = { error: { code, message, ...extra } };
  }
}

export class FakeAgention {
  constructor() {
    this.keys = new Map([
      [KEYS.full, ["threads:write", "sessions:read"]],
      [KEYS.readOnly, ["threads:read", "sessions:read"]],
      [KEYS.noSessions, ["threads:write"]],
    ]);
    this.threads = new Map();
    this.clock = Date.parse("2026-11-20T10:00:00Z");
    this.counter = 0; // a global position for the wait
    this.waiters = new Set();
    this.requests = [];
    this.writes = new Map(); // request_id -> result
    // Switches.
    this.failNext = []; // [{status, code, message, headers}] answered in turn
    this.rateLimit = 0; // answer this many requests rate_limited
    this.retryAfter = "2";
    this.delayMs = 0; // hold each answer this long
    this.dropNext = 0; // drop this many connections without answering
    this.legacy = false; // an Agention from before the operator API
    this.notAgention = false; // some other web server
    this.redirect = false;
  }

  async start() {
    this.server = createServer((req, res) => this.serve(req, res));
    this.sockets = new Set();
    this.server.on("connection", (s) => {
      this.sockets.add(s);
      s.on("close", () => this.sockets.delete(s));
    });
    await new Promise((resolve) => this.server.listen(0, "127.0.0.1", resolve));
    this.port = this.server.address().port;
    this.url = `http://127.0.0.1:${this.port}`;
    return this;
  }

  async close() {
    for (const w of this.waiters) w.finish();
    for (const s of this.sockets) s.destroy();
    await new Promise((resolve) => this.server.close(resolve));
  }

  now() {
    this.clock += 1000;
    return new Date(this.clock).toISOString();
  }

  // ---- Data ----

  addThread({ agent = "my-app", session = "s1", title = "A question", text = "Which one?", detail, asks = [] } = {}) {
    const number = [...this.threads.values()].filter((t) => t.agent === agent && t.session === session).length + 1;
    const created = this.now();
    const thread = {
      agent,
      session,
      number,
      title,
      created_at: created,
      updated_at: created,
      messages: [],
      read_through: 0,
      archived_through: 0,
      position: 0,
      nextAsk: 1,
    };
    this.threads.set(`${agent}/${session}/${number}`, thread);
    this.agentPost(`${agent}/${session}/${number}`, { text, detail, asks });
    return `${agent}/${session}/${number}`;
  }

  // The agent writes in a thread; wakes waits.
  agentPost(id, { text, detail, asks = [], withdrawals } = {}) {
    const t = this.threads.get(id);
    const message = {
      author: { kind: "session", agent: t.agent, session: t.session },
      text,
      detail,
      asks: asks.map((a) => ({
        number: t.nextAsk++,
        question: a.question,
        suggestions: (a.suggestions ?? []).map((s) => ({ label: s.label, description: s.description ?? "" })),
        multiple: a.multiple ?? false,
        impact: a.impact ?? "none",
        priority: a.priority ?? "normal",
      })),
    };
    if (withdrawals) {
      message.withdrawals = withdrawals;
      for (const w of withdrawals) {
        const ask = this.askRef(t, w.ask);
        if (ask) ask.withdrawal = { reason: w.reason };
      }
    }
    return this.append(t, message);
  }

  append(t, message) {
    const seq = t.messages.length + 1;
    const at = this.now();
    for (const a of message.asks ?? []) a.seq = seq;
    t.messages.push({ seq, created_at: at, live: true, ...message });
    t.updated_at = at;
    this.counter += 1;
    t.position = this.counter;
    for (const w of [...this.waiters]) w.check();
    return seq;
  }

  askRef(t, number) {
    for (const m of t.messages) for (const a of m.asks ?? []) if (a.number === number) return a;
    return null;
  }

  // ---- Shapes from the OpenAPI document ----

  isOpen(a) {
    return !a.answer && !a.withdrawal;
  }

  lastSeq(t) {
    return t.messages.length;
  }

  unread(t) {
    const through = Math.max(t.read_through, t.archived_through);
    return t.messages.filter((m) => m.seq > through && m.author.kind === "session").length;
  }

  summary(t) {
    const open = t.messages.flatMap((m) => (m.asks ?? []).filter((a) => this.isOpen(a)));
    const all = t.messages.flatMap((m) => m.asks ?? []);
    let status = "none";
    if (open.length > 0) status = "open";
    else if (all.length > 0) status = all.every((a) => a.withdrawal) ? "withdrawn" : "answered";
    return {
      agent: t.agent,
      session: t.session,
      number: t.number,
      title: t.title,
      created_at: t.created_at,
      updated_at: t.updated_at,
      last_seq: this.lastSeq(t),
      read_through: t.read_through,
      unread: this.unread(t),
      message_count: t.messages.length,
      archived: t.archived_through >= this.lastSeq(t),
      open_asks: {
        asks: open.map((a) => ({ number: a.number, seq: a.seq })),
        count: open.length,
        blocking: open.some((a) => a.impact === "blocking"),
        overdue: false,
      },
      status,
      credential_active: true,
    };
  }

  askRead(a) {
    const out = {
      number: a.number,
      question: a.question,
      suggestions: a.suggestions,
      multiple: a.multiple,
      impact: a.impact,
      priority: a.priority,
      open: this.isOpen(a),
    };
    if (a.answer) out.answer = a.answer;
    if (a.withdrawal) out.withdrawal = { seq: 0, reason: a.withdrawal.reason };
    return out;
  }

  messageRead(m) {
    const out = { seq: m.seq, author: m.author, created_at: m.created_at, live: m.live };
    if (m.text !== undefined) out.text = m.text;
    if (m.detail !== undefined) out.detail = m.detail;
    if (m.asks?.length) out.asks = m.asks.map((a) => this.askRead(a));
    if (m.answers) out.answers = m.answers;
    if (m.withdrawals) out.withdrawals = m.withdrawals;
    return out;
  }

  // ---- HTTP ----

  async serve(req, res) {
    const chunks = [];
    for await (const c of req) chunks.push(c);
    const raw = Buffer.concat(chunks).toString("utf8");
    const url = new URL(req.url, this.url);
    this.requests.push({ method: req.method, path: url.pathname, search: url.search, authorization: req.headers.authorization ?? null, body: raw });

    if (this.dropNext > 0) {
      this.dropNext -= 1;
      req.socket.destroy();
      return;
    }
    if (this.delayMs > 0) await new Promise((r) => setTimeout(r, this.delayMs));
    if (this.notAgention) {
      res.writeHead(200, { "content-type": "text/html" });
      res.end("<!doctype html><title>Welcome to nginx</title>");
      return;
    }
    if (this.redirect) {
      res.writeHead(302, { location: "https://elsewhere.example/login" });
      res.end();
      return;
    }
    try {
      if (this.legacy && url.pathname.startsWith("/api/operator")) throw new Refusal(404, "not_found", "not found");
      if (this.failNext.length > 0) {
        const f = this.failNext.shift();
        res.writeHead(f.status, { "content-type": "application/json", ...(f.headers ?? {}) });
        res.end(JSON.stringify({ error: { code: f.code, message: f.message ?? f.code } }));
        return;
      }
      if (this.rateLimit > 0) {
        this.rateLimit -= 1;
        res.writeHead(429, { "content-type": "application/json", "retry-after": this.retryAfter });
        res.end(JSON.stringify({ error: { code: "rate_limited", message: "too many requests" } }));
        return;
      }
      const body = await this.route(req, url, raw, res);
      if (body === undefined) return; // answered already (the wait)
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify(body));
    } catch (error) {
      if (!(error instanceof Refusal)) {
        res.writeHead(500, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: { code: "internal", message: "internal" } }));
        return;
      }
      res.writeHead(error.status, { "content-type": "application/json" });
      res.end(JSON.stringify(error.body));
    }
  }

  authenticate(req, scope) {
    if (req.headers.cookie !== undefined) throw new Refusal(401, "unauthenticated", "the operator API does not accept cookies");
    const header = req.headers.authorization ?? "";
    const m = /^Bearer (\S+)$/.exec(header);
    const scopes = m ? this.keys.get(m[1]) : undefined;
    if (!scopes) throw new Refusal(401, "unauthenticated", "the API key is missing, invalid, expired or revoked");
    // Without the scope, as if the operation did not exist; before any input.
    if (!holds(scopes, scope)) throw new Refusal(404, "not_found", "not found");
  }

  thread(params) {
    const id = `${params[0]}/${params[1]}/${params[2]}`;
    const t = this.threads.get(id);
    if (!t) throw new Refusal(404, "not_found", "no such thread");
    return t;
  }

  async route(req, url, raw, res) {
    const p = url.pathname;
    const q = url.searchParams;
    const method = req.method;
    if (method === "GET" && p === "/api/operator") return { versions: [1] };
    const threadRoute = /^\/api\/operator\/v1\/s\/([^/]+)\/([^/]+)\/threads\/(\d+)(\/[a-z]+)?$/.exec(p);
    let body = null;
    const json = () => {
      if (body !== null) return body;
      try {
        body = JSON.parse(raw);
      } catch {
        throw new Refusal(400, "invalid_input", "not JSON");
      }
      if (typeof body !== "object" || body === null) throw new Refusal(400, "invalid_input", "not an object");
      return body;
    };
    const int = (v, field) => {
      if (!Number.isSafeInteger(v) || v < 0) throw new Refusal(400, "invalid_input", "expected a number", { field });
      return v;
    };

    if (method === "GET" && p === "/api/operator/v1/threads") {
      this.authenticate(req, "threads:read");
      const limit = Number(q.get("limit") ?? 50);
      if (!(limit >= 1 && limit <= 100)) throw new Refusal(400, "invalid_input", "limit", { field: "limit" });
      const offset = q.get("cursor") ? Number(Buffer.from(q.get("cursor"), "base64url").toString()) : 0;
      const all = [...this.threads.values()]
        .filter((t) => t.archived_through < this.lastSeq(t))
        .sort((a, b) => (a.updated_at < b.updated_at ? 1 : -1));
      const page = all.slice(offset, offset + limit).map((t) => this.summary(t));
      const out = { threads: page, total: all.length };
      if (offset + limit < all.length) out.next = Buffer.from(String(offset + limit)).toString("base64url");
      return out;
    }
    if (method === "GET" && p === "/api/operator/v1/threads/counts") {
      this.authenticate(req, "threads:read");
      const live = [...this.threads.values()].filter((t) => t.archived_through < this.lastSeq(t)).map((t) => this.summary(t));
      const n = (f) => live.filter(f).length;
      return {
        status: { any: live.length, open: n((t) => t.status === "open"), answered: n((t) => t.status === "answered"), withdrawn: n((t) => t.status === "withdrawn"), none: n((t) => t.status === "none") },
        priority: { any: live.length, normal: 0, high: 0, urgent: 0 },
        impact: { any: live.length, blocking: n((t) => t.open_asks.blocking), assuming: 0 },
        overdue: { any: live.length, overdue: 0 },
        effort: { any: live.length, quick: 0, under_15m: 0, under_1h: 0 },
        unread: { any: live.length, unread: n((t) => t.unread > 0) },
        agent_read: { any: live.length, unread: 0, stranded: 0 },
        include_archived: { hide: live.length, show: this.threads.size },
        agent: { agents: [], truncated: false },
        session: { sessions: [], truncated: false },
        topic: { topics: [], truncated: false },
        session_group: { groups: [], none: live.length },
        tag: { tags: [], truncated: false },
        reset: live.length,
      };
    }
    if (method === "GET" && p === "/api/operator/v1/wait") {
      this.authenticate(req, "threads:read");
      const wait = Number(q.get("wait") ?? 0);
      if (!(wait >= 0 && wait <= 25)) throw new Refusal(400, "invalid_input", "wait", { field: "wait" });
      const since = q.get("since") ? Number(q.get("since").slice(1)) : 0;
      const limit = Number(q.get("limit") ?? 50);
      const found = () =>
        [...this.threads.values()]
          .filter((t) => t.position > since && this.unread(t) > 0)
          .sort((a, b) => a.position - b.position)
          .slice(0, limit);
      const answer = (threads) => {
        const upTo = threads.length === limit ? threads.at(-1).position : this.counter;
        return { threads: threads.map((t) => this.summary(t)), since: `w${Math.max(upTo, since)}` };
      };
      const now = found();
      if (now.length > 0 || wait === 0) return answer(now);
      return new Promise((resolve) => {
        const waiter = {
          check: () => {
            const ready = found();
            if (ready.length > 0) waiter.finish(answer(ready));
          },
          finish: (out) => {
            clearTimeout(waiter.timer);
            this.waiters.delete(waiter);
            if (!res.writableEnded && !res.destroyed) {
              res.writeHead(200, { "content-type": "application/json" });
              res.end(JSON.stringify(out ?? answer([])));
            }
            resolve(undefined);
          },
        };
        waiter.timer = setTimeout(() => waiter.finish(answer([])), wait * 1000);
        res.on("close", () => {
          clearTimeout(waiter.timer);
          this.waiters.delete(waiter);
          resolve(undefined);
        });
        this.waiters.add(waiter);
      });
    }
    if (method === "GET" && p === "/api/operator/v1/sessions") {
      this.authenticate(req, "sessions:read");
      const agents = new Map();
      for (const t of this.threads.values()) {
        if (!agents.has(t.agent)) agents.set(t.agent, new Map());
        const sessions = agents.get(t.agent);
        const s = sessions.get(t.session) ?? { session: t.session, created_at: t.created_at, tags: [], presence: { quiet: false }, registrations: [], unread_updates: 0, threads: { total: 0, unread: 0, stranded: 0 }, open_asks: { count: 0, blocking_count: 0 }, status: { state: "working", title: "Working" } };
        const sum = this.summary(t);
        s.threads.total += 1;
        if (sum.unread > 0) s.threads.unread += 1;
        s.open_asks.count += sum.open_asks.count;
        sessions.set(t.session, s);
      }
      const all = [...agents.entries()].map(([agent, sessions]) => ({ agent, tags: [], sessions: [...sessions.values()], truncated: false }));
      const limit = Number(q.get("limit") ?? 50);
      const offset = q.get("cursor") ? Number(q.get("cursor")) : 0;
      const out = { agents: all.slice(offset, offset + limit) };
      if (offset + limit < all.length) out.next = String(offset + limit);
      return out;
    }
    if (method === "POST" && (p === "/api/operator/v1/reads" || p === "/api/operator/v1/archives")) {
      this.authenticate(req, "threads:write");
      const b = json();
      if (!Array.isArray(b.threads) || b.threads.length < 1) throw new Refusal(400, "invalid_input", "Required", { field: "threads" });
      return { done: true };
    }
    if (threadRoute) {
      const [, agent, session, num, action] = threadRoute;
      if (!NAME.test(agent) || !NAME.test(session)) throw new Refusal(400, "invalid_input", "bad name");
      if (method === "GET" && action === undefined) {
        this.authenticate(req, "threads:read");
        const t = this.thread([agent, session, num]);
        const after = Number(q.get("after") ?? 0);
        const limit = Number(q.get("limit") ?? 50);
        const messages = t.messages.filter((m) => m.seq > after).slice(0, limit).map((m) => this.messageRead(m));
        return {
          thread: this.summary(t),
          messages,
          cursors: { requester: this.lastSeq(t), audience: t.read_through },
          marks: { read_through: t.read_through, archived_through: t.archived_through },
        };
      }
      if (method === "POST" && action === "/messages") {
        this.authenticate(req, "threads:write");
        const t = this.thread([agent, session, num]);
        const b = json();
        if (typeof b.request_id !== "string" || !/^[A-Za-z0-9._:-]{1,64}$/.test(b.request_id)) throw new Refusal(400, "invalid_input", "request_id", { field: "request_id" });
        int(b.seen_through, "seen_through");
        const key = `${agent}/${session}/${num}#${b.request_id}`;
        if (this.writes.has(key)) {
          const prior = this.writes.get(key);
          if (prior.raw !== raw) throw new Refusal(409, "request_id_conflict", "request id reused");
          return prior.result;
        }
        if (b.seen_through < this.lastSeq(t)) {
          const missed = t.messages.filter((m) => m.seq > b.seen_through).map((m) => this.messageRead(m));
          throw new Refusal(409, "stale", "the thread has messages you have not seen", { missed });
        }
        if (b.text !== undefined && (typeof b.text !== "string" || b.text.length < 1 || b.text.length > 1000)) throw new Refusal(400, "invalid_input", "text", { field: "text" });
        if (b.detail !== undefined && (typeof b.detail !== "string" || b.detail.length > 20000)) throw new Refusal(400, "invalid_input", "detail", { field: "detail" });
        const answers = [];
        for (const [i, a] of (b.answers ?? []).entries()) {
          const ask = this.askRef(t, a.ask);
          if (!ask || !this.isOpen(ask)) throw new Refusal(400, "invalid_input", "that ask is not open", { field: `answers.${i}.ask` });
          if (a.declined && (a.picks || a.text)) throw new Refusal(400, "invalid_input", "a decline carries nothing", { field: `answers.${i}` });
          for (const pick of a.picks ?? []) if (!(pick >= 0 && pick < ask.suggestions.length)) throw new Refusal(400, "invalid_input", "no such suggestion", { field: `answers.${i}.picks` });
          if (!ask.multiple && (a.picks ?? []).length > 1) throw new Refusal(400, "invalid_input", "one pick only", { field: `answers.${i}.picks` });
          if (!a.declined && !a.picks && !a.text) throw new Refusal(400, "invalid_input", "empty answer", { field: `answers.${i}` });
          answers.push({ ask, a });
        }
        if (!b.text && answers.length === 0) throw new Refusal(400, "invalid_input", "an empty message", { field: "text" });
        const message = { author: { kind: "person", name: "Operator" }, text: b.text, detail: b.detail, asks: [] };
        const seqNext = this.lastSeq(t) + 1;
        if (answers.length) {
          message.answers = answers.map(({ ask, a }) => ({
            ask: ask.number,
            question: ask.question,
            ask_seq: ask.seq,
            ask_live: true,
            declined: !!a.declined,
            picks: a.picks ?? [],
            labels: (a.picks ?? []).map((pk) => ask.suggestions[pk].label),
            ...(a.text ? { text: a.text } : {}),
          }));
          for (const { ask, a } of answers) {
            ask.answer = { seq: seqNext, declined: !!a.declined, picks: a.picks ?? [], labels: (a.picks ?? []).map((pk) => ask.suggestions[pk].label), ...(a.text ? { text: a.text } : {}) };
          }
        }
        const seq = this.append(t, message);
        t.read_through = seq;
        const result = { number: t.number, seq };
        this.writes.set(key, { raw, result });
        return result;
      }
      if (method === "POST" && (action === "/read" || action === "/archive" || action === "/unread")) {
        this.authenticate(req, "threads:write");
        const t = this.thread([agent, session, num]);
        const b = json();
        const through = Math.min(int(b.through, "through"), this.lastSeq(t));
        if (action === "/read") {
          t.read_through = Math.max(t.read_through, through);
          return { cursors: { requester: this.lastSeq(t), audience: t.read_through } };
        }
        if (action === "/archive") {
          t.archived_through = Math.max(t.archived_through, through);
          t.read_through = Math.max(t.read_through, through);
          return { marks: { read_through: t.read_through, archived_through: t.archived_through } };
        }
      }
    }
    throw new Refusal(404, "not_found", "not found");
  }
}

export async function startFake() {
  return new FakeAgention().start();
}
