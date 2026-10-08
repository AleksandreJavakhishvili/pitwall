// Builds the React UI (repo root src/, the web demo; src/README.md) as a static
// browser bundle into website/public/demo/. It always runs its built-in mock mode,
// so the result is a self-contained, clickable demo. The landing page embeds it as
// <base>demo/?onboarded&shots=1&demo=1 (skip the welcome screen, hide the MOCK chip,
// enable the tab bridge in src/lib/demoBridge.ts).
//
//   node scripts/build-demo.mjs [--base /pitwall/]   # base = where the SITE is served
//   node scripts/build-demo.mjs --serve [port]       # dev server of the same thing (recording the README animation)
//
// It uses the app's own Vite and vite.config.ts from the repo root, so the root
// dependencies must be installed (`pnpm install` at the root).
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const site = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const root = resolve(site, "..");
const viteEntry = resolve(root, "node_modules/vite/dist/node/index.js");
if (!existsSync(viteEntry)) {
  console.error("build-demo: run `pnpm install` at the repo root first (the demo is built with the app's Vite).");
  process.exit(1);
}
const { build, createServer, loadConfigFromFile, mergeConfig } = await import(pathToFileURL(viteEntry).href);

function normalizeBase(b) {
  let s = (b || "/").trim();
  if (!s.startsWith("/")) s = "/" + s;
  if (!s.endsWith("/")) s += "/";
  return s;
}

const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(name);
  return i === -1 ? undefined : (args[i + 1] && !args[i + 1].startsWith("--") ? args[i + 1] : "");
};
const siteBase = normalizeBase(flag("--base") ?? process.env.SITE_BASE ?? "/");
const serve = flag("--serve");

const loaded = await loadConfigFromFile(
  { command: serve !== undefined ? "serve" : "build", mode: "production" },
  resolve(root, "vite.config.ts"),
  root,
);
const common = { root, configFile: false, clearScreen: false };

if (serve !== undefined) {
  const port = Number(serve) || 5199;
  const server = await createServer(
    mergeConfig(loaded.config, { ...common, server: { port, strictPort: true, host: "127.0.0.1" } }),
  );
  await server.listen();
  server.printUrls();
} else {
  const outDir = resolve(site, "public/demo");
  await build(
    mergeConfig(loaded.config, {
      ...common,
      base: `${siteBase}demo/`,
      logLevel: "warn",
      build: { outDir, emptyOutDir: true, reportCompressedSize: false },
    }),
  );
  console.log(`build-demo: app mock → ${outDir} (served at ${siteBase}demo/)`);
}
