# Pitwall — developer docs

Pitwall is a macOS, Linux and Windows desktop app (Tauri 2 + React/TS) that hosts terminal coding
agents (Claude Code, Codex, any CLI) and shows who needs you, a per-agent
"Next up" queue, and git diffs. **It is a tool, not an agent**: it never
rewrites prompts, never decides what agents do, and delivers user text verbatim.

## Spec index

Design notes and specs live in small files under `docs/spec/`. They describe
how Pitwall behaves and why; the code is organised as:

| Area | Where |
|---|---|
| Core, providers, daemon, CLI | `crates/` (`pitwall-core`, `pitwall-providers`, `pitwall-daemon`, `pitwall-cli`, …) |
| Terminal holder | `crates/pitwall-hold/` |
| Screen detection | `crates/pitwall-detect/` (`src/screen.rs`, `src/detect.rs`, `detect/*.toml`, fixtures) |
| Desktop app | `src-tauri/` |
| UI | `src/`, `index.html`, `public/` |

| File | Contents | Area |
|---|---|---|
| [spec/status.md](spec/status.md) | Status values and sources (hooks > screen > activity) | all |
| [spec/api.md](spec/api.md) | Tauri commands, events, TS/Rust shapes | Backend, UI |
| [spec/backend.md](spec/backend.md) | Backend behaviour decisions | Backend |
| [spec/detection-interface.md](spec/detection-interface.md) | `Screen` + `detect()` Rust interface | Backend, Detection |
| [spec/layout.md](spec/layout.md) | Projects, spaces, tiling, windows, breakpoints (+ backend additions) | UI, Backend |
| [spec/wall.md](spec/wall.md) | Wall overview mode | UI, Backend |
| [spec/onboarding.md](spec/onboarding.md) | First-launch auto-detect | Backend, UI |
| [spec/review.md](spec/review.md) | Review screen, per-task diffs, merge | Backend, UI |
| [spec/rules.md](spec/rules.md) | Rules via rulesync | Backend, UI |
| [spec/agents-more.md](spec/agents-more.md) | More agent definitions | Backend, Detection |
| [spec/perf.md](spec/perf.md) | Performance pass and budgets | Backend, UI |
| [spec/engineer.md](spec/engineer.md) | Pitwall CLI, approvals, Race Engineer | Backend, UI |
| [spec/worktrees.md](spec/worktrees.md) | Worktrees via the agent's own flag | Backend, UI |
| [spec/architecture.md](spec/architecture.md) | Wave 2 design: crates, Provider/TermIo/Exec traits, capabilities, daemon protocol, migration steps | all |
| [spec/worktrees-view.md](spec/worktrees-view.md) | All git worktrees per project/agent in source control | Backend, UI |
| [spec/explorer.md](spec/explorer.md) | Read-only code explorer: file tree, viewer, quick open, search, open in editor | Backend, UI |
| [spec/roadmap.md](spec/roadmap.md) | Waves and what comes later | – |
