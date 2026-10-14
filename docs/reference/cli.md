# CLI reference

`pitwall` sets up and manages Pitwall from a terminal: agents, sessions, queued prompts, spaces, projects, rules, review and settings. It does what the UI does, so you can script Pitwall or let an agent drive it. This page is generated from the CLI's own definitions; `pitwall <command> --help` shows the same.

## Install

Settings → *Install command-line tool* links `pitwall` into `~/.local/bin` (or `/usr/local/bin`). Inside a Pitwall terminal it finds the app on its own.

## How it works

- **It talks to the running app** over a local socket: `$PITWALL_CLI_SOCKET` when set, else `~/.pitwall/run/pitwalld.sock` (`--socket` overrides both). Only `settings` also works while Pitwall is closed: it edits `~/.pitwall/ui.json` (safe settings only).
- **Output is JSON** on stdout, the call's result, so scripts and agents can parse it (`jq` works well). `--human` (`-H`) prints readable text instead.
- **Errors** are `{"error":{"code":"…","message":"…"}}` on stderr.
- **Agents and spaces** are named by id or by name.
- **Risky requests wait for you.** Stopping, restarting or removing an agent, anything that changes another machine (starting or creating a session on an agw VM), writing rules into a main checkout, importing rules from a file or a git repository, deleting a rule set, and settings that change something outside Pitwall open an approval dialog in Pitwall when anyone but you asks; the command waits for your answer. Pitwall identifies the caller from the connecting process, so an agent can't approve its own request.

## Exit status

| Status | Meaning |
|---|---|
| `0` | Done. |
| `1` | An error (see the JSON on stderr), including `wait` timing out (code `timeout`). |
| `2` | Bad arguments: unknown command or option, a missing value, or a value the server rejects (`bad_params`). |
| `3` | Not approved: you denied the request in Pitwall, or didn't answer. |

## Recipes

Start an agent in a project and give it work:

```bash
id=$(pitwall agent new --kind claude --project ~/code/api --name api | jq -r .id)
pitwall queue add "$id" "run the tests and fix what fails"
pitwall wait "$id" --for done,blocked --fresh
pitwall review changes "$id" --human
```

Run it on another login (Settings → Agents → Accounts):

```bash
pitwall agent new --kind codex --project ~/code/api --account work
```

Send the same prompt to every idle agent, then see who needs you:

```bash
pitwall queue add --status idle "pull main and rebase"
pitwall agent list --human
```

Arrange a space in its own window:

```bash
pitwall space create Review
pitwall agent move api --space Review
pitwall space move Review --to-window new
```

Change a setting, or read them all:

```bash
pitwall settings set appearance.theme dark
pitwall settings list --human
```

## Global options

These work with every command.

| Option | |
|---|---|
| `--socket <PATH>` | Pitwall's socket (default: $PITWALL_CLI_SOCKET, else ~/.pitwall/run/pitwalld.sock). |
| `-H, --human` | Readable text instead of JSON. |
| `--json` | JSON output (the default; for scripts and agents that want to say so). |


## Commands

- [`agent`](#agent): Pitwall's agents and terminals
- [`machine`](#machine): Where agents can run (this Mac, agw VMs, …)
- [`session`](#session): Sessions on other machines (agw) that can be added to Pitwall
- [`settings`](#settings): Pitwall's settings (theme, look, density, hooks, …): list, get, set, reset. Applied live when Pitwall runs; otherwise `ui.json` is edited (safe settings only)
- [`queue`](#queue): Prompts queued for agents (sent when they are free)
- [`space`](#space): Spaces (tabs of agents) and the windows they are in
- [`project`](#project): Pitwall's project list (folders in the sidebar)
- [`rules`](#rules): Rule sets (rulesync) for agents and projects
- [`review`](#review): What agents changed (read-only)
- [`wait`](#wait): Wait until an agent is idle, blocked or done (for coordinating agents)

## agent

Pitwall's agents and terminals.

### agent list

List Pitwall's agents.

```bash
pitwall agent list [OPTIONS]
```

### agent new

Start a new agent (or terminal) on this Mac, or create a session on another machine (--machine, an agw VM), and add it to Pitwall.

```bash
pitwall agent new [OPTIONS]
```

| Option | |
|---|---|
| `--kind <KIND>` | Agent kind: claude, codex, shell, … (Pitwall's New-agent list). Not used with --machine: the session template decides what runs there. |
| `--project <PATH>` | Folder it works in (default: the current folder). This Mac only. |
| `--name <NAME>` | Name shown in Pitwall (default: the kind's name). With --machine: the session's name there (required). |
| `--resume <SESSION_ID>` | Continue this conversation (session id) instead of starting fresh. |
| `--account <ACCOUNT>` | The login it runs with: an account's name or id (Settings → Agents → Accounts), or `default` for your own (default: the project's). |
| `--machine <MACHINE>` | Create it on this machine (an agw VM; see `machine list` and `machine form <machine>`). It changes that machine, so it waits for your approval in Pitwall. |
| `--provider <PROVIDER>` | The provider of --machine, when two have a machine by that name. |
| `--workspace <NAME>` | --machine: an existing workspace there. |
| `--new-workspace [NAME]` | --machine: a new workspace (named NAME; default: the session's name). |
| `--workspace-template <TEMPLATE>` | --machine: the new workspace's template. |
| `--as <WHO>` | --machine: who it runs as: `admin`, `agent:<name>` (an existing agent user) or `new-agent[:<name>]` (a new one). |
| `--agent-template <TEMPLATE>` | --machine with `--as new-agent`: the new agent user's template. |
| `--template <TEMPLATE>` | --machine: the session template (what runs in it). |
| `--option <FIELD=VALUE>` | --machine: any field of the machine's form, as FIELD=VALUE (`machine form <machine>` lists them). Repeatable. |

```bash
pitwall agent new --kind claude --project ~/code/api --name api
pitwall agent new --kind claude --resume 7f3c2a90-…      # continue a conversation
pitwall agent new --kind shell                           # a terminal in this folder
pitwall agent new --machine my-vm --name fix --workspace work --as admin --template claude
```

### agent stop

Stop an agent (asks you in Pitwall).

```bash
pitwall agent stop [OPTIONS] <AGENT>
```

| Argument | |
|---|---|
| `AGENT` | The agent (id or name). |

### agent restart

Start an agent again (asks you in Pitwall).

```bash
pitwall agent restart [OPTIONS] <AGENT>
```

| Argument | |
|---|---|
| `AGENT` | The agent (id or name). |

### agent remove

Remove an agent from Pitwall (asks you in Pitwall).

```bash
pitwall agent remove [OPTIONS] <AGENT>
```

| Argument | |
|---|---|
| `AGENT` | The agent (id or name). |

| Option | |
|---|---|
| `--worktree` | Also delete its own worktree. |

### agent rename

Rename an agent.

```bash
pitwall agent rename [OPTIONS] <AGENT> <NAME>
```

| Argument | |
|---|---|
| `AGENT` | The agent (id or name). |
| `NAME` | The new name. |

### agent status

Why an agent has its status (hooks, screen rules, activity).

```bash
pitwall agent status [OPTIONS] <AGENT>
```

| Argument | |
|---|---|
| `AGENT` | The agent (id or name). |

```bash
pitwall agent status api --human
```

### agent move

Show an agent in a space (as dragging it onto the space's tab).

```bash
pitwall agent move [OPTIONS] --space <SPACE> <AGENT>
```

| Argument | |
|---|---|
| `AGENT` | The agent (id or name). |

| Option | |
|---|---|
| `--space <SPACE>` | The space (id or name). Required. |

```bash
pitwall agent move api --space Review
```


## machine

Where agents can run (this Mac, agw VMs, …).

### machine list

List providers and their machines.

```bash
pitwall machine list [OPTIONS]
```

### machine form

What `agent new --machine` takes there: the machine's form, with the choices it has now (read-only).

```bash
pitwall machine form [OPTIONS] <MACHINE>
```

| Argument | |
|---|---|
| `MACHINE` | The machine (e.g. an agw VM name). |

| Option | |
|---|---|
| `--provider <PROVIDER>` | Its provider, when two have a machine by that name. |

```bash
pitwall machine form my-vm --human
```


## session

Sessions on other machines (agw) that can be added to Pitwall.

### session list

List sessions that can be added, and whether they already are.

```bash
pitwall session list [OPTIONS]
```

| Option | |
|---|---|
| `--provider <PROVIDER>` | Only this provider (e.g. agw). |
| `--machine <MACHINE>` | Only this machine (e.g. an agw VM name). |

### session add

Add an existing session to Pitwall (e.g. `session add agw my-vm work`). Adding only tracks it; nothing changes on that machine.

```bash
pitwall session add [OPTIONS] <PROVIDER> <MACHINE> <NAME>
```

| Argument | |
|---|---|
| `PROVIDER` | Where it runs (e.g. agw; see `machine list`). |
| `MACHINE` | The machine there (e.g. an agw VM name). |
| `NAME` | The session's name there. |

| Option | |
|---|---|
| `--start` | Also start it if it's stopped. This changes that machine, so it waits for your approval in Pitwall. |

```bash
pitwall session add agw my-vm work           # track it
pitwall session add agw my-vm work --start   # and start it (asks)
```


## settings

Pitwall's settings (theme, look, density, hooks, …): list, get, set, reset. Applied live when Pitwall runs; otherwise `ui.json` is edited (safe settings only).

### settings list

Every setting: key, current value, allowed values, default, whether it needs approval, and what it does.

```bash
pitwall settings list [OPTIONS]
```

### settings get

One setting (e.g. `settings get appearance.theme`).

```bash
pitwall settings get [OPTIONS] <KEY>
```

| Argument | |
|---|---|
| `KEY` | A key from `settings list` (e.g. appearance.theme). |

### settings set

Change a setting (e.g. `settings set appearance.theme dark`). Settings marked `approval` wait for your OK in Pitwall.

```bash
pitwall settings set [OPTIONS] <KEY> <VALUE>
```

| Argument | |
|---|---|
| `KEY` | A key from `settings list`. |
| `VALUE` | The new value: true/false for switches, one of the listed choices, or a number. |

```bash
pitwall settings set appearance.theme dark
pitwall settings set appearance.density dense
```

### settings reset

Put a setting back to its default.

```bash
pitwall settings reset [OPTIONS] <KEY>
```

| Argument | |
|---|---|
| `KEY` | A key from `settings list`. |


## queue

Prompts queued for agents (sent when they are free).

### queue add

Queue a prompt for an agent (`queue add api "run the tests"`), or for every agent with a status (`queue add --status idle "run the tests"`). `-` reads the prompt from stdin.

```bash
pitwall queue add [OPTIONS] <AGENT TEXT>...
```

| Argument | |
|---|---|
| `AGENT TEXT` | [AGENT] TEXT: the agent (id or name; not with --status) and the prompt. |

| Option | |
|---|---|
| `--status <STATUS>` | Queue it for every agent that has this status now (idle, done, …). |

```bash
pitwall queue add api "run the tests"
echo "summarize the diff" | pitwall queue add api -
pitwall queue add --status idle "pull main"
```

### queue list

What is queued: one agent's, or every agent with a queue.

```bash
pitwall queue list [OPTIONS] [AGENT]
```

| Argument | |
|---|---|
| `AGENT` (optional) | An agent (id or name). |

### queue remove

Take the Nth prompt (1 = first) out of an agent's queue.

```bash
pitwall queue remove [OPTIONS] <AGENT> <N>
```

| Argument | |
|---|---|
| `AGENT` | The agent (id or name). |
| `N` | Its place in `queue list` (1 = first). |

### queue send

Send the Nth queued prompt (default: the first) to the agent now.

```bash
pitwall queue send [OPTIONS] <AGENT> [N]
```

| Argument | |
|---|---|
| `AGENT` | The agent (id or name). |
| `N` (optional) | Its place in `queue list` (default: 1, the first). |


## space

Spaces (tabs of agents) and the windows they are in.

### space list

Every space: its window, the agents shown in it and its members.

```bash
pitwall space list [OPTIONS]
```

### space create

A new space (in the main window).

```bash
pitwall space create [OPTIONS] <NAME>
```

| Argument | |
|---|---|
| `NAME` | The space's name. |

### space rename

Rename a space (id or name).

```bash
pitwall space rename [OPTIONS] <SPACE> <NAME>
```

| Argument | |
|---|---|
| `SPACE` | The space (id or name). |
| `NAME` | The new name. |

### space move

Move a space into another window: `new`, `main` or a window's label.

```bash
pitwall space move [OPTIONS] --to-window <new|main|LABEL> <SPACE>
```

| Argument | |
|---|---|
| `SPACE` | The space (id or name). |

| Option | |
|---|---|
| `--to-window <new\|main\|LABEL>` | Required. |

```bash
pitwall space move Review --to-window new
```


## project

Pitwall's project list (folders in the sidebar).

### project list

Pitwall's projects.

```bash
pitwall project list [OPTIONS]
```

### project add

Add a folder to Pitwall's projects.

```bash
pitwall project add [OPTIONS] <PATH>
```

| Argument | |
|---|---|
| `PATH` | The folder. |

### project remove

Take a folder off the list (never deletes anything).

```bash
pitwall project remove [OPTIONS] <PATH>
```

| Argument | |
|---|---|
| `PATH` | The folder. |


## rules

Rule sets (rulesync) for agents and projects.

### rules list

Rule files in the library, and each agent's rules state.

```bash
pitwall rules list [OPTIONS]
```

### rules sets

Rule sets, and each project's default set.

```bash
pitwall rules sets [OPTIONS]
```

### rules show

What a project's agents get: its default set, each agent's own set, and the rules those add up to for the agent's kind (or one agent's, with --agent).

```bash
pitwall rules show [OPTIONS] [PROJECT]
```

| Argument | |
|---|---|
| `PROJECT` (optional) | The project folder (default: the current folder). |

| Option | |
|---|---|
| `--agent <AGENT>` | One agent (id or name) instead of a whole project. |

```bash
pitwall rules show ~/code/api --human
pitwall rules show --agent api
```

### rules apply

Generate an agent's rule files now (used from its next session), or every agent's whose rules changed (--stale).

```bash
pitwall rules apply [OPTIONS] [AGENT]
```

| Argument | |
|---|---|
| `AGENT` (optional) | The agent (id or name). |

| Option | |
|---|---|
| `--set <SET>` | Give the agent this set of its own first (id or name; `none` clears). With --stale: only agents that use this set. |
| `--main-checkout` | The agent works in the main checkout: write there anyway (asks you in Pitwall). |
| `--stale` | Every agent whose rules changed or failed since its session started (in its own worktree, or a main checkout you confirmed). |
| `--project <PATH>` | With --stale: only this project's agents. |
| `--restart` | Then restart each re-applied agent that is running, so the rules take effect now (each restart asks you in Pitwall). |

```bash
pitwall rules apply api --set strict
pitwall rules apply --stale --restart            # every agent whose rules changed
pitwall rules apply --stale --project ~/code/api
```

### rules default

A project's default rule set (`none` clears it).

```bash
pitwall rules default [OPTIONS] <PROJECT> <SET>
```

| Argument | |
|---|---|
| `PROJECT` | The project folder. |
| `SET` | A set id or name, or `none`. |

### rules set create

A new rule set (rules by id from `rules list`, or by file name when only one rule has it).

```bash
pitwall rules set create [OPTIONS] <NAME>
```

| Argument | |
|---|---|
| `NAME` | The set's name. |

| Option | |
|---|---|
| `--rule <RULE>` | A rule to put in it. Repeatable. |

```bash
pitwall rules set create Web --rule style.md --rule testing.md
```

### rules set add

Add rules to a set.

```bash
pitwall rules set add [OPTIONS] <SET> <RULE>...
```

| Argument | |
|---|---|
| `SET` | The set (id or name). |
| `RULE` | Rules to add. |

```bash
pitwall rules set add Web library:terse.md
```

### rules set remove

Take rules out of a set.

```bash
pitwall rules set remove [OPTIONS] <SET> <RULE>...
```

| Argument | |
|---|---|
| `SET` | The set (id or name). |
| `RULE` | Rules to take out. |

### rules set rename

Rename a set.

```bash
pitwall rules set rename [OPTIONS] <SET> <NAME>
```

| Argument | |
|---|---|
| `SET` | The set (id or name). |
| `NAME` | The new name. |

### rules set delete

Delete a set (asks you in Pitwall). Projects and agents that use it lose it; agents keep their files until rules are re-applied.

```bash
pitwall rules set delete [OPTIONS] <SET>
```

| Argument | |
|---|---|
| `SET` | The set (id or name). |

### rules import

Add rules to the library: a project's own `.rulesync/` (read in place), a CLAUDE.md / AGENTS.md / GEMINI.md (needs rulesync), or a git repository (cloned). A file or a repository asks you in Pitwall.

```bash
pitwall rules import [OPTIONS] <KIND> <SOURCE>
```

| Argument | |
|---|---|
| `KIND` | Where from: project, file or git. |
| `SOURCE` | The project folder, the file, or the repository's URL. |

```bash
pitwall rules import project ~/code/api          # its own .rulesync/
pitwall rules import file ~/code/api/CLAUDE.md   # asks first
pitwall rules import git https://github.com/acme/ai-rules.git   # asks first
```


## review

What agents changed (read-only).

### review changes

Files an agent changed since it started (or in one task), with lines added and removed.

```bash
pitwall review changes [OPTIONS] <AGENT>
```

| Argument | |
|---|---|
| `AGENT` | The agent (id or name). |

| Option | |
|---|---|
| `--task <TASK_ID>` | One task's changes (ids in the Review screen). |

```bash
pitwall review changes api --human
```


## wait

Wait until an agent is idle, blocked or done (for coordinating agents).

```bash
pitwall wait [OPTIONS] --for <STATUS> <AGENT>
```

| Argument | |
|---|---|
| `AGENT` | The agent (id or name). |

| Option | |
|---|---|
| `--for <STATUS>` | Any of these statuses ends the wait: idle, blocked, done, working, stopped, exited (comma-separated). Required. |
| `--timeout <SECONDS>` | Give up after this many seconds (default 300): exit 1, code `timeout`. |
| `--fresh` | Ignore the status it has now: wait for it to change first (use after sending a prompt). |

```bash
pitwall queue add api "run the tests"
pitwall wait api --for done,blocked --fresh --timeout 900
```

