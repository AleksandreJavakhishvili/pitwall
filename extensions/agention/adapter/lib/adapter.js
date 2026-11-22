// The adapter: Pitwall's adapter protocol, version 1 (docs/spec/extensions.md
// on feat/extensions, "Adapter protocol") on one side, Agention's operator
// API on the other. JSON lines in on `input`, out on `output`; logs only on
// `log`. The API key arrives in the first line, `hello`, and is never
// written anywhere.

import { AgentionClient, backoffMs, checkSettings, defaultSleep } from "./client.js";
import { AdapterError, errors } from "./errors.js";
import { KEY_ANYWHERE, LIMITS } from "./operator-v1.js";
import { PITWALL, replyToMessage, threadDetail, threadList } from "./model.js";

export const PROTOCOL = 1;
const MAX_LINE = 1 << 20;

// Errors a person fixes in the extension's settings or in Agention's.
const SIGNED_OUT = new Set(["no_key", "key_rejected", "missing_scope"]);

function statusFor(error) {
  return SIGNED_OUT.has(error.kind) ? "signedOut" : "error";
}

// Reads newline-delimited lines from a stream, refusing one over max bytes.
async function* lines(input, max) {
  let buffered = "";
  input.setEncoding?.("utf8");
  for await (const chunk of input) {
    buffered += chunk;
    let at;
    while ((at = buffered.indexOf("\n")) >= 0) {
      const one = buffered.slice(0, at);
      buffered = buffered.slice(at + 1);
      if (Buffer.byteLength(one) <= max) yield one;
    }
    if (Buffer.byteLength(buffered) > max) buffered = ""; // drop a line too long, up to its end
  }
  if (buffered.trim() !== "") yield buffered;
}

export class Adapter {
  constructor({ output, log = () => {}, clientOptions = {}, refreshEveryMs = 60_000, sleep = defaultSleep, random = Math.random }) {
    this.output = output;
    this.rawLog = log;
    this.clientOptions = clientOptions;
    this.refreshEveryMs = refreshEveryMs;
    this.sleep = sleep;
    this.random = random;
    this.stop = new AbortController();
    this.client = null;
    this.ready = false;
    this.secret = null;
    this.known = new Map(); // id -> decoded thread from the last list
    this.seenThrough = new Map(); // id -> latest sequence the person was shown
    this.open = new Set();
    this.refreshTimer = null;
    this.refreshing = null;
    this.refreshAgain = false;
  }

  // Takes out the key, and anything shaped like one, before anything leaves.
  redact(s) {
    let out = s;
    if (this.secret) out = out.split(this.secret).join("[key]");
    return out.replace(KEY_ANYWHERE, "[key]");
  }

  send(msg) {
    this.output.write(`${this.redact(JSON.stringify(msg))}\n`);
  }

  log(s) {
    this.rawLog(`${this.redact(String(s))}\n`);
  }

  status(state, message) {
    const out = { type: "status", state };
    if (message) out.message = message.slice(0, PITWALL.status);
    this.send(out);
  }

  result(req, ok, message) {
    if (typeof req !== "number") return;
    this.send(ok ? { type: "result", req, ok: true } : { type: "result", req, ok: false, message });
  }

  async run(input) {
    let greeted = false;
    try {
      for await (const raw of lines(input, MAX_LINE)) {
        let msg;
        try {
          msg = JSON.parse(raw);
        } catch {
          this.log("ignored a line that isn't JSON");
          continue;
        }
        if (typeof msg !== "object" || msg === null || Array.isArray(msg)) continue;
        if (!greeted) {
          if (msg.type !== "hello") continue;
          greeted = true;
          this.hello(msg);
          continue;
        }
        this.handle(msg).catch((error) => this.log(`request failed: ${error?.stack ?? error}`));
      }
    } finally {
      this.shutdown();
    }
  }

  shutdown() {
    this.stop.abort();
    clearTimeout(this.refreshTimer);
  }

  hello(msg) {
    this.send({ type: "hello", protocol: PROTOCOL });
    if (msg.protocol !== PROTOCOL) {
      this.status("error", `This Agention extension speaks protocol ${PROTOCOL}; Pitwall asked for ${String(msg.protocol).slice(0, 20)}. Update the extension.`);
      return;
    }
    const settings = typeof msg.settings === "object" && msg.settings !== null ? msg.settings : {};
    const secrets = typeof msg.secrets === "object" && msg.secrets !== null ? msg.secrets : {};
    if (typeof secrets.apiKey === "string" && secrets.apiKey.trim() !== "") this.secret = secrets.apiKey.trim();
    try {
      checkSettings({ url: settings.server, key: secrets.apiKey });
      this.client = new AgentionClient(
        { url: settings.server, key: secrets.apiKey },
        { sleep: this.sleep, random: this.random, log: (s) => this.log(s), ...this.clientOptions },
      );
    } catch (error) {
      const e = error instanceof AdapterError ? error : errors.badUrl();
      this.status(statusFor(e), e.message);
      return;
    }
    this.connectLoop().catch((error) => this.log(`stopped: ${error?.stack ?? error}`));
  }

  // Connects, then waits for news, until stopped or a setting is wrong.
  async connectLoop() {
    const signal = this.stop.signal;
    let failures = 0;
    while (!signal.aborted) {
      this.status("connecting", failures === 0 ? undefined : "Connecting to Agention again…");
      try {
        await this.client.connect({ signal });
        await this.refreshNow();
      } catch (error) {
        if (signal.aborted || error.kind === "cancelled") return;
        if (!error.retryable) {
          this.status(statusFor(error), error.message);
          return;
        }
        const delay = error.kind === "rate_limited" ? error.retryAfterMs : backoffMs(failures, { random: this.random });
        failures += 1;
        this.status("error", `${error.message} Trying again in ${Math.ceil(delay / 1000)} s.`);
        try {
          await this.sleep(delay, signal);
        } catch {
          return;
        }
        continue;
      }
      failures = 0;
      this.ready = true;
      this.status("ready");
      this.scheduleRefresh(this.refreshEveryMs);
      const fatal = await this.client.waitLoop({
        signal,
        onThreads: (threads) => this.onNews(threads),
        onState: ({ state, error, retryInMs }) => {
          if (state === "retrying") {
            this.ready = false;
            this.status("error", `${error.message} Trying again in ${Math.ceil(retryInMs / 1000)} s.`);
          } else if (state === "live" && !this.ready) {
            this.ready = true;
            this.status("ready");
            this.refreshSoon();
          }
        },
      });
      this.ready = false;
      clearTimeout(this.refreshTimer);
      if (fatal) {
        this.status(statusFor(fatal), fatal.message);
        return;
      }
    }
  }

  // The wait said these threads have news: refresh the list, and any of them
  // that is open.
  async onNews(threads) {
    for (const t of threads) {
      if (this.open.has(t.id)) this.sendThread(t.id).catch(() => {});
    }
    this.refreshSoon();
  }

  scheduleRefresh(ms) {
    clearTimeout(this.refreshTimer);
    if (this.stop.signal.aborted) return;
    this.refreshTimer = setTimeout(() => {
      this.refreshNow()
        .catch((error) => this.log(`refresh failed: ${error.kind ?? error}`))
        .finally(() => this.scheduleRefresh(this.refreshEveryMs));
    }, ms);
    this.refreshTimer.unref?.();
  }

  refreshSoon() {
    this.scheduleRefresh(250);
  }

  // Reads the list and sends it; one at a time, and again once more if asked
  // during one.
  async refreshNow() {
    if (this.refreshing) {
      this.refreshAgain = true;
      return this.refreshing;
    }
    this.refreshing = (async () => {
      do {
        this.refreshAgain = false;
        const { threads } = await this.client.listThreads({ signal: this.stop.signal, max: PITWALL.threads });
        const before = this.known;
        this.known = new Map(threads.map((t) => [t.id, t]));
        this.send({ type: "threads", threads: threadList(threads) });
        for (const id of this.open) {
          const was = before.get(id);
          const now = this.known.get(id);
          if (now && (!was || was.lastSeq !== now.lastSeq || was.unread !== now.unread)) {
            this.sendThread(id).catch(() => {});
          }
        }
      } while (this.refreshAgain);
    })().finally(() => {
      this.refreshing = null;
    });
    return this.refreshing;
  }

  async sendThread(id) {
    const known = this.known.get(id);
    const after = known ? Math.max(0, known.lastSeq - PITWALL.messages) : 0;
    let read;
    try {
      read = await this.client.readThread(id, { signal: this.stop.signal, after, max: LIMITS.messagesTotal });
    } catch (error) {
      if (error.kind === "cancelled") return null;
      throw error;
    }
    this.seenThrough.set(id, read.thread.lastSeq);
    this.send({ type: "thread", thread: threadDetail(read) });
    return read;
  }

  // The latest sequence of a thread the person has been shown.
  async latest(id) {
    if (this.seenThrough.has(id)) return this.seenThrough.get(id);
    if (this.known.has(id)) return this.known.get(id).lastSeq;
    const read = await this.sendThread(id);
    return read?.thread.lastSeq ?? 0;
  }

  async handle(msg) {
    const req = msg.req;
    const needsThread = ["open", "close", "answer", "message", "markRead", "archive"].includes(msg.type);
    if (needsThread && (typeof msg.thread !== "string" || msg.thread.length > 200)) {
      this.result(req, false, "That thread isn't one this extension knows.");
      return;
    }
    if (msg.type === "close") {
      this.open.delete(msg.thread);
      this.seenThrough.delete(msg.thread);
      return;
    }
    if (!this.client) {
      this.result(req, false, "The extension isn't set up: check its server address and API key.");
      return;
    }
    if (msg.type === "refresh") {
      if (this.ready) this.refreshNow().catch((e) => this.log(`refresh failed: ${e.kind ?? e}`));
      return;
    }
    if (msg.type === "open") {
      this.open.add(msg.thread);
      if (!this.ready) return;
      try {
        await this.sendThread(msg.thread);
      } catch (error) {
        this.log(`open failed: ${error.kind}`);
        this.result(req, false, error.message);
      }
      return;
    }
    if (!needsThread) {
      if (req !== undefined) this.result(req, false, `This extension doesn't do “${String(msg.type).slice(0, 40)}”.`);
      return;
    }
    if (!this.ready) {
      this.result(req, false, "Not connected to Agention right now; try again in a moment.");
      return;
    }
    try {
      await this.request(msg);
      this.result(req, true);
    } catch (error) {
      const e = error instanceof AdapterError ? error : errors.badInput(String(error?.message ?? error));
      if (e.kind === "stale" && this.open.has(msg.thread)) this.sendThread(msg.thread).catch(() => {});
      this.result(req, false, e.message);
      if (SIGNED_OUT.has(e.kind)) this.status("signedOut", e.message);
      return;
    }
    if (this.open.has(msg.thread)) this.sendThread(msg.thread).catch(() => {});
    this.refreshSoon();
  }

  async request(msg) {
    const id = msg.thread;
    const signal = this.stop.signal;
    switch (msg.type) {
      case "answer": {
        const answer = { ask: msg.ask };
        if (msg.declined === true) answer.declined = true;
        else {
          if (Array.isArray(msg.picks) && msg.picks.length > 0) answer.picks = msg.picks;
          if (typeof msg.text === "string" && msg.text.trim() !== "") answer.text = msg.text;
        }
        const seenThrough = await this.latest(id);
        return this.client.answer(id, [answer], { seenThrough, signal });
      }
      case "message": {
        if (typeof msg.text !== "string" || msg.text.trim() === "") throw errors.badInput("the message is empty");
        if (msg.text.length > PITWALL.reply) throw errors.badInput(`the message is longer than ${PITWALL.reply} characters`);
        const seenThrough = await this.latest(id);
        return this.client.sendMessage(id, replyToMessage(msg.text), { seenThrough, signal });
      }
      case "markRead":
        return this.client.markRead(id, await this.latest(id), { signal });
      case "archive":
        return this.client.archive(id, await this.latest(id), { signal });
      default:
        throw errors.badInput("unknown request");
    }
  }
}
