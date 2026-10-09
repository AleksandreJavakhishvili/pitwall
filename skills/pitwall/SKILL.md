---
name: pitwall
description: "Set up and manage Pitwall, the desktop app that runs and watches coding agents: list, add, stop, restart, remove and rename agents; queue prompts; arrange spaces and windows; manage projects and rules; read what agents changed; wait for an agent to finish; change settings; diagnose an agent's status. Use when the user asks you to do any of that in Pitwall, or to coordinate other agents running in it. Requires the `pitwall` CLI and a running Pitwall app."
---

# Pitwall

Pitwall is a desktop app that runs coding agents (Claude Code, Codex, …) and
terminals in panes, grouped into spaces and windows, and tracks agw sessions
on VMs. The `pitwall` CLI talks to the running app over a local socket and
does what its UI does. Risky actions wait for the user's OK in Pitwall.

```bash
pitwall agent list        # fails with "not_connected" if Pitwall isn't running
```

Inside a Pitwall pane, `PITWALL_AGENT_ID` (your own id) and
`PITWALL_CLI_SOCKET` are set, and `pitwall` uses that socket. Outside it uses
Pitwall's data folder (`~/.pitwall/run/pitwalld.sock`). Use `--socket <path>`
only if told to.

## Output, errors, exit codes

- JSON on stdout by default: read ids and state from it, don't guess.
  `--human` prints text for the user.
- Errors are JSON on stderr: `{"error":{"code":"…","message":"…"}}`.
- Exit status: **0** ok, **1** error (`not_found`, `not_running`,
  `unsupported`, `conflict`, `timeout`, `not_connected`, …), **2** bad
  arguments (`bad_args`, `bad_params`), **3** the user denied or didn't
  answer an approval (`denied`, `approval_timeout`).
- Agents and spaces can be named by id or by name (case-insensitive). Two
  with the same name → `conflict`; use the id.
- `unsupported` with "update Pitwall": this Pitwall is older than the CLI.

## Approvals

Approval prompts come from Pitwall itself ("Asked by <your agent name>"),
never from you. Pitwall identifies you from your process, not from anything
you pass. The command waits (up to ~2 minutes). Denied or unanswered → exit
3, nothing changed. Don't retry a denial unless the user asks, and don't try
to work around it. Read-only commands never ask.

Asks the user: `agent stop`, `agent restart`, `agent remove` (also with
`--worktree`), `rules apply --main-checkout`, `session add --start`,
`agent new --machine` (creates on an agw VM), and `settings set|reset` of an
`approval` setting. Everything else applies directly.

## Discover (read-only)

```bash
pitwall agent list                         # id, name, kind, status, running, machine, queue, caps…
pitwall agent status api                   # why it shows its status (see Diagnose)
pitwall machine list                       # providers and machines; canCreate / canAddSessions
pitwall machine form <vm>                  # what a new session on an agw VM can be
pitwall session list [--provider agw] [--machine <vm>]   # sessions that can be added
pitwall space list                         # spaces: window, agents shown, members
pitwall project list
pitwall queue list [<agent>]
pitwall rules list                         # rule files, each agent's rules state
pitwall rules sets                         # rule sets and project defaults
pitwall review changes <agent>             # files +/- since the agent started
pitwall settings list
```

`status` values: `working`, `blocked` (needs the user), `idle`, `done`
(finished a turn nobody looked at yet), `unknown`, `exited`, `stopped`.

## Add agents and sessions

```bash
pitwall agent new --kind claude --project ~/code/api [--name api] [--resume <session-id>]
pitwall agent new --kind shell --project .            # a terminal
pitwall session add agw <vm> <session>                # track an existing session (no approval)
pitwall session add agw <vm> <session> --start        # …and start it if stopped (asks)
pitwall agent new --machine <vm> --name <session> --workspace <ws> [--template <t>] [--as admin|agent:<name>|new-agent[:<name>]]
pitwall agent new --machine <vm> --name <session> --new-workspace [<name>] [--workspace-template <t>]
```

- Kind ids: `claude`, `codex`, `shell`, … (Pitwall's New-agent list).
  `--project` defaults to the current folder.
- `--machine` creates a session on an agw VM (asks the user; asked every time
  when it also creates a workspace or agent user). Read `machine form <vm>`
  first; `--option FIELD=VALUE` sets any field. Names: lowercase letters,
  digits, `-`, `_`, max 34. Removing it from Pitwall later leaves it on the VM.

## Manage agents

```bash
pitwall agent rename a1b2 api
pitwall agent stop api                     # asks; returns once it has stopped
pitwall agent restart api                  # asks; continues its conversation when it can
pitwall agent remove api                   # asks; an adopted agw session keeps running there
pitwall agent remove api --worktree        # asks; also deletes its own worktree (caps.removeWorktree)
```

Stopping or restarting an agent on another machine (agw) is asked every
time; on this Mac the user may "remember" it for you.

## Queue prompts

```bash
pitwall queue add api "run the tests and fix failures"
pitwall queue add api -  < prompt.md       # the prompt from stdin
pitwall queue add --status idle "git pull and rebuild"   # every agent that is idle now
pitwall queue list api                     # numbered 1, 2, …
pitwall queue remove api 2
pitwall queue send api [<n>]               # send the first (or nth) queued prompt now
```

Queued prompts go out on their own when the agent is free (if it
auto-sends); `queue send` sends now (the agent must be running).

## Wait (coordinate agents)

```bash
pitwall wait reviewer --for idle,done --timeout 600
pitwall wait reviewer --for blocked,done --fresh     # after sending it a prompt
```

`--fresh` ignores the status it has now and waits for a change first (use it
right after `queue send`, or an earlier `done` ends the wait at once). An
agent that stops while you wait for something else → `not_running`;
`--for stopped` / `exited` match any agent that isn't running. Timeout →
exit 1, code `timeout`.

Example: spawn a reviewer, prompt it, wait, read its changes:

```bash
id=$(pitwall agent new --kind claude --project . --name reviewer | jq -r .id)
pitwall wait "$id" --for idle --timeout 120
pitwall queue add "$id" "Review the diff on this branch; list problems only."
pitwall queue send "$id"
pitwall wait "$id" --for done,blocked --fresh --timeout 1800
pitwall review changes "$id"
```

## Spaces and windows

```bash
pitwall space create "API"                 # a new space in the main window
pitwall agent move api --space API         # show it there (as dragging it onto the tab)
pitwall space rename API "API work"
pitwall space move "API work" --to-window new     # its own window (e.g. for a second monitor)
pitwall space move "API work" --to-window main    # or an open window's label: pitwall-2, …
```

The "All" space can't be renamed and stays in the main window. Changes show
at once in every window.

## Projects

```bash
pitwall project add ~/code/api             # Pitwall's project list (sidebar)
pitwall project remove ~/code/api          # off the list; never deletes files
```

## Rules (rulesync)

```bash
pitwall rules sets                                   # ids and names
pitwall rules default ~/code/api strict              # a project's default set (`none` clears)
pitwall rules apply api [--set strict|none]          # write the agent's rule files now
pitwall rules apply api --main-checkout              # it works in the main checkout: asks first
```

Rule files take effect in the agent's next session (restart it). Without
`--main-checkout`, an agent in the main checkout gives `conflict`. Making
rule sets from an existing CLAUDE.md is done in Settings → Rules (import);
ask the user to do that, then use the set here. `rules list` shows whether
rulesync is available (`settings set rules.allowNpx true` allows npx; asks).

## Review (read-only)

```bash
pitwall review changes api                 # since the agent started: files, +/-
pitwall review changes api --task <id>     # one task
```

Committing, merging and discarding stay in Pitwall's Review screen (the user
clicks).

## Diagnose

```bash
pitwall agent status api
```

Shows the status, which signal decided it (`hooks` > `screen` rules >
output `activity`), what each signal said last, and `explanation` lines in
plain language, e.g. why an agent is `unknown`: no hooks for its kind, Codex
hooks not installed (`settings set agents.codexHooks true`, asks), no screen
rules for its kind, or no output. An agent started before hooks were
installed needs a restart.

## Settings

```bash
pitwall settings list                      # key, value, allowed values, default, sensitivity
pitwall settings get appearance.theme
pitwall settings set appearance.theme dark # switches: true/false (on/off); choices; numbers
pitwall settings reset appearance.theme
```

Never guess keys or values: take them from `settings list`. `safe`
settings apply at once in every window; `approval` ones (Codex hooks, the CLI
link, npx for rulesync) ask the user. `onlyOn: true` settings can't be turned
off (`bad_params`). Without Pitwall running, `safe` settings are written to
`ui.json` and read at start; `approval` ones fail with `not_running`.

## agw (VMs, templates, sessions)

Pitwall only adds and creates sessions on agw VMs (above). For agw itself
(VMs, templates, plugins, workspaces), use agw's own agent-mode guides:

```bash
agw guide show <topic> --agent             # topics: agw guide --help
```

Changes made with `agw` directly don't go through Pitwall's approvals: ask
the user before creating or deleting VMs, workspaces or agents there.
