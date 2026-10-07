// Records the landing-page demo from the app's mock (see website/README.md "Demo").
//   pnpm demo:serve                       # mock on http://127.0.0.1:5199
//   chrome --headless=new --remote-debugging-port=9334 …
//   node scripts/record-demo.mjs <outDir> dark|light
// Writes JPEG frames, frames.txt (ffmpeg concat list with real durations) and
// chapters.json (seconds at which each tab's moment starts).

import { mkdirSync, writeFileSync } from "node:fs";
import { open } from "./cdp.mjs";
const [out, theme = "dark"] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
const W = 1120, H = 700, DPR = 1.5;
const p = await open(9334, { W, H, DPR, theme });
const { ev, sleep, click, s } = p;
await s("Page.navigate", { url: "http://127.0.0.1:5199/?onboarded&shots=1" });
await sleep(4000);
const row = (name) => `[...document.querySelectorAll('.agent-row')].find(r=>r.querySelector('.agent-name')?.textContent===${JSON.stringify(name)})`;
const q = (sel) => `document.querySelector(${JSON.stringify(sel)})`;
const backBtn = `[...document.querySelectorAll('button.small-btn')].find(b=>b.textContent.trim().startsWith('Back'))`;
const jumpBtn = `[...document.querySelectorAll('button')].find(b=>b.textContent.trim().startsWith('Jump'))`;
const queueBtn = `[...document.querySelectorAll('aside.right button.small-btn')].find(b=>b.textContent.trim().startsWith('Queue'))`;

await click(q('.preset-btn[title="Tile 2×2"]'));
await sleep(600);
await click(row("docs"));            // (marks docs seen, as in the recorded video)
await sleep(200);
await click(row("tests"));
await sleep(1200);

const frames = [];
let n = 0;
p.listeners.push(async (m) => {
  if (m.method !== "Page.screencastFrame" || m.sessionId !== p.sessionId) return;
  const f = `${out}/f${String(n++).padStart(5, "0")}.jpg`;
  writeFileSync(f, Buffer.from(m.params.data, "base64"));
  frames.push({ f, t: m.params.metadata.timestamp });
  s("Page.screencastFrameAck", { sessionId: m.params.sessionId }).catch(() => {});
});
await s("Page.startScreencast", { format: "jpeg", quality: 90, maxWidth: W * DPR, maxHeight: H * DPR, everyNthFrame: 1 });
const t0 = Date.now() / 1000;
const chapters = {};
const mark = (k) => { chapters[k] = +(Date.now() / 1000 - t0).toFixed(2); };

mark("needs-you");
await sleep(1600);
await click(jumpBtn);
await sleep(1300);
await s("Input.insertText", { text: "1" });
await sleep(5600);

mark("wall");
await click(q('button[title^="Wall"]'));
await sleep(3600);
await click(backBtn);
await sleep(300);

mark("next-up");
await click(row("api-fix"));
await sleep(400);
if (!(await ev(`!!document.querySelector('aside.right')`))) await click(q('button[title^="Show details"]'));
await sleep(700);
await click(q("aside.right textarea"));
for (const ch of "Then run the full suite and fix anything red.") { await s("Input.insertText", { text: ch }); await sleep(35); }
await sleep(400);
await click(queueBtn);
await sleep(2200);
await click(q('aside.right button[aria-label="Close details"]'));
await sleep(300);

mark("review");
await click(q('button[title^="Review"]'));
await sleep(4200);
await click(backBtn);
await sleep(300);

mark("terminals");
await click(q('button[title^="New terminal here"]'));
await sleep(1300);
for (const ch of "claude") { await s("Input.insertText", { text: ch }); await sleep(70); }
await sleep(300);
await p.key("Enter", { code: "Enter", vk: 13 });
await sleep(3000);

mark("rules");
await click(q('button[title="Settings"]'));
await sleep(500);
await ev(`(()=>{const l=[...document.querySelectorAll('.modal .setting-head .label')].find(l=>l.textContent==='Rules'); l&&l.scrollIntoView({block:'center',behavior:'smooth'}); return !!l})()`);
await sleep(3000);
mark("end");
await s("Page.stopScreencast");
await sleep(300);

let txt = "";
for (let i = 0; i < frames.length; i++) {
  const d = i + 1 < frames.length ? frames[i + 1].t - frames[i].t : 0.1;
  txt += `file '${frames[i].f}'\nduration ${d.toFixed(4)}\n`;
}
txt += `file '${frames[frames.length - 1].f}'\n`;
writeFileSync(`${out}/frames.txt`, txt);
const offset = frames.length ? frames[0].t - t0 : 0;
writeFileSync(`${out}/chapters.json`, JSON.stringify({ chapters, firstFrameOffset: offset }, null, 2));
console.log("frames", frames.length, JSON.stringify(chapters), "offset", offset.toFixed(2), p.logs.join("\n"));
await p.close();
