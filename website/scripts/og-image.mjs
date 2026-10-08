// Renders the link-preview image, public/og.png (1200×630), from the HTML card below
// with headless Chrome. Re-run after changing the card:
//
//   node scripts/og-image.mjs                 # finds Chrome, or set CHROME=/path/to/chrome
//
// Demo content only (made-up projects and agents), same as the site's demo.
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { open } from "./cdp.mjs";

const site = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const out = resolve(site, "public/og.png");
const W = 1200, H = 630;

const CHROMES = [
  process.env.CHROME,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Chromium.app/Contents/MacOS/Chromium",
  "/usr/bin/google-chrome",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
].filter(Boolean);

const font = (pkg, file) => pathToFileURL(resolve(site, "node_modules", pkg, "files", file)).href;
const mark = readFileSync(resolve(site, "public/favicon.svg"), "utf8");

const rows = [
  { pos: 1, agent: "tests", project: "checkout-web", cli: "Claude Code", state: "blocked", flag: "▲ Needs you" },
  { pos: 2, agent: "api-fix", project: "orders-api", cli: "Codex", state: "working", flag: "◐ Working" },
  { pos: 3, agent: "docs", project: "checkout-web", cli: "Gemini CLI", state: "done", flag: "⚑ Done" },
  { pos: 4, agent: "refactor", project: "orders-api", cli: "opencode", state: "idle", flag: "● Idle" },
];

const html = `<!doctype html>
<html><head><meta charset="utf-8" /><style>
@font-face { font-family: Inter; src: url(${font("@fontsource-variable/inter", "inter-latin-wght-normal.woff2")}) format("woff2"); font-weight: 100 900; }
@font-face { font-family: Mono; src: url(${font("@fontsource-variable/jetbrains-mono", "jetbrains-mono-latin-wght-normal.woff2")}) format("woff2"); font-weight: 100 800; }
@font-face { font-family: Barlow; src: url(${font("@fontsource/barlow-condensed", "barlow-condensed-latin-600-normal.woff2")}) format("woff2"); font-weight: 600; }
@font-face { font-family: Barlow; src: url(${font("@fontsource/barlow-condensed", "barlow-condensed-latin-700-normal.woff2")}) format("woff2"); font-weight: 700; }
* { box-sizing: border-box; margin: 0; }
html, body { width: ${W}px; height: ${H}px; overflow: hidden; }
body {
  --bg: #0a0b0d; --surface: #0f1114; --line: #1e2227; --line-strong: #2a2f36;
  --text: #e6e8eb; --text-2: #a8aeb6; --text-3: #7c838d;
  --green: #3fd07f; --amber: #ffb224; --amber-soft: rgba(255, 178, 36, 0.1); --flag: #f4f5f7; --flag-dark: #2a2e35;
  background: var(--bg); color: var(--text); font-family: Inter, sans-serif; position: relative;
}
.chequer { position: absolute; left: 0; right: 0; top: 0; height: 28px;
  background: conic-gradient(var(--flag) 25%, var(--flag-dark) 0 50%, var(--flag) 0 75%, var(--flag-dark) 0) 0 0 / 28px 28px; opacity: 0.9; }
.main { position: absolute; inset: 28px 0 0 0; padding: 52px 64px 48px; display: grid; grid-template-columns: 1fr 470px; gap: 48px; }
.brand { display: flex; align-items: center; gap: 16px; font: 700 40px Barlow, sans-serif; letter-spacing: 0.14em; }
.brand svg { width: 60px; height: 60px; }
h1 { margin-top: 44px; font-size: 56px; line-height: 1.06; font-weight: 750; letter-spacing: -0.025em; }
h1 span { color: var(--text-3); }
.sub { margin-top: 26px; font-size: 24px; line-height: 1.4; color: var(--text-2); max-width: 30ch; }
.foot { position: absolute; left: 64px; bottom: 44px; font: 600 21px Barlow, sans-serif; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-3); }
.board { align-self: center; margin-top: 8px; border: 1px solid var(--line-strong); border-radius: 12px; background: var(--surface); overflow: hidden; }
.head { display: flex; justify-content: space-between; padding: 14px 20px; border-bottom: 1px solid var(--line);
  font: 600 16px Barlow, sans-serif; letter-spacing: 0.16em; text-transform: uppercase; color: var(--text-3); }
.row { display: grid; grid-template-columns: 30px 1fr auto; align-items: center; gap: 12px; padding: 16px 20px; border-bottom: 1px solid var(--line); }
.row:last-child { border-bottom: 0; }
.pos { font: 500 18px Mono, monospace; color: var(--text-3); }
.agent { font: 650 21px Mono, monospace; color: var(--text); }
.meta { margin-top: 4px; font: 400 15px Mono, monospace; color: var(--text-3); }
.flag { font: 600 19px Barlow, sans-serif; letter-spacing: 0.1em; text-transform: uppercase; white-space: nowrap; display: flex; align-items: center; gap: 8px; }
.blocked { background: var(--amber-soft); box-shadow: inset 4px 0 0 var(--amber); }
.blocked .flag { color: var(--amber); }
.working .flag { color: var(--green); }
.done .flag { color: var(--flag); }
.idle .flag { color: var(--text-3); }
.mini { display: inline-block; width: 12px; height: 16px;
  background: conic-gradient(var(--flag) 25%, var(--flag-dark) 0 50%, var(--flag) 0 75%, var(--flag-dark) 0) 0 0 / 6px 6px; }
</style></head>
<body>
  <div class="chequer"></div>
  <div class="main">
    <div>
      <div class="brand">${mark}<span>PITWALL</span></div>
      <h1>You call the strategy.<br /><span>Agents drive.</span></h1>
      <p class="sub">Run Claude Code, Codex, Gemini CLI and more side by side. See which one needs you.</p>
    </div>
    <div class="board">
      <div class="head"><span>Pos · Agent</span><span>Flag</span></div>
      ${rows.map((r) => `<div class="row ${r.state}"><span class="pos">${r.pos}</span><div><div class="agent">${r.agent}</div><div class="meta">${r.project} · ${r.cli}</div></div><span class="flag">${r.flag}${r.state === "done" ? '<span class="mini"></span>' : ""}</span></div>`).join("\n      ")}
    </div>
  </div>
  <p class="foot">Desktop app · macOS · Linux · Windows · Open source, Apache-2.0</p>
</body></html>`;

const tmp = mkdtempSync(join(tmpdir(), "pitwall-og-"));
const page = join(tmp, "og.html");
writeFileSync(page, html);

const port = 9300 + Math.floor(Math.random() * 500);
const bin = CHROMES.find((b) => existsSync(b));
if (!bin) {
  console.error("Chrome not found; set CHROME=/path/to/chrome");
  process.exit(1);
}
const chrome = spawn(bin, [
  "--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${join(tmp, "profile")}`,
  "--no-first-run", "--no-default-browser-check", "--hide-scrollbars", "--allow-file-access-from-files", "about:blank",
], { stdio: "ignore" });

try {
  let p;
  for (let i = 0; i < 50 && !p; i++) {
    try {
      p = await open(port, { W, H, DPR: 1, theme: "dark" });
    } catch {
      await new Promise((r) => setTimeout(r, 200));
    }
  }
  if (!p) throw new Error("could not connect to Chrome");
  await p.s("Page.navigate", { url: pathToFileURL(page).href });
  await p.sleep(500);
  await p.ev("document.fonts.ready.then(() => true)");
  await p.sleep(200);
  await p.shot(out);
  if (p.logs.length) console.error(p.logs.join("\n"));
  await p.close();
  console.log(`wrote ${out}`);
} finally {
  const exited = new Promise((r) => chrome.once("exit", r));
  chrome.kill();
  await exited;
  rmSync(tmp, { recursive: true, force: true });
}
