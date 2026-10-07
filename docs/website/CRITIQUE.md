# Website critique

Reviewed on 2026-10-06 from the built site (`vite preview`), screenshotted in headless
Chrome at 1440, 1024, 390 and 360 px, dark and light (`prefers-color-scheme` emulation).
Facts were checked against `docs/spec/*.md`. Reference for tone and length: herdr.dev.

The base is better than most AI-made pages. The palette is disciplined, colour is used only
for status, the copy is specific and there are no "seamless"/"powerful" adjectives. What gives
it away is the structure: it is a spec sheet laid out with one template, repeated ten times.

Ranked by impact.

## 1. One section template, used ten times

Every section is the same: a numbered eyebrow (`01 WHO NEEDS YOU` … `10 COMING, NOT SHIPPED`),
an all-caps H2 at the same size, a grey 60-character paragraph, a full-width screenshot, then a
hairline-ruled bullet list. The headings are all the same size and the gaps are all 132 px.
Ten numbered sections plus a keyboard grid is the "feature tour" pattern a template generator
makes. Nothing is emphasised, so nothing stands out. herdr.dev makes its point in five short
blocks.

## 2. Too long, and it explains too much

The page is 12,200 px tall at 1440 and 13,400 px at 360. A marketing page tells the reader
about `GIT_INDEX_FILE`, `setsid`, SIGHUP/SIGKILL timings, "re-parents the terminal view",
bracketed paste, `.git/info/exclude` and `npx -y rulesync`. Those facts are true and belong in
the docs. Here they bury the three things a first-time reader needs to learn: Pitwall shows
which agent needs you, you can see every terminal at once, and quitting doesn't kill them. The
Rules, Keyboard and Layout sections repeat the quick start almost word for word.

## 3. The hero stacks seven things before the product

Eyebrow, a 6 rem headline, lede, form, button, note, a row of mono "facts", then an amber
"radio" strip. The product screenshot starts at y≈910, below the fold on a 1440×900 screen.
The facts row (`Tauri 2 · Rust · React`, `diff editor bundled, no CDN`) is filler for the
person deciding whether to try it. The amber strip shows the same "tests needs you" line that
the screenshot's own bottom bar shows, and the timing board shows it again a screen later:
the same message three times.

## 4. The main call to action goes nowhere

The first thing the eye lands on is an email field and a solid "Join the waitlist" button.
Submitting it says "The waitlist isn't open yet, so nothing was sent." A button that pretends
to work is the most template-like thing on the page. It is also repeated in the closing
section. The one action that works today, building from source, is the secondary button.

## 5. Screenshots: four of the same picture, too small to read, and MOCK in every one

The hero, Wall, Review and 3×2 shots look alike from a distance: the same sidebar, the same
amber bar, the same dark rectangle. Each 1600–1920 px capture is shown at 1100 px (57–69 %),
so terminal text is about 7 px and can't be read. At 390 px the hero is a 358 px thumbnail of
a whole desktop app. Every shot has a **MOCK** chip in the title bar, and the caption says
"Captured from the app's built-in mock mode", which makes the screens look less real than they
are.

## 6. The status section shows only some of the flags and repeats one shape

The timing board is the most Pitwall-looking thing on the page. Below it, the legend explains
only three of the flags (needs you, done, working), all in three equal columns, and "How it
knows" repeats the same three-column layout. Idle and stopped get no explanation. Needs you is
meant to be the loudest flag but gets the same space as the others. The working dot uses a
pulse, while the app and the quick start use ◐. At 360 px the Diff column runs into the right
edge (`-3` is clipped).

## 7. The agent list is a spreadsheet

It's a six-column table with a 680 px minimum width: it scrolls sideways on phones, and 23 of
its 60 cells are "—". The Rules target column (`claudecode`, `copilotcli`) is an internal
identifier. The one thing a visitor is looking for is "is my CLI on the list?", and they have
to read past five columns of flags to find it.

## 8. Facts that are overclaimed or wrong (vs docs/spec)

- "It doesn't scan Desktop, Documents or Downloads." The spec (onboarding.md) only says they
  aren't *enumerated*; projects there are still found through transcripts and editor history.
- First-launch sources leave out the one-level scan of `~/projects ~/code ~/Developer ~/src ~/dev ~/work`.
- Presets "offered when they fit the window" is said of all presets. Only 3×3, 4×3 and 4×4
  depend on window size (layout.md).
- "An agent lives in exactly one pane": the spec says *at most* one (it can be a chip or not
  placed at all).
- Roadmap: "Plain SSH machines after that" sits under a Wave 2 tag, but roadmap.md puts the SSH
  provider in Wave 3.
- Native notifications are attributed only to needs you; done notifies too (backend.md).
- The board's diff numbers and the screenshot alt text (`+148 −37`, "five changed files") no
  longer match the app's demo data (`+174 −74`, seven files).

## 9. Smaller things

- **Docs page shows a dashed `TODO: repo URL` pill** to visitors. That's a leftover note, not copy.
- **Same-sized type everywhere**: the section H2s are all 3.1 rem, and the closing H2 is nearly
  as big as the hero's.
- **Mobile hero**: the three buttons stack with mismatched widths, and the facts row wraps into
  a ragged block.
- **Performance**: in light mode the eager, high-priority *dark* hero image downloads anyway,
  because `display:none` doesn't stop an eager image. The preload has the same problem on
  phones, which get the full-width desktop shot.
- **Keyboard section** duplicates the quick-start table, and five of its shortcuts aren't in
  any spec (⌘J, ⌘N, ⌘1–9, ⌘B, ⌘↵), so they can't be checked from the docs.
- **Contrast is fine.** `--text-3` is about 5:1 on both backgrounds and the amber row text
  passes. The 0.74 rem condensed table headers are the weakest text on the page.

## After

Halfway through, the brief changed. The new brief asked for a short, interactive page with
no screenshots, so the landing page was rebuilt rather than polished section by section.

What changed:

- **Landing is 1,960 px at 1440 (about 2.2 screens), down from 12,200.** It now has a hero,
  a demo, three one-line rows (agents, ⌘Q, coming) and the waitlist. The ten numbered
  sections and the keyboard grid are gone.
- **The demo is the centrepiece.** It's a 26-second, seamlessly looping recording of the real
  app in mock mode, with no MOCK chip: an agent needs you → Jump → answer → working → done
  → Wall → Review → 3×2 → Rules. About 1 MB per theme, with a dark and a light version that
  follow the site toggle. Six tabs (Needs you ⌘J · Wall ⌘E · Next up ⌘. · Review ⌘R ·
  Terminals ⌘Q · Rules ⌘,) each give one or two lines of copy and seek the video to that
  moment. The tab strip follows the video while it plays, and arrow keys move between tabs.
  With reduced motion the video stays paused and a Play button is shown.
- **Live app, optional.** `scripts/build-demo.mjs` and `src/lib/demoBridge.ts` (one import in
  `src/main.tsx`) let wide screens swap the video for the real app in an iframe that the tabs
  drive. It starts only when clicked, shows an HTML timing-tower skeleton while it loads,
  and hands keyboard focus back to the tab. I led with the video rather than the live app:
  the app bundle carries Monaco (several MB against ~1 MB of video), its terminal grabs
  focus and keys, it doesn't fit phones, and it can't follow the site's theme toggle.
- **The CTA is honest.** The hero offers Join the waitlist (it scrolls to the single form)
  and Quick start. Build from source sits in the form's note. There is one form, not two.
- **Phones** crop the video to the sidebar and the first tile (4:5) instead of shrinking a
  1600 px window to 358 px.
- **Details moved to `/docs/how-it-works/`.** The timing board, the full flag list
  (including unknown), how detection works, Wall, Next up, Review, pitwall-hold, spaces, the
  agent table (cut to 4 columns), rules, first launch and the roadmap now live there.
  Section 8's facts are fixed there: "aren't enumerated", the `~/projects …` scan, the
  conditional presets, "at most one pane", SSH in Wave 3, and done also notifying.
- `public/shots/` is deleted, so no static screenshots remain. The docs page's visible
  `TODO: repo URL` pill is gone. The MOCK chip is hidden with `?shots=1`
  (`src/components/TopBar.tsx`).

Still weak:

- **The live demo is unbuilt.** Running the root Vite build was blocked by permissions in
  this session, so `public/demo/` doesn't exist and the "Try it live" path, including the
  bridge, is untested end to end. Run `pnpm demo`, then click every tab once.
- **The video is a recording of a 1600 px window.** At 1100 px the terminal text is about
  9 px. A tighter 1280 px capture, or chapters that zoom in, would read better.
- **The video's chapter order** (next up → needs you → wall → review → terminals → rules)
  doesn't match the tab order, so auto-follow jumps the highlight from the third tab to the
  first.
- **The hero line and "Box, box"** are still slogans. Real visitor quotes or a concrete number
  would carry more weight once there are users.
- **No Open Graph image** or social card.
