import { defineConfig } from "vite";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { REPO_URL, PAGES_URL, RELEASES_URL } from "./site.config.js";
import { renderPage, SOURCES, ROOT } from "./scripts/pages.mjs";

// Pages are written with root-absolute links (`/docs/quick-start/`, `/video/…`).
// Vite already prefixes the base on the assets it handles (scripts, styles, icons,
// <video>/<source>); this rewrites the rest after it, so the site also works from a
// sub-path such as https://<owner>.github.io/<repo>/: anchors and data-* URLs.
// It also fills in %REPO_URL%, %RELEASES_URL% and %PAGES_URL% from site.config.js.
function siteLinks() {
  let base = "/";
  return {
    name: "pitwall-site-links",
    configResolved(c) {
      base = c.base;
    },
    transformIndexHtml: {
      order: "post",
      handler(html) {
        html = html
          .replaceAll("%RELEASES_URL%", RELEASES_URL)
          .replaceAll("%REPO_URL%", REPO_URL)
          .replaceAll("%PAGES_URL%", PAGES_URL);
        if (base === "/" || !base.startsWith("/")) return html;
        const already = base.slice(1).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        const attr = new RegExp(`(\\s(?:href|src|poster|data-[\\w-]+)=")/(?!/|${already})`, "g");
        return html.replace(attr, `$1${base}`);
      },
    },
  };
}

// /changelog/ and /roadmap/ are rendered from the repository's CHANGELOG.md and
// ROADMAP.md (scripts/pages.mjs): each page has a <!-- markdown:<name> --> marker
// where the content goes. Runs before pitwall-site-links, so the generated
// root-absolute links get the base too. In dev, editing either file reloads the page.
function markdownPages() {
  return {
    name: "pitwall-markdown-pages",
    configureServer(server) {
      server.watcher.add(Object.values(SOURCES));
      server.watcher.on("change", (file) => {
        if (Object.values(SOURCES).includes(file)) server.ws.send({ type: "full-reload" });
      });
    },
    transformIndexHtml: {
      order: "pre",
      handler(html) {
        if (!html.includes("<!-- markdown:")) return html;
        const appVersion = JSON.parse(readFileSync(resolve(ROOT, "package.json"), "utf8")).version;
        return html.replace(/<!-- markdown:(changelog|roadmap) -->/g, (_, name) =>
          renderPage(name, { repoUrl: REPO_URL, appVersion }),
        );
      },
    },
  };
}

// The self-hosted fonts are under the SIL Open Font License 1.1, which asks for
// the licence to travel with them: copy each package's LICENSE into dist/licenses/.
const FONT_LICENSES = {
  "OFL-1.1-barlow-condensed.txt": "@fontsource/barlow-condensed",
  "OFL-1.1-inter.txt": "@fontsource-variable/inter",
  "OFL-1.1-jetbrains-mono.txt": "@fontsource-variable/jetbrains-mono",
};
function fontLicenses() {
  return {
    name: "pitwall-font-licenses",
    apply: "build",
    generateBundle() {
      for (const [name, pkg] of Object.entries(FONT_LICENSES)) {
        const source = readFileSync(resolve(import.meta.dirname, "node_modules", pkg, "LICENSE"), "utf8");
        this.emitFile({ type: "asset", fileName: `licenses/${name}`, source });
      }
    },
  };
}

// Static multi-page site. Every page is a plain HTML file; Vite only bundles
// the shared CSS/JS and the self-hosted fonts. The base path comes from
// `--base` (see scripts/build.mjs); it defaults to "/".
export default defineConfig({
  server: { port: 4321 },
  plugins: [markdownPages(), siteLinks(), fontLicenses()],
  build: {
    outDir: "dist",
    rollupOptions: {
      input: {
        index: resolve(import.meta.dirname, "index.html"),
        quickstart: resolve(import.meta.dirname, "docs/quick-start/index.html"),
        howitworks: resolve(import.meta.dirname, "docs/how-it-works/index.html"),
        changelog: resolve(import.meta.dirname, "changelog/index.html"),
        roadmap: resolve(import.meta.dirname, "roadmap/index.html"),
        notfound: resolve(import.meta.dirname, "404.html"),
      },
    },
  },
});
