// A machine's New-agent form (pitwall-proto `create.rs`), as data: which
// fields apply, the values to send, and the summary line. Mirrors
// `CreateForm::{values, summarize}` in Rust (same cases in the tests), so the
// dialog never needs to know which provider it is (architecture.md §3).
import type { CreateChoice, CreateField, CreateForm, NameRule } from "../types";

export type Values = Record<string, string>;

/** Whether `f` applies, given the values chosen so far. */
export function shown(f: CreateField, values: Values): boolean {
  return !f.when || values[f.when.field] === f.when.value;
}

/** What gets sent: chosen values (else defaults) for the fields that apply; empty text left out. */
export function formValues(form: CreateForm, chosen: Values): Values {
  const out: Values = {};
  for (const f of form.fields) {
    if (!shown(f, out)) continue;
    const given = (chosen[f.id] ?? "").trim();
    const v = given || f.default || "";
    if (v) out[f.id] = v;
  }
  return out;
}

function choiceOf(f: CreateField, values: Values): CreateChoice | undefined {
  return f.choices.find((c) => c.value === values[f.id]);
}

function fill(form: CreateForm, t: string, name: string, values: Values, depth: number): string {
  const text = t.replace(/\{([^}]*)\}/g, (_, key: string) => valueText(form, key, name, values, depth));
  return text.split(/\s+/).filter(Boolean).join(" ").replace(/ ,/g, ",").replace(/ \./g, ".");
}

function valueText(form: CreateForm, key: string, name: string, values: Values, depth: number): string {
  if (key === "name") return name;
  const f = form.fields.find((x) => x.id === key);
  if (!f || !shown(f, values)) return "";
  if (f.input === "text") return values[key] ?? (f.defaultsToName ? name : "");
  const c = choiceOf(f, values);
  if (!c) return "";
  return c.phrase && depth < 2 ? fill(form, c.phrase, name, values, depth + 1) : c.value;
}

export interface Summary {
  text: string;
  /** Other things made on the machine (a new workspace, a new agent user). */
  creates: string[];
}

export function summarize(form: CreateForm, name: string, values: Values): Summary {
  const creates = form.fields
    .filter((f) => shown(f, values))
    .map((f) => choiceOf(f, values)?.creates)
    .filter((t): t is string => !!t)
    .map((t) => fill(form, t, name, values, 0));
  return { text: form.summary ? fill(form, form.summary, name, values, 0) : "", creates };
}

/** Why `name` doesn't fit `rule`, or null. */
export function nameProblem(rule: NameRule, name: string): string | null {
  if (name.length > rule.maxLen) return `At most ${rule.maxLen} characters`;
  let re: RegExp;
  try {
    re = new RegExp(rule.pattern);
  } catch {
    return null;
  }
  return re.test(name) ? null : rule.hint;
}
