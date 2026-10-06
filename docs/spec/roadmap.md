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

## Wave 2 — Architecture: providers + daemon (in design)
Goal: clean, low-coupling core so new places-to-run (agw, SSH, cloud) and new
agents plug in without touching the rest. Design first, user reviews, then build.
1. Architecture design → [architecture.md](architecture.md) (written by an
   architect agent, reviewed by the user before implementation).
2. Core extraction + background daemon (`pitwalld`) + Local provider: agents
   survive ⌘Q; the Tauri app becomes a thin client; menu-bar item.
3. agw provider: agw sessions as real agents (terminal, status, Next up), then
   create on VM, diffs/Review over ssh, rules via agw artifact bundles.
4. Pitwall CLI + API with approval prompts for risky actions → [engineer.md](engineer.md)
5. Race Engineer preset (optional assistant) → [engineer.md](engineer.md)

## Wave 3
- Windows port (ConPTY via portable-pty, named pipes, pitwall-hook binary,
  PowerShell launch, tray + taskbar badge); test in a Parallels Windows VM and a
  CI windows build. Linux comes nearly free with the same work.
- Performance pass with hard budgets → [perf.md](perf.md) (after the
  restructure, so we optimise the final shape).
- Plain SSH machines provider; drag a space tab out to create a window.
