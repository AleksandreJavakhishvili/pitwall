// Renders the repository's CHANGELOG.md and ROADMAP.md into the /changelog/ and
// /roadmap/ pages. vite.config.js replaces <!-- markdown:changelog --> and
// <!-- markdown:roadmap --> in those pages with this output at build (and dev) time,
// so the Markdown files stay the only source.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { render, sections, slug, inline } from "./markdown.mjs";

export const ROOT = resolve(import.meta.dirname, "../..");
export const SOURCES = {
  changelog: resolve(ROOT, "CHANGELOG.md"),
  roadmap: resolve(ROOT, "ROADMAP.md"),
  cli: resolve(ROOT, "docs/reference/cli.md"),
  settings: resolve(ROOT, "docs/reference/settings.md"),
  agents: resolve(ROOT, "docs/reference/agents.md"),
  detection: resolve(ROOT, "docs/reference/detection.md"),
};

// Where each Markdown file is shown on the site (links between them stay on it).
const ON_SITE = {
  "CHANGELOG.md": "/changelog/",
  "ROADMAP.md": "/roadmap/",
  "docs/reference/cli.md": "/docs/cli/",
  "docs/reference/settings.md": "/docs/settings/",
  "docs/reference/agents.md": "/docs/agents/",
  "docs/reference/detection.md": "/docs/detection/",
};

/** The docs sidebar: sections, then pages (path, label). */
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
];

/** The sidebar with `current` (a page path such as "/docs/cli/index.html") marked. */
export function docsNavHtml(current) {
  const here = current.replace(/index\.html$/, "");
  const groups = DOCS_NAV.map(
    ([title, pages]) =>
      `<p class="docs-nav-title">${title}</p>\n<ul>\n${pages
        .map(([href, label]) => `  <li><a href="${href}"${href === here ? ' aria-current="page"' : ""}>${label}</a></li>`)
        .join("\n")}\n</ul>`,
  );
  return `<nav class="docs-nav" aria-label="Docs">\n${groups.join("\n")}\n</nav>`;
}

// Links between the two files stay on the site; any other repository path goes to GitHub.
function linker(repoUrl) {
  return (href) => {
    if (/^([a-z]+:|#|\/)/i.test(href)) return href;
    const [path, hash = ""] = href.replace(/^\.\//, "").split("#");
    const site = ON_SITE[path] ?? ON_SITE[`docs/reference/${path}`];
    if (site) return site + (hash && "#" + hash);
    return `${repoUrl}/blob/main/${path.replace(/^(\.\.\/)+/, "")}${hash && "#" + hash}`;
  };
}

/** One <section class="release"> per "## " section; "## Unreleased" gets the in-development pill. */
export function changelogHtml(md, { repoUrl, appVersion }) {
  const link = linker(repoUrl);
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
export function roadmapHtml(md, { repoUrl }) {
  const link = linker(repoUrl);
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
 */
export function referenceHtml(md, { repoUrl }) {
  const link = linker(repoUrl);
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
  return [
    `<p class="label">Reference</p>`,
    `<h1>${inline(doc.title, link)}</h1>`,
    render(doc.intro, { link }),
    parts.length > 1 ? `<ul class="toc" aria-label="On this page">${toc}</ul>` : "",
    ...parts.map((s) => `<h2 id="${s.id}">${inline(s.heading, link)}</h2>\n${render(s.body, { link })}`),
  ].join("\n");
}

export function renderPage(name, opts) {
  const md = readFileSync(SOURCES[name], "utf8");
  if (name === "changelog") return changelogHtml(md, opts);
  if (name === "roadmap") return roadmapHtml(md, opts);
  return referenceHtml(md, opts);
}
