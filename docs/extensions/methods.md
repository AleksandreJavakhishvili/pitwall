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
| `agent.create` | `agents.start` | Asks the user to start an agent (`kind`, `project`: one of Pitwall's projects; maybe `name` and a first `prompt`, put in its Next up). Asked every time, showing all of it; no answer in time is a no. One request at a time, three in ten minutes: see [actions](actions.md). |
| `agent.focus` | `agents.focus` | Moves the focus to an agent's tile (`agentId`) while Pitwall is in front; otherwise does nothing and says so (`focused: false`, `note`). |
| `agent.status` | `agents.read` | What one agent is doing and why (`agentId`: its id). |
| `queue.add` | `agents.queue` | Puts `text` in an agent's Next up (`agentId`), marked with the extension's name. It waits for the user's Send unless the user lets the extension send without asking. At most 4000 characters, one item per agent every 10 seconds, three of its items waiting per agent: see [actions](actions.md). |
| `project.list` | `projects.read` | The user's projects: name and folder. |
| `worktree.list` | `worktrees.read` | Each repository with its worktrees: folder, branch, and the agents working there. |
| `accounts.list` | `profiles.read` | The profiles, only the extension's own connections (`ext:<id>`); git identities are left out. |
| `project.accounts` | `profiles.read` | Which of the extension's own connections a project uses. |
| `viewer.open` | `show` | Shows files (`files`, `line`) or a folder in an agent's file viewer (`agentId` required), next to its terminal; never a window of its own, never brings Pitwall forward. Off while the user turned off "Agents can show you files". |
| `web.open` | `show` | Shows a web page (`url`, http or https) in an agent's page tile (`agentId` required), next to its terminal; never brings Pitwall forward. Off while the user turned off agents' web pages. |
| `events.subscribe` | `agents.read` | Events as they happen (`topics`: `agents`, `attention`, `blocked`), each as an `event` line. Calling it again replaces the topics. |

## Never

Everything else is the user's own, refused with `unsupported` whatever is
granted: `agent.refresh`, `machine.list`, `machine.form`, `session.list`, `session.add`, `approval.list`, `approval.answer`, `settings.list`, `settings.get`, `settings.set`, `settings.reset`, `agent.stop`, `agent.restart`, `agent.remove`, `agent.rename`, `agent.track`, `agent.wait`, `queue.list`, `queue.remove`, `queue.send`, `project.add`, `project.remove`, `profile.save`, `profile.set`, `profile.delete`, `project.pick`, `project.profile`, `rules.list`, `rules.sets`, `rules.apply`, `rules.default`, `rules.show`, `rules.library.new`, `rules.project.status`, `rules.project.updates`, `rules.project.assign`, `rules.project.unassign`, `rules.project.update`, `rules.project.import`, `rules.project.generate`, `rules.project.move`, `rules.reapply`, `rules.set.save`, `rules.set.delete`, `rules.import`, `review.changes`, `space.list`, `space.create`, `space.rename`, `space.move`, `agent.move`, `mcp.tools`, `mcp.call`.

## Errors

- `denied`: the permission isn't granted (or was revoked: it applies to the
  next call). The message names it and where the user grants it.
- `unsupported`: not an extension's method, or not in this Pitwall yet.
- `busy`: past 20 calls a second, socket and adapter together, or past
  an action's own limit ([actions](actions.md)). Wait and call again.
- `approval_timeout`: nobody answered a request to start an agent in
  time; nothing was started.
- `unknown_method`: no such method.
