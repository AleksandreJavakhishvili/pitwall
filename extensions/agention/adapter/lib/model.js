// The model Pitwall draws, from what operator-v1.js decoded: the shapes of
// Pitwall's adapter protocol, version 1 (docs/spec/extensions.md on
// feat/extensions, "Adapter protocol"). operator-v1.js knows Agention;
// this file knows Pitwall. Both sides' limits are applied here, so what is
// sent is never cut by Pitwall.

export const PITWALL = Object.freeze({
  threads: 500,
  messages: 200,
  asks: 10,
  suggestions: 10,
  title: 300,
  text: 8000,
  detail: 40000,
  question: 2000,
  label: 200,
  description: 500,
  answerText: 8000,
  withdrawn: 500,
  name: 100,
  status: 500,
  reply: 4000,
});

function cut(s, max) {
  if (typeof s !== "string") return "";
  return s.length <= max ? s : `${s.slice(0, max - 1)}…`;
}

// A thread in the list.
export function listThread(t) {
  return {
    id: t.id,
    title: cut(t.title, PITWALL.title),
    from: { agent: t.from.agent, session: t.from.session },
    unread: t.unread,
    openAsks: t.openAsks,
    blocking: t.blocking,
    lastActivity: t.lastActivity,
  };
}

// The inbox's totals, for the sidebar heading.
export function counts(c) {
  return { type: "counts", unread: c.unread, openAsks: c.open, blocking: c.blocking };
}

// The list: threads not archived, newest activity first, at most 500.
export function threadList(threads) {
  return threads
    .filter((t) => !t.archived)
    .sort((a, b) => (a.lastActivity < b.lastActivity ? 1 : a.lastActivity > b.lastActivity ? -1 : 0))
    .slice(0, PITWALL.threads)
    .map(listThread);
}

function ask(a) {
  const out = {
    number: a.number,
    question: cut(a.question, PITWALL.question),
    multiple: a.multiple,
    open: a.state === "open",
    blocking: a.blocking === true,
    suggestions: a.suggestions.slice(0, PITWALL.suggestions).map((s) => ({
      label: cut(s.label, PITWALL.label),
      description: cut(s.description, PITWALL.description),
    })),
  };
  if (a.answer) {
    out.answer = {
      declined: a.answer.declined,
      picks: a.answer.picks.filter((p) => p >= 0 && p < out.suggestions.length),
      text: cut(a.answer.text, PITWALL.answerText),
    };
  }
  if (a.state === "withdrawn") out.withdrawn = cut(a.withdrawnBecause || "Withdrawn", PITWALL.withdrawn);
  return out;
}

// An answer a message gives, in words that read without the ask.
function answered(a) {
  return {
    ask: a.ask,
    question: cut(a.question, PITWALL.question),
    declined: a.declined,
    labels: a.declined ? [] : a.labels.slice(0, PITWALL.suggestions).map((l) => cut(l, PITWALL.label)),
    text: a.declined ? "" : cut(a.text, PITWALL.answerText),
  };
}

export function message(m) {
  const author =
    m.author.kind === "operator"
      ? { kind: "you", name: cut(m.author.name || "You", PITWALL.name) }
      : { kind: "agent", name: cut(m.author.name, PITWALL.name) };
  return {
    id: m.id,
    author,
    time: m.time,
    text: cut(m.text, PITWALL.text),
    detail: cut(m.detail, PITWALL.detail),
    asks: m.asks.slice(0, PITWALL.asks).map(ask),
    answers: m.answers.slice(0, PITWALL.asks).map(answered),
    withdrawals: m.withdrawals.slice(0, PITWALL.asks).map((w) => ({ ask: w.ask, reason: cut(w.reason, PITWALL.withdrawn) })),
  };
}

// A thread with its messages: those holding open asks however old, then the
// latest, 200 in all, in order. `earlier` says older ones weren't sent.
export function threadDetail(read, { older = [], earlier = false } = {}) {
  const t = listThread(read.thread);
  const seen = new Set(read.messages.map((m) => m.seq));
  const asking = older.filter((m) => !seen.has(m.seq) && m.asks.some((a) => a.state === "open"));
  const room = Math.max(0, PITWALL.messages - asking.length);
  const latest = room > 0 ? read.messages.slice(-room) : [];
  const all = [...asking.slice(0, PITWALL.messages), ...latest].sort((a, b) => a.seq - b.seq);
  return {
    id: t.id,
    title: t.title,
    from: t.from,
    unread: t.unread,
    canReply: true,
    earlier: earlier || latest.length < read.messages.length || read.truncated === true,
    messages: all.map(message),
  };
}

// Agention's text is a summary of at most 1 000 characters, with the rest in
// detail; Pitwall's reply box allows 4 000. A longer reply keeps its first
// paragraph (cut) as the summary and goes whole into detail.
export function replyToMessage(reply, summaryMax = 1000) {
  const words = String(reply ?? "").trim();
  if (words.length <= summaryMax) return { text: words };
  const first = words.split(/\n\s*\n/)[0];
  let summary = first.length <= summaryMax ? first : first.slice(0, summaryMax - 1);
  if (summary.length < first.length) {
    const space = summary.lastIndexOf(" ");
    if (space > summaryMax / 2) summary = summary.slice(0, space);
    summary = `${summary}…`;
  }
  return { text: summary, detail: words };
}
