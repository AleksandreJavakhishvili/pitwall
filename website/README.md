# Pitwall website

Static marketing site for Pitwall: a one-screen landing page built around a demo,
`/download/`, `/docs/how-it-works/`, `/docs/quick-start/`, `/changelog/`, `/roadmap/` and a 404.

`/changelog/` and `/roadmap/` are rendered at build time from the repository's
`CHANGELOG.md` and `ROADMAP.md`: each page has a `<!-- markdown:<name> -->` marker
that the `pitwall-markdown-pages` plugin in `vite.config.js` replaces with the output
of `scripts/pages.mjs` (a small Markdown converter in `scripts/markdown.mjs`, for the
subset those files use). Edit the Markdown files, not the pages.

Plain HTML pages and one CSS file, bundled by Vite. No framework, no analytics, no
external CDNs. The one outside request is the download page asking GitHub's API for
the latest release. Fonts (Inter, JetBrains Mono, Barlow Condensed) are self-hosted
from `@fontsource` packages.

## Run

```sh
cd website
pnpm install --ignore-workspace
pnpm dev                       # http://localhost:4321 (pages only; no live demo in dev)
pnpm build                     # live demo + pages → website/dist, served from /
pnpm build --base /pitwall/    # same, for a sub-path (GitHub Pages project site)
pnpm build --skip-demo         # pages only
pnpm preview                   # serve dist locally
```

`pnpm build` runs `scripts/build.mjs`: first `scripts/build-demo.mjs` builds the app's
browser mock (repo root, needs `pnpm install` there) into `public/demo/` with the base
`<base>demo/`, then `vite build` builds the pages with the same base. Pages are written
with root-absolute links (`/docs/…`, `/video/…`); the `pitwall-site-links` plugin in
`vite.config.js` prefixes the base on the ones Vite doesn't handle itself (anchors,
`data-*`), so the same source works at `/` and under `/<repo>/`. In JS, use
`import.meta.env.BASE_URL`.

The GitHub owner and repository name live only in `site.config.js`; pages use
`%REPO_URL%`, `%RELEASES_URL%`, `%RELEASE_API_URL%` and `%PAGES_URL%`. Deployment is
`.github/workflows/pages.yml` (base from the repository name).

## Layout

```
index.html                    landing: hero, demo with tabs, agents, download / GitHub
download/index.html           per-OS downloads, first-launch notes, build from source
docs/how-it-works/index.html  flags, Wall, Next up, Review, terminals, agents, rules, agw, CLI, roadmap
docs/quick-start/index.html   install, build from source, first steps
changelog/index.html          page chrome; content from ../CHANGELOG.md
roadmap/index.html            page chrome; content from ../ROADMAP.md (Now / Next / Later lanes)
404.html
site.config.js                OWNER / REPO and the URLs derived from them
src/site.css                  all styles; tokens mirror the app's src/styles/tokens.css
src/main.js                   theme toggle, demo tabs, live demo, download links
scripts/build.mjs             full build (demo + pages)
scripts/pages.mjs             renders CHANGELOG.md / ROADMAP.md into the two pages
scripts/markdown.mjs          minimal Markdown → HTML
scripts/build-demo.mjs        builds the app's browser mock into public/demo/ (or serves it)
public/video/                 demo recording, dark + light, with poster frames
public/favicon.svg            copy of docs/brand/pitwall-mark.svg
```

## Downloads

`/download/` has a card per OS; the visitor's OS (`navigator.userAgentData`, else the
user agent) goes first. Every button links to `%RELEASES_URL%` in the HTML, so the
page works without JavaScript. `src/main.js` then reads `%RELEASE_API_URL%` (GitHub's
latest release, cached for 10 minutes in `sessionStorage`) and points each button at
the asset whose name ends with its `data-asset` suffix (`_universal.dmg`,
`_x64-setup.exe`, `_amd64.AppImage`, …), with the version, date and file sizes. When
the API fails, is rate-limited or there's no release, the buttons stay on the
Releases page and a note says so. Asset names come from `.github/workflows/release.yml`.

## Demo

The landing page shows a ~31-second looping recording of the real app in its browser
mock mode (`public/video/demo-{dark,light}.mp4`, 1680×1050, ~1.4 MB each, H.264,
muted). It was recorded at a 1120×700 window so UI text is close to its real size in
the 1100 px frame. The tabs seek to chapters in it (`data-t` on each tab in
`index.html`); the chapters are in tab order. Until a visitor picks a tab the whole
tour plays and the tabs follow it; after that, the video loops the picked chapter.
With `prefers-reduced-motion` the video doesn't autoplay; a Play button is shown.

On wide screens a **Try it live** button swaps the video for the app itself in an
iframe (`<base>demo/?onboarded&shots=1&demo=1`), rendered at 1120×700 and scaled to
the frame (below 1100 px the app collapses its sidebar). The tabs then drive it via
`postMessage`, handled by `src/lib/demoBridge.ts` in the app, which is inert outside
the browser mock and without `?demo=1`. Each start clears the mock's `pitwall.*`
localStorage keys. The mock data only uses
placeholder paths and names (`/Users/dev`, `dev@mac`).

**Re-recording the video:** `pnpm demo:serve` (the mock on port 5199), open `?onboarded&shots=1`
(`shots=1` hides the MOCK chip) in headless Chrome at 1120×700, device scale 1.5, and script it over CDP: 2×2, tests focused → Jump →
type `1` → Wall → api-fix, queue a prompt in Next up → Review → ⌘T terminal, type
`claude` → Settings/Rules. Record with `Page.startScreencast`, then encode with ffmpeg:
30 fps, 1680×1050, a 0.8 s crossfade from the end into the start for a seamless loop,
`-crf 27 -preset veryslow -tune animation -movflags +faststart`. Posters are a frame
at 1.5 s (`cwebp -q 78`). If the chapter times change, update `data-t` on the tabs.
The README's animated `docs/media/demo.webp` comes from the dark video (10 fps,
1120 px wide, `img2webp -lossy -q 55`).

## TODO

- Re-record the demo video when the UI changes (see Demo).
- No Open Graph image or social card yet.
