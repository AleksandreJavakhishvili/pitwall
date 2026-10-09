---
name: pitwall
description: "Add agents and sessions to Pitwall, the desktop app that runs and watches coding agents, and read or change its settings. Use only when the user asks to add an agent, terminal or an existing agw session to Pitwall, to see what Pitwall tracks, or to change a Pitwall setting (theme, look, density, font, hooks, …). Requires the `pitwall` CLI; most commands need a running Pitwall app."
---

# Pitwall

Pitwall is a desktop app that runs coding agents (Claude Code, Codex, …) and
terminals in panes, and tracks agw sessions on VMs. The `pitwall` CLI talks to
the running app over a local socket. It can **list** what Pitwall knows, **add**
things to it and **change its settings**; nothing else (no stopping, removing or
prompting agents).

Check that you can reach it:

```bash
pitwall agent list            # fails with "not_connected" if Pitwall isn't running
```

Inside a Pitwall pane, `PITWALL_ENV=1`, `PITWALL_AGENT_ID` (your own agent id)
and `PITWALL_CLI_SOCKET` are set, and `pitwall` uses that socket. Outside
Pitwall it uses `~/.pitwall/run/pitwalld.sock` (Linux:
`~/.local/share/pitwall/run/pitwalld.sock`; Windows: the named pipe for
`%APPDATA%\Pitwall\run\pitwalld.sock`). Use `--socket <path>` only if
told to.

## Output

JSON on stdout by default; read ids and state from it instead of guessing.
Add `--human` for text meant for the user. Errors are JSON on stderr:
`{"error":{"code":"…","message":"…"}}`. Exit status: 0 ok, 1 error,
2 bad arguments, 3 the user denied (or didn't answer) an approval.

## Discover (read-only, never asks the user)

```bash
pitwall agent list                                   # Pitwall's agents: id, name, kind, status, running, machine…
pitwall machine list                                 # providers and machines; canCreate / canAddSessions
pitwall session list [--provider agw] [--machine <vm>]   # sessions that can be added; inPitwall, status
```

Sessions are identified by `provider` + `machine` + `native` (the session's
name there) — take them from `session list`, not from memory.

## Add

```bash
pitwall session add agw <vm> <session>            # track an existing session
pitwall session add agw <vm> <session> --start    # …and start it if it's stopped
pitwall agent new --kind claude --project ~/code/api [--name api] [--resume <session-id>]
pitwall machine form <vm>                         # what a new session there can be (read-only)
pitwall agent new --machine <vm> --name <session> --workspace <ws> [--template <t>] [--as admin|agent:<name>|new-agent[:<name>]]
pitwall agent new --machine <vm> --name <session> --new-workspace [<name>] [--workspace-template <t>] …
```

- `session add` only starts tracking: nothing changes on the VM, no approval.
  A running session is attached; a stopped one shows as stopped. Adding the
  same session twice returns the same agent (`already: true`).
- `--start` starts a stopped session on its machine (e.g. `agw session start`).
  That changes another machine, so Pitwall shows the user an approval dialog
  and the command **waits** for the answer (up to ~2 minutes). Allowed →
  `started: true`. Denied or unanswered → exit 3 with code `denied` or
  `approval_timeout`; the session is still added, still stopped. Don't retry a
  denial unless the user asks.
- `agent new` starts a new agent or terminal on this Mac (kind ids:
  `claude`, `codex`, `shell`, … — see Pitwall's New-agent list). `--project`
  defaults to the current folder. `--resume` continues a conversation.
- `agent new --machine <vm>` creates and starts a session on an agw VM
  (`agw session create`), then attaches it in Pitwall. Look at `machine form
  <vm>` first for its workspaces, agent users and templates (`--option
  FIELD=VALUE` sets any field). Names: lowercase letters, digits, `-`, `_`,
  no `--`, max 34. It changes the VM, so it **waits for the user's approval**
  (asked every time when it also makes a new workspace or agent user); denied
  → exit 3, nothing created. Removing it from Pitwall later leaves the session
  on the VM (deleting it there is not offered).

## Settings

Every setting has a dotted key. Discover them first; never guess keys or values:

```bash
pitwall settings list                       # key, value, allowed values (choices/min/max), default,
                                            # description, page, sensitivity ("safe" or "approval")
pitwall settings get appearance.theme
pitwall settings set appearance.theme dark  # switches: true/false (also on/off); choices; numbers
pitwall settings set appearance.terminalFontSize 15
pitwall settings reset appearance.theme     # back to the default
```

`get`/`set`/`reset` print the setting as JSON (`value` is what is now in
effect). Keys today: `general.showElsewhere`, `general.cliTool`,
`agents.codexHooks`, `rules.allowNpx`, `appearance.theme`, `appearance.look`,
`appearance.reduceMotion`, `appearance.density`, `appearance.terminalFontSize`
— but trust `settings list`, not this line.

- With Pitwall running, a change applies at once in every window.
- `"sensitivity": "safe"` settings (Pitwall's own look) apply directly.
- `"sensitivity": "approval"` settings change something outside Pitwall
  (installing Codex hooks or the `pitwall` link, letting rulesync run through
  npx): Pitwall shows the user an approval dialog naming you, and the command
  **waits**. Denied or unanswered → exit 3 (`denied` / `approval_timeout`),
  nothing changed. Setting a value it already has asks nothing.
- `onlyOn: true` settings can only be turned on (installing); turning them off
  is refused (`bad_params`).
- An unknown key → `not_found`; a value that isn't allowed → `bad_params`
  with the allowed values in the message (exit 1).
- Without Pitwall running, `safe` settings are written to `ui.json` in
  Pitwall's data folder (Pitwall reads them when it starts); `approval` ones
  fail with `not_running`, and `settings list` shows them as `null`.

## Approvals

Approvals come from the user in Pitwall's own window, never from you. Pitwall
identifies the caller from the connecting process (the agent pane it runs in),
not from anything you pass, and shows that name in the dialog. You cannot
approve your own request; don't try to work around a denial.
