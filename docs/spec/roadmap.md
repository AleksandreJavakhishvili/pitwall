# Roadmap

> The public summary is [ROADMAP.md](../../ROADMAP.md) (also on the website); update it when items here move.

## Wave 1 — DONE
- First-launch auto-detect → [onboarding.md](onboarding.md)
- Review screen → [review.md](review.md)
- Rules via rulesync → [rules.md](rules.md)
- More agents → [agents-more.md](agents-more.md)

## Wave 1.5 — DONE
- Worktrees via the agent's own flag, nothing tied to Pitwall → [worktrees.md](worktrees.md)
- Error boundaries, ⌘, + app menu Settings, native folder picker, new agents
  add their project, api.md refreshed, clippy clean.
- In progress: denser tiles (density setting, per-tile font, more presets) → layout.md revision.

## Queued UI
- Terminals anywhere (⌘T, "Open terminal here") + agents started by hand are recognised (inside Pitwall terminals and "Elsewhere") → [terminals.md](terminals.md)
- Better git diff UI: VS Code-style file icons, status letters, folder tree → [diff-ui.md](diff-ui.md)

## Wave 2 — Architecture: providers + daemon (in design)
Goal: clean, low-coupling core so new places-to-run (agw, SSH, cloud) and new
agents plug in without touching the rest. Design first, review, then build.
1. Architecture design → [architecture.md](architecture.md) (designed and
   reviewed before implementation).
2. Core extraction + background daemon (`pitwalld`) + Local provider: agents
   survive ⌘Q; the Tauri app becomes a thin client; menu-bar item.
3. agw provider: agw sessions as real agents (terminal, status, Next up), then
   create on VM, diffs/Review over ssh, rules via agw artifact bundles.
4. Pitwall CLI + API with approval prompts for risky actions → [engineer.md](engineer.md)
5. Race Engineer preset (optional assistant) → [engineer.md](engineer.md)

## Wave 3
- macOS permissions for other users: welcome-screen step explaining Full Disk
  Access with an "Open Settings" button and a granted/not-granted check;
  Info.plist usage strings (NSDesktopFolderUsageDescription,
  NSDocumentsFolderUsageDescription, NSDownloadsFolderUsageDescription) so
  the prompt says why; only poll git for visible or recently active agents.
- Release hygiene: stable install location (/Applications) + Developer ID code
  signing + notarization, so macOS privacy grants (Desktop/Documents access)
  persist across builds instead of re-prompting for every new unsigned copy.
- Windows port (ConPTY via portable-pty, named pipes, pitwall-hook binary,
  PowerShell launch, tray + taskbar badge) — implemented behind the platform
  layers, CI windows job + NSIS/MSI release; still to do: verify on a real
  Windows machine (Parallels VM), port the POSIX-shell test fixtures
  (provider contract, agw fakes) to run on Windows, code signing.
- Linux port — DONE: `/proc` process facts, XDG folders, Ctrl+Shift shortcuts,
  no native menu, AppImage + .deb releases, CI on Ubuntu (see
  architecture.md §9 decision 7 and api.md `host_info`).
- DONE: performance pass 1 with hard budgets → [perf.md](perf.md): idle, cold
  start and per-agent memory within budget; 20 agents 882 → 613 MB, CPU 68 → 42 %.
- DONE: performance pass 2 → [perf.md](perf.md) "Pass 2": headless screens on
  alacritty_terminal behind `Screen` (the parser was ~0.01 % of app CPU, not
  the bottleneck); Wall tiles drawn from the backend's screen copy instead of
  xterm.js (pixel-identical; Wall WebContent −40–55 MB, WebKit CPU −15–20
  points with 20 busy agents).
- Performance pass 3 (still over budget / unverified):
  - ~220 MB in WebKit's GPU process as soon as any terminal (or Wall tile) is
    visible and repainting (independent of agent count, and not xterm-specific:
    snapshot tiles cost the same): audit compositing layers, repaint rate,
    continuous animations (pulse dots, blocked glow), `contain: paint`.
  - Monaco's ~100 MB of code stays resident after Review closes: host Review in
    a separate lightweight webview/window that is destroyed on close.
  - CPU with 20 busy agents: the app's share is WebKit IPC for the output
    channels (not parsing or detection); WebContent's is xterm.js in visible
    panes. Batch harder for panes that aren't focused; React updates.
  - Visual check that 20 agents + Wall shows no blank tiles and scrolls smoothly.
  - Keep cold start < 1 s with the login-PATH probe (now non-blocking).
  - Per-window cost (each window is its own WebKit process) and agw polling cost.
  - DONE: git refresh on file changes instead of polling for local agents
    (`ProviderCaps.fs_events`, `notify` watch per checkout, 30–60 s safety
    poll; agw still polls) → [perf.md](perf.md) "Git refresh". `gix` instead
    of spawning git: measured, not worth it (see there).
- Plain SSH machines provider; drag a space tab out to create a window.

## Website — DONE
- `website/` deploys to GitHub Pages (`.github/workflows/pages.yml`); the live demo
  is built with it; video re-recorded at 1120×700 (legible text), tabs follow the
  video in order and a picked tab loops its chapter; Terminals tab is ⌘T;
  waitlist replaced by Download / Star on GitHub / Build from source.
  Open: Open Graph image.
