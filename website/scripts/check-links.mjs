// Link check over the built site (docs/spec/tile-sdk/authoring.md §9.6): every
// href and src on every page must name a file in the build, and every #anchor an
// id on the page it points to. scripts/build.mjs runs it after `vite build`.
//
//   node scripts/check-links.mjs                       # check dist/, built for the base "/"
//   node scripts/check-links.mjs --base /pitwall/      # built for a sub-path
//   node scripts/check-links.mjs --dir <folder>        # another build folder
//   node scripts/check-links.mjs --external            # also request each http(s) link (needs the network)
//
// An anchor that a split page moved to a child page counts when the parent's
// moved-anchors map (scripts/pages.mjs) sends it to an id that exists there.
// External links are skipped by default, so the check (and CI) works offline.
// Links that JavaScript builds at run time aren't seen. Exits 1 when a link is broken.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";

const ORIGIN = "http://site.invalid";

function htmlFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const path = join(dir, e.name);
    return e.isDirectory() ? htmlFiles(path) : e.name.endsWith(".html") ? [path] : [];
  });
}

/** Every id (and <a name>) in a page. */
export function idsIn(html) {
  const ids = new Set();
  for (const m of html.matchAll(/\s(?:id|name)="([^"]+)"/g)) ids.add(decode(m[1]));
  return ids;
}

/** Every href and src in a page, outside <script>, <style> and comments. */
export function linksIn(html) {
  const body = html.replace(/<!--[\s\S]*?-->/g, "").replace(/<(script|style)\b[^>]*>[\s\S]*?<\/\1>/gi, (m) => m.replace(/>[\s\S]*</, "><"));
  const links = [];
  for (const m of body.matchAll(/<([a-z][\w-]*)\b([^>]*)>/gi)) {
    for (const a of m[2].matchAll(/\s(href|src)="([^"]*)"/gi)) links.push(decode(a[2]));
  }
  return links;
}

/** A split page's map of old anchors to child pages (scripts/pages.mjs), or {}. */
export function movedAnchors(html) {
  const m = html.match(/<script type="application\/json" id="moved-anchors">([\s\S]*?)<\/script>/);
  try {
    return m ? JSON.parse(m[1]) : {};
  } catch {
    return {};
  }
}

const decode = (s) =>
  s.replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");

/**
 * Checks a build folder. Returns { pages, links, broken: [{ page, href, reason }], external: [{ page, href }] }.
 * `base` is the path the site is served from ("/" or "/pitwall/").
 */
export function checkSite(dir, { base = "/" } = {}) {
  const files = htmlFiles(dir);
  const idCache = new Map();
  const ids = (file) => {
    if (!idCache.has(file)) idCache.set(file, idsIn(readFileSync(file, "utf8")));
    return idCache.get(file);
  };
  const movedCache = new Map();
  const moved = (file) => {
    if (!movedCache.has(file)) movedCache.set(file, movedAnchors(readFileSync(file, "utf8")));
    return movedCache.get(file);
  };
  const broken = [];
  const external = [];
  let count = 0;

  // A path under the base → the file that serves it, or null.
  const fileFor = (path) => {
    let rel = decodeURIComponent(path.slice(base.length));
    const candidates = rel === "" || rel.endsWith("/") ? [rel + "index.html"] : [rel, rel + "/index.html", rel + ".html"];
    for (const c of candidates) {
      const file = resolve(dir, c);
      if (!file.startsWith(resolve(dir))) return null;
      if (existsSync(file) && statSync(file).isFile()) return file;
    }
    return null;
  };

  for (const file of files) {
    const page = "/" + relative(dir, file).split(sep).join("/");
    const pageUrl = new URL(base.replace(/\/$/, "") + page.replace(/(^|\/)index\.html$/, "$1"), ORIGIN);
    for (const href of linksIn(readFileSync(file, "utf8"))) {
      count++;
      const fail = (reason) => broken.push({ page, href, reason });
      if (href === "") {
        fail("empty link");
        continue;
      }
      if (/%[A-Z_]+%/.test(href)) {
        fail("placeholder left unfilled");
        continue;
      }
      if (/^(mailto|tel|data|javascript|blob):/i.test(href)) continue;
      if (/^(https?:)?\/\//i.test(href)) {
        external.push({ page, href });
        continue;
      }
      if (/^[a-z][a-z0-9+.-]*:/i.test(href)) {
        fail("unknown scheme");
        continue;
      }
      const url = new URL(href, pageUrl);
      if (!url.pathname.startsWith(base)) {
        fail(`outside the site's base ${base}`);
        continue;
      }
      const target = fileFor(url.pathname);
      if (!target) {
        fail("no such page or file in the build");
        continue;
      }
      const hash = decodeURIComponent(url.hash.slice(1));
      if (!hash || !target.endsWith(".html") || ids(target).has(hash)) continue;
      // A split page's old anchor: its moved-anchors map sends it to a child page.
      const to = moved(target)[hash];
      if (to) {
        const next = new URL(to, url);
        const file = fileFor(next.pathname);
        const h = decodeURIComponent(next.hash.slice(1));
        if (file && (!h || ids(file).has(h))) continue;
        fail(`moved to ${to}, which doesn't exist`);
        continue;
      }
      fail(`no id "${hash}" on ${relative(dir, target).split(sep).join("/")}`);
    }
  }
  return { pages: files.length, links: count, broken, external };
}

/** Requests each external link once (HEAD, then GET when HEAD isn't allowed); returns the failures. */
export async function checkExternal(external, { timeout = 15000, concurrency = 8 } = {}) {
  const urls = [...new Set(external.map((e) => e.href.replace(/#.*$/, "")))].map((u) => (u.startsWith("//") ? "https:" + u : u));
  const failed = new Map();
  const request = async (url, method) => {
    const res = await fetch(url, { method, redirect: "follow", signal: AbortSignal.timeout(timeout), headers: { "user-agent": "pitwall-site-link-check" } });
    return res.status;
  };
  let next = 0;
  await Promise.all(
    Array.from({ length: concurrency }, async () => {
      while (next < urls.length) {
        const url = urls[next++];
        try {
          let status = await request(url, "HEAD");
          if (status === 405 || status === 403 || status === 404) status = await request(url, "GET");
          if (status >= 400 && status !== 429) failed.set(url, `HTTP ${status}`);
        } catch (e) {
          failed.set(url, e.name === "TimeoutError" ? "timed out" : e.message);
        }
      }
    }),
  );
  return external.filter((e) => failed.has(e.href.replace(/#.*$/, ""))).map((e) => ({ ...e, reason: failed.get(e.href.replace(/#.*$/, "")) }));
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const args = process.argv.slice(2);
  const opt = (name) => {
    const i = args.indexOf(name);
    return i === -1 ? undefined : args[i + 1];
  };
  const dir = resolve(process.cwd(), opt("--dir") ?? resolve(import.meta.dirname, "../dist"));
  let base = opt("--base") ?? process.env.SITE_BASE ?? "/";
  if (!base.startsWith("/")) base = "/" + base;
  if (!base.endsWith("/")) base += "/";
  if (!existsSync(dir)) {
    console.error(`check-links: ${dir} doesn't exist; build the site first`);
    process.exit(1);
  }
  const result = checkSite(dir, { base });
  let { broken } = result;
  if (args.includes("--external")) broken = broken.concat(await checkExternal(result.external));
  if (broken.length) {
    console.error(`check-links: ${broken.length} broken link(s):`);
    for (const b of broken) console.error(`  ${b.page}: ${b.href} (${b.reason})`);
    process.exit(1);
  }
  const ext = args.includes("--external") ? "checked" : "not checked (--external)";
  console.log(`check-links: ${result.links} links on ${result.pages} pages are fine; ${result.external.length} external links ${ext}`);
}
