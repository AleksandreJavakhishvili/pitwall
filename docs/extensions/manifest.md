# The manifest (`extension.json`, format 1)

Every extension has an `extension.json` at the top of its folder. Pitwall
reads it and nothing else to know what the extension is, what the user
fills in and what it runs. Run [`pitwall extension check`](checks.md) to
see how Pitwall reads yours.

```json
{
  "v": 1,
  "id": "my-inbox",
  "name": "My inbox",
  "version": "0.1.0",
  "icon": "inbox",
  "description": "Questions from my service, in Pitwall's inbox",
  "docs": "https://example.com/my-inbox",
  "settings": [
    { "key": "server", "type": "url", "label": "Server address", "placeholder": "https://…", "required": true },
    { "key": "apiKey", "type": "secret", "label": "API key", "hint": "From your account page", "required": true }
  ],
  "adapter": { "command": ["node", "adapter.mjs"] },
  "agentEnv": [{ "name": "MY_SERVER", "value": "{setting:server}" }],
  "matchAgents": { "agent": "{project}", "session": "{agent_id}" },
  "actions": []
}
```

Only `id` and `name` are required. Keys Pitwall doesn't know are
ignored, so a manifest written for a later Pitwall still loads (a typo
goes unnoticed too: `check` shows what Pitwall read).

## Who it is

- `v`: the format, 1. A higher one is refused with "it needs a newer
  Pitwall".
- `id`: 1–40 characters of lower-case letters, digits and `-`. It starts
  with a letter and doesn't end with `-`. An installed extension's folder
  is named after it.
- `name`: one line, at most 40 characters.
- `version`: optional, at most 32 characters. It starts with a digit and
  uses letters, digits and `.-+`. Pitwall shows it, and compares it to
  offer updates.
- `icon`: optional, the name of one of Pitwall's icons (`inbox`, `bell`,
  `chat`, …). An unknown name draws a puzzle piece.
- `description`: optional, one line, at most 200 characters.
- `docs`: optional, an `https://` link shown as "Docs" on the card.
- `demo`: optional, `true` for a demonstration (listed last, marked
  "demo").

## Settings

What the user fills in. Pitwall draws the fields in Settings →
Extensions. There are at most 10, and each is kept per connection: one
extension can have several connections, each with its own adapter.

- `key`: letters, digits and `_`, starting with a letter, at most 32
  characters. Unique.
- `type`:
  - `url`: an http(s) address.
  - `secret`: kept in the system's secure store and never shown again.
- `label` (at most 60 characters); optional `hint` (at most 300) and
  `placeholder` (at most 100).
- `required`: the adapter doesn't start until it is set.

Settings and secrets reach your adapter **only** in its `hello`. Nothing
of the extension's goes in its argv or environment.

## The adapter

```json
"adapter": { "command": ["node", "adapter.mjs"] }
```

`command` is the program and its arguments, 1–16 strings, run in the
extension's folder:

- A program with a `/` is a path in the folder.
- Otherwise the program is looked up on the user's login shell PATH.
- Arguments that name files in the folder must exist and stay inside
  it. `check` makes sure.

Pitwall starts the adapter only while the extension is on and its
required settings are set. See [protocol 1](protocol.md).

## Agent variables

`agentEnv` sets up to 10 variables on agents Pitwall starts while the
extension is on. Settings lists them, so the user sees them.

- `name`: upper-case letters, digits and `_`. Pitwall's own (`PITWALL_*`)
  and ones a program's start depends on (`PATH`, `HOME`, `LD_*`, …) are
  refused.
- `value`: a template. It can use:
  - `{project}`: the last part of the agent's project folder;
  - `{agent}`: the agent's name when it starts;
  - `{agent_id}`: Pitwall's id for the agent;
  - `{setting:KEY}`: a `url` setting.

A secret can't be used, because secrets are never given to agents. A
variable whose setting isn't set is left out.

## Matching threads to agents

```json
"matchAgents": { "agent": "{project}", "session": "{agent_id}" }
```

`matchAgents` tells Pitwall which agent a thread is from. For each
agent, it fills in these two templates. A thread whose `from.agent` and
`from.session` equal the results belongs to that agent. That agent's row
then shows its open asks, and the thread offers "Go to agent".

## Setup actions

`actions` lists up to 5 buttons in Settings that run commands. Use them,
for example, to install your tool's plugin in the user's agents. Nothing
runs until the user presses one and confirms the exact commands listed.

```json
{
  "id": "setup",
  "label": "Set up my tool in my agents",
  "description": "Installs its plugin in Claude Code",
  "setup": true,
  "steps": [
    { "run": ["claude", "plugin", "install", "my-tool"], "for": "claude" }
  ]
}
```

- `id`, `label` (at most 40 characters), `description` (one line).
- `setup: true`: a step of the extension's setup checklist.
- `steps`, 1–12 of them. Each is:
  - `run`: a list of 1–32 strings, never one shell line. The program is a
    name looked up on PATH. Arguments may use `{setting:KEY}` of a plain
    setting.
  - `for`: optional, the agent kind it is for (`claude`, `codex`, …). The
    step is skipped when that agent isn't installed.
  - `optional: true`: its failure doesn't stop the steps after it.

Check them with an [`actions` test](checks.md#actions).

When a release changes an action's steps (a new version pin, say), a
user who ran it is asked to run it again: Pitwall remembers the steps
each tick was for.

## Releases from git

`source` (optional) lets Pitwall update your extension from your own
repository's release tags, without waiting for a Pitwall release:

```json
"source": { "git": "https://github.com/me/my-tools", "dir": "extensions/my-inbox", "tags": "my-inbox-v{version}" }
```

- `git`: the repository's `https://` address (no user or password in
  it), at most 300 characters. A private one works through the user's
  own git sign-in; Pitwall never asks for a password.
- `dir`: optional, the folder in it that holds `extension.json`
  (relative, `/`-separated). Empty or left out: the repository's root.
- `tags`: which tags are releases, a pattern with `{version}` once
  (letters, digits and `._/-` around it), such as `v{version}` or
  `my-inbox-v{version}`.

To release: raise `version`, commit, then tag that commit with the
pattern (`my-inbox-v1.2.0`) and push the tag. Never move a tag once it is
pushed: Pitwall records the commit a tag pointed at and refuses one that
moved. A release holds everything that runs; Pitwall runs no build step
or package manager on it, and copies only `dir` (without `.git`,
`node_modules` or `.DS_Store`), at most 50 MiB and 5 000 files, with no
links pointing outside it. Its `extension.json` must have the tag's version.

Pitwall checks at start, once a day and when the user presses "Check for
updates", and shows what a newer release changes on the computer before
the user presses Update. Users can also install it with "Add from Git…"
in Settings → Extensions. `pitwall extension check` says which tag the
current version's release is. A folder loaded with "Load from folder…"
is never updated.

Every limit is in [the specification](../spec/extensions.md#the-manifest-extensionjson-format-1).
