// Renders the repository's Markdown into site pages: CHANGELOG.md and ROADMAP.md
// into /changelog/ and /roadmap/, docs/reference/*.md into the reference pages and
// docs/reference/extensions/*.md into the Extensions pages under /docs/extensions/.
// vite.config.js replaces each page's <!-- markdown:<name> --> marker with this
// output at build (and dev) time, so the Markdown files stay the only source.
//
// Before rendering, {{#include <path>#<region>}} lines pull examples from source
// files (scripts/includes.mjs), and a <!-- wit-reference --> marker is replaced by
// the reference generated from the tile world's WIT (scripts/wit-docs.mjs).
//
// A page can have child pages, shown as a sub-list in the docs sidebar, with
// breadcrumbs and Previous / Next on each (FAMILIES below). Children come from
// their own Markdown files (the Extensions pages) or from one file split at its
// "## " sections at build time (SPLIT: the CLI reference), so that file stays one
// source. A split parent keeps its old #anchors working: a small script sends
// them on to the child page that now holds them.
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, posix, relative, resolve, sep } from "node:path";
import { render, sections, slug, inline } from "./markdown.mjs";
import { resolveIncludes } from "./includes.mjs";
import { witMarkdown, WIT_DIR } from "./wit-docs.mjs";

export const ROOT = resolve(import.meta.dirname, "../..");
export const SITE = resolve(import.meta.dirname, "..");

// The Extensions pages (docs/spec/tile-sdk/authoring.md §9.6), in sidebar order:
// [name, Markdown file in docs/reference/extensions/, path on the site, sidebar label].
// The first is the overview; the others are its child pages. Each has a page at
// website/docs/extensions/<page>/index.html holding a <!-- markdown:<name> -->
// marker. All carry the Preview pill until WIT 1.0.
export const EXTENSION_PAGES = [
  ["extensions", "README.md", "/docs/extensions/", "Overview"],
  ["extensions-quick-start", "quick-start.md", "/docs/extensions/quick-start/", "Quick start"],
  ["extensions-ui", "ui.md", "/docs/extensions/ui/", "Tiles and UI components"],
  ["extensions-components-forms", "components-forms.md", "/docs/extensions/components-forms/", "Form controls"],
  ["extensions-components-feedback", "components-feedback.md", "/docs/extensions/components-feedback/", "Feedback"],
  ["extensions-components-layout", "components-layout.md", "/docs/extensions/components-layout/", "Layout"],
  ["extensions-components-data", "components-data.md", "/docs/extensions/components-data/", "Data display"],
  ["extensions-components-charts", "components-charts.md", "/docs/extensions/components-charts/", "Charts"],
  ["extensions-canvas", "canvas.md", "/docs/extensions/canvas/", "Canvas and drag"],
  ["extensions-blocks", "blocks.md", "/docs/extensions/blocks/", "Native building blocks"],
  ["extensions-agent-tools", "agent-tools.md", "/docs/extensions/agent-tools/", "Agent tools"],
  ["extensions-seed-tiles", "seed-tiles.md", "/docs/extensions/seed-tiles/", "C4 diagrams and Notes"],
  ["extensions-permissions", "permissions.md", "/docs/extensions/permissions/", "Permissions"],
  ["extensions-packaging", "packaging.md", "/docs/extensions/packaging/", "Packaging"],
  ["extensions-install", "install.md", "/docs/extensions/install/", "Install from a file"],
  ["extensions-marketplace", "marketplace.md", "/docs/extensions/marketplace/", "Marketplace"],
  ["extensions-publishing", "publishing.md", "/docs/extensions/publishing/", "Publishing"],
  ["extensions-platform-services", "platform-services.md", "/docs/extensions/platform-services/", "Platform services"],
  ["extensions-servers", "servers.md", "/docs/extensions/servers/", "Servers in extensions"],
  ["extensions-wit", "wit.md", "/docs/extensions/wit/", "WIT reference 0.1"],
  ["extensions-for-agents", "for-agents.md", "/docs/extensions/for-agents/", "For coding agents"],
];

/**
 * The tile authoring skill (authoring.md §9.5), served for download at
 * /sdk/<SKILL.path> (the For coding agents page links to it). The file is the
 * SDK's own, which `pitwall extension new` also writes into new projects.
 */
export const SKILL = {
  path: "pitwall-extension-authoring/SKILL.md",
  source: resolve(ROOT, "sdk/pitwall-extension/skills/pitwall-extension-authoring/SKILL.md"),
};

export const SOURCES = {
  changelog: resolve(ROOT, "CHANGELOG.md"),
  roadmap: resolve(ROOT, "ROADMAP.md"),
  cli: resolve(ROOT, "docs/reference/cli.md"),
  settings: resolve(ROOT, "docs/reference/settings.md"),
  agents: resolve(ROOT, "docs/reference/agents.md"),
  detection: resolve(ROOT, "docs/reference/detection.md"),
  ...Object.fromEntries(EXTENSION_PAGES.map(([name, file]) => [name, resolve(ROOT, "docs/reference/extensions", file)])),
};

// Where each Markdown page is on the site.
const PAGE_PATH = {
  changelog: "/changelog/",
  roadmap: "/roadmap/",
  cli: "/docs/cli/",
  settings: "/docs/settings/",
  agents: "/docs/agents/",
  detection: "/docs/detection/",
  ...Object.fromEntries(EXTENSION_PAGES.map(([name, , path]) => [name, path])),
};

// Where each Markdown file is shown on the site (links between them stay on it).
const ON_SITE = Object.fromEntries(Object.entries(PAGE_PATH).map(([name, path]) => [relative(ROOT, SOURCES[name]).split(sep).join("/"), path]));

// How a reference page is labelled: the eyebrow above its title, and whether the
// Preview pill goes beside the title.
const PAGE_KIND = Object.fromEntries(EXTENSION_PAGES.map(([name]) => [name, { label: "Extensions", preview: true }]));

/**
 * Pages split into child pages at their "## " sections, at build time. The
 * sections named in `keep` stay on the parent; every other one becomes a child
 * page at <parent path><slug>/, named "<name>/<slug>", with its "### " headings
 * moved up a level. `heading` gives a child's page heading, `title` its <title>.
 * The child pages' HTML is generated from the parent's (writeChildPages) and not
 * committed (website/.gitignore).
 *
 * The CLI reference is split: one page per command. Settings (≈260 lines),
 * Agent definitions (≈280) and Detection rules (≈190) read as one flow and stay
 * whole.
 */
export const SPLIT = {
  cli: {
    keep: ["Install", "How it works", "Exit status", "Recipes", "Global options", "Commands"],
    heading: (h) => `pitwall ${h}`,
    title: (h) => `pitwall ${h}: Pitwall CLI reference`,
  },
};

/** The docs sidebar: groups of items; an item may have children (a sub-list). */
function docsNav() {
  const [overview, ...extensionChildren] = EXTENSION_PAGES;
  return [
    { title: "Start", items: [{ path: "/docs/", label: "Overview" }, { path: "/docs/quick-start/", label: "Quick start" }] },
    { title: "Guide", items: [{ path: "/docs/how-it-works/", label: "How it works" }] },
    {
      title: "Reference",
      items: [
        { path: "/docs/cli/", label: "CLI", children: splitChildren("cli").map((c) => ({ path: c.path, label: c.section })) },
        { path: "/docs/settings/", label: "Settings" },
        { path: "/docs/agents/", label: "Agent definitions" },
        { path: "/docs/detection/", label: "Detection rules" },
      ],
    },
    {
      // The group is the overview's family: its children are listed beside it, not under it.
      title: "Extensions",
      pill: "Preview",
      items: [{ path: overview[2], label: overview[3], crumb: "Extensions", flat: true, children: extensionChildren.map(([, , path, label]) => ({ path, label })) }],
    },
  ];
}

/** The family of `path`: { parent, children } when it is a parent or a child page, else null. */
function familyOf(path) {
  for (const group of docsNav())
    for (const item of group.items)
      if (item.children?.length && (item.path === path || item.children.some((c) => c.path === path))) return { parent: item, children: item.children };
  return null;
}

const esc = (s) => s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");

/**
 * The sidebar with `current` (a page path such as "/docs/cli/index.html") marked.
 * A parent's sub-list is open on its own pages and closed elsewhere. Narrow
 * screens show the top level as a row, and the current family's pages in a
 * second row (.docs-subnav).
 */
export function docsNavHtml(current) {
  const here = current.replace(/index\.html$/, "");
  const link = ({ path, label }) => `<a href="${path}"${path === here ? ' aria-current="page"' : ""}>${esc(label)}</a>`;
  const list = (items) => items.map((c) => `  <li>${link(c)}</li>`).join("\n");
  const groups = docsNav().map(({ title, pill, items }) => {
    const lis = items.map((item) => {
      if (!item.children?.length) return `  <li>${link(item)}</li>`;
      if (item.flat) return `  <li>${link(item)}</li>\n${list(item.children)}`;
      const open = item.path === here || item.children.some((c) => c.path === here);
      return `  <li class="docs-nav-parent"><details${open ? " open" : ""}><summary>${link(item)}</summary>\n<ul>\n${list(item.children)}\n</ul></details></li>`;
    });
    return `<p class="docs-nav-title">${title}${pill ? ` <span class="pill">${pill}</span>` : ""}</p>\n<ul>\n${lis.join("\n")}\n</ul>`;
  });
  const fam = familyOf(here);
  const sub =
    fam && !fam.parent.flat
      ? `\n<nav class="docs-subnav" aria-label="${esc(fam.parent.label)} pages">\n${[fam.parent, ...fam.children].map(link).join("\n")}\n</nav>`
      : "";
  return `<nav class="docs-nav" aria-label="Docs">\n${groups.join("\n")}\n</nav>${sub}`;
}

// ---------- splitting ----------

// "### x" → "## x" (and so on) outside fenced code, for a section shown as a page.
function promote(md) {
  let fence = false;
  return md
    .split("\n")
    .map((line) => {
      if (/^\s*(```|~~~)/.test(line)) fence = !fence;
      return !fence && /^#{3,6}\s/.test(line) ? line.slice(1) : line;
    })
    .join("\n");
}

// The first paragraph of a section, as plain text for a meta description.
function firstParagraph(md) {
  const para = [];
  for (const line of md.split("\n")) {
    if (!line.trim()) {
      if (para.length) break;
      continue;
    }
    if (/^(#|```|~~~|\||\s*[-*]\s|\s*\d+\.\s)/.test(line)) {
      if (para.length) break;
      continue;
    }
    para.push(line.trim());
  }
  let text = para
    .join(" ")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/[`*]/g, "")
    .trim();
  if (text.length > 160) text = text.slice(0, 157).replace(/\s+\S*$/, "") + "…";
  return text;
}

const splitCache = new Map();

/** A split page's parts: { parent: md, children: [{ name, slug, heading, title, description, path, md }] }. */
function splitParts(name) {
  const conf = SPLIT[name];
  const { md } = loadMarkdown(name);
  const hit = splitCache.get(name);
  if (hit?.md === md) return hit.parts;
  const doc = sections(md);
  const keep = new Set(conf.keep);
  const kept = doc.sections.filter((s) => keep.has(s.heading));
  const used = new Set();
  const children = doc.sections
    .filter((s) => !keep.has(s.heading))
    .map((s) => {
      let base = slug(s.heading) || "section";
      let sl = base;
      for (let i = 2; used.has(sl); i++) sl = `${base}-${i}`;
      used.add(sl);
      return {
        name: `${name}/${sl}`,
        slug: sl,
        section: s.heading,
        heading: conf.heading(s.heading),
        title: conf.title(s.heading),
        description: [conf.heading(s.heading), firstParagraph(s.body)].filter(Boolean).join(": "),
        path: `${PAGE_PATH[name]}${sl}/`,
        md: `# ${conf.heading(s.heading)}\n\n${promote(s.body)}\n`,
      };
    });
  const parent = [`# ${doc.title}`, "", doc.intro, "", ...kept.map((s) => `## ${s.heading}\n\n${s.body}\n`)].join("\n");
  const parts = { parent, children };
  splitCache.set(name, { md, parts });
  return parts;
}

/** The child pages of a split page (empty for any other page). */
export function splitChildren(name) {
  return SPLIT[name] ? splitParts(name).children : [];
}

/** The ids a reference page's HTML will have. */
function idsOf(md) {
  const html = referenceHtml(md, { link: (h) => h });
  return [...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1]);
}

const anchorCache = new Map();

/**
 * For a split page: where each of its old #anchors is now, id → page path (the
 * parent's own first, then each child's; a child's section slug maps to the child).
 */
function anchorMap(name) {
  const parts = splitParts(name);
  const hit = anchorCache.get(name);
  if (hit?.parts === parts) return hit.map;
  const map = new Map();
  for (const id of idsOf(parts.parent)) if (!map.has(id)) map.set(id, PAGE_PATH[name]);
  for (const c of parts.children) {
    const section = slug(c.section);
    if (!map.has(section)) map.set(section, c.path);
    for (const id of idsOf(c.md)) if (!map.has(id)) map.set(id, c.path + "#" + id);
  }
  anchorCache.set(name, { parts, map });
  return map;
}

/** `path#hash` for a page `name`, following its split when there is one. */
function anchored(name, hash) {
  const path = PAGE_PATH[name];
  if (!hash) return path;
  if (!SPLIT[name]) return `${path}#${hash}`;
  return anchorMap(name).get(hash) ?? `${path}#${hash}`;
}

/**
 * The Extensions pages' pictures (headless screenshots of components):
 * `source` in the repository, served at `/<path>/` with the same names.
 */
export const DOC_IMAGES = { source: "docs/reference/extensions/images", path: "docs/extensions/images" };

/** Every picture under DOC_IMAGES.source, as paths relative to it. */
export function docImages() {
  const dir = resolve(ROOT, DOC_IMAGES.source);
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { recursive: true, withFileTypes: true })
    .filter((e) => e.isFile() && /\.(png|svg|jpe?g|webp)$/i.test(e.name))
    .map((e) => relative(dir, join(e.parentPath, e.name)).split(sep).join("/"))
    .sort();
}

const NAME_OF = Object.fromEntries(Object.entries(SOURCES).map(([name, file]) => [file, name]));

// Relative links are read from the Markdown file's own folder, as GitHub reads
// them. A link to a Markdown file shown on the site stays on the site (and finds
// the child page an anchor moved to); any other repository path goes to GitHub.
function linker(repoUrl, source = resolve(ROOT, "docs/reference/index.md")) {
  const dir = relative(ROOT, dirname(source)).split(sep).join("/");
  const self = NAME_OF[source];
  return (href) => {
    if (href.startsWith("#")) return self && SPLIT[self] ? anchored(self, href.slice(1)) : href;
    if (/^([a-z]+:|\/)/i.test(href)) return href;
    const [rel, hash = ""] = href.split("#");
    const path = posix.normalize(posix.join(dir, rel)).replace(/^(\.\.\/)+/, "");
    const site = ON_SITE[path];
    if (site) return anchored(NAME_OF[resolve(ROOT, path)], hash);
    if (path.startsWith(DOC_IMAGES.source + "/")) return `/${DOC_IMAGES.path}/${path.slice(DOC_IMAGES.source.length + 1)}`;
    return `${repoUrl}/blob/main/${path}${hash && "#" + hash}`;
  };
}

// ---------- pages ----------

/** One <section class="release"> per "## " section; "## Unreleased" gets the in-development pill. */
export function changelogHtml(md, { repoUrl, appVersion, source = SOURCES.changelog }) {
  const link = linker(repoUrl, source);
  const { sections: releases } = sections(md);
  const released = new Set(releases.map((r) => r.heading.match(/^\[?v?([\w.+-]+?)\]?(?:\(|\s|$)/)?.[1]));
  return releases
    .map(({ heading, body }) => {
      let title, pill, id;
      if (/^unreleased$/i.test(heading)) {
        // The version in package.json is the next one until it shows up as released.
        const next = appVersion && !released.has(appVersion) ? appVersion : "Next";
        title = next;
        pill = "Unreleased · in development";
        id = "unreleased";
      } else {
        // git-cliff: "[0.1.0](https://…/releases/tag/v0.1.0) - 2026-10-07"
        const m = heading.match(/^(.*?)\s+-\s+(\d{4}-\d{2}-\d{2})$/);
        title = inline(m ? m[1] : heading, link);
        pill = m ? m[2] : "";
        id = "v" + slug(heading.match(/^\[?v?([\w.+-]+)/)?.[1] ?? heading);
      }
      return [
        `<section class="release" aria-labelledby="${id}">`,
        `<div class="release-head"><h2 id="${id}">${title}</h2>${pill ? `<span class="pill">${pill}</span>` : ""}</div>`,
        render(body, { link, idPrefix: id + "-" }),
        `</section>`,
      ].join("\n");
    })
    .join("\n\n");
}

const LANES = ["now", "next", "later"];

/** Intro as prose, Now / Next / Later as lanes, any other "## " section as prose after them. */
export function roadmapHtml(md, { repoUrl, source = SOURCES.roadmap }) {
  const link = linker(repoUrl, source);
  const doc = sections(md);
  const lanes = doc.sections.filter((s) => LANES.includes(slug(s.heading)));
  const rest = doc.sections.filter((s) => !LANES.includes(slug(s.heading)));
  const lane = ({ heading, body }) => {
    const id = slug(heading);
    return [
      `<section class="lane lane-${id}" aria-labelledby="${id}">`,
      `<h2 id="${id}">${inline(heading, link)}</h2>`,
      render(body, { link }),
      `</section>`,
    ].join("\n");
  };
  const prose = rest.map(({ heading, body }) => `<h2 id="${slug(heading)}">${inline(heading, link)}</h2>\n${render(body, { link })}`);
  return [
    `<div class="prose roadmap-intro">\n${render(doc.intro, { link })}\n</div>`,
    `<div class="lanes">\n${lanes.map(lane).join("\n")}\n</div>`,
    prose.length ? `<div class="prose roadmap-rest">\n${prose.join("\n")}\n</div>` : "",
  ].join("\n");
}

/**
 * A reference page (docs/reference/*.md): its "# " title as the page heading,
 * the intro, "On this page" links to each "## " section, then the sections.
 * `label` is the eyebrow above the title, replaced by breadcrumbs (`crumbs`, a
 * list of { path, label }) on a child page; `preview` puts the Preview pill beside
 * the title; `pager` ({ prev, next }) adds Previous / Next; `moved` (id → URL
 * relative to the page) sends old anchors on.
 */
export function referenceHtml(md, { repoUrl, source, link: linkFn, label = "Reference", preview = false, crumbs, pager, moved }) {
  const link = linkFn ?? linker(repoUrl, source);
  const doc = sections(md);
  const ids = new Set();
  const id = (heading) => {
    let base = slug(heading) || "section";
    let out = base;
    for (let i = 2; ids.has(out); i++) out = `${base}-${i}`;
    ids.add(out);
    return out;
  };
  const parts = doc.sections.map((s) => ({ ...s, id: id(s.heading) }));
  const toc = parts.map((s) => `<li><a href="#${s.id}">${inline(s.heading, link)}</a></li>`).join("");
  const title = inline(doc.title, link);
  const eyebrow = crumbs?.length
    ? `<nav class="eyebrow crumbs" aria-label="Breadcrumb"><ol>${crumbs.map((c) => `<li><a href="${c.path}">${esc(c.label)}</a></li>`).join("")}</ol></nav>`
    : `<p class="label">${label}</p>`;
  const pagerHtml = pager && (pager.prev || pager.next)
    ? `<nav class="docs-pager" aria-label="Previous and next pages"><ul class="doc-cards">${[
        pager.prev && `<li class="prev"><a href="${pager.prev.path}" rel="prev"><span>Previous</span><strong>${esc(pager.prev.label)}</strong></a></li>`,
        pager.next && `<li class="next"><a href="${pager.next.path}" rel="next"><span>Next</span><strong>${esc(pager.next.label)}</strong></a></li>`,
      ]
        .filter(Boolean)
        .join("")}</ul></nav>`
    : "";
  const movedHtml = moved && Object.keys(moved).length
    ? [
        `<script type="application/json" id="moved-anchors">${JSON.stringify(moved).replace(/</g, "\\u003c")}</script>`,
        `<script>(function(){var m;try{m=JSON.parse(document.getElementById("moved-anchors").textContent)}catch(e){return}function go(){var h=decodeURIComponent(location.hash.slice(1));if(h&&!document.getElementById(h)&&m[h])location.replace(m[h])}go();addEventListener("hashchange",go)})();</script>`,
      ].join("\n")
    : "";
  return [
    eyebrow,
    preview ? `<div class="docs-title"><h1>${title}</h1><span class="pill">Preview</span></div>` : `<h1>${title}</h1>`,
    render(doc.intro, { link, ids }),
    parts.length > 1 ? `<ul class="toc" aria-label="On this page">${toc}</ul>` : "",
    ...parts.map((s) => `<h2 id="${s.id}">${inline(s.heading, link)}</h2>\n${render(s.body, { link, ids })}`),
    pagerHtml,
    movedHtml,
  ]
    .filter(Boolean)
    .join("\n");
}

/**
 * A page's Markdown with its includes resolved and the WIT reference filled in.
 * Returns { md, files }: `files` are the other files it was built from.
 */
export function loadMarkdown(name) {
  const source = SOURCES[name];
  if (!source) throw new Error(`no Markdown source "${name}"`);
  const { md, files } = resolveIncludes(readFileSync(source, "utf8"), { root: ROOT, source });
  return { md: md.replace("<!-- wit-reference -->", () => witMarkdown()), files };
}

/** What a dev server watches: every Markdown source and the WIT folder. */
export function watchedFiles() {
  return [...Object.values(SOURCES), WIT_DIR];
}

// Breadcrumbs and Previous / Next for the page at `path`.
function familyLinks(path) {
  const fam = familyOf(path);
  if (!fam) return {};
  const seq = [fam.parent, ...fam.children];
  const i = seq.findIndex((p) => p.path === path);
  const isChild = i > 0;
  return {
    crumbs: isChild ? [{ path: "/docs/", label: "Docs" }, { path: fam.parent.path, label: fam.parent.crumb ?? fam.parent.label }] : undefined,
    pager: { prev: seq[i - 1], next: seq[i + 1] },
  };
}

/** The HTML for a page's <!-- markdown:<name> --> marker; `name` may be a split child ("cli/agent"). */
export function renderPage(name, opts) {
  const [base, child] = name.split("/");
  if (child !== undefined) {
    const c = splitChildren(base).find((x) => x.slug === child);
    if (!c) throw new Error(`no section "${child}" in ${relative(ROOT, SOURCES[base] ?? base)}`);
    opts.onFiles?.(loadMarkdown(base).files);
    return referenceHtml(c.md, { ...opts, source: SOURCES[base], ...familyLinks(c.path) });
  }
  const source = SOURCES[name];
  if (!source) throw new Error(`no Markdown source "${name}"`);
  const { md, files } = loadMarkdown(name);
  opts.onFiles?.(files);
  if (name === "changelog") return changelogHtml(md, { ...opts, source });
  if (name === "roadmap") return roadmapHtml(md, { ...opts, source });
  if (SPLIT[name]) {
    const map = anchorMap(name);
    const moved = {};
    for (const [id, to] of map) if (to !== PAGE_PATH[name]) moved[id] = to.slice(PAGE_PATH[name].length);
    return referenceHtml(splitParts(name).parent, { ...opts, source, ...PAGE_KIND[name], ...familyLinks(PAGE_PATH[name]), moved });
  }
  return referenceHtml(md, { ...opts, source, ...PAGE_KIND[name], ...familyLinks(PAGE_PATH[name]) });
}

// ---------- generated child pages ----------

const GENERATED = "<!-- Generated by website/scripts/pages.mjs from the parent page; edit the Markdown, not this. -->";

/**
 * Writes website/<child path>/index.html for every split child, from its parent's
 * page (title, description and marker changed), and removes generated pages whose
 * section is gone. Returns { <vite input name>: file } for vite.config.js.
 */
export function writeChildPages() {
  const inputs = {};
  for (const name of Object.keys(SPLIT)) {
    const parentDir = resolve(SITE, "." + PAGE_PATH[name]);
    const template = readFileSync(resolve(parentDir, "index.html"), "utf8");
    const children = splitChildren(name);
    const keep = new Set(children.map((c) => c.slug));
    for (const entry of readdirSync(parentDir, { withFileTypes: true })) {
      const file = resolve(parentDir, entry.name, "index.html");
      if (entry.isDirectory() && !keep.has(entry.name) && existsSync(file) && readFileSync(file, "utf8").includes(GENERATED))
        rmSync(resolve(parentDir, entry.name), { recursive: true });
    }
    for (const c of children) {
      const html = template
        .replace(/^<!doctype html>/i, (d) => `${d}\n${GENERATED}`)
        .replace(/<title>[^<]*<\/title>/, `<title>${esc(c.title)}</title>`)
        .replace(/<meta name="description" content="[^"]*"/, `<meta name="description" content="${esc(c.description)}"`)
        .replace(`<!-- markdown:${name} -->`, `<!-- markdown:${c.name} -->`);
      const dir = resolve(parentDir, c.slug);
      const file = resolve(dir, "index.html");
      mkdirSync(dir, { recursive: true });
      if (!existsSync(file) || readFileSync(file, "utf8") !== html) writeFileSync(file, html);
      inputs[c.name.replace("/", "-")] = file;
    }
  }
  return inputs;
}

/**
 * /sdk/llms.txt (authoring.md §9.5): the Extensions pages as one plain Markdown
 * file for coding agents, in sidebar order, with includes and the WIT reference
 * filled in, comments dropped and every link absolute against `pagesUrl`.
 */
export function llmsTxt({ repoUrl, pagesUrl }) {
  const absolute = (target) => (target.startsWith("/") ? new URL(target.slice(1), pagesUrl).href : target);
  const pages = EXTENSION_PAGES.map(([name, , path]) => {
    const link = linker(repoUrl, SOURCES[name]);
    const md = loadMarkdown(name)
      .md.replace(/<!--[\s\S]*?-->\n?/g, "")
      .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, label, href) => `[${label}](${absolute(href.startsWith("#") ? path + href : link(href))})`)
      .replace(/\n{3,}/g, "\n\n")
      .trim();
    return `Source: ${absolute(path)}\n\n${md}`;
  });
  return [
    "# Pitwall extensions (Preview)",
    "",
    "> Pitwall's extension and Tile SDK docs in one file, for coding agents. Generated from docs/reference/extensions/ and the tile world's WIT; Preview until WIT 1.0, so names may still change.",
    "",
    pages.join("\n\n---\n\n"),
    "",
  ].join("\n");
}
