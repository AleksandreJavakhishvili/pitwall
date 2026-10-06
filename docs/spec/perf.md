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
