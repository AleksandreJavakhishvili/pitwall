# Agention (example Pitwall extension)

[Agention](https://github.com/WayfarerLabs/agention) (MIT, by Wayfarer
Labs, not part of Pitwall) is a self-hosted "attention inbox": your agents
post questions into threads through an MCP bridge, and you answer them
from Agention's web UI, served by your own Agention server.

This folder is an example of a Pitwall extension (see
[docs/spec/extensions.md](../../docs/spec/extensions.md)): only a manifest,
`extension.json`, and this README. Pitwall runs nothing from it.

**Status:** draft. Turned on, the extension gets a section of its own in
Pitwall's sidebar, with nothing in it yet: what an extension shows there is
still being designed. Until then, use Agention in your browser.

## 1. Run an Agention server

Agention's own [operator guide](https://github.com/WayfarerLabs/agention/blob/main/docs/operator-guide.md)
is the reference; check it for the current version. In short:

- It needs Node.js 24 or later.
- It is served at an `https` origin with a domain name, not an IP address
  (browsers refuse passkeys for an IP). With Tailscale, `tailscale serve`
  gives you one, such as `https://agention.example.ts.net`.

```bash
npm install -g @wayfarerlabs/agention
tailscale serve --bg http://127.0.0.1:8080
agention serve --data ~/.local/share/agention --origin https://agention.example.ts.net
```

You sign in with a passkey. The first one is registered from a one-time
link made on the server's machine
(`agention passkey-link --data … --origin …`); see the operator guide.

## 2. Give your agents Agention's plugin

Agents reach Agention through its own Claude Code / Codex plugin, which you
install in the agent yourself (Pitwall doesn't install it and doesn't
change how agents behave). From the operator guide:

```bash
# Claude Code
claude plugin marketplace add WayfarerLabs/agention
claude plugin install agention@agention

# Codex
codex plugin marketplace add WayfarerLabs/agention
codex plugin add agention@agention
```

Point the bridge at your server, for example in the environment the agents
start in:

```bash
export AGENTION_URL=https://agention.example.ts.net
```

## 3. Put this folder into Pitwall

Copy the folder into the `extensions` folder of Pitwall's data folder
(`~/.pitwall` on macOS, `$XDG_DATA_HOME/pitwall` on Linux, or
`$PITWALL_HOME` when set; Settings → General → Extensions shows the
exact folder). The folder's name must be the extension's id,
`agention`:

```bash
mkdir -p ~/.pitwall/extensions
cp -R extensions/agention ~/.pitwall/extensions/
```

Then, in Pitwall: Settings → General → Extensions → "Look again", and turn
Agention on. It is off until you do.
