// Demo bridge for the marketing site (website/ embeds the browser mock build in an
// iframe and lets visitors switch views from tabs outside it).
//
// Active only in the browser mock (never inside Tauri) and only when the page was
// opened with `?demo=1`. It accepts same-origin messages
// `{ type: "pitwall-demo", view }` and drives the UI by clicking the app's own
// controls, so it needs no access to app state. Replies `{ type: "pitwall-demo-ready" }`
// once agents are on screen. `{ type: "pitwall-demo", theme: "dark" | "light" }` (or
// `?theme=` in the URL) makes the app follow the site's theme toggle instead of the OS.

import { applyTheme } from "./theme";

type View = "needs-you" | "files" | "web" | "review" | "wall" | "next-up";

const enabled =
  typeof window !== "undefined" &&
  !("__TAURI_INTERNALS__" in window) &&
  new URLSearchParams(window.location.search).get("demo") === "1";

const $ = (sel: string) => document.querySelector<HTMLElement>(sel);
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
/** The element once it's on screen (the mock answers after a short delay), or null. */
async function until(sel: string, ms = 2000): Promise<HTMLElement | null> {
  for (let t = 0; t < ms; t += 50) {
    const el = $(sel);
    if (el) return el;
    await wait(50);
  }
  return null;
}

function clickBack() {
  for (const b of document.querySelectorAll<HTMLButtonElement>("button.small-btn")) {
    if (b.textContent?.trim().startsWith("Back")) b.click();
  }
  // A files or web tile beside the space.
  $(".side-tile [data-tile-close]")?.click();
}

function selectAgent(name: string) {
  const rows = [...document.querySelectorAll<HTMLElement>(".agent-row")];
  rows.find((r) => r.querySelector(".agent-name")?.textContent === name)?.click();
}

const preset = (label: string) => $(`.preset-btn[title="Tile ${label}"]`)?.click();

/** The focused agent's files in the file viewer, from the palette's "Browse files". */
async function browseFiles() {
  $("button.palette-hint")?.click();
  await until(".palette-item");
  const items = [...document.querySelectorAll<HTMLElement>(".palette-item")];
  items.find((i) => i.textContent?.trim().startsWith("Browse files"))?.click();
}

/** Open a folder of the file viewer's tree, unless it is open already. */
async function unfold(path: string) {
  const row = await until(`.ex-side [data-ex-row="${path}"]`);
  if (row && row.closest("li")?.getAttribute("aria-expanded") !== "true") row.click();
}

const views: Record<View, () => void | Promise<void>> = {
  "needs-you": () => {
    preset("2×2");
    selectAgent("tests");
  },
  // api-fix's files in the file viewer: a files tile beside its terminal.
  files: async () => {
    preset("1");
    selectAgent("api-fix");
    await wait(100);
    await browseFiles();
    await unfold("src");
    await unfold("src/orders");
    (await until('.ex-side [data-ex-row="src/orders/handler.ts"]'))?.click();
  },
  // docs' Preview of the dev server its terminal printed: a web tile beside it.
  web: async () => {
    preset("1");
    selectAgent("docs");
    // Not the button of the tile that was there before the layout changed.
    await wait(150);
    (await until('.pane[data-agent-id="a-docs"] [data-preview]'))?.click();
  },
  review: () => $('button[title^="Review"]')?.click(),
  wall: () => $('button[title^="Wall"]')?.click(),
  // api-fix's Next up: the popover its tile's "queued" chip opens.
  "next-up": async () => {
    preset("2×2");
    selectAgent("api-fix");
    await wait(100);
    (await until('.pane[data-agent-id="a-api-fix"] .next-up-chip'))?.click();
  },
};

async function show(view: View) {
  $('.modal-head button[aria-label="Close"]')?.click();
  clickBack();
  await wait(150);
  await views[view]?.();
}

if (enabled) {
  window.addEventListener("message", (e) => {
    if (e.origin !== window.location.origin) return;
    const d = e.data as { type?: string; view?: View; theme?: string } | null;
    if (d?.type !== "pitwall-demo") return;
    if (d.theme === "dark" || d.theme === "light") applyTheme(d.theme);
    if (d.view && d.view in views) void show(d.view);
  });
  const ready = setInterval(() => {
    if (!document.querySelector(".agent-row")) return;
    clearInterval(ready);
    // After the app has applied its own (System) preference from its saved state.
    const theme = new URLSearchParams(window.location.search).get("theme");
    if (theme === "dark" || theme === "light") applyTheme(theme);
    window.parent?.postMessage({ type: "pitwall-demo-ready" }, window.location.origin);
  }, 200);
}

export {};
