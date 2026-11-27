// Renders the repository's Markdown into site pages: CHANGELOG.md and ROADMAP.md
// into /changelog/ and /roadmap/, docs/reference/*.md into the reference pages and
// docs/reference/extensions/*.md into the Extensions pages under /docs/extensions/.
// vite.config.js replaces each page's <!-- markdown:<name> --> marker with this
// output at build (and dev) time, so the Markdown files stay the only source.
//
// Before rendering, {{#include <path>#<region>}} lines pull examples from source
// files (scripts/includes.mjs), and a <!-- wit-reference --> marker is replaced by
// the reference generated from the tile world's WIT (scripts/wit-docs.mjs).
import { readFileSync } from "node:fs";
import { dirname, posix, relative, resolve, sep } from "node:path";
import { render, sections, slug, inline } from "./markdown.mjs";
import { resolveIncludes } from "./includes.mjs";
import { witMarkdown, WIT_DIR } from "./wit-docs.mjs";

export const ROOT = resolve(import.meta.dirname, "../..");

// The Extensions pages (docs/spec/tile-sdk/authoring.md §9.6), in sidebar order:
// [name, Markdown file in docs/reference/extensions/, path on the site, sidebar label].
// Each has a page at website/docs/extensions/<page>/index.html holding a
// <!-- markdown:<name> --> marker, and an input in vite.config.js. All carry the
// Preview pill until WIT 1.0.
export const EXTENSION_PAGES = [
  ["extensions", "README.md", "/docs/extensions/", "Overview"],
  ["extensions-quick-start", "quick-start.md", "/docs/extensions/quick-start/", "Quick start"],
  ["extensions-ui", "ui.md", "/docs/extensions/ui/", "Tiles and UI components"],
  ["extensions-permissions", "permissions.md", "/docs/extensions/permissions/", "Permissions"],
  ["extensions-platform-services", "platform-services.md", "/docs/extensions/platform-services/", "Platform services"],
  ["extensions-wit", "wit.md", "/docs/extensions/wit/", "WIT reference 0.1"],
];

export const SOURCES = {
  changelog: resolve(ROOT, "CHANGELOG.md"),
  roadmap: resolve(ROOT, "ROADMAP.md"),
  cli: resolve(ROOT, "docs/reference/cli.md"),
  settings: resolve(ROOT, "docs/reference/settings.md"),
  agents: resolve(ROOT, "docs/reference/agents.md"),
  detection: resolve(ROOT, "docs/reference/detection.md"),
  ...Object.fromEntries(EXTENSION_PAGES.map(([name, file]) => [name, resolve(ROOT, "docs/reference/extensions", file)])),
};

// Where each Markdown file is shown on the site (links between them stay on it).
const ON_SITE = {
  "CHANGELOG.md": "/changelog/",
  "ROADMAP.md": "/roadmap/",
  "docs/reference/cli.md": "/docs/cli/",
  "docs/reference/settings.md": "/docs/settings/",
  "docs/reference/agents.md": "/docs/agents/",
  "docs/reference/detection.md": "/docs/detection/",
  ...Object.fromEntries(EXTENSION_PAGES.map(([, file, path]) => [`docs/reference/extensions/${file}`, path])),
};

// How a reference page is labelled: the eyebrow above its title, and whether the
// Preview pill goes beside the title.
const PAGE_KIND = Object.fromEntries(EXTENSION_PAGES.map(([name]) => [name, { label: "Extensions", preview: true }]));

/** The docs sidebar: sections (title, pages, optional pill), then pages (path, label). */
export const DOCS_NAV = [
  ["Start", [["/docs/", "Overview"], ["/docs/quick-start/", "Quick start"]]],
  ["Guide", [["/docs/how-it-works/", "How it works"]]],
  [
    "Reference",
    [
      ["/docs/cli/", "CLI"],
      ["/docs/settings/", "Settings"],
      ["/docs/agents/", "Agent definitions"],
      ["/docs/detection/", "Detection rules"],
    ],
  ],
  ["Extensions", EXTENSION_PAGES.map(([, , path, label]) => [path, label]), "Preview"],
];

/** The sidebar with `current` (a page path such as "/docs/cli/index.html") marked. */
export function docsNavHtml(current) {
  const here = current.replace(/index\.html$/, "");
  const groups = DOCS_NAV.map(
    ([title, pages, pill]) =>
      `<p class="docs-nav-title">${title}${pill ? ` <span class="pill">${pill}</span>` : ""}</p>\n<ul>\n${pages
        .map(([href, label]) => `  <li><a href="${href}"${href === here ? ' aria-current="page"' : ""}>${label}</a></li>`)
        .join("\n")}\n</ul>`,
  );
  return `<nav class="docs-nav" aria-label="Docs">\n${groups.join("\n")}\n</nav>`;
}

// Relative links are read from the Markdown file's own folder, as GitHub reads
// them. A link to a Markdown file shown on the site stays on the site; any other
// repository path goes to GitHub.
function linker(repoUrl, source = resolve(ROOT, "docs/reference/index.md")) {
  const dir = relative(ROOT, dirname(source)).split(sep).join("/");
  return (href) => {
    if (/^([a-z]+:|#|\/)/i.test(href)) return href;
    const [rel, hash = ""] = href.split("#");
    const path = posix.normalize(posix.join(dir, rel)).replace(/^(\.\.\/)+/, "");
    const site = ON_SITE[path];
    if (site) return site + (hash && "#" + hash);
    return `${repoUrl}/blob/main/${path}${hash && "#" + hash}`;
  };
}

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
 * `label` is the eyebrow above the title; `preview` puts the Preview pill beside it.
 */
export function referenceHtml(md, { repoUrl, source, label = "Reference", preview = false }) {
  const link = linker(repoUrl, source);
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
  return [
    `<p class="label">${label}</p>`,
    preview ? `<div class="docs-title"><h1>${title}</h1><span class="pill">Preview</span></div>` : `<h1>${title}</h1>`,
    render(doc.intro, { link, ids }),
    parts.length > 1 ? `<ul class="toc" aria-label="On this page">${toc}</ul>` : "",
    ...parts.map((s) => `<h2 id="${s.id}">${inline(s.heading, link)}</h2>\n${render(s.body, { link, ids })}`),
  ].join("\n");
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

export function renderPage(name, opts) {
  const { md, files } = loadMarkdown(name);
  opts.onFiles?.(files);
  const source = SOURCES[name];
  if (name === "changelog") return changelogHtml(md, { ...opts, source });
  if (name === "roadmap") return roadmapHtml(md, { ...opts, source });
  return referenceHtml(md, { ...opts, source, ...PAGE_KIND[name] });
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
