// Demo bridge for the marketing site (website/ embeds the browser mock build in an
// iframe and lets visitors switch views from tabs outside it).
//
// Active only in the browser mock (never inside Tauri) and only when the page was
// opened with `?demo=1`. It accepts same-origin messages
// `{ type: "pitwall-demo", view }` and drives the UI by clicking the app's own
// controls, so it needs no access to app state. Replies `{ type: "pitwall-demo-ready" }`
// once agents are on screen.

type View = "needs-you" | "wall" | "next-up" | "review" | "terminals" | "rules";

const enabled =
  typeof window !== "undefined" &&
  !("__TAURI_INTERNALS__" in window) &&
  new URLSearchParams(window.location.search).get("demo") === "1";

const $ = (sel: string) => document.querySelector<HTMLElement>(sel);
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function clickBack() {
  for (const b of document.querySelectorAll<HTMLButtonElement>("button.small-btn")) {
    if (b.textContent?.trim().startsWith("Back")) b.click();
  }
}

function selectAgent(name: string) {
  const rows = [...document.querySelectorAll<HTMLElement>(".agent-row")];
  rows.find((r) => r.querySelector(".agent-name")?.textContent === name)?.click();
}

const preset = (label: string) => $(`.preset-btn[title="Tile ${label}"]`)?.click();

const views: Record<View, () => void | Promise<void>> = {
  "needs-you": () => {
    preset("2×2");
    selectAgent("tests");
  },
  wall: () => $('button[title^="Wall"]')?.click(),
  "next-up": async () => {
    preset("2×2");
    selectAgent("api-fix");
    await wait(100);
    if (!$("aside.right")) $('button[title^="Show details"]')?.click();
  },
  review: () => $('button[title^="Review"]')?.click(),
  terminals: () => preset("3×2"),
  rules: async () => {
    $('button[title="Settings"]')?.click();
    await wait(200);
    const label = [...document.querySelectorAll<HTMLElement>(".modal .setting-head .label")].find(
      (l) => l.textContent === "Rules",
    );
    label?.scrollIntoView({ block: "start" });
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
    const d = e.data as { type?: string; view?: View } | null;
    if (d?.type === "pitwall-demo" && d.view && d.view in views) void show(d.view);
  });
  const ready = setInterval(() => {
    if (!document.querySelector(".agent-row")) return;
    clearInterval(ready);
    window.parent?.postMessage({ type: "pitwall-demo-ready" }, window.location.origin);
  }, 200);
}

export {};
