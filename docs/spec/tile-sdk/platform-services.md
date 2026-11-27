# Tile SDK: platform services

Part of the [Tile SDK and marketplace](README.md) design. What Pitwall gives every tile: the minimum, then on demand. Sources (S1…) and *unverified* claims are listed in [sources.md](sources.md).

## 14. Platform services: what Pitwall gives every tile

### 14.1 The rule: the minimum, then on demand
Every tile author would otherwise build the same few things: somewhere
to keep data, a way to call a service, a settings form. Pitwall gives
those, inside the sandbox it controls, and nothing more until a real
tile needs it.

- **A service is added only when a first-party seed tile ([10.1](README.md#101-seed-tiles-first-party-public-sdk-only)), built
  solely on the public SDK, needs it and can't reasonably do without
  it.** "Would be nice" is not a reason; a seed tile that can't ship is.
  Authors on the SDK preview ([15.1](extension-model.md#151-one-concept-not-two)) count the same: their extension that
  can't ship without a service is the "a real tile needs it" signal.
- **The WIT stays 0.x until the seed tiles ship.** Breaking changes are
  allowed in 0.x, announced in the changelog one release ahead ([8.2](runtime.md#82-how-old-tiles-keep-working)).
  **1.0 freezes only then**, with whatever the seed tiles proved they
  need (phase 7).
- Pitwall runs no servers of its own: it gives what runs on this
  computer and is the same for every tile.

### 14.2 The v0.x minimum (built in phases 1–2)
| Service | WIT interface | Permission | Default |
|---|---|---|---|
| Key-value store | `storage` ([3.8](runtime.md#38-host-apis)) | none (own data only) | on; 10 MiB |
| SQLite database, run by Pitwall | `sql` (as designed for services, [13.3](services.md#133-b-a-headless-wasm-service)) | none (own data only) | on; 100 MiB |
| HTTP client to granted hosts | `net` ([3.8](runtime.md#38-host-apis), [5.1](permissions.md#51-new-permissions-new)) | `net` with its hosts | as declared |
| Settings and secrets: get and set, secrets by reference, the UI in the extension's Setup view ([14.6](#146-settings-secrets-and-the-setup-view)) | `settings` ([3.8](runtime.md#38-host-apis)) | none (own only) | on [today's settings and secure store]; the generated form is removed (2026-11-28) |
| Notifications and toasts | `pitwall` (notify), `ui.toast` | `notify` for notifications | on when asked for, ≤ 3 a minute [today] |
| Open a file at a line, the editor, a web tile | `show` (`file`: the editor, `view`: the file viewer, `viewer.open`; `web`) [built, [3.11](ui.md#311-the-seed-tiles-gaps-as-built)] | `show` | off [today] |
| Timers | `tasks` ([3.8](runtime.md#38-host-apis)) | none | on; paused while hidden |
| Agents and their status | `agents` (`list`, the `agents-changed` event) [built: a typed interface over the daemon's `agent.list`] | `agents.read` | on when asked for [today] |
| A toolbar item with a badge ([3.9](ui.md#39-toolbar-items)) | `toolbar` [new] | none; the install screen lists "Adds a button to the toolbar"; per-project badges need `projects.read` | on when the manifest has `toolbar` |
| Badges and row notes on sidebar rows | `badges` (`set`) [built] | `agents.read` for an agent by id, `projects.read` for a project [today's rules]; `matchAgents` is removed with adapters | on when the manifest says `"badges": true` |
| Questions in the floating control | `asks` [built] | none (its own questions) | on when the manifest says `"asks": true` |
| An answer or instruction into an agent's Next up | `agents` (`queue-add`) [built: the daemon's `queue.add`] | `agents.queue` | off; items marked "From <name>" and waiting for Send [today], shown in the agent's queue: the "N queued" chip on its terminal tile and the popover it opens (Send now, Remove, Auto-send) |

**The agents' questions group: why it is in the minimum.** All four
exist for today's extensions; tiles get the same host code with the
same permission names, not new platform:
- `queue.add` and the agent reads are methods of the daemon's table
  ([methods.md](../../extensions/methods.md) [today]), already reachable
  through the `pitwall` interface ([3.8](runtime.md#38-host-apis)).
- Badges and row notes are the adapter's `badges` message
  ([views.md](../../extensions/views.md#badges-and-notes) [today]); a tile
  sends the same set through `contrib.badges(list)`: the whole set each
  time, ≤ 500, a note ≤ 16 characters, at most four sets a second, gone
  when the extension is turned off.
- Questions are what the floating control already shows for
  extensions' inboxes ([extensions.md](../extensions.md), "In the floating
  control" [today]): a card with who asks, the question, suggestions,
  a field, Send and Decline. Today they come from an adapter's inbox
  threads, not from the method table.
- `agents.read` comes along because Next up and badges by agent id need
  agent ids; it is the existing read permission, with its existing risk
  note. Projects, worktrees and the focused agent stay on demand.

With these, an inbox-style tile (questions from a service or from
agents, answered in the tile or the floating control, an answer sent to
an agent's Next up) is a v0.x target.

**As built** (WIT 0.1, extended in place; `crates/pitwall-tile-host/wit/`
`agents.wit`, `badges.wit`, `asks.wit`). Typed interfaces, not a general
`pitwall` method call: each is one daemon method or one message the
adapter path already has, called as the extension (same grants, limits
and Activity, marked "tile"), so a tile can't reach methods outside the
group.

```wit
interface agents {
  enum agent-state { working, blocked, idle, done, stopped, unknown }
  record agent { id: string, name: string, kind: string, kind-name: string,
    project: string, branch: option<string>, state: agent-state,
    waiting-on-you: bool, queued: u32 }
  %list: func() -> result<list<agent>, host-error>;          // agents.read
  queue-add: func(agent: string, text: string) -> result<_, host-error>;  // agents.queue; ≤ 4 000
}
interface badges {
  variant row { agent(string), project(string) }   // by id (agents.read) or folder (projects.read)
  record badge { row: row, count: option<u32>, note: option<string>, tone: tone,
    tooltip: option<string>, open: option<string> }   // open: one of its tiles
  set: func(badges: list<badge>) -> result<_, host-error>;   // the whole set, ≤ 500
}
interface asks {
  record suggestion { label: string, description: option<string> }
  record ask { id: string, %from: option<string>,   // an agent by id, with agents.read
    title: string, question: string, suggestions: list<suggestion>,
    multiple: bool, blocking: bool }
  set: func(open: list<ask>) -> result<_, host-error>;       // the whole set, ≤ 50
  reply: func(id: string, refused: option<string>);           // `result` is a WIT keyword
}
```
The tile gets `ask-answered(ask-answer { id, picks, text, declined })`
and `agents-changed` (coalesced; only with `agents.read`) as events.
`queue-add` returns at once (no task): the daemon's checks are
synchronous.
Pitwall keeps the last set while the tile is suspended or dropped ([2.3](runtime.md#23-lifecycle)),
so questions stay in the control; an answer to a dropped tile
instantiates it first. "Open in inbox" on the card becomes "Open
<tile>". The same answer path and "Sending…" state as inbox asks apply
[today], so the control doesn't learn a second model.

Plus what every tile has anyway: its Log (`log`), the theme and
appearance in `init` and the `theme` event ([3.4](ui.md#34-events)), and its own `/data`
folder through WASI, counted in the same quota.

**Storage details.**
- **SQLite run by Pitwall**, not compiled into the tile: SQLite in a
  WASI guest is unproven and lacks file locking there (S22), and one
  host-run database can later be shared safely between a tile and its
  wasm service ([13.3](services.md#133-b-a-headless-wasm-service)). The tile sends statements with parameters, gets
  rows, and opens transactions as resources.
  ```rust
  let db = sql::open()?;
  sql::migrations!(db, "migrations/")?;   // numbered .sql files, a version table
  let rows = db.query("SELECT id, title FROM notes WHERE done = ?", (false,))?;
  ```
- **One scope in 0.x: per extension**, in
  `<data>/extension-data/<id>/`, outside the installed folder, so
  updates and Go back keep it [today's rule for `{data}`]. A tile that
  needs data per project keys it by project itself. Per-project scopes
  are a "later" item ([14.3](#143-later-on-demand)).
- **Quotas** shown on the card with the current size and **Clear**;
  the manifest may ask for more (≤ 2 GiB, shown on the permission
  screen); over quota, writes fail with `quota`.
- **Migrations on update** run inside a transaction on first open;
  `safe_update` backs the databases up first (SQLite's online backup)
  so Go back can restore them, and the review says the update changes
  stored data.
- **Uninstall** asks "Also delete its data (12 MiB)?", default keep
  [today's rule for services' data].
- **No other tile's data**: no interface takes another extension's id
  or a path outside its folder.

*Not done: "included in Pitwall's backups".* Pitwall has no backup or
export of its own data today. Tile data sits in Pitwall's data folder,
so the user's own backups cover it, and the per-update backups exist
[today]. A Pitwall-wide backup is a separate feature.

**What an adapter gave, for any tile [built].** So that a tile can be
what an adapter-fed extension was (an inbox from a service, say),
without Pitwall knowing that extension:

- **Status** (`status.set`, `"status": true`): connecting, ready,
  signed out or error, with a sentence; the card's pill and the
  update's health check read it (the checklist that also read it is
  removed, [15.5](extension-model.md#155-removed)). The component starting is "Connecting", its
  stopping "Stopped"; Restart on the card restarts it.
- **A shortcut** (`contributes.tiles[].shortcut`, permission
  `shortcut`, off by default): keys that open the tile from anywhere,
  `cmd-` or `cmd-shift-` with a letter, digit or F-key, never
  Pitwall's own; shown beside "Open <tile>" in the palette, changed by
  the user in the card. Bound once per keystroke to an action that
  looks the tile up when pressed (gpui can't take one binding back).
- **Number keys** need nothing new: `bind_key("1")`… fire while the
  view has focus and no field does, as in Pitwall's inbox.
- **Send with take-back** is in the SDK, not the host: `TakeBack<T>`
  holds a send with a countdown banner and Undo on the tile's timers.

### 14.3 Later, on demand
Each is designed enough to know it fits (the sections named), and is
built only when its trigger happens.

| Service | Where designed | Trigger that justifies it |
|---|---|---|
| Files in projects (`files.read`, `files.write`) | [3.8](runtime.md#38-host-apis), [5.1](permissions.md#51-new-permissions-new), as built in [14.5](#145-files-in-projects-as-built) | Built (#136): Notes keeps notes as Markdown in a project, and the C4 tile draws the diagrams in a project's docs and follows their changes |
| Git (`git.read/write/remote`) | [3.8](runtime.md#38-host-apis), [5.1](permissions.md#51-new-permissions-new) | The review port ([16.3](native-blocks.md#163-which-built-in-tiles-move-to-the-sdk)) brings `git.read/write` into the plan when it starts; `git.remote` waits for the git client seed tile |
| File and folder pickers (grant one item per pick) | — | A tile must open a file the user chooses without a project-wide `files` grant (database browser opening a SQLite file) |
| Reads of Pitwall data beyond agents: projects, worktrees, the focused agent | [3.8](runtime.md#38-host-apis) (method table) | A seed tile must place data by project or follow the focused agent |
| Starting agents (`agents.start`) | [3.8](runtime.md#38-host-apis) [today for adapters] | A seed tile's flow can't continue without a new agent |
| Tile-to-tile tool calls | — (would reuse 11's kinds and grants, with a caller of kind `tile`) | Two seed tiles must cooperate and the user can't reasonably copy between them |
| OAuth sign-in (system browser, loopback redirect with PKCE as RFC 8252 describes, S23; tokens in the secure store, added to requests by Pitwall) | — | A seed tile's service has no personal-token option the user can paste into a secret setting |
| Clipboard (write after a user action; read with `clipboard.read`) | [3.8](runtime.md#38-host-apis) | The API client's "copy as curl" |
| Drag and drop between tiles | [3.6](ui.md#36-drag-and-drop) | The files tile dropping a file on the database browser |
| Per-project storage scopes | [14.2](#142-the-v0x-minimum-built-in-phases-12) | Two seed tiles key data by project by hand |
| `net.ask` (hosts approved at run time) | [5.1](permissions.md#51-new-permissions-new) | The API client seed tile |

### 14.4 Not provided
- **A hosted database, sync between devices or users, accounts,
  payments or push.** These need a server, and Pitwall runs none. A
  tile that needs them talks to its own backend through `net`, or brings
  a service ([13](services.md#13-tile-apps-with-a-server)).
- **Terminal contents, at any grant.** Agents' screens and scrollback
  carry what passes through them: secrets printed by tools, prompts,
  approval text, other people's code. A tile that could read them would
  be a keylogger with a reason field. The same limit holds for adapters
  [today] and agents' tools ([11.9](agent-tools.md#119-built-in-tiles-same-model)).
- **Other tiles' or extensions' data, settings or secrets.**
- **Credentials**: account logins, tool logins, Pitwall's own settings.
- **Background work while hidden** for a tile ([2.3](runtime.md#23-lifecycle)): that is a
  service's job ([13](services.md#13-tile-apps-with-a-server)).

### 14.5 Files in projects, as built
Built in #136 (`files.wit`, the SDK's `files` module), for the C4 tile's
docs diagrams and Notes' project notes.

- **The project is the view's, not picked.** [5.1](permissions.md#51-new-permissions-new)
  had "projects the user picks for this tile". As built, a view is
  open in one project: its space's, else the project it was opened for
  (a project's sidebar menu, the calling agent's project), else the one
  project of the agents beside it in its space. `files::project(view)`
  says which (name and folder), and `project-changed` tells the tile
  when it changes (closing that view's watches). A tile never names a
  folder, so there is no pick dialog; the permission screen lists the
  folders its open views are in now. This also closes Notes' G5: a
  tile learns its project.
- **Its own files folder.** `root.data` is
  `<data>/extension-data/<id>/files/`, needing no permission, at most
  100 MiB; deleting there is for good.
- **Paths.** Relative with `/`; absolute paths, `\`, drive letters,
  `..` and NUL are refused; each part is resolved with links followed
  and refused when it leaves the root or reaches `.git` or Pitwall's
  data folder. `.git` is never listed, read or changed (stricter than
  "never writable"); `.pitwall.json` and `.pitwall/` at the project's
  top are never changed.
- **Ignore files.** `list` and `watch` leave out what the project's
  `.gitignore` files and `.git/info/exclude` ignore (the user's global
  excludes aren't read); `read` and `stat` reach ignored files by name.
- **Limits.** 8 MiB a file, 10 000 list entries, 8 watches a tile;
  watch events are coalesced (250 ms quiet, at most 1 s late).
- **Writes.** Whole files, written to a temporary file and renamed
  over (its permissions kept); `create` refuses an existing file;
  `delete` moves to the system Trash (macOS's, Windows' Recycle Bin,
  the freedesktop Trash).
- **Agents' calls.** `files.write` is an acting permission: during a
  `show`, `present` or `read` call writes are `denied`.
- **Activity.** Every change to a project (`files.write`,
  `files.rename`…, with the project and path) and every refusal is a
  line in the extension's Activity; the same refusal within a minute
  is one line.
- **Not done.** WASI preopens under `/projects/<n>` aren't given (the
  `files` interface is the only route), and a `files.write` that takes
  the expected version (Notes' hla §4.1) isn't there: Notes checks the
  version with `stat` first.

### 14.6 Settings, secrets and the Setup view
Decided by the owner on 2026-11-28 ([15.9](extension-model.md#159-decisions) item 8). The extension
draws its own settings and setup screen; Pitwall keeps the values, the
secrets and anything that grants something.

**The schema stays; the form goes.** `settings` in the manifest is a
list of `{ key, type, default }` (`text`, `url`, `number`, `bool`,
`choice` with its values, `secret`); `label`, `hint`, `placeholder`
and `required` are dropped (the Setup view says those things). The
schema tells Pitwall which values to keep in `ui.json`, which go to the
secure store, and which a `{setting:KEY}` in `net` hosts or `programs`
patterns may name.

**Get and set.** `settings.get(key)` as today. New
`settings.set(key, value) -> result<_, host-error>` for the component's
own Setup view: validated against the schema (a `url` must parse),
written to `ui.json`, `settings-changed(key)` sent to every instance.
Two kinds of setting refuse `set` with `host-error.denied` and are
changed only by the user in a **host-drawn field** the Setup view places
([3.12](ui.md#312-the-setup-view-and-host-drawn-fields)):
- **secrets** (`type: "secret"`), and
- **grant-bearing settings**: those a `{setting:KEY}` names in `net`
  hosts or `programs` patterns. A component that could set them could
  point its own `net` grant, or a confirmed command, anywhere.

**Secrets by reference.** `settings.secret` (which returned the value)
is removed ([plan](plan.md) 8c4, once Agention 3.0.0 no longer uses it). The component learns only whether a secret is set:
```wit
secret-set: func(key: string) -> bool;     // and the event secret-changed(key, set: bool)
```
and uses it where the host puts the value in:
- `net`: a header value `secret(key)` or `bearer(key)` (`Authorization:
  Bearer <value>`), substituted only when the request's host is allowed
  by the extension's `net` grant at the moment it is sent, redirects
  included; otherwise the request fails with `denied` and nothing is
  sent;
- `programs`: an environment value `secret(key)` ([13.2](services.md#132-a-programs-the-component-starts)).
Secrets are taken out of Log lines and of program output returned to
the component. A service that answers with the secret in its body
defeats this; that is the service's bug, and the threat model says so
([5.5](permissions.md#55-threat-model-and-outside-review)).

**The Setup view.** One of the component's views, named in the manifest:
```json
"contributes": { "setup": { "view": "setup" } }
```
drawn on the extension's page ([3.12](ui.md#312-the-setup-view-and-host-drawn-fields)) whenever the page is shown,
with `init`'s view kind `setup`. `settings.open()` opens the page at it.
It is an ordinary view (any component, events, tasks, `net`, programs),
held while the page is open and let go as a hidden tile is. An extension
without a Setup view shows only the host's parts of its page.

**Agent environment** (`agents.env`, replacing `agentEnv`):
```wit
set-env: func(vars: list<env-var>) -> result<_, host-error>;  // the whole set, ≤ 16
```
on the `agents` interface. Decided (owner, 2026-11-28): generic and
limited.
- **Names** must be in the permission's `names`, which the permission
  screen shows as written.
- **Values** are only: a literal; a non-secret setting
  (`{setting:KEY}`); or the agent's own identity, filled in for each
  agent (`{agent_name}`, `{agent_id}` for its session, `{project}`).
  Never a secret: a `secret` setting in a value is refused.
- **Which agents:** only agents Pitwall starts, and only in that
  extension's projects (the projects the extension is on for). Never
  agents started elsewhere.
Applied to agents started after the call; agents already running with
an older set are "running agents that use an old setup"
([updates.md](../../extensions/updates.md)), handled as today. The page shows the names and values.

