// Full site build: the live app demo (scripts/build-demo.mjs → public/demo/), then
// the static pages (vite build → dist/), then the link check over dist/
// (scripts/check-links.mjs). All get the same base path.
//
//   pnpm build                      # served from the domain root
//   pnpm build --base /pitwall/     # served from a sub-path (GitHub Pages project site)
//   pnpm build --skip-demo          # pages only; the demo frame says the demo is missing
//   pnpm build --skip-link-check    # don't check links after building
//   pnpm build --external-links     # also request every external link (needs the network)
//
// SITE_BASE=/pitwall/ works too, instead of --base.
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const site = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const i = args.indexOf("--base");
let base = (i !== -1 ? args[i + 1] : process.env.SITE_BASE) || "/";
if (!base.startsWith("/")) base = "/" + base;
if (!base.endsWith("/")) base += "/";

function run(file, argv) {
  const r = spawnSync(process.execPath, [file, ...argv], { cwd: site, stdio: "inherit" });
  if (r.status !== 0) process.exit(r.status ?? 1);
}

if (!args.includes("--skip-demo")) run(resolve(site, "scripts/build-demo.mjs"), ["--base", base]);
run(resolve(site, "node_modules/vite/bin/vite.js"), ["build", "--base", base]);
if (!args.includes("--skip-link-check")) {
  run(resolve(site, "scripts/check-links.mjs"), ["--base", base, ...(args.includes("--external-links") ? ["--external"] : [])]);
}
console.log(`site built into ${resolve(site, "dist")} for base ${base}`);
