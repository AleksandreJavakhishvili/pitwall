# Roadmap

Where Pitwall is heading. This is a direction, not a promise: there are no dates,
and plans change as we learn. To ask for something, open an
[issue on GitHub](https://github.com/AleksandreJavakhishvili/pitwall/issues), or add a
thumbs-up to an existing one.

## Now

Being worked on.

- **Signed builds.** Apple and Windows code signing, so macOS and Windows stop warning on first launch and macOS stops asking for folder access again after every update.
- **Windows out of preview.** Windows builds ship since 0.1.1 and pass CI; next is running them day to day on real PCs and fixing what turns up.
- **The floating control, checked everywhere.** Answering from the floating control shipped in 0.6.0, tested on macOS with Claude Code and Codex. Next: check the Accept and Reject keys against the other CLIs (Gemini, Qwen, Cursor, Copilot, OpenCode, Amp, aider) and run the control on Linux and Windows.

## Next

Designed, starting after the current work.

- **Agents that don't depend on the app.** A small background service owns the agents, so they keep running, with their status and Next up queues, while the app is closed or being updated. With a menu-bar item and Start at login.
- **Rules and hooks on agw machines.** The same rules and status hooks for agw sessions as for local agents.
- **Plain SSH machines.** Run and watch agents on any machine you can reach over SSH, not only agw sessions.

## Later

Ideas we want, not scheduled yet.

- **A clearer first launch on macOS.** A welcome step that explains folder access and shows whether it's granted.

## Recently shipped

0.6.0: answer agents without going back to Pitwall. While you're in another app, a
small tile floats over your desktop; open it for a card per asking agent with the
live part of its terminal, Accept and Reject for yes/no prompts, and the keyboard
for anything else.

0.5: Settings → Accounts shows every agent login with its plan's limits and usage
next to your git identities, agents can tell Pitwall which worktrees they work in
(`pitwall agent track`), Review shows only the agent's own work, and Pitwall uses
much less CPU with many worktrees and scrolls smoothly in long lists.

0.4.0: watch another of your machines' agents live, view only, with the sidebar
grouped by machine and Settings → Peers listing them all.

0.3.0: more than one login for Claude Code and Codex, git identities per project,
a sidebar you arrange by dragging, a preview of a dev server beside its terminal,
rules and skills copied into the project, and your other machines' agents in the
sidebar over Tailscale.

0.2.0 rebuilt Pitwall as a native Rust app on GPUI, Zed's GPU-accelerated UI
framework, with no web view, and added the Race Engineer. 0.1 was the first
release, for macOS and Linux, with Windows builds as a preview from 0.1.1.
Everything is in the [changelog](CHANGELOG.md).
