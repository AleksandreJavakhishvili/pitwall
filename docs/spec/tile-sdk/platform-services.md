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
  Authors on the SDK preview ([15.4](extension-model.md#154-from-adapter-to-component)) count the same: their extension that
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
| Settings and secrets, form drawn by Pitwall | `settings` ([3.8](runtime.md#38-host-apis)) | none (own only) | on [today's settings and secure store] |
| Notifications and toasts | `pitwall` (notify), `ui.toast` | `notify` for notifications | on when asked for, ≤ 3 a minute [today] |
| Open a file at a line, the editor, a web tile | `pitwall` (`viewer.open`, `web.open`) | `show` | off [today] |
| Timers | `tasks` ([3.8](runtime.md#38-host-apis)) | none | on; paused while hidden |
| Agents and their status | `pitwall` (`agent.list`, `agent.status`, `events.subscribe`) | `agents.read` | on when asked for [today] |
| Badges and row notes on sidebar rows | `contrib.badges` [new for tiles] | none for rows named by `matchAgents`; `agents.read` for an agent by id, `projects.read` for a project [today's rules] | on when the manifest says `"badges": true` |
| Questions in the floating control | `asks` [new for tiles] | none (its own questions) | on when the manifest says `"asks": true` |
| An answer or instruction into an agent's Next up | `pitwall` (`queue.add`) | `agents.queue` | off; items marked "From <name>" and waiting for Send [today] |

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

```wit
interface asks {
  record suggestion { label: string, description: option<string> }
  record ask {
    id: string,                 // the tile's own, stable while open
    from: option<string>,       // an agent by matchAgents name, or by id with agents.read
    title: string,              // ≤ 200
    question: string,           // ≤ 2 000, Markdown drawn inertly
    suggestions: list<suggestion>,   // ≤ 8
    multiple: bool,
    blocking: bool,             // holds an agent up: shown first, "Waiting on you"
  }
  /// The whole set of open questions, each time (≤ 50; the control shows
  /// the first 6, as it does for an inbox).
  set: func(open: list<ask>);
  /// After an `ask-answered` event: accepted, or refused with a note the card shows.
  result: func(id: string, refused: option<string>);
}
```
The tile gets `ask-answered(id, picks, text, declined)` as an event.
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

### 14.3 Later, on demand
Each is designed enough to know it fits (the sections named), and is
built only when its trigger happens.

| Service | Where designed | Trigger that justifies it |
|---|---|---|
| Files in projects (`files.read`, `files.write`) | [3.8](runtime.md#38-host-apis), [5.1](permissions.md#51-new-permissions-new) | Notes keeps notes as Markdown in a project, or Diagrams' `export_svg` writes into one; in the plan from the start of the review port ([16.3](native-blocks.md#163-which-built-in-tiles-move-to-the-sdk)), whichever comes first |
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
