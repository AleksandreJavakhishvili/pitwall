# Agention (example Pitwall extension)

[Agention](https://github.com/WayfarerLabs/agention) (MIT, by Wayfarer
Labs, not part of Pitwall) is a self-hosted "attention inbox": your agents
post questions into threads through an MCP bridge, and you answer them.
With this extension you answer them in Pitwall, in an inbox tile, next to
the agents that asked.

This folder is a Pitwall extension (see
[docs/spec/extensions.md](../../docs/spec/extensions.md)): a manifest,
`extension.json`, and an adapter, `adapter/` (Node.js), which Pitwall runs
while the extension is on. The adapter talks to your Agention server's
operator API and to Pitwall over stdin and stdout; it never touches your
agents.

## What you need

- An Agention server, **v0.6.0 or later**, served at an `https` origin.
  Pitwall can install it and keep it running on this computer
  (section 1), or you run your own.
- Node.js 24 or later on the machine running Pitwall (the adapter runs with
  `node` from your login shell's PATH).
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
   `npm install -g @wayfarerlabs/agention@0.6.0` after you confirm.
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
starts it and checks that it is up and that the inbox connects again.
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
server. Below 0.6.0, the card says so ("Agention's server 0.5.0; this
extension needs 0.6.0 or later") with one button. It installs 0.6.0 after
you confirm, then starts the server again.

## 2. Install Agention's plugin in your agents (once)

Agents reach Agention through its plugin, which runs the bridge
(`agention-mcp`) as an MCP server, pinned to the release that matches
your server. Either press **Set up Agention in my agents** in Settings →
Extensions (step 3 of its checklist, once the extension is on): it shows the exact
commands below and runs them only when you confirm, skipping a tool you
don't have. Or run them yourself:

```bash
# Claude Code
claude plugin marketplace add WayfarerLabs/agention#v0.6.0
claude plugin install agention@agention

# Codex
codex plugin marketplace add WayfarerLabs/agention --ref v0.6.0
codex plugin add agention@agention
```

The button first removes Agention's marketplace (which uninstalls an
older plugin; a failure there just means it wasn't added), because
Claude Code refuses to add a marketplace again from another tag.
**Remove Agention from my agents** removes the marketplace and plugin
again.

The `v0.6.0` pin lives in `extension.json` (the `setup` action's steps):
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
   Service); Pitwall's files only record that it is set. It is given to
   the adapter when it starts, never to agents or the server.
5. Set up Agention in my agents (section 2), if you haven't.
6. Connected: ticked once the adapter reaches your server; otherwise
   its words on why (signed out, unreachable, or why it stopped, with
   Try again).

The status pill on the card's top row says the same at a glance. Open the inbox with ⌘I (Ctrl+Shift+I on
Windows and Linux) or "Open Agention inbox" in the command palette: a
tile of your layout, with your agents' questions first. An agent with an
open question also gets a small badge on its sidebar row; a click opens
the inbox there.

## When something is wrong

- **Signed out**: the key is missing, revoked, or lacks a scope. Agention
  answers a missing scope as "not found", so the adapter checks the scopes
  when it connects by sending requests that should be refused for bad
  input (400) rather than not found (404). That depends on the order in
  which the server checks requests and is the most fragile part; a future
  Agention version may change it.
- **Stopped: “node” isn't installed**: install Node.js 24+, then Restart.
- **Its server stopped**: the card shows its last lines; the Log has the
  rest. Fix what they say, then press Start.
- Threads don't show a badge on an agent: the agent was started before the
  extension was turned on, or it registered with names of its own.

The adapter's own tests: `cd adapter && node --test test/*.test.js` (they use a fake
server, never a real one).

## Releasing a new version of this extension

Pitwall updates this extension from this repository (`source` in
`extension.json`), without waiting for a Pitwall release. To release one:

1. Change what it needs (an adapter fix, the pinned Agention version in
   the setup action and the server action), and raise `version` in
   `extension.json` (say to `1.4.0`). Commit it to `main`.
2. Tag that commit `agention-v<version>`, the same version as in
   `extension.json`, and push the tag:
   `git tag agention-v1.4.0 && git push origin agention-v1.4.0`.

A tag is never moved once pushed: Pitwall records the commit a tag
pointed at and refuses a tag that moved. Pitwall finds the new tag when it
checks (at start, once a day, or with "Check for updates" on the card),
shows what changes on the computer, and installs it only when the user
presses Update. A changed setup action asks the user to run it again.
