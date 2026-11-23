# Methods an extension may call

<!-- Generated from crates/pitwall-daemon/src/methods.rs (METHODS): run
`PITWALL_WRITE_DOCS=1 cargo test -p pitwall-daemon methods_doc`. -->

An adapter speaking [protocol 2](protocol-2.md) calls these with a `call`
line; a program it started can call them on Pitwall's socket
(`pitwall` or `pitwall-client`) and is the extension there too. Each needs
a [permission](permissions.md) the user granted. Params and results are
those of the command-line API (`pitwall --json`).

| Method | Needs | What it gives |
|---|---|---|
| `agent.list` | `agents.read` | Every agent Pitwall shows, with its state, folder, machine and branch. |
| `agent.status` | `agents.read` | What one agent is doing and why (`agentId`: its id). |
| `project.list` | `projects.read` | The user's projects: name and folder. |
| `worktree.list` | `worktrees.read` | Each repository with its worktrees: folder, branch, and the agents working there. |
| `accounts.list` | `profiles.read` | The profiles, only the extension's own connections (`ext:<id>`); git identities are left out. |
| `project.accounts` | `profiles.read` | Which of the extension's own connections a project uses. |
| `events.subscribe` | `agents.read` | Events as they happen (`topics`: `agents`, `attention`, `blocked`), each as an `event` line. Calling it again replaces the topics. |

## Not yet

These act, and are refused with `unsupported` in this Pitwall. A later one
will offer them with the permission named.

| Method | Will need |
|---|---|
| `agent.create` | `agents.start` |
| `queue.add` | `agents.queue` |
| `queue.send` | `agents.queue` |
| `viewer.open` | `show` |
| `web.open` | `show` |

## Never

Everything else is the user's own, refused with `unsupported` whatever is
granted: `agent.refresh`, `machine.list`, `machine.form`, `session.list`, `session.add`, `approval.list`, `approval.answer`, `settings.list`, `settings.get`, `settings.set`, `settings.reset`, `agent.stop`, `agent.restart`, `agent.remove`, `agent.rename`, `agent.track`, `agent.wait`, `queue.list`, `queue.remove`, `project.add`, `project.remove`, `profile.save`, `profile.set`, `profile.delete`, `project.pick`, `project.profile`, `rules.list`, `rules.sets`, `rules.apply`, `rules.default`, `rules.show`, `rules.library.new`, `rules.project.status`, `rules.project.updates`, `rules.project.assign`, `rules.project.unassign`, `rules.project.update`, `rules.project.import`, `rules.project.generate`, `rules.project.move`, `rules.reapply`, `rules.set.save`, `rules.set.delete`, `rules.import`, `review.changes`, `space.list`, `space.create`, `space.rename`, `space.move`, `agent.move`, `mcp.tools`, `mcp.call`.

## Errors

- `denied`: the permission isn't granted (or was revoked: it applies to the
  next call). The message names it and where the user grants it.
- `unsupported`: not an extension's method, or not in this Pitwall yet.
- `busy`: past 20 calls a second, socket and adapter together. Wait and
  call again.
- `unknown_method`: no such method.
