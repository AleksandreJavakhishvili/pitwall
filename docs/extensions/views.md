# Views, commands, badges and notifications

Beyond an inbox, an extension can add four things to Pitwall's UI. Each is
declared in a [format 2 manifest](manifest-2.md) under `contributes`, and
sent by the adapter as JSON lines. Pitwall draws all of it itself: it
loads no code, and none of it changes what an agent may do.

| Contribution | In the manifest | What the user gets |
| --- | --- | --- |
| [View tiles](#view-tiles) | `"views": [{"id","title","icon"}]` (≤ 10) | A tile of the layout, opened from the command palette |
| [Commands](#commands) | `"commands": [{"id","title","context"}]` (≤ 20) | Entries in the command palette |
| [Badges and notes](#badges-and-notes) | `"badges": true` | A count or a few words on agents' and projects' rows |
| [Notifications](#notifications) | `"notifications": true` and the `notify` permission | A notification, at most 3 a minute |

Each needs an `adapter`. The protocol version doesn't matter: a protocol 1
adapter may send them too. Pitwall's `hello` lists what it draws in
`offers` (`views`, `commands`, `badges`, `notify`); check it there rather
than by version. `pitwall extension new --view` writes a starter with a
list view, a command and badges ([the starter](starter.md)).

```json
{
  "v": 2,
  "id": "ci-runs",
  "name": "CI runs",
  "version": "0.1.0",
  "permissions": {
    "agents.read": "To note a failing run on the agent that pushed it",
    "notify": "To tell you when a run on your branch fails"
  },
  "contributes": {
    "views": [{ "id": "runs", "title": "Runs", "icon": "list" }],
    "commands": [
      { "id": "refresh", "title": "Refresh runs" },
      { "id": "rerun-mine", "title": "Re-run the focused agent's last run", "context": "agent" }
    ],
    "badges": true,
    "notifications": true
  },
  "adapter": { "command": ["node", "adapter.mjs"] }
}
```

A view, a command or a badge from an extension the manifest doesn't
declare is refused and noted in its **Activity**. Ids are words (`a-z`,
`0-9`, `-`, `_`, `.`), unique within their list; titles are at most 40
characters. The icons Pitwall has are named in
[`docs/spec/extensions.md`](../spec/extensions.md); an unknown one is
replaced, never an error.

## View tiles

A view is a tile of the layout, a peer of the inbox: the user opens it
with **Open <title>** in the command palette (or from a badge or a
notification that names it), and moves it, maximises it (⌘⏎), puts it in
a window of its own and closes it as any tile. It is back after a
restart, where it was.

Pitwall tells the adapter when a tile shows the view, and the adapter
sends the **whole view** each time something in it changes. There are no
patches: send it again.

| Pitwall → adapter | When |
| --- | --- |
| `{"type":"show","view":"runs"}` | A tile shows it (again after a restart of the adapter). Send the view now. |
| `{"type":"hide","view":"runs"}` | No tile shows it. Stop sending it. |
| `{"type":"select","view":"runs","item":"r812"}` | The user selected a row: send the view with that row's `detail`. |
| `{"type":"action","req":12,"view":"runs","item":"r812","action":"rerun"}` | A button was pressed (`item` is absent for a button of the view itself). Answer with `result`. |

| Adapter → Pitwall | |
| --- | --- |
| `{"type":"view","view":"runs","root":{…}}` | The view's content, below. A view that isn't shown is ignored. |
| `{"type":"result","req":12,"ok":false,"message":"No runner is free."}` | The answer to an `action`; a failure's message shows in the tile. |

### Components

A `root` is a **list**, with a pane beside it for the selected row, or a
single pane: a **detail**, **markdown** or a **status**. There is no
nesting beyond that, and no layout, colours or fonts: Pitwall draws them
in its own style, in both themes.

#### `list`

```json
{"type":"view","view":"runs","root":{
  "kind": "list",
  "empty": "Nothing failing",
  "sections": [
    {"title": "Failing", "items": [
      {"id": "r812", "title": "api: tests failed", "subtitle": "agent/fix-auth",
       "tone": "bad", "icon": "alert", "time": "2026-11-20T10:02:00Z",
       "badges": [{"text": "2 jobs", "tone": "warn"}],
       "actions": [{"id": "rerun", "label": "Re-run", "primary": true}, {"id": "log", "label": "Open log"}]}
    ]},
    {"title": "Passed", "items": [{"id": "r811", "title": "web: build", "tone": "good"}]}
  ],
  "detail": {"kind": "markdown", "text": "Select a run to see its log."}
}}
```

- `sections`: each with an optional `title` and its `items`. At most 50
  sections and 500 items in all; an item id seen twice keeps the first.
- An item: `id` (required, ≤ 200 characters), `title` (≤ 200),
  `subtitle` (≤ 300), `tone` (`good`, `bad`, `warn`, `busy` or `none`:
  the colour of its dot or icon), `icon`, `time` (an ISO time, shown
  short, or a few words), ≤ 3 `badges` (`text` ≤ 24 characters and a
  `tone`), ≤ 5 `actions`.
- An action: `id`, `label` (≤ 40) and `primary` (the first primary one
  is the row's main button). A row's buttons show when it is selected.
- `empty`: what an empty list says.
- `detail`: what is beside the list, a `detail`, `markdown` or `status`
  pane. Send it for the row last selected.

Keys while the tile has the keyboard: ↑↓ or J K select a row (the adapter
gets `select`), Enter presses its main button.

#### `detail`

```json
{"kind": "detail", "title": "api: tests failed",
 "fields": [{"label": "Branch", "value": "agent/fix-auth"},
            {"label": "Run", "value": "#812", "link": "https://ci.example.com/runs/812"}],
 "body": "**2 jobs failed.**\n\n```\nexpected 200, got 500\n```",
 "actions": [{"id": "rerun", "label": "Re-run", "primary": true}]}
```

A title, ≤ 30 fields (`label` ≤ 60, `value` ≤ 500, an optional http(s)
`link`, opened in the browser), a Markdown `body` (≤ 40 000 characters,
drawn by Pitwall's inert renderer: no HTML, no remote images), and ≤ 5
actions. Beside a list, its actions are on the selected row (`item` is
that row's id).

#### `markdown`

```json
{"kind": "markdown", "text": "## Today\n\n- 12 runs\n- 2 failed"}
```

#### `status`

```json
{"kind": "status", "state": "error", "icon": "lock", "title": "Signed out",
 "text": "Your token expired.", "actions": [{"id": "signin", "label": "Sign in again"}]}
```

An empty, loading or error state (`state`: `empty`, `loading`, `error`)
with an icon, a title, a line and ≤ 2 actions.

#### Later: `form` and `table`

A `form` or a `table` is refused: the tile says "It sent a form, which a
later Pitwall draws", and nothing else of that view is drawn until it is
sent again in a kind this Pitwall draws.

### Limits

A `view` line is at most 256 KiB. Over that, or over 500 items or 50
sections, the view is refused with a plain line in its tile; the adapter
keeps running. Text longer than its limit is cut, and control characters
are taken out.

### Several connections

With [connections](../spec/extensions.md#connections), each connection's
adapter is shown the view. A list's sections are shown one after
another, each named after its connection; the ids your adapter gets back
in `select` and `action` are its own. Any other root is the first
connection's.

## Commands

```json
"commands": [{ "id": "refresh", "title": "Refresh runs" },
             { "id": "rerun-mine", "title": "Re-run the focused agent's last run", "context": "agent" }]
```

Each shows in the command palette as "<Extension>: <title>" while the
extension is on and its adapter runs. One with `"context": "agent"` shows
only while an agent is focused. Commands have no keys of their own (a
`keys` field is refused): Pitwall's shortcuts are dense and two extensions
would collide.

Picking one sends the first running adapter:

```json
{"type":"command","req":5,"command":"rerun-mine","agentId":"a1"}
```

`agentId` is there for an `agent` command when the extension is granted
`agents.read`. Answer with `result`; its `message`, or a failure, shows
as a toast.

## Badges and notes

With `"badges": true`, an adapter puts a count or a short note on
agents' and projects' rows in the sidebar, beside the inbox's ask badge.
It sends the whole set each time (an empty list clears them):

```json
{"type":"badges","badges":[
  {"agent": {"agentId": "a1"}, "count": 2, "tone": "bad", "tooltip": "2 failing runs", "open": {"view": "runs", "item": "r812"}},
  {"agent": {"from": {"agent": "api", "session": "a7f3"}}, "note": "PR #42 ✓", "tone": "good"},
  {"project": {"path": "/Users/me/code/api"}, "note": "deploying", "icon": "refresh"}
]}
```

- The row: an agent by Pitwall's id (needs `agents.read`), an agent by
  the names its [`matchAgents`](manifest.md#matchagents) gives it (no
  permission needed), or a project by its folder (needs
  `projects.read`). One the extension may not name is dropped, and noted
  once in its Activity.
- A `count` (≤ 9999) and/or a `note` (≤ 16 characters), a `tone`, an
  `icon`, a `tooltip` (≤ 120, after the extension's name) and what a
  click opens: `{"view": "runs", "item": "r812"}` or `{"thread": "t1"}`
  (an inbox thread).
- At most 500 in a set, and at most four sets a second are applied (the
  newest waits for its turn). They go when the extension is turned off or
  its adapter stops.

## Notifications

With `"notifications": true` and the `notify` permission (asked for with
a reason, on by default in the permission screen, turned off there or in
the extension's card at any time):

```json
{"type":"notify","id":"r812","title":"api: tests failed","body":"agent/fix-auth","open":{"view":"runs","item":"r812"}}
```

- The title (≤ 100) is shown after the extension's name, then the body
  (≤ 300).
- It follows Pitwall's own rule for notifications: a system notification
  only while Pitwall isn't in front (never from a test instance). In
  Pitwall it is also a toast; its **Open** button opens what `open`
  names. A system notification can't report a click, so `open` is used
  only by the toast.
- At most 3 a minute per extension; more are dropped and noted in its
  Activity. The same `id` with the same words isn't shown again within
  10 minutes.

## In `pitwall extension check` and `test`

`check` lists what an extension contributes and warns about a `notify`
permission without `notifications`, or badges that can name no row.
`test` runs the `views` part of `test/pitwall.json` against a fake
Pitwall ([checks](checks.md#views)).
