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
};

// Links between the two files stay on the site; any other repository path goes to GitHub.
function linker(repoUrl) {
  return (href) => {
    if (/^([a-z]+:|#|\/)/i.test(href)) return href;
    const [path, hash = ""] = href.replace(/^\.\//, "").split("#");
    if (path === "CHANGELOG.md") return "/changelog/" + (hash && "#" + hash);
    if (path === "ROADMAP.md") return "/roadmap/" + (hash && "#" + hash);
    return `${repoUrl}/blob/main/${path}${hash && "#" + hash}`;
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

export function renderPage(name, opts) {
  const md = readFileSync(SOURCES[name], "utf8");
  return name === "changelog" ? changelogHtml(md, opts) : roadmapHtml(md, opts);
}
