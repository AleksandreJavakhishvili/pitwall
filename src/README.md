# src/: the web demo

This React UI was Pitwall's desktop interface up to v0.1.x (inside the
Tauri shell). Since v0.2.0 the desktop app is the native GPUI app in
`crates/pitwall-app`, and this code is **the website's live demo only**:

- It always runs against the in-browser mock (`mock.ts`, `mockExplorer.ts`,
  `mockReview.ts`, `mockWorktrees.ts`, `rules/mock.ts`); there is no
  backend and no Tauri.
- `website/scripts/build-demo.mjs` builds it into `website/public/demo/`;
  the landing page embeds it (`?onboarded&shots=1&demo=1`, with the tab
  bridge in `lib/demoBridge.ts`).
- `pnpm dev` serves it locally, `pnpm build` type-checks and builds it,
  `pnpm test` runs its unit tests (vitest). CI runs all of them.

Keep it close to what the GPUI app shows when a visible change lands there;
no new features go here.
