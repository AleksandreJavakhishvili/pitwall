# The manifest, format 2

Format 2 is [format 1](manifest.md) with five additions: the Pitwall it
needs, the permissions it asks for, what it contributes, more kinds
of settings, and a [service](service.md) it keeps running. Everything
else reads as in format 1. Use it when your adapter speaks
[protocol 2](protocol-2.md), or when you need the new setting types or
a service.

```json
{
  "v": 2,
  "id": "ci-runs",
  "name": "CI runs",
  "version": "0.1.0",
  "pitwall": ">=0.9.0",
  "permissions": {
    "agents.read": "To match each run to the agent that pushed it",
    "worktrees.read": "To show which worktree a run built"
  },
  "contributes": { "inbox": true },
  "settings": [
    { "key": "server", "type": "url", "label": "CI server", "required": true },
    { "key": "token", "type": "secret", "label": "Token", "required": true },
    { "key": "team", "type": "text", "label": "Team", "placeholder": "platform" },
    { "key": "level", "type": "choice", "label": "Show", "default": "failed",
      "options": [{ "value": "all", "label": "Every run" }, { "value": "failed", "label": "Failed runs" }] },
    { "key": "branchesOnly", "type": "bool", "label": "Only agents' branches", "default": true },
    { "key": "limit", "type": "number", "label": "Runs to keep", "min": 1, "max": 200, "default": 50 }
  ],
  "adapter": { "command": ["node", "adapter.mjs"] }
}
```

## `v` and `pitwall`

- `v`: 2. A Pitwall that reads only format 1 refuses the manifest with
  "it needs a newer Pitwall".
- `pitwall`: the lowest Pitwall it needs, as `>=X.Y.Z`. An older Pitwall
  refuses it with "it needs Pitwall X.Y.Z or later (this is …)", before
  anything else in it. `check` warns when it is missing.

## `permissions`

Each permission's name, and your reason for it in one line of at most
200 characters. There are at most 16. The user sees your reason next to
what the permission gives, and grants or refuses each one. See
[permissions](permissions.md) for the list and what the user sees.

A name this Pitwall doesn't serve still loads: it is shown as not
available and never granted.

## `contributes`

What it adds to Pitwall's UI. Each needs an `adapter`:

- `inbox`: `true` when its threads show in the inbox.
- `views`: view tiles, `[{"id", "title", "icon"}]`, at most 10.
- `commands`: command palette entries, `[{"id", "title", "context"}]`
  (`context`: `none`, or `agent` for one shown only while an agent is
  focused), at most 20, no keys.
- `badges`: `true` when it puts badges and notes on sidebar rows.
- `notifications`: `true` when it shows notifications; it needs the
  `notify` permission.
- `panels`: right-panel sections, from before Pitwall's right panel was
  removed. Still read, so an older manifest installs, and never drawn;
  `pitwall extension check` warns. Use `views` instead.
- `webViews`: web pages in a web tile, `[{"id", "title", "icon", "url"}]`,
  at most 5. `url` is `https://…`, `http://localhost…` (or
  `127.0.0.1`, `[::1]`), or starts with `{setting:KEY}` (a `url` setting);
  without it the adapter sends the address. No script bridge: `bridge`,
  `scripts` and `preload` are refused.

Ids are unique across `views` and `webViews`.

[Views, commands, badges and notifications](views.md) says what
each draws and what the adapter sends. `cards` (floating-control cards)
is refused with "it contributes “cards”, which needs a newer Pitwall":
the floating control shows only what blocks an agent. A key no Pitwall
knows is refused too. In format 1, an extension with an adapter is an inbox.

An adapter that contributes nothing is allowed: it can read what it is
granted and work outside Pitwall's UI. `check` mentions it.

## Setting types

On top of format 1's `url` and `secret`:

| Type | The field | In `hello` | Keys |
|---|---|---|---|
| `text` | One line, at most 500 characters | a string | `placeholder`, `default` |
| `choice` | One button for each option | the option's `value` | `options` (2–20): each a string, or `{"value","label"}`; `default` |
| `bool` | A switch | `true` or `false` | `default` (`true` or `false`) |
| `number` | A number field, checked | a number | `min`, `max`, `default` |

- `default` is what the adapter gets while the user hasn't set one. A
  `required` setting with a `default` counts as set.
- A `number` outside `min`–`max` isn't saved: the field says
  "Between 1 and 200.".
- A `secret` can't have a `default`.

The new types are read in format 1 too, but a Pitwall from before them
refuses a manifest that uses them. Give such a manifest `v: 2` and a
`pitwall`, so an older Pitwall says what it needs.

## `service`

One program Pitwall keeps running on this computer while the extension
is on, such as the server its agents talk to: its command (`{setting:KEY}`
and `{data}`, its own private folder), the action that installs it, the
lowest version it needs, how Pitwall tells it is up (an address on this
computer or a line it prints), its variables and an optional first run
such as a first sign-in. The user confirms its exact commands once.
[A service](service.md) has the keys and the rules. A format 1 manifest
ignores `service`.

## `update`

How an update of it goes: `"update": { "order": "serviceFirst", "agents":
"compatible" }`. `order` (`serviceFirst`, the default, or `agentsFirst`)
says whether the new service starts before or after the agents' setup
runs again; `agents` (`compatible`, the default, or `restart`) says
whether agents started before an update keep working with what they
started with. With a service's `backup`,
[Updates](updates.md) has the steps, the backups and what happens when a
step fails. A format 1 manifest ignores `update`.

## Checking it

`pitwall extension check <folder> --human` prints the format, each
setting with its type, options, range and default, the permissions with
your reasons, what it contributes and its service (which runs
continuously). It warns about format-2 keys in a
format 1 manifest (they are ignored there) and about permissions this
Pitwall doesn't serve.
