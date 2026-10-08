import { defineConfig } from "vite";
import { readFileSync } from "node:fs";
import { relative, resolve, sep } from "node:path";
import { REPO_URL, PAGES_URL, RELEASES_URL, RELEASE_API_URL } from "./site.config.js";
import { renderPage, SOURCES, ROOT } from "./scripts/pages.mjs";

// Pages are written with root-absolute links (`/docs/quick-start/`, `/video/…`).
// Vite already prefixes the base on the assets it handles (scripts, styles, icons,
// <video>/<source>); this rewrites the rest after it, so the site also works from a
// sub-path such as https://<owner>.github.io/<repo>/: anchors and data-* URLs.
// It also fills in %REPO_URL%, %RELEASES_URL%, %RELEASE_API_URL% and %PAGES_URL% from site.config.js.
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
          .replaceAll("%RELEASE_API_URL%", RELEASE_API_URL)
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

// Search and link-preview metadata, generated from each page's own <title> and
// <meta name="description">: canonical URL, Open Graph and Twitter card tags,
// theme-color, and JSON-LD on the home page. Absolute URLs are built from
// PAGES_URL (which already carries the /<repo>/ path), so they are right whatever
// base the build uses. The 404 page gets noindex and no canonical. At build time it
// also writes sitemap.xml and robots.txt into dist.
const OG_IMAGE = { path: "og.png", width: 1200, height: 630, alt: "Pitwall: a timing board of coding agents, one flagged Needs you" };
const NOT_FOUND = "404.html";

// "/docs/quick-start/index.html" -> "docs/quick-start/", "/index.html" -> "".
function pagePath(file) {
  return file.replace(/^\//, "").replace(/(^|\/)index\.html$/, "$1");
}
const absolute = (path) => new URL(path, PAGES_URL).href;
const attr = (s) => s.replace(/&(?!(?:[a-z]+|#\d+);)/gi, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
const decode = (s) =>
  s.replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");

function seo() {
  let pages = [];
  return {
    name: "pitwall-seo",
    configResolved(c) {
      const input = Object.values(c.build.rollupOptions.input ?? {});
      pages = input.map((file) => pagePath(relative(c.root, file).split(sep).join("/"))).filter((p) => p !== NOT_FOUND);
    },
    transformIndexHtml: {
      order: "pre",
      handler(html, ctx) {
        const path = pagePath(ctx.path);
        const title = html.match(/<title>([^<]*)<\/title>/)?.[1];
        const description = html.match(/<meta name="description" content="([^"]*)"/)?.[1];
        if (!title || !description) throw new Error(`${ctx.path}: needs a <title> and a meta description`);
        const notFound = path === NOT_FOUND;
        const url = absolute(path);
        const image = absolute(OG_IMAGE.path);
        const tags = [
          notFound ? `<meta name="robots" content="noindex" />` : `<link rel="canonical" href="${url}" />`,
          `<meta name="theme-color" content="#0a0b0d" media="(prefers-color-scheme: dark)" />`,
          `<meta name="theme-color" content="#f3f4f6" media="(prefers-color-scheme: light)" />`,
          `<meta property="og:type" content="website" />`,
          `<meta property="og:site_name" content="Pitwall" />`,
          `<meta property="og:title" content="${title}" />`,
          `<meta property="og:description" content="${description}" />`,
          ...(notFound ? [] : [`<meta property="og:url" content="${url}" />`]),
          `<meta property="og:image" content="${image}" />`,
          `<meta property="og:image:type" content="image/png" />`,
          `<meta property="og:image:width" content="${OG_IMAGE.width}" />`,
          `<meta property="og:image:height" content="${OG_IMAGE.height}" />`,
          `<meta property="og:image:alt" content="${attr(OG_IMAGE.alt)}" />`,
          `<meta name="twitter:card" content="summary_large_image" />`,
          `<meta name="twitter:title" content="${title}" />`,
          `<meta name="twitter:description" content="${description}" />`,
          `<meta name="twitter:image" content="${image}" />`,
          `<meta name="twitter:image:alt" content="${attr(OG_IMAGE.alt)}" />`,
        ];
        if (path === "") {
          const app = {
            "@context": "https://schema.org",
            "@type": "SoftwareApplication",
            name: "Pitwall",
            description: decode(description),
            applicationCategory: "DeveloperApplication",
            operatingSystem: "macOS, Linux, Windows",
            offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
            license: "https://www.apache.org/licenses/LICENSE-2.0",
            url,
            downloadUrl: absolute("download/"),
            image,
            codeRepository: REPO_URL,
          };
          const json = JSON.stringify(app, null, 2).replace(/</g, "\\u003c").replace(/\n/g, "\n  ");
          tags.push(`<script type="application/ld+json">\n  ${json}\n  </script>`);
        }
        return html.replace(/(<meta name="description"[^>]*>)/, `$1\n  ${tags.join("\n  ")}`);
      },
    },
    generateBundle() {
      const urls = pages.map((p) => `  <url><loc>${absolute(p)}</loc></url>`);
      const sitemap = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls.join("\n")}\n</urlset>\n`;
      this.emitFile({ type: "asset", fileName: "sitemap.xml", source: sitemap });
      this.emitFile({ type: "asset", fileName: "robots.txt", source: `User-agent: *\nAllow: /\n\nSitemap: ${absolute("sitemap.xml")}\n` });
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
  plugins: [markdownPages(), seo(), siteLinks(), fontLicenses()],
  build: {
    outDir: "dist",
    rollupOptions: {
      input: {
        index: resolve(import.meta.dirname, "index.html"),
        download: resolve(import.meta.dirname, "download/index.html"),
        quickstart: resolve(import.meta.dirname, "docs/quick-start/index.html"),
        howitworks: resolve(import.meta.dirname, "docs/how-it-works/index.html"),
        changelog: resolve(import.meta.dirname, "changelog/index.html"),
        roadmap: resolve(import.meta.dirname, "roadmap/index.html"),
        notfound: resolve(import.meta.dirname, "404.html"),
      },
    },
  },
});
