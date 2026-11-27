// Examples pulled from source files, so a docs page never holds example code of
// its own (docs/spec/tile-sdk/authoring.md §9.6, "Examples are compiled").
//
// In a docs Markdown file, a line inside a fenced code block
//
//   ```rust
//   {{#include crates/some-crate/src/lib.rs#add-shape}}
//   ```
//
// is replaced by the region `add-shape` of that file; without `#<region>` the
// whole file goes in. Paths are relative to the repository root. A region is
// marked in the source with comment lines holding `docs:start <name>` and
// `docs:end` (or `docs:end <name>`), in any comment syntax:
//
//   // docs:start add-shape
//   let shape = Shape::new();
//   // docs:end
//
// Regions may nest; marker lines of other regions are left out of what is
// included, and the region is dedented. A missing file or region throws, so the
// site build fails instead of publishing a stale or empty example.
import { existsSync, readFileSync, statSync } from "node:fs";
import { resolve, relative, isAbsolute } from "node:path";

const DIRECTIVE = /^([ \t]*)\{\{#include\s+([^\s}#]+)(?:#([\w.-]+))?\s*\}\}[ \t]*$/;
const START = /docs:start\s+([\w.-]+)/;
const END = /docs:end(?:\s+([\w.-]+))?/;

/** The lines of `region` in `text` (or all of it), without marker lines, dedented. */
export function extractRegion(text, region, where = "file") {
  const lines = text.replace(/\r\n?/g, "\n").replace(/\n$/, "").split("\n");
  let picked;
  if (!region) {
    picked = lines;
  } else {
    picked = null;
    const open = [];
    for (let i = 0; i < lines.length; i++) {
      const start = lines[i].match(START);
      const end = !start && lines[i].match(END);
      if (start) {
        open.push(start[1]);
        if (start[1] === region) {
          if (picked) throw new Error(`${where}: region "${region}" is marked more than once`);
          picked = [];
        }
        continue;
      }
      if (end) {
        const name = end[1] ?? open[open.length - 1];
        const at = open.lastIndexOf(name);
        if (at === -1) throw new Error(`${where}: "docs:end${end[1] ? " " + end[1] : ""}" on line ${i + 1} closes no region`);
        open.splice(at);
        if (name === region) {
          if (!picked) throw new Error(`${where}: region "${region}" ends before it starts`);
          picked.done = true;
        }
        continue;
      }
      if (picked && !picked.done && open.includes(region)) picked.push(lines[i]);
    }
    if (!picked) throw new Error(`${where}: no region "${region}" (mark it with docs:start ${region} … docs:end)`);
    if (!picked.done) throw new Error(`${where}: region "${region}" has no docs:end`);
  }
  const indent = Math.min(...picked.filter((l) => l.trim()).map((l) => l.match(/^[ \t]*/)[0].length));
  return picked.map((l) => l.slice(Number.isFinite(indent) ? indent : 0)).join("\n");
}

/**
 * Replaces every {{#include …}} line in `md` with the file or region it names.
 * `source` is the Markdown file's path (for error messages); `root` the repository.
 * Returns { md, files } where `files` lists the files read (for dev reloads).
 */
export function resolveIncludes(md, { root, source = "Markdown" }) {
  const files = [];
  const where = relative(root, source) || source;
  let fence = null;
  const out = md.split("\n").map((line, n) => {
    const f = line.match(/^\s*(`{3,}|~{3,})/);
    if (f) {
      if (!fence) fence = f[1];
      else if (line.trim().startsWith(fence)) fence = null;
      return line;
    }
    const m = line.match(DIRECTIVE);
    if (!m) return line;
    const [, indent, path, region] = m;
    const at = `${where}:${n + 1}`;
    if (!fence) throw new Error(`${at}: {{#include ${path}}} must be inside a fenced code block`);
    if (isAbsolute(path) || path.split("/").includes("..")) throw new Error(`${at}: include paths are relative to the repository root, without ".."`);
    const file = resolve(root, path);
    if (!existsSync(file) || !statSync(file).isFile()) throw new Error(`${at}: included file ${path} doesn't exist`);
    files.push(file);
    const body = extractRegion(readFileSync(file, "utf8"), region, `${at}: ${path}`);
    return body
      .split("\n")
      .map((l) => (l ? indent + l : l))
      .join("\n");
  });
  return { md: out.join("\n"), files };
}
