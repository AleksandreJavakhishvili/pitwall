# Roadmap

Where Pitwall is heading. This is a direction, not a promise: there are no dates,
and plans change as we learn. To ask for something, open an
[issue on GitHub](https://github.com/AleksandreJavakhishvili/pitwall/issues), or add a
thumbs-up to an existing one.

## Now

Being worked on.

- **Signed builds.** Apple and Windows code signing, so macOS and Windows stop warning on first launch and macOS stops asking for folder access again after every update.
- **Windows out of preview.** Windows builds ship since 0.1.1 and pass CI; next is running them day to day on real PCs and fixing what turns up.

## Next

Designed, starting after the current work.

- **Agents that don't depend on the app.** A small background service owns the agents, so they keep running, with their status and Next up queues, while the app is closed or being updated. With a menu-bar item and Start at login.
- **Rules and hooks on agw machines.** The same rules and status hooks for agw sessions as for local agents.
- **Plain SSH machines.** Run and watch agents on any machine you can reach over SSH, not only agw sessions.

## Later

Ideas we want, not scheduled yet.

- **Drag a space out into its own window.** Pull a space tab off the tab bar to open it in a new window.
- **A clearer first launch on macOS.** A welcome step that explains folder access and shows whether it's granted.

## Recently shipped

0.2.0 rebuilt Pitwall as a native Rust app on GPUI, Zed's GPU-accelerated UI
framework, with no web view: the same design, shortcuts and agents with a
fraction of the CPU and less memory, native glass (Liquid Glass on macOS 26,
Mica on Windows 11), and the Race Engineer, an optional agent that knows
Pitwall's command line and agw and sets things up when you ask.

The first release, 0.1.0, for macOS and Linux: a read-only code explorer
(file tree, quick open, search), a Changes panel that matches `git status`, Review
scoped to the current project, git refreshes when files change, and a lighter
Review and Wall. 0.1.1 added Windows builds as a preview, and link previews for the
website. Everything is in the [changelog](CHANGELOG.md).
