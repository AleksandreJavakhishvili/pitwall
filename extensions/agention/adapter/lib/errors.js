// What went wrong, as a kind the UI can act on and a sentence a person can
// read. Messages never hold the API key, and never echo what the server
// said word for word where that could be long or hostile; the server's
// message is kept apart, bounded, in `detail`.

export const NEEDED_SCOPES = Object.freeze(["threads:write", "sessions:read"]);

// Kinds a retry can't fix: the person has to change a setting.
const NEEDS_PERSON = new Set([
  "no_key",
  "bad_url",
  "insecure_url",
  "key_rejected",
  "missing_scope",
  "not_agention",
  "too_old",
  "unsupported_version",
]);

export class AdapterError extends Error {
  constructor(kind, message, extra = {}) {
    super(message);
    this.name = "AdapterError";
    this.kind = kind;
    Object.assign(this, extra);
  }

  // Whether trying again later, unchanged, may work.
  get retryable() {
    return !NEEDS_PERSON.has(this.kind) && this.kind !== "cancelled";
  }

  // What the stdio layer sends: kind, message and the few fields a UI uses.
  toJSON() {
    const out = { kind: this.kind, message: this.message };
    for (const field of ["scope", "status", "code", "retryAfterMs", "detail"]) {
      if (this[field] !== undefined) out[field] = this[field];
    }
    return out;
  }
}

export const errors = {
  noKey: () =>
    new AdapterError(
      "no_key",
      "No Agention API key is set. Create one in Agention under Settings → API keys, with the threads:write and sessions:read scopes, and paste it here.",
    ),
  badKeyFormat: () =>
    new AdapterError(
      "no_key",
      "That doesn't look like an Agention API key; a key starts with agention_ followed by 43 letters, digits, - and _.",
    ),
  noUrl: () => new AdapterError("bad_url", "No Agention server address is set."),
  badUrl: () =>
    new AdapterError(
      "bad_url",
      "The Agention server address isn't a web address; it should look like https://agention.example.ts.net.",
    ),
  insecureUrl: () =>
    new AdapterError(
      "insecure_url",
      "The Agention server address must start with https:// (plain http:// is allowed only for this computer), so the API key isn't sent in the clear.",
    ),
  keyRejected: (detail) =>
    new AdapterError(
      "key_rejected",
      "Agention refused the API key: it is mistyped, revoked or expired. Create a new one in Agention under Settings → API keys.",
      { status: 401, detail },
    ),
  missingScope: (scope, status, detail) =>
    new AdapterError(
      "missing_scope",
      `The API key doesn't have the ${scope} scope. Create a key with the threads:write and sessions:read scopes in Agention under Settings → API keys.`,
      { scope, status, detail },
    ),
  notAgention: (why) =>
    new AdapterError(
      "not_agention",
      `That address doesn't answer like an Agention server (${why}). Check the address: it is the one you open Agention at in your browser.`,
    ),
  tooOld: () =>
    new AdapterError(
      "too_old",
      "This Agention server has no operator API. Update Agention to the version that adds API keys.",
    ),
  unsupportedVersion: (versions) =>
    new AdapterError(
      "unsupported_version",
      `This Agention server serves operator API version ${versions.join(", ") || "none"}, and this extension speaks version 1. Update the extension.`,
    ),
  network: (detail) =>
    new AdapterError(
      "network",
      "Can't reach the Agention server: the network is down, the address is wrong, or the server isn't running.",
      { detail },
    ),
  timeout: () =>
    new AdapterError("timeout", "The Agention server took too long to answer."),
  tls: (detail) =>
    new AdapterError(
      "tls",
      "Couldn't make a secure connection to the Agention server: its certificate isn't valid for this address or isn't trusted.",
      { detail },
    ),
  rateLimited: (retryAfterMs) =>
    new AdapterError(
      "rate_limited",
      `Agention asked to slow down; trying again in ${Math.ceil(retryAfterMs / 1000)} seconds.`,
      { status: 429, retryAfterMs },
    ),
  notFound: (detail) =>
    new AdapterError("not_found", "That thread isn't on the Agention server any more.", {
      status: 404,
      detail,
    }),
  stale: (missed) =>
    new AdapterError(
      "stale",
      "The agent wrote something new in this thread; read it, then send again.",
      { status: 409, code: "stale", missed },
    ),
  invalidInput: (detail, field) =>
    new AdapterError("invalid_input", `Agention refused what was sent${field ? ` (${field})` : ""}.`, {
      status: 400,
      detail,
    }),
  badInput: (detail) =>
    new AdapterError("invalid_input", `That can't be sent: ${detail}.`, { detail }),
  server: (status, detail) =>
    new AdapterError("server", `The Agention server had a problem (status ${status}).`, {
      status,
      detail,
    }),
  badResponse: (detail) =>
    new AdapterError(
      "bad_response",
      "The Agention server sent an answer this extension couldn't read.",
      { detail },
    ),
  cancelled: () => new AdapterError("cancelled", "Stopped."),
};

// Error codes in a TLS failure: certificate problems, and a handshake that
// failed (such as https:// to a plain-http port).
const TLS_CODE = /^(ERR_TLS_|ERR_SSL_|CERT_|UNABLE_TO_|DEPTH_ZERO_|SELF_SIGNED_|EPROTO$|HOSTNAME_MISMATCH|ERR_TLS_CERT_ALTNAME_INVALID)/;
const NETWORK_CODE = /^(ECONNREFUSED|ECONNRESET|ENOTFOUND|EAI_AGAIN|EHOSTUNREACH|ENETUNREACH|ENETDOWN|ETIMEDOUT|EPIPE|ECONNABORTED|UND_ERR_)/;

// classifyFetchError reads what fetch threw (a TypeError whose cause, or the
// cause's cause, has a code) into an AdapterError.
export function classifyFetchError(error, { timedOut = false, cancelled = false } = {}) {
  if (error instanceof AdapterError) return error;
  if (cancelled) return errors.cancelled();
  if (timedOut || error?.name === "TimeoutError") return errors.timeout();
  const codes = [];
  for (let e = error, depth = 0; e && depth < 5; e = e.cause, depth += 1) {
    if (typeof e.code === "string") codes.push(e.code);
    if (Array.isArray(e.errors)) {
      for (const inner of e.errors) if (typeof inner?.code === "string") codes.push(inner.code);
    }
  }
  const tlsCode = codes.find((code) => TLS_CODE.test(code));
  if (tlsCode) return errors.tls(tlsCode);
  const netCode = codes.find((code) => NETWORK_CODE.test(code));
  if (netCode === "UND_ERR_HEADERS_TIMEOUT" || netCode === "UND_ERR_BODY_TIMEOUT") {
    return errors.timeout();
  }
  return errors.network(netCode ?? codes[0] ?? "fetch failed");
}
