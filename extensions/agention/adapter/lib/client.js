// A client for Agention's operator API: transport, errors, retries, paging
// and the wait loop. What the routes and fields are is in operator-v1.js.
// No dependencies: Node 24's fetch.

import { randomUUID } from "node:crypto";
import { BadData } from "./bounds.js";
import { AdapterError, classifyFetchError, errors } from "./errors.js";
import * as api from "./operator-v1.js";

const LOOPBACK = new Set(["127.0.0.1", "localhost", "[::1]", "::1"]);

// checkSettings reads the address and key the person gave, before anything
// is sent, and answers the base URL or throws an AdapterError.
export function checkSettings({ url, key } = {}) {
  if (typeof url !== "string" || url.trim() === "") throw errors.noUrl();
  let parsed;
  try {
    parsed = new URL(url.trim());
  } catch {
    throw errors.badUrl();
  }
  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") throw errors.badUrl();
  if (parsed.username || parsed.password || parsed.search || parsed.hash) throw errors.badUrl();
  if (parsed.protocol === "http:" && !LOOPBACK.has(parsed.hostname)) throw errors.insecureUrl();
  if (typeof key !== "string" || key.trim() === "") throw errors.noKey();
  if (!api.KEY.test(key.trim())) throw errors.badKeyFormat();
  const base = `${parsed.origin}${parsed.pathname.replace(/\/+$/, "")}`;
  return { base, key: key.trim() };
}

// Sleeps ms, or until signal aborts (then throws `cancelled`).
export function defaultSleep(ms, signal) {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(errors.cancelled());
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    function onAbort() {
      clearTimeout(timer);
      reject(errors.cancelled());
    }
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

// Retry-After as seconds or an HTTP date, held to 1 second .. 1 hour.
export function parseRetryAfter(value, now = Date.now()) {
  let ms;
  if (typeof value === "string" && /^\s*\d{1,7}\s*$/.test(value)) ms = Number(value) * 1000;
  else if (typeof value === "string" && !Number.isNaN(Date.parse(value))) ms = Date.parse(value) - now;
  else ms = 5000;
  return Math.min(Math.max(ms, 1000), 3_600_000);
}

// Exponential backoff with jitter: half the step fixed, half random, so
// clients that failed together don't come back together.
export function backoffMs(attempt, { baseMs = 1000, maxMs = 60_000, random = Math.random } = {}) {
  const step = Math.min(maxMs, baseMs * 2 ** Math.min(attempt, 30));
  return Math.round(step / 2 + random() * (step / 2));
}

async function readCapped(response, max) {
  const reader = response.body?.getReader();
  if (!reader) return "";
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > max) {
      await reader.cancel().catch(() => {});
      throw errors.badResponse(`answer larger than ${max} bytes`);
    }
    chunks.push(value);
  }
  return new TextDecoder().decode(Buffer.concat(chunks));
}

export class AgentionClient {
  constructor(settings, options = {}) {
    const { base, key } = checkSettings(settings);
    this.base = base;
    // Kept off the instance's enumerable fields, so logging the client
    // can't print it.
    Object.defineProperty(this, "key", { value: key, enumerable: false });
    this.fetch = options.fetch ?? globalThis.fetch;
    this.sleep = options.sleep ?? defaultSleep;
    this.random = options.random ?? Math.random;
    this.timeoutMs = options.timeoutMs ?? 15_000;
    this.waitSeconds = options.waitSeconds ?? 25;
    this.maxRetries = options.maxRetries ?? 2;
    this.maxRetryAfterMs = options.maxRetryAfterMs ?? 30_000;
    this.log = options.log ?? (() => {});
  }

  toJSON() {
    return { base: this.base };
  }

  // One request: a route from operator-v1.js, answered as parsed JSON or
  // thrown as an AdapterError.
  async send(route, { signal, timeoutMs = this.timeoutMs } = {}) {
    const headers = { accept: "application/json" };
    if (route.scope !== null) headers.authorization = `Bearer ${this.key}`;
    let body;
    if (route.body !== undefined) {
      body = JSON.stringify(route.body);
      headers["content-type"] = "application/json";
    }
    const timeout = AbortSignal.timeout(timeoutMs);
    const combined = signal ? AbortSignal.any([signal, timeout]) : timeout;
    let response;
    let textBody;
    try {
      response = await this.fetch(`${this.base}${route.path}`, {
        method: route.method,
        headers,
        body,
        redirect: "manual",
        signal: combined,
      });
      textBody = await readCapped(response, api.LIMITS.responseBytes);
    } catch (error) {
      throw classifyFetchError(error, {
        cancelled: signal?.aborted === true,
        timedOut: timeout.aborted,
      });
    }
    const type = response.headers.get("content-type") ?? "";
    let json = null;
    if (/\bjson\b/i.test(type)) {
      try {
        json = JSON.parse(textBody);
      } catch {
        json = null;
      }
    }
    if (response.status >= 200 && response.status < 300) {
      if (json === null) {
        if (route.scope === null) throw errors.notAgention("it didn't answer with JSON");
        throw errors.badResponse("not JSON");
      }
      return json;
    }
    throw this.refusal(route, response, json, textBody);
  }

  refusal(route, response, json, textBody) {
    const status = response.status;
    const err = api.decodeError(json);
    const detail = err?.message || undefined;
    if (status >= 300 && status < 400) return errors.notAgention("it sends requests elsewhere");
    if (route.scope === null) {
      // The versions route, the first thing asked of a server.
      if (status === 404) {
        // An Agention server from before the operator API answers its own
        // error shape, or its web page, for a route it doesn't have.
        if (err || /agention/i.test(textBody.slice(0, 64 * 1024))) return errors.tooOld();
        return errors.notAgention(`it answered status ${status}`);
      }
      if (!err) return errors.notAgention(`it answered status ${status}`);
    } else if (!err && status !== 429 && status < 500) {
      return errors.notAgention(`it answered status ${status} without Agention's error shape`);
    }
    switch (status) {
      case 401:
        return errors.keyRejected(detail);
      case 403:
        return errors.missingScope(route.scope ?? "needed", 403, detail);
      case 404:
        // A key without an operation's scope is answered not_found, as if
        // the operation didn't exist (operator guide, "Using the operator
        // API"); only a thread's own routes can be missing for real.
        if (!route.thread) return errors.missingScope(route.scope, 404, detail);
        return errors.notFound(detail);
      case 409:
        if (err?.code === "stale") return errors.stale(err.missed ?? []);
        return errors.invalidInput(detail, err?.field);
      case 429:
        return errors.rateLimited(parseRetryAfter(response.headers.get("retry-after") ?? undefined));
      case 400:
        if (err?.code === "unsupported_version") return errors.unsupportedVersion([]);
        return errors.invalidInput(detail, err?.field);
      case 411:
      case 413:
        return errors.invalidInput(detail, err?.field);
      default:
        return errors.server(status, detail);
    }
  }

  // send with retries for what may pass: the network, a timeout, a server
  // error or a rate limit (after its Retry-After). Every route here is safe
  // to repeat: reads, marks "through" a sequence, and writes carrying a
  // request_id the server deduplicates.
  async request(route, { signal, timeoutMs, retries = this.maxRetries } = {}) {
    for (let attempt = 0; ; attempt += 1) {
      try {
        return await this.send(route, { signal, timeoutMs });
      } catch (error) {
        const e = error instanceof AdapterError ? error : classifyFetchError(error);
        const transient =
          e.kind === "network" ||
          e.kind === "timeout" ||
          e.kind === "rate_limited" ||
          (e.kind === "server" && e.status >= 500);
        if (!transient || attempt >= retries) throw e;
        if (e.kind === "rate_limited" && e.retryAfterMs > this.maxRetryAfterMs) throw e;
        const delay =
          e.kind === "rate_limited"
            ? e.retryAfterMs
            : backoffMs(attempt, { baseMs: 500, maxMs: 5000, random: this.random });
        this.log(`retrying ${route.method} after ${e.kind} in ${delay} ms`);
        await this.sleep(delay, signal);
      }
    }
  }

  // Reads the answer with decode; a shape it can't read is bad_response.
  async call(route, decode, options) {
    const json = await this.request(route, options);
    try {
      return decode(json);
    } catch (error) {
      if (error instanceof BadData) throw errors.badResponse(error.message);
      throw error;
    }
  }

  // ---- Operations ----

  // The versions of the operator API the server serves; throws too_old,
  // not_agention or unsupported_version when this one can't be used.
  async checkVersion(options) {
    let versions;
    try {
      versions = await this.call(api.routes.versions(), api.decodeVersions, options);
    } catch (error) {
      if (error.kind === "bad_response") throw errors.notAgention("its answer isn't a list of versions");
      throw error;
    }
    if (!versions.includes(api.API_VERSION)) throw errors.unsupportedVersion(versions);
    return versions;
  }

  // Checks the key holds the scopes this extension needs, naming the first
  // one it lacks. Nothing on the server changes.
  async checkScopes(options) {
    try {
      await this.request(api.routes.probeThreadsWrite(), options);
    } catch (error) {
      if (error.kind !== "invalid_input") throw error;
    }
    await this.request(api.routes.probeSessionsRead(), options);
  }

  async connect(options) {
    const versions = await this.checkVersion(options);
    await this.checkScopes(options);
    return { versions };
  }

  // Every thread (up to LIMITS.threadsTotal), newest activity first.
  async listThreads({ signal, max = api.LIMITS.threadsTotal, pageSize = api.LIMITS.pageSize } = {}) {
    const threads = [];
    const seen = new Set();
    let cursor;
    let total = 0;
    do {
      const page = await this.call(
        api.routes.listThreads({ cursor, limit: pageSize }),
        api.decodeThreadPage,
        { signal },
      );
      total = page.total;
      for (const t of page.threads) {
        if (!seen.has(t.id) && threads.length < max) {
          seen.add(t.id);
          threads.push(t);
        }
      }
      cursor = page.next;
      if (page.threads.length === 0) break;
    } while (cursor !== undefined && threads.length < max);
    return { threads, total, truncated: cursor !== undefined };
  }

  counts(options) {
    return this.call(api.routes.counts(), api.decodeCounts, options);
  }

  ref(id) {
    const ref = api.parseThreadId(id);
    if (!ref) throw errors.badInput("that isn't a thread id");
    return ref;
  }

  // A thread and its messages, page by page (up to LIMITS.messagesTotal).
  // `after` starts past the messages before it (to read only the latest).
  async readThread(id, { signal, after: start = 0, max = api.LIMITS.messagesTotal, pageSize = api.LIMITS.pageSize } = {}) {
    const ref = this.ref(id);
    const messages = [];
    let after = Number.isSafeInteger(start) && start > 0 ? start : 0;
    let last;
    for (;;) {
      const page = await this.call(api.routes.getThread(ref, { after, limit: pageSize }), api.decodeThreadRead, {
        signal,
      });
      last = page;
      for (const m of page.messages) if (m.seq > after) messages.push(m);
      if (page.pageCount < pageSize || page.pageEnd <= after || page.pageEnd >= page.thread.lastSeq) break;
      if (messages.length >= max) break;
      after = page.pageEnd;
    }
    const truncated = messages.length > max;
    return {
      thread: last.thread,
      messages: messages.slice(0, max),
      readThrough: last.readThrough,
      truncated: truncated || (last.pageEnd < last.thread.lastSeq && last.pageCount === pageSize),
    };
  }

  // Answers asks: [{ask, picks?, text?, declined?}]. seenThrough is the
  // latest message the person saw; the server refuses as `stale` when the
  // agent wrote since, carrying what they missed.
  async answer(id, answers, { seenThrough, requestId = newRequestId(), signal } = {}) {
    const ref = this.ref(id);
    let body;
    try {
      body = api.encodeAnswers(requestId, seen(seenThrough), answers);
    } catch (error) {
      throw errors.badInput(error.message);
    }
    return this.call(api.routes.sendMessage(ref, body), api.decodeWriteResult, { signal });
  }

  async sendMessage(id, message, { seenThrough, requestId = newRequestId(), signal } = {}) {
    const ref = this.ref(id);
    let body;
    try {
      body = api.encodeMessage(requestId, seen(seenThrough), message ?? {});
    } catch (error) {
      throw errors.badInput(error.message);
    }
    return this.call(api.routes.sendMessage(ref, body), api.decodeWriteResult, { signal });
  }

  async markRead(id, through, options) {
    await this.request(api.routes.markRead(this.ref(id), seen(through)), options);
  }

  async archive(id, through, options) {
    await this.request(api.routes.archive(this.ref(id), seen(through)), options);
  }

  async listSessions({ signal, maxPages = 20 } = {}) {
    const sessions = [];
    let cursor;
    let pages = 0;
    do {
      const page = await this.call(api.routes.listSessions({ cursor }), api.decodeSessionsPage, { signal });
      sessions.push(...page.sessions);
      cursor = page.next;
      pages += 1;
    } while (cursor !== undefined && pages < maxPages);
    return { sessions, truncated: cursor !== undefined };
  }

  // One long poll: threads with unread messages newer than since.
  waitOnce({ since, waitSeconds = this.waitSeconds, signal } = {}) {
    return this.call(api.routes.wait({ since, wait: waitSeconds }), api.decodeWait, {
      signal,
      // The server holds the request up to waitSeconds; allow for the trip.
      timeoutMs: waitSeconds * 1000 + this.timeoutMs,
      retries: 0,
    });
  }

  // Waits for new messages until signal aborts or an error a retry can't
  // fix. onThreads(threads, since) hears of each answer holding threads;
  // onState({state, error?, retryInMs?}) of the connection: "live" while
  // waits succeed, "retrying" while backing off, "stopped" at the end.
  // Resolves with the error that stopped it, or undefined when cancelled.
  async waitLoop({ signal, since, onThreads = () => {}, onState = () => {}, maxBackoffMs = 60_000 } = {}) {
    let cursor = since;
    let failures = 0;
    let live = false;
    for (;;) {
      if (signal?.aborted) break;
      try {
        const answer = await this.waitOnce({ since: cursor, signal });
        if (!live || failures > 0) onState({ state: "live" });
        live = true;
        failures = 0;
        if (answer.since !== undefined) cursor = answer.since;
        if (answer.threads.length > 0) await onThreads(answer.threads, cursor);
      } catch (error) {
        const e = error instanceof AdapterError ? error : classifyFetchError(error);
        if (e.kind === "cancelled" || signal?.aborted) break;
        if (!e.retryable) {
          onState({ state: "stopped", error: e });
          return e;
        }
        const delay =
          e.kind === "rate_limited"
            ? e.retryAfterMs
            : backoffMs(failures, { baseMs: 1000, maxMs: maxBackoffMs, random: this.random });
        failures += 1;
        live = false;
        onState({ state: "retrying", error: e, retryInMs: delay });
        try {
          await this.sleep(delay, signal);
        } catch {
          break;
        }
      }
    }
    onState({ state: "stopped" });
    return undefined;
  }
}

function seen(value) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw errors.badInput("the latest message seen isn't known; open the thread first");
  }
  return value;
}

export function newRequestId() {
  return `pitwall-${randomUUID()}`;
}
