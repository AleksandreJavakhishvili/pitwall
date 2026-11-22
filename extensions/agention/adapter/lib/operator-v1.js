// Everything this adapter knows about Agention's operator API, version 1:
// routes, scopes, request bodies, and the mapping from the server's answers
// into the model the UI draws (see model.md in this folder's README). The API
// is in a draft pull request (WayfarerLabs/agention#41) and may change; when
// it does, this is the file to change.
//
// Contract: docs/api/openapi.json on Agention's build/operator-api branch.

import {
  BadData,
  bool,
  int,
  isObject,
  line,
  list,
  name,
  object,
  oneOf,
  optionalBool,
  optionalInt,
  optionalLine,
  optionalList,
  optionalText,
  optionalTime,
  text,
  time,
} from "./bounds.js";

export const API_VERSION = 1;
export const VERSIONS_PATH = "/api/operator";
const BASE = `/api/operator/v${API_VERSION}`;

// Agent, session and topic names (packages/api names.ts): these go into
// request paths, so anything else is refused.
export const NAME = /^[a-z0-9][a-z0-9_-]{0,62}$/;

// The API key's form (packages/api api-keys.ts): agention_ and 256 bits in
// unpadded base64url.
export const KEY = /^agention_[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$/;
// Anything that looks like a key, to take out of logs.
export const KEY_ANYWHERE = /agention_[A-Za-z0-9_-]{8,}/g;

// How much this adapter accepts from the server. The API's own bounds are
// noted; text limits leave room above them (a later server may raise them)
// and longer text is cut, while a list over its limit is refused.
export const LIMITS = Object.freeze({
  responseBytes: 8 * 1024 * 1024, // a page of 100 messages with 20 000-character details fits
  pageSize: 100, // pageLimit: 1 to 100
  threadsTotal: 1000, // threads read across pages for one list
  messagesTotal: 1000, // messages read across pages for one thread
  title: 500, // titles: 300 on input
  text: 2000, // text: 1 000
  detail: 40000, // detail: 20 000
  question: 600, // question: 300
  label: 200, // suggestion label: 100
  description: 600, // suggestion description: 300
  answerText: 8000, // answer text: 4 000
  reason: 600, // withdrawal reason: 300
  personName: 100,
  asks: 20, // asks per message: 10
  suggestions: 20, // suggestions per ask: 10
  answers: 20, // answers per message: 10
  picks: 20, // picks per answer: 10
  openAsks: 50, // open_asks.asks: 50
  agents: 100, // agents per sessions page
  sessionsPerAgent: 500, // maxListed
  versions: 50,
  cursor: 1024, // cursor: 1 to 1 024 characters
  errorMessage: 300,
});

// What this adapter writes, held to the API's bounds before sending.
export const INPUT = Object.freeze({
  text: 1000,
  detail: 20000,
  answerText: 4000,
  picks: 10,
  pickIndex: 9,
  answers: 10,
});

// Scopes (packages/api scopes.ts). A write scope includes its read.
export const SCOPES = Object.freeze({
  threadsRead: "threads:read",
  threadsWrite: "threads:write",
  sessionsRead: "sessions:read",
});

function enc(value) {
  return encodeURIComponent(value);
}

function threadPath(ref) {
  return `${BASE}/s/${enc(ref.agent)}/${enc(ref.session)}/threads/${ref.number}`;
}

function query(params) {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null) search.append(key, String(value));
  }
  const out = search.toString();
  return out === "" ? "" : `?${out}`;
}

// Each operation: method, path, the scope it needs, and its body.
export const routes = {
  versions: () => ({ method: "GET", path: VERSIONS_PATH, scope: null }),
  listThreads: ({ cursor, limit = LIMITS.pageSize } = {}) => ({
    method: "GET",
    path: `${BASE}/threads${query({ sort: "latest", limit, cursor })}`,
    scope: SCOPES.threadsRead,
  }),
  counts: () => ({ method: "GET", path: `${BASE}/threads/counts`, scope: SCOPES.threadsRead }),
  getThread: (ref, { after = 0, limit = LIMITS.pageSize } = {}) => ({
    method: "GET",
    path: `${threadPath(ref)}${query({ after, limit })}`,
    scope: SCOPES.threadsRead,
    thread: true,
  }),
  sendMessage: (ref, body) => ({
    method: "POST",
    path: `${threadPath(ref)}/messages`,
    scope: SCOPES.threadsWrite,
    thread: true,
    body,
  }),
  markRead: (ref, through) => ({
    method: "POST",
    path: `${threadPath(ref)}/read`,
    scope: SCOPES.threadsWrite,
    thread: true,
    body: { through },
  }),
  archive: (ref, through) => ({
    method: "POST",
    path: `${threadPath(ref)}/archive`,
    scope: SCOPES.threadsWrite,
    thread: true,
    body: { through },
  }),
  listSessions: ({ cursor, limit = LIMITS.pageSize } = {}) => ({
    method: "GET",
    path: `${BASE}/sessions${query({ limit, cursor })}`,
    scope: SCOPES.sessionsRead,
  }),
  wait: ({ since, wait, limit = LIMITS.pageSize } = {}) => ({
    method: "GET",
    path: `${BASE}/wait${query({ since, wait, limit })}`,
    scope: SCOPES.threadsRead,
  }),
  // The server checks the scope before it reads the body (apps/server
  // operations.ts), so an empty bulk mark-read is refused as invalid_input
  // with threads:write, and as not_found without it; nothing is changed.
  probeThreadsWrite: () => ({
    method: "POST",
    path: `${BASE}/reads`,
    scope: SCOPES.threadsWrite,
    body: {},
    probe: true,
  }),
  probeSessionsRead: () => ({
    method: "GET",
    path: `${BASE}/sessions${query({ limit: 1 })}`,
    scope: SCOPES.sessionsRead,
    probe: true,
  }),
};

// Thread ids in the model: agent/session/number. Names can't hold "/".
export function threadId(ref) {
  return `${ref.agent}/${ref.session}/${ref.number}`;
}

export function parseThreadId(id) {
  if (typeof id !== "string" || id.length > 200) return null;
  const parts = id.split("/");
  if (parts.length !== 3) return null;
  const [agent, session, num] = parts;
  if (!NAME.test(agent) || !NAME.test(session) || !/^\d{1,15}$/.test(num)) return null;
  return { agent, session, number: Number(num) };
}

// ---- Decoding answers into the model ----

function cursor(value, where) {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== "string" || value.length < 1 || value.length > LIMITS.cursor) {
    throw new BadData(where, "expected a cursor");
  }
  return value;
}

export function decodeVersions(body) {
  object(body, "versions");
  return list(body.versions, "versions", LIMITS.versions, (v, w) => int(v, w, { min: 1 }));
}

function decodeOpenAsks(value, where) {
  object(value, where);
  const count = int(value.count, `${where}.count`);
  optionalList(value.asks, `${where}.asks`, LIMITS.openAsks, (a, w) => object(a, w));
  return { count, blocking: bool(value.blocking, `${where}.blocking`) };
}

// A thread as the list, a thread read and the wait give it.
export function decodeThread(value, where = "thread") {
  object(value, where);
  const ref = {
    agent: name(value.agent, `${where}.agent`, NAME),
    session: name(value.session, `${where}.session`, NAME),
    number: int(value.number, `${where}.number`),
  };
  const openAsks = decodeOpenAsks(value.open_asks, `${where}.open_asks`);
  const lastSeq = int(value.last_seq, `${where}.last_seq`);
  return {
    id: threadId(ref),
    title: line(value.title, `${where}.title`, LIMITS.title),
    from: { agent: ref.agent, session: ref.session },
    unread: int(value.unread, `${where}.unread`),
    openAsks: openAsks.count,
    blocking: openAsks.blocking,
    status: oneOf(value.status, `${where}.status`, ["open", "answered", "withdrawn", "none"], "none"),
    archived: bool(value.archived, `${where}.archived`),
    lastActivity: time(value.updated_at, `${where}.updated_at`),
    lastSeq,
  };
}

export function decodeThreadPage(body) {
  object(body, "threads page");
  return {
    threads: list(body.threads, "threads", LIMITS.pageSize, (t, w) => decodeThread(t, w)),
    next: cursor(body.next, "next"),
    total: int(body.total, "total"),
  };
}

export function decodeCounts(body) {
  object(body, "counts");
  const status = object(body.status, "counts.status");
  const unread = object(body.unread, "counts.unread");
  const impact = object(body.impact, "counts.impact");
  const overdue = object(body.overdue, "counts.overdue");
  return {
    threads: int(status.any, "counts.status.any"),
    unread: int(unread.unread, "counts.unread.unread"),
    open: int(status.open, "counts.status.open"),
    blocking: int(impact.blocking, "counts.impact.blocking"),
    overdue: int(overdue.overdue, "counts.overdue.overdue"),
  };
}

function decodeAuthor(value, where) {
  object(value, where);
  if (value.kind === "session") {
    return {
      kind: "agent",
      name: name(value.agent, `${where}.agent`, NAME),
      session: name(value.session, `${where}.session`, NAME),
    };
  }
  if (value.kind === "person") {
    // The one operator: whoever holds the passkey, or a key acting for them.
    return { kind: "operator", name: line(value.name, `${where}.name`, LIMITS.personName) };
  }
  throw new BadData(where, "expected a session or a person");
}

function decodeAnswerOf(value, where) {
  object(value, where);
  return {
    declined: bool(value.declined, `${where}.declined`),
    picks: optionalList(value.picks, `${where}.picks`, LIMITS.picks, (p, w) => int(p, w, { max: 1000 })),
    labels: optionalList(value.labels, `${where}.labels`, LIMITS.picks, (l, w) => line(l, w, LIMITS.label)),
    text: optionalText(value.text, `${where}.text`, LIMITS.answerText),
  };
}

function decodeAsk(value, where) {
  object(value, where);
  const open = bool(value.open, `${where}.open`);
  const answer = value.answer === undefined || value.answer === null
    ? null
    : decodeAnswerOf(value.answer, `${where}.answer`);
  const withdrawal = value.withdrawal === undefined || value.withdrawal === null
    ? null
    : object(value.withdrawal, `${where}.withdrawal`);
  // open; answered or declined; withdrawn; or closed, when its message was
  // replaced (and asked again under new numbers).
  let state;
  if (open) state = "open";
  else if (answer) state = answer.declined ? "declined" : "answered";
  else if (withdrawal) state = "withdrawn";
  else state = "closed";
  return {
    number: int(value.number, `${where}.number`, { min: 1 }),
    question: line(value.question, `${where}.question`, LIMITS.question),
    suggestions: list(value.suggestions, `${where}.suggestions`, LIMITS.suggestions, (s, w) => {
      object(s, w);
      return {
        label: line(s.label, `${w}.label`, LIMITS.label),
        description: optionalLine(s.description, `${w}.description`, LIMITS.description),
      };
    }),
    multiple: bool(value.multiple, `${where}.multiple`),
    blocking: value.impact === "blocking",
    state,
    answer,
    withdrawnBecause: withdrawal
      ? optionalLine(withdrawal.reason, `${where}.withdrawal.reason`, LIMITS.reason)
      : undefined,
  };
}

// A message in a thread; null for one replaced by a later one, which the
// thread no longer counts.
export function decodeMessage(value, where = "message") {
  object(value, where);
  if (!optionalBool(value.live, `${where}.live`, true)) return null;
  const seq = int(value.seq, `${where}.seq`);
  return {
    id: String(seq),
    seq,
    author: decodeAuthor(value.author, `${where}.author`),
    time: time(value.created_at, `${where}.created_at`),
    text: optionalText(value.text, `${where}.text`, LIMITS.text),
    detail: optionalText(value.detail, `${where}.detail`, LIMITS.detail),
    asks: optionalList(value.asks, `${where}.asks`, LIMITS.asks, decodeAsk),
    withdrawals: optionalList(value.withdrawals, `${where}.withdrawals`, LIMITS.answers, (w, ww) => {
      object(w, ww);
      return { ask: int(w.ask, `${ww}.ask`, { min: 1 }), reason: line(w.reason, `${ww}.reason`, LIMITS.reason) };
    }),
    answers: optionalList(value.answers, `${where}.answers`, LIMITS.answers, (a, w) => {
      object(a, w);
      return {
        ask: int(a.ask, `${w}.ask`, { min: 1 }),
        question: line(a.question, `${w}.question`, LIMITS.question),
        ...decodeAnswerOf(a, w),
      };
    }),
  };
}

function decodeMessages(value, where) {
  return list(value, where, LIMITS.pageSize, decodeMessage).filter((m) => m !== null);
}

export function decodeThreadRead(body) {
  object(body, "thread read");
  const all = list(body.messages, "messages", LIMITS.pageSize, (m, w) => object(m, w));
  const marks = object(body.marks, "marks");
  return {
    thread: decodeThread(body.thread, "thread"),
    messages: decodeMessages(all, "messages"),
    // The highest sequence on this page, live or not, to page from.
    pageEnd: all.reduce((max, m, i) => Math.max(max, int(m.seq, `messages[${i}].seq`)), 0),
    pageCount: all.length,
    readThrough: int(marks.read_through, "marks.read_through"),
  };
}

export function decodeWriteResult(body) {
  object(body, "write result");
  return { seq: int(body.seq, "seq") };
}

export function decodeSessionsPage(body) {
  object(body, "sessions page");
  const sessions = [];
  list(body.agents, "agents", LIMITS.agents, (a, w) => {
    object(a, w);
    const agent = name(a.agent, `${w}.agent`, NAME);
    list(a.sessions, `${w}.sessions`, LIMITS.sessionsPerAgent, (s, sw) => {
      object(s, sw);
      const status = isObject(s.status) ? s.status : {};
      const threads = object(s.threads, `${sw}.threads`);
      const asks = object(s.open_asks, `${sw}.open_asks`);
      sessions.push({
        agent,
        session: name(s.session, `${sw}.session`, NAME),
        title: optionalLine(status.title, `${sw}.status.title`, LIMITS.title),
        state: optionalLine(status.state, `${sw}.status.state`, 40),
        quiet: optionalBool(object(s.presence, `${sw}.presence`).quiet, `${sw}.presence.quiet`),
        threads: int(threads.total, `${sw}.threads.total`),
        unreadThreads: int(threads.unread, `${sw}.threads.unread`),
        openAsks: int(asks.count, `${sw}.open_asks.count`),
      });
    });
  });
  return { sessions, next: cursor(body.next, "next") };
}

export function decodeWait(body) {
  object(body, "wait");
  return {
    threads: list(body.threads, "threads", LIMITS.pageSize, (t, w) => decodeThread(t, w)),
    since: cursor(body.since, "since"),
  };
}

// The shared error shape, read leniently: a server that isn't Agention, or a
// proxy in front of it, may answer anything.
export function decodeError(body) {
  if (!isObject(body) || !isObject(body.error) || typeof body.error.code !== "string") return null;
  const e = body.error;
  const out = {
    code: e.code.slice(0, 40),
    message: typeof e.message === "string" ? line(e.message, "error.message", LIMITS.errorMessage) : "",
  };
  if (typeof e.field === "string") out.field = line(e.field, "error.field", 100);
  if (Array.isArray(e.missed)) {
    try {
      out.missed = decodeMessages(e.missed.slice(0, LIMITS.pageSize), "error.missed");
    } catch {
      out.missed = [];
    }
  }
  return out;
}

// ---- Encoding what the operator writes ----

function inputText(value, field, max) {
  if (typeof value !== "string") throw new TypeError(`${field} must be text`);
  const trimmed = value.trim();
  if (trimmed.length === 0) throw new TypeError(`${field} is empty`);
  if (trimmed.length > max) throw new TypeError(`${field} is longer than ${max} characters`);
  return trimmed;
}

// One answer from the model ({ask, picks, text, declined}) into the API's.
export function encodeAnswer(answer) {
  if (!isObject(answer)) throw new TypeError("an answer must be an object");
  const ask = answer.ask;
  if (!Number.isSafeInteger(ask) || ask < 1) throw new TypeError("answer.ask must be an ask number");
  if (answer.declined === true) {
    if ((answer.picks && answer.picks.length) || answer.text) {
      throw new TypeError("a decline carries no picks and no text");
    }
    return { ask, declined: true };
  }
  const out = { ask };
  if (answer.picks !== undefined && answer.picks.length > 0) {
    if (!Array.isArray(answer.picks) || answer.picks.length > INPUT.picks) {
      throw new TypeError(`answer.picks must be a list of at most ${INPUT.picks}`);
    }
    const picks = answer.picks.map((p) => {
      if (!Number.isSafeInteger(p) || p < 0 || p > INPUT.pickIndex) {
        throw new TypeError(`a pick must be 0 to ${INPUT.pickIndex}`);
      }
      return p;
    });
    if (new Set(picks).size !== picks.length) throw new TypeError("a pick is repeated");
    out.picks = picks;
  }
  if (answer.text !== undefined && answer.text !== null && answer.text !== "") {
    out.text = inputText(answer.text, "answer.text", INPUT.answerText);
  }
  if (!out.picks && !out.text) throw new TypeError("an answer needs a pick, words or a decline");
  return out;
}

export function encodeAnswers(requestId, seenThrough, answers) {
  if (!Array.isArray(answers) || answers.length < 1 || answers.length > INPUT.answers) {
    throw new TypeError(`answers must be a list of 1 to ${INPUT.answers}`);
  }
  const encoded = answers.map(encodeAnswer);
  if (new Set(encoded.map((a) => a.ask)).size !== encoded.length) {
    throw new TypeError("an ask is answered twice");
  }
  return { request_id: requestId, seen_through: seenThrough, answers: encoded };
}

export function encodeMessage(requestId, seenThrough, { text: words, detail }) {
  const body = {
    request_id: requestId,
    seen_through: seenThrough,
    text: inputText(words, "text", INPUT.text),
  };
  if (detail !== undefined && detail !== null && detail !== "") {
    body.detail = inputText(detail, "detail", INPUT.detail);
  }
  return body;
}
