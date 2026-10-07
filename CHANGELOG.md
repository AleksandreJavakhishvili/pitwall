# Changelog

All notable changes to Pitwall. Generated with [git-cliff](https://git-cliff.org) from [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/); see [CONTRIBUTING.md](CONTRIBUTING.md#commit-messages).

## Unreleased

### Highlights

Pitwall's first public release. These highlights cover the work from before the
commit convention, which the entries generated from commits don't include.

#### Hosting and status

- Terminal agents in tiled spaces with status flags: needs you, done, working, idle, exited, stopped.
- Status from hooks, then screen rules, then activity. Screen rules for Claude Code, Codex, Gemini CLI, opencode, Cursor Agent, GitHub Copilot CLI, Qwen Code, Amp and Aider.
- Needs-you bar with <kbd>⌘J</kbd>, Dock badge, native notifications.
- Per-agent Next up queue with auto-send.
- Terminals live in `pitwall-hold` processes and survive <kbd>⌘Q</kbd>.

#### Screens

- Wall (<kbd>⌘E</kbd>): every terminal, view-only, grouped by project.
- Review (<kbd>⌘R</kbd>): per-task diffs, line comments sent as one prompt, commit & merge.
- First-launch read-only scan that resumes recent conversations.
- Density setting (Comfortable / Compact / Dense), presets up to 4×4 and Auto grid, spaces in separate windows.

#### Setup

- Rules via `rulesync`: library, rule sets, per-project and per-agent assignment.
- Worktrees through the agent's own `--worktree` flag; Pitwall creates no worktrees or branches of its own.

#### Terminals and machines

- <kbd>⌘T</kbd> opens a terminal in the focused agent's folder; an agent started in it is recognised, and Restart resumes it.
- Agents running in other terminal apps are listed under Elsewhere, with "Bring in".
- agw sessions on VMs: add or create them, live terminal, status, Next up, diffs and Review.
- `pitwall` CLI to list and add agents and sessions; changes to another machine wait for approval in the app.
