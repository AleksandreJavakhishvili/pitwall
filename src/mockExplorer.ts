// Mock backend for the read-only code explorer (browser `pnpm dev`, the site's
// demo). A small made-up folder per agent; changed files come from the review
// mock so letters and contents agree with Changes and Review.
import type { AgentView, FileChange } from "./types";
import type { DirListing, ExplorerApi, FileEntry, FileView, SearchMatch, SearchResult } from "./explorerApi";
import { SEARCH_CANCELLED, LARGE_CAP, TEXT_CAP } from "./explorerApi";
import { versionsFor } from "./mockReview";

interface Deps {
  find(id: string): AgentView;
  changes(id: string): FileChange[];
}

const wait = <T>(v: T, ms = 80) => new Promise<T>((r) => setTimeout(() => r(v), ms));

/** A file of the made-up folder: text, or a size for binary / huge ones. */
interface MockFile {
  text?: string;
  size?: number;
  binary?: boolean;
  ignored?: boolean;
}

const lines = (...l: string[]) => l.join("\n") + "\n";

const TS_INDEX = lines(
  'import { buildApp } from "./app";',
  'import { log } from "./log";',
  "",
  "const port = Number(process.env.PORT ?? 3000);",
  "",
  "buildApp()",
  "  .then((app) => app.listen({ port, host: \"0.0.0.0\" }))",
  "  .then(() => log.info({ port }, \"orders-api listening\"))",
  "  .catch((err) => {",
  "    log.error(err, \"failed to start\");",
  "    process.exit(1);",
  "  });",
);

const TS_APP = lines(
  'import Fastify from "fastify";',
  'import { createOrder, cancelOrder } from "./orders/handler";',
  'import { errorHandler } from "./orders/errors";',
  "",
  "export async function buildApp() {",
  "  const app = Fastify({ logger: false });",
  "  app.setErrorHandler(errorHandler);",
  '  app.post("/v2/orders", createOrder);',
  '  app.delete("/v2/orders/:id", cancelOrder);',
  '  app.get("/health", async () => ({ ok: true }));',
  "  return app;",
  "}",
);

const TS_PRICING = lines(
  'import type { Item } from "./schema";',
  "",
  "/** Sum of the items in the order's currency, in minor units. */",
  "export function priceItems(items: Item[], currency: string): number {",
  "  let total = 0;",
  "  for (const item of items) {",
  "    if (item.currency !== currency) {",
  "      throw new Error(`mixed currencies: ${item.currency} and ${currency}`);",
  "    }",
  "    total += item.unitPrice * item.quantity;",
  "  }",
  "  return Math.round(total);",
  "}",
  "",
  "export function firstCurrency(items: Item[], fallback: string): string {",
  "  return items[0]?.currency ?? fallback;",
  "}",
);

const README = (name: string) =>
  lines(
    `# ${name}`,
    "",
    "Internal service. See `docs/` for the API and how to run it locally.",
    "",
    "## Development",
    "",
    "```sh",
    "pnpm install",
    "pnpm dev",
    "pnpm test",
    "```",
    "",
    "Orders are validated in `src/orders/schema.ts` and priced in `src/orders/pricing.ts`.",
  );

const PACKAGE = (name: string) =>
  JSON.stringify(
    {
      name,
      private: true,
      type: "module",
      scripts: { dev: "tsx watch src/index.ts", build: "tsc -p .", test: "vitest run" },
      dependencies: { fastify: "^5.2.0", zod: "^3.24.1" },
      devDependencies: { typescript: "^5.7.2", vitest: "^3.0.0", tsx: "^4.19.2" },
    },
    null,
    2,
  ) + "\n";

function generic(path: string): string {
  const name = path.split("/").pop() ?? path;
  if (name.endsWith(".md")) return lines(`# ${name.replace(/\.md$/, "")}`, "", "Notes kept next to the code. Read-only here.");
  if (name.endsWith(".json")) return JSON.stringify({ name, version: 1 }, null, 2) + "\n";
  if (/\.(ts|tsx|js)$/.test(name)) {
    const fn = name.replace(/\.\w+$/, "").replace(/[^A-Za-z0-9]/g, "");
    return lines(`// ${path}`, "", `export function ${fn || "main"}() {`, `  return "${name}";`, "}");
  }
  if (name.endsWith(".yml") || name.endsWith(".yaml")) return lines("site_name: Handbook", "nav:", "  - Home: index.md", "  - Onboarding: onboarding.md");
  return lines(name);
}

/** The made-up folder of an agent, by its project. */
function baseFiles(a: AgentView): Record<string, MockFile> {
  const p = a.projectDisplay;
  const common: Record<string, MockFile> = {
    ".gitignore": { text: lines("node_modules/", "dist/", ".env.local", "*.log") },
    "README.md": { text: README(p) },
    "package.json": { text: PACKAGE(p) },
    "node_modules/fastify/package.json": { text: lines('{ "name": "fastify", "version": "5.2.0" }'), ignored: true },
    "node_modules/zod/package.json": { text: lines('{ "name": "zod", "version": "3.24.1" }'), ignored: true },
    "dist/index.js": { text: lines('"use strict";', "// built output"), ignored: true },
    ".env.local": { text: lines("PORT=3000", "DATABASE_URL=postgres://localhost/orders"), ignored: true },
  };
  if (p === "checkout-web") {
    return {
      ...common,
      "index.html": { text: lines("<!doctype html>", '<div id="root"></div>', '<script type="module" src="/src/main.tsx"></script>') },
      "vite.config.ts": { text: lines('import { defineConfig } from "vite";', "export default defineConfig({});") },
      "src/main.tsx": { text: lines('import { createRoot } from "react-dom/client";', 'import { App } from "./App";', "", 'createRoot(document.getElementById("root")!).render(<App />);') },
      "src/App.tsx": { text: lines('import { Cart } from "./cart/Cart";', "", "export function App() {", "  return <Cart />;", "}") },
      "src/cart/Cart.tsx": { text: lines('import { useCart } from "./useCart";', "", "export function Cart() {", "  const { items, total } = useCart();", "  return <p>{items.length} items · {total}</p>;", "}") },
      "public/logo.png": { binary: true, size: 18_204 },
    };
  }
  if (p === "handbook") {
    return {
      ".gitignore": { text: lines("site/") },
      "README.md": { text: README(p) },
      "mkdocs.yml": { text: generic("mkdocs.yml") },
      "docs/index.md": { text: generic("docs/index.md") },
      "site/index.html": { text: lines("<!doctype html>"), ignored: true },
    };
  }
  return {
    ...common,
    "tsconfig.json": { text: JSON.stringify({ compilerOptions: { strict: true, target: "ES2022", module: "NodeNext" } }, null, 2) + "\n" },
    "src/index.ts": { text: TS_INDEX },
    "src/app.ts": { text: TS_APP },
    "src/log.ts": { text: lines('import pino from "pino";', "", 'export const log = pino({ level: process.env.LOG_LEVEL ?? "info" });') },
    "src/store.ts": { text: lines("export async function loadStore(id: string) {", '  return { id, currency: "EUR" };', "}") },
    "src/orders/handler.ts": { text: versionsFor({ path: "src/orders/handler.ts", added: 1, removed: 1, untracked: false, binary: false }, "src/orders/handler.ts").modified ?? "" },
    "src/orders/pricing.ts": { text: TS_PRICING },
    "src/orders/schema.ts": { text: generic("src/orders/schema.ts") },
    "src/orders/legacy-handler.ts": { text: generic("src/orders/legacy-handler.ts") },
    "test/helpers.ts": { text: lines('import { buildApp as build } from "../src/app";', "export const buildApp = build;") },
    "test/orders/pricing.test.ts": { text: lines('import { priceItems } from "../../src/orders/pricing";', 'import { expect, it } from "vitest";', "", 'it("prices in minor units", () => {', '  expect(priceItems([{ unitPrice: 250, quantity: 2, currency: "EUR" }], "EUR")).toBe(500);', "});") },
    "docs/api.md": { text: lines("# API", "", "## POST /v2/orders", "", "Creates an order. An empty cart answers 422 `cart_empty`.", "", "## DELETE /v2/orders/:id", "", "Cancels a pending order.") },
    "assets/logo.png": { binary: true, size: 24_576 },
    "fixtures/orders-dump.json": { size: 3_480_000 },
  };
}

/** The agent's files now: its base folder plus its changes (deleted ones gone). */
function filesOf(a: AgentView, changes: FileChange[]): Record<string, MockFile> {
  const files = { ...baseFiles(a) };
  for (const c of changes) {
    if (c.status === "D") {
      delete files[c.path];
      continue;
    }
    if (c.binary) files[c.path] = { binary: true, size: 42_000 };
    else files[c.path] = { text: versionsFor(c, c.path).modified ?? "" };
  }
  return files;
}

const statusOf = (c: FileChange) => c.status ?? (c.untracked ? "U" : "M");

function hugeText(size: number): string {
  const row = '  { "id": 1042, "sku": "CART-7781", "currency": "EUR", "quantity": 2, "unitPrice": 1299 },\n';
  return "[\n" + row.repeat(Math.ceil(size / row.length)) + "]\n";
}

function globRe(glob: string): RegExp {
  let g = glob.trim();
  if (!g.includes("/")) g = `**/${g}`;
  const re = g
    .replace(/[.+^${}()|[\]\\]/g, "\\$&")
    .replace(/\*\*\//g, "\u0000")
    .replace(/\*\*/g, ".*")
    .replace(/\*/g, "[^/]*")
    .replace(/\?/g, "[^/]")
    .replace(/\u0000/g, "(?:.*/)?");
  return new RegExp(`^${re}(?:/.*)?$`);
}

export function createExplorerMock(d: Deps): ExplorerApi {
  let searchSeq = 0;
  const all = (id: string) => {
    const a = d.find(id);
    return { a, files: filesOf(a, d.changes(id)), changes: d.changes(id) };
  };

  return {
    async listFiles(agentId, dir = "", ignored = false) {
      const { files, changes } = all(agentId);
      const prefix = dir ? dir + "/" : "";
      if (dir && !Object.keys(files).some((p) => p.startsWith(prefix))) throw `${dir}: not found`;
      const kids = new Map<string, FileEntry>();
      for (const [path, f] of Object.entries(files)) {
        if (!path.startsWith(prefix)) continue;
        const rest = path.slice(prefix.length);
        const name = rest.split("/")[0];
        const isDir = rest.includes("/");
        const full = prefix + name;
        const ign = !!f.ignored;
        const prev = kids.get(name);
        if (prev) {
          if (isDir) prev.ignored = prev.ignored && ign;
          continue;
        }
        kids.set(name, { name, path: full, kind: isDir ? "dir" : "file", status: null, changes: 0, ignored: ign });
      }
      for (const c of changes) {
        if (!c.path.startsWith(prefix) || c.status === "D") continue;
        const rest = c.path.slice(prefix.length);
        const k = kids.get(rest.split("/")[0]);
        if (!k) continue;
        if (rest.includes("/")) k.changes++;
        else k.status = statusOf(c);
      }
      const entries = [...kids.values()]
        .filter((e) => ignored || !e.ignored)
        .sort((x, y) => Number(x.kind !== "dir") - Number(y.kind !== "dir") || x.name.toLowerCase().localeCompare(y.name.toLowerCase()));
      const listing: DirListing = { dir, entries, truncated: false, git: true };
      return wait(listing, 90);
    },
    async listAllFiles(agentId) {
      const { files } = all(agentId);
      const list = Object.entries(files)
        .filter(([, f]) => !f.ignored)
        .map(([p]) => p)
        .sort();
      return wait({ files: list, truncated: false, git: true }, 120);
    },
    async readFile(agentId, path, large = false) {
      const { files } = all(agentId);
      const f = files[path];
      if (!f) throw `${path}: not found`;
      const lang = null;
      let view: FileView;
      if (f.binary) view = { path, size: f.size ?? 0, kind: "binary", text: null, lang };
      else if (f.text === undefined) {
        const size = f.size ?? 0;
        view = size > (large ? LARGE_CAP : TEXT_CAP) ? { path, size, kind: "tooLarge", text: null, lang } : { path, size, kind: "text", text: hugeText(size), lang };
      } else view = { path, size: new TextEncoder().encode(f.text).length, kind: "text", text: f.text, lang };
      return wait(view, large ? 400 : 70);
    },
    async searchFiles(agentId, q) {
      const seq = ++searchSeq;
      const { files } = all(agentId);
      await wait(null, 250);
      if (seq !== searchSeq) throw SEARCH_CANCELLED;
      let re: RegExp;
      try {
        const src = q.regex ? q.query : q.query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        re = new RegExp(q.wholeWord ? `\\b(?:${src})\\b` : src, q.caseSensitive ? "g" : "gi");
      } catch (e) {
        throw `regex parse error: ${e instanceof Error ? e.message : e}`;
      }
      const inc = q.include.map(globRe);
      const exc = [...q.exclude, ".git", ...(q.defaultExcludes === false ? [] : ["node_modules", "bower_components"])].map(globRe);
      const matches: SearchMatch[] = [];
      const max = q.maxResults ?? 2000;
      let truncated = false;
      for (const [path, f] of Object.entries(files).sort(([x], [y]) => x.localeCompare(y))) {
        if (f.text === undefined || f.ignored) continue;
        if (inc.length && !inc.some((r) => r.test(path))) continue;
        if (exc.some((r) => r.test(path))) continue;
        f.text.split("\n").forEach((line, i) => {
          if (truncated) return;
          re.lastIndex = 0;
          const ranges: { start: number; end: number }[] = [];
          for (let m = re.exec(line); m; m = re.exec(line)) {
            if (m[0] === "") {
              re.lastIndex++;
              continue;
            }
            ranges.push({ start: m.index, end: m.index + m[0].length });
          }
          if (!ranges.length) return;
          if (matches.length >= max) {
            truncated = true;
            return;
          }
          matches.push({ path, line: i + 1, column: ranges[0].start + 1, text: line, textOffset: 0, ranges });
        });
      }
      const result: SearchResult = { matches, files: new Set(matches.map((m) => m.path)).size, truncated, engine: "ripgrep" };
      return result;
    },
    async cancelSearch() {
      searchSeq++;
    },
  };
}
