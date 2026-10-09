# Performance pass

> **History.** The measurements below are of the Tauri app (up to v0.1.x):
> a Rust process plus WebKit content and GPU processes. Since v0.2.0 Pitwall
> is the GPUI app (`crates/pitwall-app`): one process, no WebKit. The budgets
> still apply; the numbers need a new pass, and `scripts/bench.py` needs the
> GPUI app's bench hook first (open item, docs/spec/gpui/packaging.md "Tests").

Measure first, then fix against budgets. Add a repeatable benchmark script
(`scripts/bench.sh` or similar) that reports RSS/CPU of the Pitwall process
tree (Rust process + WebKit content/GPU processes, excluding agent processes).

## Budgets
| Metric | Target |
|---|---|
| App, 0 agents, idle (whole Pitwall process tree) | < 150 MB RSS, ~0% CPU |
| Per extra agent (Pitwall's share only) | < 15 MB |
| 20 agents + Wall open | no blank terminals, smooth scroll |
| Cold start to first window | < 1 s |

## Known risks / likely fixes
- WebGL renderer only for visible panes; DOM/canvas renderer for hidden panes
  and Wall tiles (WebKit caps WebGL contexts at ~16 per page).
- Scrollback cap per xterm; dispose view-only Wall instances on exit.
- Hidden terminals: buffer output and write in batches; no render work.
- Each extra window is a separate WebKit process — keep per-window state minimal.
- Lazy-load heavy chunks: Review (and its diff editor), onboarding, rules UI.
- Git polling only for visible or recently active agents; back off when idle
  (pass 2: local agents refresh on file changes instead, see "Git refresh").
- Avoid React re-render storms on `agents-changed` (memoised rows/panes,
  per-agent selectors).
- Ticker does no work when nothing changed.

## Benchmark (`scripts/bench.sh`)
`scripts/bench.sh <binary>` runs `scripts/bench.py` against a **separately
built** app (build with `CARGO_TARGET_DIR=<scratch> pnpm tauri build --no-bundle
--config '{"identifier":"dev.pitwall.bench"}'`, so WebKit storage is separate too).
The instance gets a fresh `PITWALL_HOME` under `/tmp` (state, sockets, holders;
`PITWALL_HOME` overrides `~/.pitwall` in core, client and detect) and
`PITWALL_BENCH=1`, which turns on a small hook (`src-tauri/src/bench.rs`,
`src/lib/bench.ts`: wall on/off, review on/off, visit-all, and a readiness
event for cold start). Agents are a `bench` kind (a shell loop printing ~1.5 KB
of coloured text every 0.5 s) created with `pitwall-cli` in a temp git repo
with an uncommitted change (so Review opens a diff). Only processes it
started are stopped, by exact PID; the user's app, `~/.pitwall` and holders
are never touched.

Measured, after 8 s settle, over a 10 s window: the app process (+ reaped
children such as git), WebKit processes whose *responsible pid* is the app
(WebContent, GPU, Networking); holders separately (agents never).
Memory = `phys_footprint` (Activity Monitor's "Memory"); RSS also shown (it
double-counts shared WebKit framework pages, so it is ~120 MB high at idle).
CPU = user+sys time as % of one core. Each count is a fresh app launch;
`visit-all` shows every agent once (so each gets its xterm), then Auto grid.

## Results (M-series Mac, release builds, 2026-10-06/07)
Footprint MB / RSS MB / CPU %. "Before" = 58ef59e + bench hook; "after" =
this pass. GPU column = WebKit GPU process footprint.

| Scenario | Budget | Before | After | GPU before → after |
|---|---|---|---|---|
| Cold start to first window (median of 4) | < 1 s | 323 ms | 352 ms | |
| 0 agents, idle | < 150 MB, ~0 % | 103 / 227 / 0.6 % | 103 / 228 / 0.2 % | 16 → 15 |
| 1 agent | | 581 / 300 / 7.8 % | 420 / 279 / 4.8 % | 394 → 220 |
| 1 agent + Wall | | 624 / 309 / 7.6 % | 425 / 344 / 22 %* | 399 → 221 |
| 5 agents | | 657 / 401 / 22 % | 430 / 372 / 21 %† | 403 → 220 |
| 5 agents + Wall | | 715 / 424 / 19 % | 469 / 393 / 19 %† | 413 → 224 |
| 5, Review open | | 417 / 660 / 21 % | 381 / 637 / 20 %† | 31 → 20 |
| 5, Review closed again | | 741 / 662 / 16 % | 578 / 649 / 17 %† | 375 → 223 |
| 15 agents | | 880 / 640 / 42 % | 592 / 583 / 30 %† | 413 → 224 |
| 15 agents + Wall | | 917 / 677 / 42 % | 652 / 625 / 23 %† | 415 → 227 |
| 20 agents | | 882 / 657 / 68 % | 613 / 575 / 42 %† | 412 → 226 |
| 20 agents + Wall | no blank tiles | 1107 / 838 / 74 % | 768 / 709 / 26 %† | 413 → 229 |
| Per extra agent (1→20) | < 15 MB | 15.8 / 18.8 | 12.7 / 14.8 | |
| Holder (each, separate) | | 1.3–2.0 | 1.3–1.9 | |

† first full "after" run (`after1`). A second full run on the final tree
(includes the parallel permissions work) gave the same app/WebContent numbers
and CPU of 33 % (20 agents) and 8.7 % (20 + Wall), but the GPU process was
sometimes purged to ~20–30 MB while the bench window was not frontmost, so
its totals (e.g. 20 agents 363 MB) flatter us; the table uses the run where
the window stayed visible. *1-agent + Wall CPU varies 9–22 % run to run.
Rust app process: 32 → 44 MB at 20 agents (≈0.6 MB/agent); app CPU at 20
agents 54 % → 20–25 % (output coalescing + git backoff).

Off-screen runs: bench instances now never activate, send no notifications
and open off-screen (`scripts/bench-tauri.json` + `PITWALL_BENCH=1`). A full
off-screen run on the final tree matched the on-screen app/WebContent/CPU
numbers (20 agents: app 41 MB, 34 % CPU; 20 + Wall: 752 MB / 25 %), but
WebKit treats the window as occluded and usually drops its GPU tiles
(GPU ~20 MB instead of ~220 MB: idle 100, 1 agent 128, 5 agents 199,
20 agents 301 MB). Compare like with like; the table above was on-screen.

Findings while measuring:
- **WebGL** cost ~380 MB in the GPU process as soon as one terminal was
  visible (before: GPU 16 → 394 MB). The DOM renderer still costs ~200 MB of
  GPU-process memory once any terminal paints (hiding `.xterm-screen` alone
  drops GPU to 16 MB; scrollback, cursor blink, sidebar animations don't
  matter). That ~200 MB is WebKit compositing a constantly repainting layer,
  is flat in agent count, and is now the biggest single item.
- **Review**: the ~585 MB from the roadmap was mostly the GPU process (WebGL
  terminals) plus Monaco. Monaco's loaded code (3.2 MB chunk) stays in the
  WebContent heap after closing (~+100 MB vs before opening); editors,
  models and the worker are now disposed on close.

## Fixes in this pass
- Terminals (`src/terminal/registry.ts`): DOM renderer by default; WebGL only
  for visible panes, LRU-capped (`MAX_WEBGL`, 0 = off; addon lazy-loaded),
  disposed when parked or shown in the Wall. Scrollback 10 000 → 5 000
  (Wall viewers 500 → 50). Hidden terminals buffer output and write it in
  batches (1 s / 256 KB) and flush when shown.
- Output to the webview is coalesced in the backend (`term::coalesce`, 12 ms
  window; a lone chunk goes at once, so typing stays instant).
- React: `agents-changed` keeps unchanged `AgentView` objects (and the
  list itself when nothing changed: no render at all); the Actions context
  value is stable; `PaneView`, `WallTile`, `AgentRow` are memoised.
- Lazy chunks: Onboarding and Settings (rules UI) join Review/Monaco; main
  chunk 806 → 712 KB.
- Review close disposes all Monaco editors and models, which also stops the
  editor worker immediately (instead of after 5 idle minutes).
- Git polling backs off (3 s doubling to 30 s, provider `git_poll_ms` as the
  floor) while refreshes find nothing new; resets on a change or a new turn.
  Idle agents were already never polled. The ticker already did no work
  without new output (atomics only; detection only on new output).

## Git refresh (pass 2)
Local agents' git numbers (Changes list, +/−, branch) are refreshed when
their files change instead of by polling. Same data, same pace limit
(at most every 3 s per agent), never later than polling would have.

- **Capability, not platform**: `ProviderCaps.fs_events` (local: yes, agw:
  no) says the machine's `Exec` can `watch`; `LocalExec::watch`
  (`exec/watch.rs`) implements it with the `notify` crate, other execs answer
  `Unsupported` and are polled exactly as before.
- **One watch per checkout** (`engine/gitwatch.rs`): on an agent's first
  refresh, `git rev-parse --show-toplevel --git-dir --git-common-dir` finds
  the checkout and `git ls-files --others --ignored --exclude-standard
  --directory` what git ignores; agents in the same checkout (same machine +
  root) share the watch, which ends with its last agent (stopped agents keep
  none). At most 64 checkouts are watched; beyond, agents are polled.
- **What counts**: anything under the working tree except git internals
  (HEAD, index, packed-refs and refs do count — branch switches, commits;
  a linked worktree's own git folder and the repository's refs are watched
  too) and git-ignored paths. A refresh woken by a change that finds nothing
  new lists git's ignored paths again (≤ once a minute per checkout), so a
  build's new output folder stops counting. Access events are ignored, so
  git's own reads never wake anything.
- **One OS watcher** for all checkouts and one thread that filters events:
  FSEvents (macOS) and ReadDirectoryChangesW (Windows) watch each tree
  recursively; inotify/kqueue get one watch per folder, skipping git-ignored
  folders and `NOISE_DIRS` (node_modules, target, .venv, …), at most 32 768
  folders. Past that, when the OS refuses (`max_user_watches`), or when a
  new folder can't be added later, that checkout falls back to polling
  (retried after 5 min). FSEvents restarts its stream when a path is added
  or removed; every other checkout is then treated as changed once.
- **Pace** (`ticker.rs git_due`): a watched agent is refreshed when its
  watch saw a change (≥ 3 s after the last look, idle or not), when asked
  (task end, Changes/Review, ↻), and by a safety poll while it works or
  prints: 60 s where the watch sees everything git status can (FSEvents,
  Windows), 30 s where folders are skipped by name (inotify) — polling's
  longest back-off, so no case is staler than before. Unwatched agents:
  unchanged (3 s doubling to 30 s while active, never while idle).

Measured (`engine/ticker/bench.rs`, `#[ignore]`: 20 temp repos, real git and
the real ticker on a wall-clock-driven clock, 60 s after a 5 s settle,
release build, M-series Mac). Git processes per minute and CPU s/min spent
by the test process and its children, minus an idle run of the same mode
(= git + watching). "Polling" is today's path (`BENCH_WATCH=0`, the exact
pre-change logic; the original binary gave the same counts).

| 20 local agents | Polling: git/min | CPU s/min | Watching: git/min | CPU s/min |
|---|---|---|---|---|
| Idle, nothing changes | 0 | 0 | 0 | ~0.1 (watcher) |
| Working, no file changes ("thinking") | 180 (239 in the first minute) | 11.0 | 60 | 2.8 |
| Working, an edit every 10 s each | 613 | 21.0 | 353 | 3.3 |
| Working, an edit every 2 s each (stress) | 1076 | 43 | 1124 | 31 |

Each refresh is 3 git processes (diff, untracked, branch). Edits every 2 s
keep both paths at the 3 s cap, so they do the same work; the gain is
everywhere else. Freshness: an edit shows ≤ 3.4 s later (polling: up to 30 s
once backed off). One-off cost per checkout: 2 git processes to start
watching (and one extra refresh per FSEvents restart).

**`gix` instead of spawning git — not done.** Once refreshes only follow
real changes, git processes are the remaining cost only while agents edit
files, and identical output would need gix to match `git diff --raw
--numstat -z -M <base>` exactly (rename pairing and similarity, line counts
from git's xdiff vs imara-diff, binary detection, submodules, untracked with
`--exclude-standard`), plus a second implementation next to the git CLI that
agw machines still need. A large dependency (dozens of crates, build time,
binary size) for a cost that is now small; revisit if busy-agent CPU shows
up in a profile.

## Still over budget / next
- Per-agent cost is now inside 15 MB, idle is inside 150 MB (footprint), but
  any visible terminal adds ~200 MB of WebKit GPU-process memory (DOM
  renderer). Candidates: try `will-change`/layer hints or the canvas renderer
  addon, and measure; investigate WebKit's tile cache for the scrolling
  `.xterm-rows`.
- Monaco's code staying resident after Review: done in pass 2 (below),
  Review's diff is CodeMirror now.
- Wall with many agents is WebContent-bound (~5 MB per view-only xterm);
  view-only instances could be replaced by reusing the interactive ones when
  sizes match more often.
- "No blank terminals / smooth scroll" at 20 + Wall was not verified by eye
  (the bench can't see pixels); no WebGL contexts are used any more, so the
  ~16-context cap can't blank tiles.

## Pass 2 (2026-10-07): parser and Wall tiles

### Tooling
- `scripts/tui-agent.py`: a synthetic busy coding agent (Claude Code / Codex
  style: ~10 frames/s of Ink-style erase + redraw of a live region, truecolor
  and 256-colour SGR, diff lines with backgrounds, wide characters; ~12 KB/s).
  `bench.py --agent tui` uses it instead of the shell loop; `--profile <dir>`
  also runs macOS `sample` on the app per scenario (build with
  `CARGO_PROFILE_RELEASE_STRIP=false` for symbols).
- `cargo run --release -p pitwall-detect --example screen_bench -- <rec>`:
  per-emulator cost on a recording (`tui-agent.py --frames 3000 --cols 100`),
  with allocation counts. `cargo run -p pitwall-core --example screen_frame
  -- <rec> <cols> <rows>` prints the `ScreenFrame` the engine sends, for
  comparing with xterm.js.

### Part A — where backend CPU goes (20 busy TUI agents)
Profile of the release app (`sample`, 5 s, 20 × tui-agent): the app process
is ~17 % of a core; almost all of it is in WebKit IPC / tao event-loop
plumbing for the output channels (`tauri::ipc::channel` → `send_event`), the
holder socket threads and `term-out` coalescing; **vt100 feed + detection
was ~10 samples of ~79 000 (≈ 0.01 %)**. The parser is not the bottleneck,
and detection already runs on a dirty flag (ticker, 400 ms, only after new
output: ≤ 2.5×/s per agent, never per chunk). The rest of the tree's CPU is
WebContent (xterm.js parsing + DOM rendering of the visible panes).

`screen_bench` (3000 frames, 1.1 KB/frame, 30 × 100, M-series):

| | vt100 0.16 | alacritty_terminal 0.26 |
|---|---|---|
| feed | 3.96 µs/frame, 1 alloc (3.2 KB) | 3.63 µs/frame, 0 allocs |
| feed + text + rules every 4th frame | 12.2 µs/frame, 71 allocs (22.6 KB) | 8.3 µs/frame, 10.5 allocs (4.4 KB) |
| 20 agents × 10 frames/s | 0.24 % of a core | 0.17 % of a core |

Switched to alacritty_terminal anyway (faster, allocation-free feed, and it
gives the styled screen the Wall needs); `Screen` stays the abstraction
(detection-interface.md). End-to-end app CPU with 20 tui agents did not
change measurably (17.5 → 17.6 %): as predicted, the parser never mattered.

### Part B — Wall tiles from the backend's screen copy
Wall tiles no longer run xterm.js: the engine sends `ScreenFrame`s (styled
runs; full screen, then changed rows; ≤ 10/s; only for tiles on screen) and
`src/terminal/screenView.ts` draws them as xterm's DOM renderer would
(wall.md). Fidelity, checked in headless Chrome with the mock UI: the
Wall before/after (dark + light, DPR 2) differs only in the mock's own
time-dependent sidebar row and xterm's blank-cursor quirk (fixed); a
style torture screen (16/256/truecolor, bold-bright, dim, inverse, all
underline styles, strike, hidden, italic, wide CJK/emoji, tabs, cursor) and
a tui-agent screen render **pixel-identical** to xterm at DPR 1 and 2,
font 11 and 13, scale 0.5 / 0.7, in both themes — both from xterm's own
buffer and from the Rust frames (cell-identical to xterm's buffer).

Measured (release, off-screen bench window; footprint MB / CPU % of a core;
3 runs each for 20 tui agents + Wall):

| Scenario | Before | After |
|---|---|---|
| 20 tui agents + Wall: WebContent | 344 / 343 / 306 MB | 290 / 299 / 277 MB |
| 20 tui agents + Wall: WebKit CPU | 52 / 56 / 28 % | 36 / 36 / 35 % |
| 20 tui agents + Wall: tree CPU | 63 / 68 / 40 % | 48 / 49 / 46 % |
| 20 tui agents + Wall: GPU process | 225 / 228 / 221 MB | 230 / 226 / 231 MB |
| 20 bench agents + Wall: WebContent | 328 MB | 300 MB |
| 20 tui agents (no Wall), app process | 57 MB, 17.5 % | 60 MB, 17.6 % |

So: the Wall costs ~40–55 MB less WebContent memory and ~15–20 points less
WebKit CPU with 20 busy agents (no xterm parse/render per tile; changed rows
only, ≤ 10/s). **The ~220 MB in the GPU process did not move**: it is not
xterm-specific. Any visible, frequently repainting text layer makes WebKit
keep its tiles (pass 1: hiding `.xterm-screen` drops it to 16 MB), and the
snapshot tiles repaint too. Next candidates: repaint Wall tiles less often
(e.g. 2–4/s when not hovered), test whether `contain: paint` / one canvas
for all tiles shrinks the tile cache, and check whether WebKit's GPU memory
is per window area rather than per layer.

## Pass 2: Review memory (2026-10-07)
Monaco (`monaco-editor` + `@monaco-editor/react`) is replaced by CodeMirror 6 +
`@codemirror/merge` in Review's diff (`src/components/review/diffView.ts`,
`editorSetup.ts`, `findPanel.ts`, `lineDiff.ts`). It still loads only when a
file is opened in Review (`ReviewDiff` is a lazy chunk; each language is its
own chunk, loaded for the open file). The UI is unchanged: side by side and
inline, Monaco's colours (vs / vs-dark + the Pitwall tokens), margin (glyph,
number(s), +/−), hatched filler, "N hidden lines" bars, word highlights,
bracket-pair colours, indent guides, overlay scrollbars + cursor lane, the
diff overview ruler, Find widget, context menu (Add review comment · Copy),
the read-only message, comment glyphs/hover/composer and `reveal`. Line
alignment follows Monaco's (`lineDiff.ts`: lines first, inserted blocks slid
to Monaco's boundary, then characters). Checked by screenshotting the browser
mock before/after in both themes and layouts (comment, hover, composer,
reveal, find, context menu, read-only message, new/deleted/markdown files):
0.3–2 % of diff-area pixels differ, mostly the "hidden lines" label (Monaco
drew it clipped) and character-level highlights where the two diff
algorithms split a rewritten line differently.

Bundle (`vite build`): the diff chunk 3,234 KB (827 KB gzip) + editor worker
304 KB + 146 KB CSS + 153 KB codicon font → 412 KB (132 KB gzip) plus the
open file's language (e.g. TypeScript ~100 KB); whole `dist` 6.3 → 3.8 MB.
Main chunk unchanged (727 KB).

Memory, `scripts/bench.sh … --counts 5`, two alternating runs per build,
off-screen (so GPU is not comparable, see above); WebContent MB:

| 5 agents | Before (Monaco) | After (CodeMirror) |
|---|---|---|
| before opening Review | 178 / 188 | 204 / 111 |
| Review open | 285 / 293 | 226 / 171 |
| Review closed again | 280 / 263 | 205 / 134 |
| kept after closing | +102 / +75 | +1 / +23 |

Browser mock, JS heap after GC (median of 3; open = four files, both
layouts): idle 3.7 MB; Review open 16.5 → 8.5 MB; closed 15.9 → 7.9 MB.
Cold start unchanged (318–327 ms). Monaco's dispose-on-close hook is gone:
closing Review destroys the editors with the component, and there is no
worker.

## Glass look and motion (2026-10-08)
Settings → Appearance → Look: Glass (layout.md "Appearance"). macOS draws
the frosting itself (`NSVisualEffectView` behind a transparent webview), so
the webview only tints; `backdrop-filter` is used only on the palette,
dialogs, menus, toasts and the overlay sidebar / drawer, never on tiles or
rows; terminals and Wall tiles stay opaque. Motion (`motion.css`): transform
/ opacity only, the working dot is the only loop, "needs you" no longer
breathes forever (a halo twice), nothing animates on the Wall or while the
page is hidden.

Bench additions: `--ui '<json>'` seeds the instance's UI state (look,
reduceMotion, …); `--visible` (`PITWALL_BENCH_VISIBLE=1`, with a config
that puts the window on screen) keeps the window on screen and above other
windows so WebKit keeps its GPU tiles and the window material is drawn;
WindowServer's CPU is reported too (shared with every app on screen: only
differences between runs mean anything).

Measured on-screen (`--visible --quick --counts 5,20`, bench agents, M-series
Mac, release builds). Footprint MB / CPU % of a core; "base" = 0ce352b
(before), two runs where there are two values:

| Scenario | base Flat | Flat | Glass | Glass, Reduce motion |
|---|---|---|---|---|
| 0 agents: tree | 100 / 100 | 102 / 100 | 102 | 110 |
| 0 agents: GPU · WebContent | 15 · 47 | 16 · 48 | 16 · 48 | 18 · 54 |
| 0 agents: WindowServer CPU | 11 / 11 | 12 / 10 | 19 | 17 |
| 5 agents: tree | 421 / 406 | 425 / 396 | 477 | 505 |
| 5 agents: GPU · WebContent | 221 · 149 / 216 · 139 | 223 · 151 / 221 · 124 | 259 · 170 | 264 · 194 |
| 5 agents: tree CPU (WebKit) | 17.3 (15.0) / 11.2 (9.9) | 13.0 (11.3) / 11.7 (10.3) | 13.6 (12.1) | 11.8 (10.6) |
| 5 agents: WindowServer CPU | 33 / 30 | 33 / 32 | 44 | 20 |
| 20 agents: tree | 458 / 447 | 480 / 454 | 528 | 557 |
| 20 agents: GPU · WebContent | 221 · 174 / 219 · 167 | 225 · 194 / 224 · 169 | 267 · 200 | 262 · 235 |
| 20 agents: tree CPU (WebKit) | 28.5 (22.2) / 15.4 (10.2) | 28.1 (21.7) / 20.7 (14.8) | 31.6 (25.1) | 21.5 (16.3) |
| 20 agents: WindowServer CPU | 46 / 21 | 46 / 33 | 45 | 44 |

- **Flat is unchanged** by this work (the new motion included): within the
  run-to-run spread of the base build.
- **Glass costs ~40–45 MB in the WebKit GPU process** (≈ 220 → 260 MB) once
  terminals paint: the window-sized layers are now composited with alpha
  over the window material. WebContent +10–30 MB (noisy). App process
  unchanged. Idle (no terminal painting) costs nothing measurable.
- CPU: Glass is within noise of Flat for the app tree (+0.6 / +3.5 points);
  WindowServer +11 points at 5 agents in this run, equal at 20 — it blurs
  the desktop behind the window, which macOS does for every vibrant window.
- Animations: with Reduce motion the tree CPU at 20 agents was ~10 points
  lower (31.6 → 21.5 %): the working-dot ring (one per working agent in the
  sidebar) is the only loop left and is what costs; at 5 agents ~2 points.
  A Flat + Reduce motion run had WebKit drop its GPU tiles (GPU ~26 MB, as
  seen before in this doc), so it is not comparable and not shown.
- Glass lite (Linux / Windows 10; `PITWALL_GLASS=lite` on a Mac) paints an
  opaque gradient once and uses no blur at all; it was not benchmarked
  separately (it is Flat plus a static background).
- Not measured here: Windows (Mica is drawn by DWM like vibrancy by
  WindowServer) and Linux.

## GPUI port: re-baseline (2026-10-09)

The bench hook is ported (`crates/pitwall-app/src/bench.rs`): the same
`PITWALL_BENCH=1` / `PITWALL_BENCH_VISIBLE=1`, `bench-cmd` commands and
`bench-ready` file, so `scripts/bench.sh target/release/pitwall-app` runs
against the native app (the WebKit group stays empty: one process draws).
Use `--visible`: GPUI draws on the display link, which macOS stops for an
off-screen or covered window, so off-screen numbers are an idle app.
`PITWALL_FRAME_STATS=1` logs frame timing every 2 s.

First numbers (release, M-series Mac, 20 made-up working agents, each
printing a line every 0.5 s, window on screen, app process only):

| | Motion on | Reduce motion |
|---|---|---|
| CPU % of a core | 6.9 · 6.1 | 3.0 · 3.4 |
| Frames | ~32/s (working-dot rings at 30 fps) | on output only |

- The working dots' rings come from one 30 fps clock drawn by a small layer
  over the main screen; the screen is a cached view, so a ring frame replays
  its last paint (render → paint p50 0.6 ms). The ring costs ~3–4 points at
  20 agents, against ~10 for the React app's (above).
- Covered, minimised or on another Space, the app draws nothing and its
  polls pause: 0.4–0.5 % with the same 20 agents.
- To do: a full `--counts 1,5,15,20` run on a quiet machine to fill the
  budgets table for GPUI (memory, cold start, Wall, Review).

## GPUI port: memory pass (2026-10-09)

Standard load: 15 made-up TUI agents (`scripts/tui-agent.py`: claude,
codex and full-screen styles, 600 KB of history each), one space with 2×2
panes and the rest folded into chips (bench command `visit-all-4`),
release build, 60 s after a 30 s settle. Footprint of the app process
(`footprint`, Activity Monitor's "Memory"); per component from
`MallocStackLogging=lite` + `malloc_history -allBySize` on a profiling
build.

| Component | Before | After |
|---|---|---|
| Pane terminals: grids + scrollback (alacritty rows) | 228 MB | 45 MB |
| Frozen scrollback (text + style runs) | – | 11 MB |
| Engine's headless parsers (grids) | ~9 MB | 0 (shared) |
| Parsers (vte, 2 MiB sync buffer each, mostly untouched) | 30 | 15 |
| Engine output rings (1 MiB each) | 20 MB | 16 MB |
| GPUI scene, layout, text shaping | 23 MB | 23 MB |
| Metal driver memory while drawing ("unmapped graphics") | 184 MB | 184 MB |
| Drawables (3 IOSurfaces at the window's size) | 59 MB | 59 MB |
| **Total footprint (off-screen / on screen)** | **589 / 588 MB** | **364 / 364 MB** |

- Scrollback (5 000 lines, as xterm.js kept) costs 24 bytes a cell in
  alacritty, twice xterm.js's 12. A terminal no view has locked for 15 s
  (folded into a chip, another space) moves its scrollback out of the
  grid into text plus style runs, in batches of 1 000 rows, and gets it
  back on the next view lock (~3 ms for 5 000 rows), before any read;
  the main screen's scrollback also freezes while a program runs on the
  alternate screen (`pitwall-term-view/src/frozen.rs`).
- One parser per agent: the pane's terminal is the engine's screen
  (`TermHost::share_screen`, `ScreenSource`); the status rules and the
  Wall's frames read its grid. Before, each agent was parsed twice.
- The ~180 MB of Metal driver memory is the platform's: a 30-line Swift
  app that only clears a `CAMetalLayer` holds the same while it draws,
  at any window size and frame rate, and gives it back ~2 s after the
  last frame. The drawables scale with the window.
- Holders (`pitwall-hold`, separate processes) are ~2.5 MB each.
