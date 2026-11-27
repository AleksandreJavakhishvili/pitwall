# Agention (example Pitwall extension)

[Agention](https://github.com/WayfarerLabs/agention) (MIT, by Wayfarer
Labs, not part of Pitwall) is a self-hosted "attention inbox": your agents
post questions into threads through an MCP bridge, and you answer them.
With this extension you answer them in Pitwall, in a tile, next to the
agents that asked.

This folder is a Pitwall extension (see
[docs/spec/extensions.md](../../docs/spec/extensions.md)): a manifest,
`extension.json`, and a tile, `tile.wasm`, built from the Rust crate here
(`src/`) with Pitwall's public Tile SDK, like the C4 and Notes tiles. The
tile runs in Pitwall's sandbox and reaches only your Agention server (the
`net` permission names the **Server address** setting's host,
`{setting:server}`); it never touches your agents. Before 2.0.0 this was a
Node.js adapter; see [From the adapter to the tile](#from-the-adapter-to-the-tile).

## What you need

- An Agention server, **v0.7.0 or later**, served at an `https` origin.
  Pitwall can install it and keep it running on this computer
  (section 1), or you run your own. The inbox itself still works with a
  v0.6.0 server, but the plugin this extension installs in your agents
  (v0.7.0) refuses a server older than itself, so a server you run
  yourself needs upgrading first.
- Agention's plugin in Claude Code and/or Codex (step 2), installed once by
  you.

## 1. The server: Pitwall runs it for you

Agention's own [operator guide](https://github.com/WayfarerLabs/agention/blob/main/docs/operator-guide.md)
is the reference. Its server is served at an `https` origin with a domain
name, not an IP address. With Tailscale, `tailscale serve` gives you one,
such as `https://agention.example.ts.net`, in front of the address the
server listens on:

```bash
tailscale serve --bg http://127.0.0.1:8787
```

Pitwall does the rest from the extension's checklist (section 4):

1. **Install Agention's server**: while `agention` isn't on your login
   shell's PATH, the step's button runs
   `npm install -g @wayfarerlabs/agention@0.7.0` after you confirm.
2. **Confirm and start**: the card shows the exact command once,
   `agention serve --data <its data folder> --origin <Server address> --listen <Local server listens on>`,
   and Pitwall keeps it running while the extension is on. It is started
   again if it stops (after 1, 5 and 30 seconds; then Pitwall gives up and
   shows its last lines, with Start). It is stopped when you turn the
   extension off, update or remove it, or quit Pitwall. Its output is in
   the card's Log.
3. **Sign in for the first time**: on a fresh server, the button runs
   `agention passkey-link` and opens the one-time link it prints in your
   browser, where you create your passkey. Pitwall never shows or keeps
   the link. The button stays under More afterwards (a new link replaces
   your passkeys).

Its data is in a folder of its own, `extension-data/agention` in Pitwall's
data folder. It is private to you. Remove asks before deleting it and
keeps it unless you say otherwise.

**Updates are one step** ([docs/extensions/updates.md](../../docs/extensions/updates.md)).
When you press Update, Pitwall stops the server and backs up its database
with `agention backup --data <its data folder> <backup folder>/agention.db`
(a consistent copy, made while the server is stopped) into
`extension-backups/agention/<time>` in Pitwall's data folder (private; the
last three are kept). Then it installs the new files, updates the server
with **Install Agention's server** if the new version needs a newer one,
starts it and checks that it is up.
Only then does it update the plugin in your agents. If the server doesn't
come up, Pitwall puts the previous extension, the database and the
previous server back by itself and says what failed. A server program it
updated stays updated: Pitwall can't downgrade npm packages, so the card
says what to do. **Go back** asks whether to put the database from before
the update back too: it moves `agention.db` and its `-wal` and `-shm`
files into the backup folder and copies the backup in, as Agention's
`backup --help` describes.

Agents you started before an update keep the plugin they started with
until restarted (`update.agents: "compatible"`). Their row says *old
setup*. Settings → Extensions → "After an update, restart agents that use
the old setup" decides what happens: Ask (default), When idle, or Never.

**Already running a server yourself?** If something already listens on
**Local server listens on** (127.0.0.1:8787 by default), Pitwall says
"running elsewhere" and uses it. It never starts, stops or signals that
server. Keep starting yours as you do, with your own `--data`. Pitwall's
server runs only while Pitwall does. For agents and devices that need the
server while Pitwall is closed, run your own (at login, if you like).

**Too old?** Pitwall checks `agention version` before it starts the
server. Below 0.7.0, the card says so ("Agention's server 0.6.0; this
extension needs 0.7.0 or later") with one button. It installs 0.7.0 after
you confirm, then starts the server again. A server migrates its database
forward when it starts and refuses one a newer release wrote, so going
back to an older server also means putting the backup from before the
update back (**Go back** offers it).

## 2. Install Agention's plugin in your agents (once)

Agents reach Agention through its plugin, which runs the bridge
(`agention-mcp`) as an MCP server, pinned to the release that matches
your server. Either press **Set up Agention in my agents** in Settings →
Extensions (step 3 of its checklist, once the extension is on): it shows the exact
commands below and runs them only when you confirm, skipping a tool you
don't have. Or run them yourself:

```bash
# Claude Code
claude plugin marketplace add WayfarerLabs/agention#v0.7.0
claude plugin install agention@agention

# Codex
codex plugin marketplace add WayfarerLabs/agention --ref v0.7.0
codex plugin add agention@agention
```

The button first removes Agention's marketplace (which uninstalls an
older plugin; a failure there just means it wasn't added), because
Claude Code refuses to add a marketplace again from another tag.
**Remove Agention from my agents** removes the marketplace and plugin
again.

The `v0.7.0` pin lives in `extension.json` (the `setup` action's steps):
when you upgrade your Agention server, update that ref in both `add`
steps to the new release's tag, then press the button again.

You don't need to set anything else for agents Pitwall starts: while the
extension is on, Pitwall gives them, in their environment,

| Variable | Value |
| --- | --- |
| `AGENTION_URL` | the extension's **Server address** setting |
| `AGENTWORKS_AGENT` | the agent's project folder name, in name form (`my-app`) |
| `AGENTWORKS_SESSION` | Pitwall's id for the agent, in name form (stable across restarts) |

The bridge reads exactly these (Agention's operator guide, "Agent bridge").
Claude Code gives the bridge its whole environment; Codex gives MCP
servers only a few common variables plus those a plugin lists, and
Agention's plugin lists these, so they reach the bridge in both. Agents
started outside Pitwall still need `AGENTION_URL` from your shell profile
or Claude Code's settings, as the operator guide says. An agent that
calls the bridge's `register` with its own names uses those instead, and
Pitwall can't match its threads to it.

Settings → Extensions ("What this extension does on your computer")
shows exactly which variables the
extension sets. They take effect for agents started (or restarted) after
you turn it on.

## 3. Create an API key

In Agention's web UI: Settings → API keys → create one with the scopes
**`threads:write`** (read and answer threads) and **`sessions:read`**
(match threads to agents). Copy it.

## 4. Install it in Pitwall and turn it on

Pitwall ships this extension. In Pitwall, open Settings → Extensions and
press **Install** on Agention under Available. Installing runs nothing:
it stays off until you turn it on. When a later Pitwall ships a newer
version, the card offers Update (your settings and API key stay).

(Working from a checkout instead? Copy this folder into the `extensions`
folder of Pitwall's data folder — `~/.pitwall` on macOS,
`$XDG_DATA_HOME/pitwall` on Linux, or `$PITWALL_HOME` when set; Settings
→ Extensions shows the exact folder — named `agention`, then "Look
again".)

Then turn Agention on; its card becomes a checklist:

1. Server address: your server's origin, such as
   `https://agention.example.ts.net`.
2. Local server listens on: `127.0.0.1:8787` unless you change it.
3. Install Agention's server, confirm and start it, then sign in for the
   first time (section 1).
4. API key: create it in Agention's web UI (section 3), copy it, then
   "Paste from clipboard". It goes to your
   system's secure store (Keychain, Credential Manager or Secret
   Service); Pitwall's files only record that it is set. Only the tile
   reads it, and sends it only in the `Authorization` header to your
   server; never to agents, never shown or logged.
5. Set up Agention in my agents (section 2), if you haven't.

Turning it on asks for two permissions: **network** (your server, the
host of Server address; when you change the address, the permission
follows it) and **agents.read** (to tell which of your agents a thread is
from).

## 5. Use it

Press the **Agention** button on the toolbar (its count is the open
questions) or open the Agention tile from the command palette: a tile of
your layout.

- **The list**: threads grouped by agent and session, under the agent's
  name when it is one of Pitwall's agents, with its session's status.
  "Waiting on you" (default) shows threads with open questions or news,
  those that block first; "All" shows the rest too.
- **A thread** (beside the list when the tile is wide, over it when
  narrow): its messages, and each open question with its suggestions (one
  click answers; with several to pick, pick then Send), your own words,
  or Decline. Write to the agent in the box below. **Mark read** and
  **Archive** are on its bar.
- **Elsewhere in Pitwall**: the same open questions in Pitwall's floating
  control, where you can answer them too, and a badge on the sidebar row
  of each agent that waits on you (a click opens the tile).

News arrives at once (the tile keeps a long poll open with your server
while Pitwall runs, even with the tile closed) and everything is read
again every minute.

## When something is wrong

The tile says it at its top, with a button:

- **Set up Agention**: a setting is missing or wrong, the network
  permission isn't granted, or the address doesn't answer like an
  Agention server (or one too old for the operator API). Open Settings.
- **Signed out**: the key is refused, or lacks a scope. Agention answers
  a missing scope as "not found", so the tile checks `threads:write` when
  it connects by sending a request that should be refused for bad input
  (400) rather than not found (404). That depends on the order in which
  the server checks requests and is the most fragile part; a future
  Agention version may change it.
- **Not connected**: the server can't be reached or failed; the tile
  tries again after 1, 2, 5, 10 and 30 seconds, then every minute. When
  it is the server Pitwall runs and it isn't up, **Open its card** shows
  why; its last lines are in the card's Log.
- Threads don't show a badge on an agent: the agent was started before the
  extension was turned on, or it registered with names of its own.

## From the adapter to the tile

Version 2.0.0 replaces the Node.js adapter (1.x) with the tile; Node.js
is no longer needed. Its settings, API key, managed server, actions and
variables are the same. Not yet possible without an adapter, so missing
for now:

- **Pitwall's built-in inbox** (⌘I, "Open Agention inbox", number keys to
  answer, Send with take-back): it is fed only by adapters. A tile can't
  serve it yet (the Tile SDK's `inbox` interface is a separate item of
  the Tile SDK plan). The toolbar button and the tile replace it.
- **The checklist's "Connected" step and the card's status pill**: Pitwall
  draws them from an adapter's status, and a tile has no way to report
  one. The tile shows the connection itself, at its top.
- **The update's health check** is now "the server is up" only (it was
  "and the adapter connects again", from the adapter's status).

## Building and testing

The tile is its own Cargo workspace, not part of Pitwall's:

```bash
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/agention_tile.wasm tile.wasm
cargo test
```

`cargo test` runs the unit tests of the operator API and model code
(`src/api.rs`, `src/model.rs`), and the tests in `tests/`, which run
`tile.wasm` in Pitwall's own tile runtime against a fake operator API
(`tests/fake/`, in memory). Pitwall's own tests open the tile in the app
against a fake server too (`test/app.json`). None uses a real server or
key. `scripts/check-deps.sh` checks it uses no Pitwall crate but the
public SDK; `pitwall extension check .` checks the folder.

## Releasing a new version of this extension

Pitwall updates this extension from this repository (`source` in
`extension.json`), without waiting for a Pitwall release. To release one:

1. Change what it needs (a tile fix, with `tile.wasm` built again as
   above; the pinned Agention version in the setup action and the server
   action), and raise `version` in `extension.json` and `Cargo.toml` (say
   to `2.1.0`). Commit it to `main`.
2. Tag that commit `agention-v<version>`, the same version as in
   `extension.json`, and push the tag:
   `git tag agention-v2.1.0 && git push origin agention-v2.1.0`.

A tag is never moved once pushed: Pitwall records the commit a tag
pointed at and refuses a tag that moved. Pitwall finds the new tag when it
checks (at start, once a day, or with "Check for updates" on the card),
shows what changes on the computer, and installs it only when the user
presses Update. A changed setup action asks the user to run it again.
