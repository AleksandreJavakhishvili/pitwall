// Tests for the docs pipeline: includes, the WIT reference, the link check and the
// Extensions pages. Run with `pnpm test` in website/.
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { extractRegion, resolveIncludes } from "./includes.mjs";
import { parseWit, readWit, undocumented, witMarkdown } from "./wit-docs.mjs";
import { checkSite } from "./check-links.mjs";
import { EXTENSION_PAGES, SOURCES, docsNavHtml, llmsTxt, renderPage, splitChildren } from "./pages.mjs";

const SITE = resolve(import.meta.dirname, "..");
const temp = () => mkdtempSync(join(tmpdir(), "pitwall-docs-"));

// ---------- includes ----------

const SOURCE = `fn main() {
    // docs:start outer
    let a = 1;
    // docs:start inner
    let b = 2;
    // docs:end inner
    let c = 3;
    // docs:end
}
`;

test("a region is included without marker lines, dedented", () => {
  assert.equal(extractRegion(SOURCE, "outer"), "let a = 1;\nlet b = 2;\nlet c = 3;");
  assert.equal(extractRegion(SOURCE, "inner"), "let b = 2;");
  assert.equal(extractRegion("x\ny\n"), "x\ny");
});

test("a missing or unclosed region throws", () => {
  assert.throws(() => extractRegion(SOURCE, "nope"), /no region "nope"/);
  assert.throws(() => extractRegion("// docs:start a\nx\n", "a"), /has no docs:end/);
});

test("{{#include}} lines in a fence are replaced; missing files fail the build", () => {
  const root = temp();
  mkdirSync(join(root, "src"));
  writeFileSync(join(root, "src/lib.rs"), SOURCE);
  const md = "# T\n\n```rust\n  {{#include src/lib.rs#inner}}\n```\n";
  const { md: out, files } = resolveIncludes(md, { root });
  assert.equal(out, "# T\n\n```rust\n  let b = 2;\n```\n");
  assert.deepEqual(files, [join(root, "src/lib.rs")]);
  assert.throws(() => resolveIncludes("```\n{{#include src/gone.rs}}\n```", { root }), /doesn't exist/);
  assert.throws(() => resolveIncludes("```\n{{#include src/lib.rs#gone}}\n```", { root }), /no region "gone"/);
  assert.throws(() => resolveIncludes("{{#include src/lib.rs}}", { root }), /inside a fenced code block/);
  assert.throws(() => resolveIncludes("```\n{{#include ../x.rs}}\n```", { root }), /relative to the repository root/);
});

// ---------- WIT reference ----------

const WIT = `/// The sample package.
package pitwall:sample@0.1.0;

/// Things to draw.
interface ui {
  /// A colour by meaning.
  enum tone { good, bad }

  /// Style switches.
  flags style { bold, italic }

  /// A rectangle.
  record rect {
    /// Left edge.
    x: f32,
    width: f32,
  }

  /// A length.
  variant length { px(f32), auto }

  /// A list of lengths.
  type lengths = list<length>;

  /// A handle to a canvas.
  resource canvas {
    /// Makes one.
    constructor(width: u32, height: u32);
    /// Clears it.
    clear: func();
  }

  /// Draws a rectangle.
  @since(version = 0.1.0)
  draw: func(r: rect, tone: option<tone>) -> result<_, string>;
}

/// What a tile is.
world tile {
  use ui.{rect};
  import ui;
  /// Renders the tile.
  export render: func() -> list<rect>;
  export events: interface {
    /// Handles a click.
    click: func(x: f32, y: f32);
  }
}
`;

test("the WIT parser reads packages, interfaces, worlds and their items", () => {
  const pkg = parseWit(WIT, "sample.wit");
  assert.equal(pkg.name, "pitwall:sample@0.1.0");
  const ui = pkg.interfaces[0];
  assert.deepEqual(
    ui.items.map((i) => [i.kind, i.name]),
    [["enum", "tone"], ["flags", "style"], ["record", "rect"], ["variant", "length"], ["type", "lengths"], ["resource", "canvas"], ["func", "draw"]],
  );
  assert.equal(ui.items[6].sig, "draw: func(r: rect, tone: option<tone>) -> result<_, string>;");
  assert.equal(ui.items[6].docs, "Draws a rectangle.");
  const world = pkg.worlds[0];
  assert.deepEqual(world.items.map((i) => [i.dir, i.kind, i.name]), [["import", "ref", "ui"], ["export", "func", "render"], ["export", "interface", "events"]]);
});

test("the WIT reference is Markdown with a section per world and interface", () => {
  const dir = temp();
  writeFileSync(join(dir, "sample.wit"), WIT);
  const md = witMarkdown(dir);
  assert.match(md, /^## World `tile`$/m);
  assert.match(md, /^## Interface `ui`$/m);
  assert.match(md, /^### `ui.draw`$/m);
  assert.match(md, /^### `tile.events.click`$/m);
  assert.match(md, /\| `x` \| `f32` \| Left edge. \|/);
  assert.match(md, /type lengths = list<length>;/);
});

test("undocumented WIT items are reported; no WIT is not an error", () => {
  const dir = temp();
  writeFileSync(join(dir, "sample.wit"), WIT);
  const missing = undocumented(readWit(dir)).map((m) => m.replace(/^.*?: /, ""));
  assert.deepEqual(missing.sort(), ["export interface tile.events", "import interface tile.ui", "record ui.rect, width"].sort());
  assert.deepEqual(readWit(join(dir, "nowhere")), []);
  assert.equal(witMarkdown(join(dir, "nowhere")), "");
});

// ---------- link check ----------

test("the link check finds missing pages, anchors and unfilled placeholders", () => {
  const dist = temp();
  const page = (path, body) => {
    mkdirSync(join(dist, path, ".."), { recursive: true });
    writeFileSync(join(dist, path), `<!doctype html><html><body>${body}</body></html>`);
  };
  page("index.html", `<h1 id="top">Home</h1><a href="/b/docs/">ok</a><a href="/b/docs/#intro">ok</a><a href="#top">ok</a><a href="https://example.com/">ext</a>`);
  page("docs/index.html", `<h2 id="intro">Intro</h2><a href="../">ok</a><a href="/b/docs/#nope">bad anchor</a><a href="/b/gone/">bad page</a><a href="%REPO_URL%/x">bad</a><a href="/docs/">outside base</a><script>var x = '<a href="/b/fake/">';</script>`);
  const { broken, external, pages } = checkSite(dist, { base: "/b/" });
  assert.equal(pages, 2);
  assert.equal(external.length, 1);
  assert.deepEqual(
    broken.map((b) => b.href),
    ["/b/docs/#nope", "/b/gone/", "%REPO_URL%/x", "/docs/"],
  );
});

// ---------- Extensions pages ----------

test("every Extensions page has a source, an HTML page and a sidebar entry", () => {
  const nav = docsNavHtml("/docs/extensions/index.html");
  assert.match(nav, /Extensions <span class="pill">Preview<\/span>/);
  assert.match(nav, /<a href="\/docs\/extensions\/" aria-current="page">Overview<\/a>/);
  for (const [name, , path] of EXTENSION_PAGES) {
    assert.ok(existsSync(SOURCES[name]), `${SOURCES[name]} exists`);
    const html = readFileSync(join(SITE, `.${path}index.html`), "utf8");
    assert.ok(html.includes(`<!-- markdown:${name} -->`), `${path} has its marker`);
    assert.ok(nav.includes(`href="${path}"`), `${path} is in the sidebar`);
    const out = renderPage(name, { repoUrl: "https://github.com/o/r" });
    assert.match(out, /<div class="docs-title"><h1>.+<\/h1><span class="pill">Preview<\/span><\/div>/);
    if (path === "/docs/extensions/") assert.match(out, /^<p class="label">Extensions<\/p>/);
    else assert.match(out, /^<nav class="eyebrow crumbs" aria-label="Breadcrumb"><ol><li><a href="\/docs\/">Docs<\/a><\/li><li><a href="\/docs\/extensions\/">Extensions<\/a><\/li><\/ol><\/nav>/);
    assert.match(out, /<nav class="docs-pager"/);
  }
});

// ---------- split pages ----------

test("the CLI reference is split into a page per command, its source stays one file", () => {
  const children = splitChildren("cli");
  assert.ok(children.length > 5);
  const agent = children.find((c) => c.slug === "agent");
  assert.equal(agent.path, "/docs/cli/agent/");
  assert.match(agent.md, /^# pitwall agent\n/);
  assert.match(agent.md, /^## agent list$/m, "### headings move up a level");
  const parent = renderPage("cli", { repoUrl: "https://github.com/o/r" });
  assert.match(parent, /<h2 id="global-options">/);
  assert.doesNotMatch(parent, /<h2 id="agent">/);
  assert.match(parent, /<a href="\/docs\/cli\/agent\/"><code>agent<\/code><\/a>/, "the command list links to the child pages");
  const moved = JSON.parse(parent.match(/id="moved-anchors">(.*?)<\/script>/)[1]);
  assert.equal(moved.agent, "agent/");
  assert.equal(moved["agent-list"], "agent/#agent-list");
  const child = renderPage("cli/agent", { repoUrl: "https://github.com/o/r" });
  assert.match(child, /<h1>pitwall agent<\/h1>/);
  assert.match(child, /<h2 id="agent-list">/);
  assert.match(child, /<a href="\/docs\/cli\/">CLI<\/a>/);
  assert.match(child, /rel="prev"><span>Previous<\/span><strong>CLI<\/strong>/);
});

test("the sidebar nests a parent's pages, open only in its family", () => {
  const onAgent = docsNavHtml("/docs/cli/agent/index.html");
  assert.match(onAgent, /<details open><summary><a href="\/docs\/cli\/">CLI<\/a><\/summary>/);
  assert.match(onAgent, /<a href="\/docs\/cli\/agent\/" aria-current="page">agent<\/a>/);
  assert.match(onAgent, /<nav class="docs-subnav" aria-label="CLI pages">/);
  const elsewhere = docsNavHtml("/docs/settings/index.html");
  assert.match(elsewhere, /<details><summary><a href="\/docs\/cli\/">CLI<\/a><\/summary>/);
  assert.doesNotMatch(elsewhere, /docs-subnav/);
});

test("the link check follows a split page's moved anchors", () => {
  const dist = temp();
  const page = (path, body) => {
    mkdirSync(join(dist, path, ".."), { recursive: true });
    writeFileSync(join(dist, path), `<!doctype html><html><body>${body}</body></html>`);
  };
  const map = `<script type="application/json" id="moved-anchors">{"a":"a/","a-x":"a/#a-x","gone":"b/#nope"}</script>`;
  page("cli/index.html", `<h2 id="top">CLI</h2>${map}`);
  page("cli/a/index.html", `<h1>a</h1><h2 id="a-x">x</h2>`);
  page("index.html", `<a href="/cli/#a">ok</a><a href="/cli/#a-x">ok</a><a href="/cli/#gone">bad</a><a href="/cli/#missing">bad</a>`);
  const { broken } = checkSite(dist);
  assert.deepEqual(broken.map((b) => b.href), ["/cli/#gone", "/cli/#missing"]);
});

test("llms.txt holds every Extensions page with absolute links", () => {
  const txt = llmsTxt({ repoUrl: "https://github.com/o/r", pagesUrl: "https://o.github.io/r/" });
  assert.match(txt, /^# Pitwall extensions \(Preview\)/);
  for (const [, , path] of EXTENSION_PAGES) assert.ok(txt.includes(`Source: https://o.github.io/r${path}`), path);
  assert.match(txt, /\]\(https:\/\/o\.github\.io\/r\/docs\/extensions\/quick-start\/\)/);
  assert.doesNotMatch(txt, /\]\((?!https?:)[^)]*\)/, "no relative links left");
  assert.doesNotMatch(txt, /<!--/);
});
