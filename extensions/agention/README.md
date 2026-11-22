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

- An Agention server, **v0.4.0 or later** (the operator API), served at an
  `https` origin.
- Node.js 24 or later on the machine running Pitwall (the adapter runs with
  `node` from your login shell's PATH).
- Agention's plugin in Claude Code and/or Codex (step 2), installed once by
  you.

## 1. Run (or upgrade) an Agention server

Agention's own [operator guide](https://github.com/WayfarerLabs/agention/blob/main/docs/operator-guide.md)
is the reference. In short:

- It is served at an `https` origin with a domain name, not an IP address.
  With Tailscale, `tailscale serve` gives you one, such as
  `https://agention.example.ts.net`.

```bash
npm install -g @wayfarerlabs/agention@0.4.0
tailscale serve --bg http://127.0.0.1:8080
agention serve --data ~/.local/share/agention --origin https://agention.example.ts.net
```

To upgrade an older server, install `@wayfarerlabs/agention@0.4.0` the same
way and start it again with the same `--data` and `--origin`.

## 2. Install Agention's plugin in your agents (once)

Agents reach Agention through its plugin, which runs the bridge
(`agention-mcp`) as an MCP server, pinned to the release that matches
your server. Either press **Set up Agention in my agents** in Settings →
Extensions (step 3 of its checklist, once the extension is on): it shows the exact
commands below and runs them only when you confirm, skipping a tool you
don't have. Or run them yourself:

```bash
# Claude Code
claude plugin marketplace add WayfarerLabs/agention#v0.4.0
claude plugin install agention@agention

# Codex
codex plugin marketplace add WayfarerLabs/agention --ref v0.4.0
codex plugin add agention@agention
```

The button first removes Agention's marketplace (which uninstalls an
older plugin; a failure there just means it wasn't added), because
Claude Code refuses to add a marketplace again from another tag.
**Remove Agention from my agents** removes the marketplace and plugin
again.

The `v0.4.0` pin lives in `extension.json` (the `setup` action's steps):
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
2. API key: copy the key, then "Paste from clipboard". It goes to your
   system's secure store (Keychain, Credential Manager or Secret
   Service); Pitwall's files only record that it is set. It is given to
   the adapter when it starts, never to agents.
3. Set up Agention in my agents (section 2), if you haven't.
4. Connected: ticked once the adapter reaches your server; otherwise
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
- Threads don't show a badge on an agent: the agent was started before the
  extension was turned on, or it registered with names of its own.

The adapter's own tests: `cd adapter && node --test test/*.test.js` (they use a fake
server, never a real one).
