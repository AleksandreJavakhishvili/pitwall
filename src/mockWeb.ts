// Web tiles in the web demo (docs: CHANGELOG 0.8.0 "Web pages in tiles").
//
// The desktop app shows web pages in tiles with the system's web view. The
// demo can't load real pages, and mustn't: it makes no outside requests. So an
// agent's Preview here opens a made-up page drawn by components/WebTile.tsx.
// Only the agents listed below print a dev server address in their terminal
// (mock.ts, `screens`), so only their header offers Preview.

/** The dev server address an agent's terminal printed, by agent id. */
const PREVIEWS: Record<string, string> = {
  "a-docs": "http://127.0.0.1:8000/onboarding/",
};

export function previewOf(agentId: string): string | null {
  return PREVIEWS[agentId] ?? null;
}

/** `http://127.0.0.1:8000/onboarding/` → `127.0.0.1:8000/onboarding/`, as the address bar shows it. */
export const shortUrl = (url: string) => url.replace(/^https?:\/\//, "");

/** `:8000`, for the header's Preview button. */
export function portOf(url: string): string {
  const m = /:(\d+)(?:\/|$)/.exec(url.replace(/^https?:\/\//, ""));
  return m ? `:${m[1]}` : "";
}
