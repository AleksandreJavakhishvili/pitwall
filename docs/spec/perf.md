# Performance pass

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
- Lazy-load heavy chunks: Monaco/Review, onboarding, rules UI.
- Git polling only for visible or recently active agents; back off when idle.
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
with an uncommitted change (so Review opens Monaco). Only processes it
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

## Still over budget / next
- Per-agent cost is now inside 15 MB, idle is inside 150 MB (footprint), but
  any visible terminal adds ~200 MB of WebKit GPU-process memory (DOM
  renderer). Candidates: try `will-change`/layer hints or the canvas renderer
  addon, and measure; investigate WebKit's tile cache for the scrolling
  `.xterm-rows`.
- Monaco code stays resident after Review; to free it, host Review in an
  iframe that is removed on close, or ask before loading.
- Wall with many agents is WebContent-bound (~5 MB per view-only xterm);
  view-only instances could be replaced by reusing the interactive ones when
  sizes match more often.
- "No blank terminals / smooth scroll" at 20 + Wall was not verified by eye
  (the bench can't see pixels); no WebGL contexts are used any more, so the
  ~16-context cap can't blank tiles.
