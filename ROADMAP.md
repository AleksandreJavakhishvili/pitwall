# Roadmap

Where Pitwall is heading. This is a direction, not a promise: there are no dates,
and plans change as we learn. To ask for something, open an
[issue on GitHub](https://github.com/AleksandreJavakhishvili/pitwall/issues), or add a
thumbs-up to an existing one.

## Now

Being worked on.

- **The first release, 0.1.0.** Downloads for macOS, Linux and Windows, with signed builds so macOS stops asking for folder access again after every update.
- **Windows, tested on a real machine.** The Windows build exists; next is running it day to day on real hardware and fixing what turns up.
- **Lighter with terminals on screen.** Use less memory as soon as a terminal is visible, however many agents you run.
- **Review in its own window.** Open Review in a lightweight window that gives its memory back when you close it.
- **Calmer with many busy agents.** Lower CPU when twenty agents are all printing at once, with no blank tiles and smooth scrolling on the Wall.

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

Linux builds (AppImage and .deb), a first performance pass (less memory and CPU with
many agents), terminals you can open anywhere, and a source control view that
refreshes when you open it. Everything is in the [changelog](CHANGELOG.md).
