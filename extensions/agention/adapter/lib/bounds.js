// Checks for data read from another party (the Agention server): types,
// sizes and counts. A value of the wrong type is refused with BadData, naming
// where it was; text that is merely too long is cut, since a long title is
// still worth showing.

export class BadData extends Error {
  constructor(where, what) {
    super(`${where}: ${what}`);
    this.name = "BadData";
    this.where = where;
  }
}

const MAX_SAFE = Number.MAX_SAFE_INTEGER;

export function isObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function object(value, where) {
  if (!isObject(value)) throw new BadData(where, "expected an object");
  return value;
}

// A whole number from 0 (or min) to max.
export function int(value, where, { min = 0, max = MAX_SAFE } = {}) {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < min || value > max) {
    throw new BadData(where, `expected a whole number from ${min} to ${max}`);
  }
  return value;
}

export function optionalInt(value, where, opts) {
  return value === undefined || value === null ? undefined : int(value, where, opts);
}

export function bool(value, where) {
  if (typeof value !== "boolean") throw new BadData(where, "expected true or false");
  return value;
}

export function optionalBool(value, where, fallback = false) {
  return value === undefined || value === null ? fallback : bool(value, where);
}

// Control characters other than tab and newline, and the Unicode bidi
// overrides, which could make a line read as something it isn't.
// eslint-disable-next-line no-control-regex
const UNSAFE = /[\u0000-\u0008\u000B-\u001F\u007F-\u009F‪-‮⁦-⁩]/g;

function cut(text, max) {
  if (text.length <= max) return text;
  // Don't leave half a surrogate pair at the end.
  let end = max;
  const code = text.charCodeAt(end - 1);
  if (code >= 0xd800 && code <= 0xdbff) end -= 1;
  return `${text.slice(0, end)}…`;
}

// Text of several lines (Markdown, for instance), cut to max characters.
export function text(value, where, max) {
  if (typeof value !== "string") throw new BadData(where, "expected text");
  return cut(value.replace(/\r\n?/g, "\n").replace(UNSAFE, ""), max);
}

export function optionalText(value, where, max, fallback = "") {
  return value === undefined || value === null ? fallback : text(value, where, max);
}

// One line of text: newlines and tabs become spaces.
export function line(value, where, max) {
  if (typeof value !== "string") throw new BadData(where, "expected text");
  return cut(value.replace(/[\r\n\t]+/g, " ").replace(UNSAFE, "").trim(), max);
}

export function optionalLine(value, where, max, fallback = "") {
  return value === undefined || value === null ? fallback : line(value, where, max);
}

// A name matching pattern exactly; these are used in request paths, so one
// that doesn't match is refused, never cut or cleaned.
export function name(value, where, pattern) {
  if (typeof value !== "string" || !pattern.test(value)) {
    throw new BadData(where, "expected a name of the form the API defines");
  }
  return value;
}

export function oneOf(value, where, allowed, fallback) {
  if (typeof value === "string" && allowed.includes(value)) return value;
  if (fallback !== undefined && (value === undefined || value === null || typeof value === "string")) {
    // A value a later server version adds is read as the fallback.
    return fallback;
  }
  throw new BadData(where, `expected one of ${allowed.join(", ")}`);
}

// A list of at most max items, each read by each(item, where).
export function list(value, where, max, each) {
  if (!Array.isArray(value)) throw new BadData(where, "expected a list");
  if (value.length > max) throw new BadData(where, `more than ${max} items`);
  return value.map((item, i) => each(item, `${where}[${i}]`));
}

export function optionalList(value, where, max, each) {
  return value === undefined || value === null ? [] : list(value, where, max, each);
}

// A time as ISO 8601 text; one that doesn't parse is refused.
export function time(value, where) {
  if (typeof value !== "string" || value.length > 64) throw new BadData(where, "expected a time");
  const ms = Date.parse(value);
  if (Number.isNaN(ms)) throw new BadData(where, "expected a time");
  return new Date(ms).toISOString();
}

export function optionalTime(value, where) {
  return value === undefined || value === null ? undefined : time(value, where);
}
