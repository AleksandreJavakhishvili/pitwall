# Roadmap

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
- Perf note kept for wave 3: WebKit content process ~585 MB after opening Review.

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
  PowerShell launch, tray + taskbar badge); test in a Parallels Windows VM and a
  CI windows build. Linux comes nearly free with the same work.
- Performance pass with hard budgets → [perf.md](perf.md) (after the
  restructure, so we optimise the final shape).
- Plain SSH machines provider; drag a space tab out to create a window.

## Website — DONE
- `website/` deploys to GitHub Pages (`.github/workflows/pages.yml`); the live demo
  is built with it; video re-recorded at 1120×700 (legible text), tabs follow the
  video in order and a picked tab loops its chapter; Terminals tab is ⌘T;
  waitlist replaced by Download / Star on GitHub / Build from source.
  Open: Open Graph image.
