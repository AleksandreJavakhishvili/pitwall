# Roadmap

Where Pitwall is heading. This is a direction, not a promise: there are no dates,
and plans change as we learn. To ask for something, open an
[issue on GitHub](https://github.com/AleksandreJavakhishvili/pitwall/issues), or add a
thumbs-up to an existing one.

## Now

Being worked on.

- **Signed builds.** Apple and Windows code signing, so macOS and Windows stop warning on first launch and macOS stops asking for folder access again after every update.
- **Windows out of preview.** Windows builds ship since 0.1.1 and pass CI; next is running them day to day on real PCs and fixing what turns up.
- **Lighter with terminals on screen.** WebKit keeps about 220 MB for drawing as soon as anything on screen repaints; try repainting the Wall less often to bring that down.
- **Calmer with many busy agents.** Lower CPU when twenty agents are all printing at once.

## Next

Designed, starting after the current work.

- **Agents that don't depend on the app.** A small background service owns the agents, so they keep running, with their status and Next up queues, while the app is closed or being updated. With a menu-bar item and Start at login.
- **Rules and hooks on agw machines.** The same rules and status hooks for agw sessions as for local agents.
- **Race Engineer.** An optional assistant: an ordinary agent, on your own subscription, that knows Pitwall's command line and can set things up or rearrange agents when you ask. Anything risky still waits for your approval in the app, and Pitwall works fully without it.
- **Plain SSH machines.** Run and watch agents on any machine you can reach over SSH, not only agw sessions.

## Later

Ideas we want, not scheduled yet.

- **Drag a space out into its own window.** Pull a space tab off the tab bar to open it in a new window.
- **A clearer first launch on macOS.** A welcome step that explains folder access and shows whether it's granted.
- **Link previews.** A proper image when someone shares a link to Pitwall.

## Recently shipped

The first release, 0.1.0, for macOS, Linux and Windows: a read-only code explorer
(file tree, quick open, search), a Changes panel that matches `git status`, Review
scoped to the current project, git refreshes when files change, and a lighter
Review and Wall. Everything is in the [changelog](CHANGELOG.md).
