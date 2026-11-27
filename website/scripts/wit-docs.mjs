// The WIT reference: turns the tile world's WIT files and their `///` doc comments
// into Markdown (docs/spec/tile-sdk/authoring.md §9.5, §9.6). The site build puts
// it where docs/reference/extensions/wit.md has a <!-- wit-reference --> marker,
// and into /sdk/llms.txt; the WIT files stay the only source.
//
//   node scripts/wit-docs.mjs                     # Markdown for crates/pitwall-tile-host/wit/ on stdout
//   node scripts/wit-docs.mjs --wit <dir>         # another folder of .wit files
//   node scripts/wit-docs.mjs --out <file.md>     # write it to a file
//   node scripts/wit-docs.mjs --strict            # also fail when an item has no doc comment
//
// Only the folder's own *.wit files are read (not wit/deps/). With no WIT there it
// prints a note and exits 0, so the site builds before the tile host exists.
//
// It parses the parts of WIT a reference needs (packages, interfaces, worlds,
// records, variants, enums, flags, resources, type aliases, functions, imports and
// exports); `use`, `include` and gates such as @since(…) are read past. Not a
// validator: wasm-tools and the host's build check the WIT itself.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const ROOT = resolve(import.meta.dirname, "../..");
// PITWALL_WIT_DIR points the site build at another folder (to preview a WIT that isn't on this branch).
export const WIT_DIR = process.env.PITWALL_WIT_DIR ? resolve(process.env.PITWALL_WIT_DIR) : resolve(ROOT, "crates/pitwall-tile-host/wit");

// ---------- tokens ----------

function tokenize(src) {
  const toks = [];
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (/\s/.test(c)) {
      i++;
    } else if (src.startsWith("///", i)) {
      const end = src.indexOf("\n", i);
      const stop = end === -1 ? src.length : end;
      toks.push({ t: "doc", v: src.slice(i + 3, stop).replace(/^ /, ""), s: i, e: stop });
      i = stop;
    } else if (src.startsWith("/**", i) && !src.startsWith("/**/", i)) {
      const end = src.indexOf("*/", i);
      const stop = end === -1 ? src.length : end + 2;
      const body = src.slice(i + 3, stop - 2).split("\n").map((l) => l.replace(/^\s*\* ?/, ""));
      for (const v of body.join("\n").trim().split("\n")) toks.push({ t: "doc", v, s: i, e: stop });
      i = stop;
    } else if (src.startsWith("//", i)) {
      const end = src.indexOf("\n", i);
      i = end === -1 ? src.length : end;
    } else if (src.startsWith("/*", i)) {
      const end = src.indexOf("*/", i);
      i = end === -1 ? src.length : end + 2;
    } else if (src.startsWith("->", i)) {
      toks.push({ t: "p", v: "->", s: i, e: i + 2 });
      i += 2;
    } else if (/[A-Za-z0-9%_]/.test(c)) {
      const m = src.slice(i).match(/^%?[A-Za-z0-9_][A-Za-z0-9_-]*/);
      toks.push({ t: "id", v: m[0].replace(/^%/, ""), s: i, e: i + m[0].length });
      i += m[0].length;
    } else {
      toks.push({ t: "p", v: c, s: i, e: i + 1 });
      i++;
    }
  }
  return toks;
}

// ---------- parser ----------

const ITEM_KINDS = new Set(["record", "variant", "enum", "flags", "resource", "type"]);

class Parser {
  constructor(src, file) {
    this.src = src;
    this.file = file;
    this.toks = tokenize(src);
    this.i = 0;
  }
  peek(o = 0) {
    return this.toks[this.i + o];
  }
  next() {
    return this.toks[this.i++];
  }
  is(v, o = 0) {
    const t = this.peek(o);
    return t && t.t !== "doc" && t.v === v;
  }
  fail(msg) {
    const t = this.peek();
    const line = t ? this.src.slice(0, t.s).split("\n").length : "end";
    throw new Error(`${this.file}:${line}: ${msg}`);
  }
  expect(v) {
    if (!this.is(v)) this.fail(`expected "${v}"`);
    return this.next();
  }
  ident() {
    const t = this.next();
    if (!t || t.t !== "id") this.fail("expected a name");
    return t.v;
  }
  /** Doc lines before an item, then any gates (@since(…), @unstable(…), @deprecated(…)). */
  docs() {
    const lines = [];
    for (;;) {
      if (this.peek()?.t === "doc") lines.push(this.next().v);
      else if (this.is("@")) {
        this.next();
        this.ident();
        if (this.is("(")) this.skipBalanced();
      } else break;
    }
    return lines.join("\n").trim();
  }
  /** Skips a (…), {…} or <…> group starting at the current token. */
  skipBalanced() {
    const open = this.next().v;
    const close = { "(": ")", "{": "}", "<": ">" }[open];
    let depth = 1;
    while (depth && this.peek()) {
      const t = this.next();
      if (t.t === "doc") continue;
      if (t.v === open) depth++;
      else if (t.v === close) depth--;
    }
  }
  /** Source text from the current token up to (not including) one of `stops` at depth 0. */
  textUntil(stops) {
    const from = this.peek();
    let last = null;
    let depth = 0;
    while (this.peek()) {
      const t = this.peek();
      if (t.t === "doc") {
        this.next();
        continue;
      }
      if (depth === 0 && stops.includes(t.v)) break;
      if ("({<".includes(t.v)) depth++;
      if (")}>".includes(t.v)) depth--;
      last = this.next();
    }
    if (!last) return "";
    return squash(this.src.slice(from.s, last.e));
  }
  skipStatement() {
    this.textUntil([";"]);
    if (this.is(";")) this.next();
  }

  file_() {
    const pkg = { name: null, docs: "", interfaces: [], worlds: [] };
    while (this.peek()) {
      const docs = this.docs();
      if (!this.peek()) break;
      if (this.is("package")) {
        this.next();
        pkg.name = this.textUntil([";", "{"]);
        pkg.docs = docs;
        if (this.is(";")) this.next();
        else if (this.is("{")) this.skipBalanced(); // nested package: not documented here
      } else if (this.is("interface")) {
        this.next();
        pkg.interfaces.push(this.interface_(this.ident(), docs));
      } else if (this.is("world")) {
        this.next();
        pkg.worlds.push(this.world_(this.ident(), docs));
      } else if (this.is("use")) {
        this.skipStatement();
      } else {
        this.fail(`unexpected "${this.peek().v}"`);
      }
    }
    return pkg;
  }

  interface_(name, docs) {
    const iface = { name, docs, items: [] };
    this.expect("{");
    while (!this.is("}")) {
      if (!this.peek()) this.fail("unclosed interface");
      const d = this.docs();
      if (this.is("}")) break;
      const item = this.item(d);
      if (item) iface.items.push(item);
    }
    this.expect("}");
    return iface;
  }

  world_(name, docs) {
    const world = { name, docs, items: [] };
    this.expect("{");
    while (!this.is("}")) {
      if (!this.peek()) this.fail("unclosed world");
      const d = this.docs();
      if (this.is("}")) break;
      if (this.is("import") || this.is("export")) {
        const dir = this.next().v;
        if (this.peek()?.t === "id" && this.is(":", 1)) {
          const n = this.ident();
          this.next();
          if (this.is("interface")) {
            this.next();
            const inner = this.interface_(n, d);
            world.items.push({ kind: "interface", dir, name: n, docs: d, iface: inner });
            if (this.is(";")) this.next();
          } else {
            const sig = this.textUntil([";"]);
            this.expect(";");
            world.items.push({ kind: "func", dir, name: n, docs: d, sig: `${dir} ${n}: ${sig};` });
          }
        } else {
          const path = this.textUntil([";"]);
          this.expect(";");
          world.items.push({ kind: "ref", dir, name: path, docs: d, sig: `${dir} ${path};` });
        }
      } else if (this.is("include") || this.is("use")) {
        this.skipStatement();
      } else {
        const item = this.item(d);
        if (item) world.items.push(item);
      }
    }
    this.expect("}");
    return world;
  }

  /** A type or function inside an interface or world. */
  item(docs) {
    if (this.is("use")) {
      this.skipStatement();
      return null;
    }
    const t = this.peek();
    if (t.t === "id" && ITEM_KINDS.has(t.v) && this.peek(1)?.t === "id") {
      const kind = this.next().v;
      const name = this.ident();
      if (kind === "type") {
        this.expect("=");
        const def = this.textUntil([";"]);
        this.expect(";");
        return { kind, name, docs, sig: `type ${name} = ${def};` };
      }
      if (kind === "resource") {
        const res = { kind, name, docs, members: [] };
        if (this.is(";")) {
          this.next();
          return res;
        }
        this.expect("{");
        while (!this.is("}")) {
          if (!this.peek()) this.fail("unclosed resource");
          const md = this.docs();
          if (this.is("}")) break;
          if (this.is("constructor")) {
            this.next();
            const sig = this.textUntil([";"]);
            this.expect(";");
            res.members.push({ name: "constructor", docs: md, sig: `constructor${sig}` });
          } else {
            const n = this.ident();
            this.expect(":");
            const sig = this.textUntil([";"]);
            this.expect(";");
            res.members.push({ name: n, docs: md, sig: `${n}: ${sig}` });
          }
        }
        this.expect("}");
        return res;
      }
      // record, variant, enum, flags
      const it = { kind, name, docs, members: [] };
      this.expect("{");
      while (!this.is("}")) {
        if (!this.peek()) this.fail(`unclosed ${kind}`);
        const md = this.docs();
        if (this.is("}")) break;
        const n = this.ident();
        let type = "";
        if (kind === "record") {
          this.expect(":");
          type = this.textUntil([",", "}"]);
        } else if (kind === "variant" && this.is("(")) {
          this.next();
          type = this.textUntil([")"]);
          this.expect(")");
        }
        it.members.push({ name: n, type, docs: md });
        if (this.is(",")) this.next();
      }
      this.expect("}");
      return it;
    }
    if (t.t === "id" && this.is(":", 1)) {
      const name = this.ident();
      this.next();
      const sig = this.textUntil([";"]);
      this.expect(";");
      return { kind: "func", name, docs, sig: `${name}: ${sig};` };
    }
    this.fail(`unexpected "${t.v}"`);
  }
}

const squash = (s) =>
  s
    .replace(/\s+/g, " ")
    .replace(/\s*([,:)>])/g, "$1")
    .replace(/([(<])\s*/g, "$1")
    .replace(/,(?=\S)/g, ", ")
    .trim();

/** Parses one WIT file's text into { name, docs, interfaces, worlds }. */
export function parseWit(src, file = "wit") {
  return new Parser(src, file).file_();
}

// ---------- Markdown ----------

const cell = (s) => (s || "").replace(/\s*\n\s*/g, " ").replaceAll("|", "\\|");
const code = (s) => "`" + s + "`";

function declaration(it) {
  if (it.sig) return it.sig;
  const body = (m) => {
    if (it.kind === "record") return `${m.name}: ${m.type}`;
    if (it.kind === "variant" && m.type) return `${m.name}(${m.type})`;
    return m.name;
  };
  if (it.kind === "resource") {
    if (!it.members.length) return `resource ${it.name};`;
    return `resource ${it.name} {\n${it.members.map((m) => `  ${m.sig};`).join("\n")}\n}`;
  }
  return `${it.kind} ${it.name} {\n${it.members.map((m) => `  ${body(m)},`).join("\n")}\n}`;
}

// "function", "record", …; world items get their direction: "export, function".
const kindName = (it) => {
  const k = it.kind === "func" ? "function" : it.kind === "ref" ? "interface" : it.kind;
  return it.dir ? `${it.dir}, ${k}` : k;
};
// An imported or exported interface named by path ("wasi:io/streams@0.2.0") is shown by its last part.
const shortName = (it) => (it.kind === "ref" ? it.name.replace(/^.*[/:]/, "").replace(/@.*/, "") : it.name);

function itemMd(it, owner) {
  const out = [`### ${code(`${owner}.${shortName(it)}`)}`, "", `*${kindName(it)}*`, ""];
  if (it.docs) out.push(it.docs, "");
  if (it.kind === "interface") {
    out.push("```wit", `${it.dir} ${it.name}: interface { … }`, "```", "");
    for (const inner of it.iface.items) out.push(...itemMd(inner, `${owner}.${it.name}`));
    return out;
  }
  out.push("```wit", declaration(it), "```", "");
  // A table of members only when one has a doc comment; the declaration above lists them anyway.
  if (it.members?.some((m) => m.docs)) {
    if (it.kind === "record") out.push("| Field | Type | Description |", "|---|---|---|", ...it.members.map((m) => `| ${code(m.name)} | ${code(m.type)} | ${cell(m.docs)} |`));
    else if (it.kind === "variant") out.push("| Case | Payload | Description |", "|---|---|---|", ...it.members.map((m) => `| ${code(m.name)} | ${m.type ? code(m.type) : ""} | ${cell(m.docs)} |`));
    else if (it.kind === "resource") out.push("| Method | Signature | Description |", "|---|---|---|", ...it.members.map((m) => `| ${code(m.name)} | ${code(m.sig)} | ${cell(m.docs)} |`));
    else out.push(`| ${it.kind === "flags" ? "Flag" : "Case"} | Description |`, "|---|---|", ...it.members.map((m) => `| ${code(m.name)} | ${cell(m.docs)} |`));
    out.push("");
  }
  return out;
}

/**
 * Every WIT item without a doc comment, as "file: what" strings: worlds,
 * interfaces, types, functions, imports and exports, record fields and resource
 * methods. Enum, flags and variant cases may go without one.
 */
export function undocumented(pkgs) {
  const missing = [];
  for (const { file, pkg } of pkgs) {
    const walk = (items, owner) => {
      for (const it of items) {
        const name = `${owner}.${shortName(it)}`;
        if (!it.docs) missing.push(`${file}: ${kindName(it).replace(", ", " ")} ${name}`);
        if (it.kind === "record" || it.kind === "resource")
          for (const m of it.members) if (!m.docs) missing.push(`${file}: ${it.kind} ${name}, ${m.name}`);
        if (it.iface) walk(it.iface.items, name);
      }
    };
    for (const i of pkg.interfaces) {
      if (!i.docs) missing.push(`${file}: interface ${i.name}`);
      walk(i.items, i.name);
    }
    for (const w of pkg.worlds) {
      if (!w.docs) missing.push(`${file}: world ${w.name}`);
      walk(w.items, w.name);
    }
  }
  return missing;
}

/** Reads the folder's *.wit files: [{ file, pkg }], or [] when there are none. */
export function readWit(dir = WIT_DIR) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => f.endsWith(".wit"))
    .sort()
    .map((f) => {
      const path = resolve(dir, f);
      const file = relative(ROOT, path).split("\\").join("/");
      return { file, pkg: parseWit(readFileSync(path, "utf8"), file) };
    });
}

/**
 * The reference as Markdown: one "## " section per world and interface (worlds
 * first), one "### " per item. Returns "" when there is no WIT.
 */
export function witMarkdown(dir = WIT_DIR) {
  const pkgs = readWit(dir);
  if (!pkgs.length) return "";
  const names = [...new Set(pkgs.map((p) => p.pkg.name).filter(Boolean))];
  const out = [
    `<!-- Generated from ${pkgs.map((p) => p.file).join(", ")} by website/scripts/wit-docs.mjs. Edit the WIT's doc comments, not this. -->`,
    "",
    `Package ${names.map(code).join(", ") || "(unnamed)"}, generated from ${pkgs.map((p) => `[${code(p.file.replace(/^.*\//, ""))}](../../../${p.file})`).join(", ")}.`,
    "",
  ];
  for (const { pkg } of pkgs) if (pkg.docs) out.push(pkg.docs, "");
  for (const { pkg } of pkgs)
    for (const w of pkg.worlds) {
      out.push(`## World ${code(w.name)}`, "");
      if (w.docs) out.push(w.docs, "");
      for (const it of w.items) out.push(...itemMd(it, w.name));
    }
  for (const { pkg } of pkgs)
    for (const i of pkg.interfaces) {
      out.push(`## Interface ${code(i.name)}`, "");
      if (i.docs) out.push(i.docs, "");
      for (const it of i.items) out.push(...itemMd(it, i.name));
    }
  return out.join("\n").replace(/\n{3,}/g, "\n\n").trim() + "\n";
}

// ---------- CLI ----------

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const args = process.argv.slice(2);
  const opt = (name) => {
    const i = args.indexOf(name);
    return i === -1 ? undefined : args[i + 1];
  };
  const dir = resolve(process.cwd(), opt("--wit") ?? WIT_DIR);
  const pkgs = readWit(dir);
  if (!pkgs.length) {
    console.error(`wit-docs: no .wit files in ${relative(process.cwd(), dir) || dir}; nothing to generate`);
    process.exit(0);
  }
  const md = witMarkdown(dir);
  const out = opt("--out");
  if (out) writeFileSync(resolve(process.cwd(), out), md);
  else process.stdout.write(md);
  if (args.includes("--strict")) {
    const missing = undocumented(pkgs);
    if (missing.length) {
      console.error(`wit-docs: ${missing.length} WIT item(s) without a doc comment:\n  ${missing.join("\n  ")}`);
      process.exit(1);
    }
  }
}
