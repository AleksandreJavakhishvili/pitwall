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
  with the same name → `conflict`; use the id. An agent nobody named
  takes its conversation's title (`nameAuto`), so its name can change:
  keep ids for agents you come back to.
- `unsupported` with "update Pitwall": this Pitwall is older than the CLI.

## Approvals

Approval prompts come from Pitwall itself ("Asked by <your agent name>"),
never from you. Pitwall identifies you from your process, not from anything
you pass. The command waits (up to ~2 minutes). Denied or unanswered → exit
3, nothing changed. Don't retry a denial unless the user asks, and don't try
to work around it. Read-only commands never ask.

Asks the user: `agent stop`, `agent restart`, `agent remove` (also with
`--worktree`), `rules apply --main-checkout`, `rules import file|git`,
`rules set delete`, `session add --start`, `agent new --machine` (creates on
an agw VM), and `settings set|reset` of an `approval` setting. Everything
else applies directly (including creating and editing rule sets and
importing a project's own `.rulesync/`).

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
pitwall rules show ~/code/api              # a project's default set; each agent's own set and effective rules
pitwall rules show --agent api             # one agent's
pitwall review changes <agent>             # files +/- since the agent started
pitwall settings list
```

`status` values: `working`, `blocked` (needs the user), `idle`, `done`
(finished a turn nobody looked at yet), `unknown`, `exited`, `stopped`.

## Add agents and sessions

```bash
pitwall agent new --kind claude --project ~/code/api [--name api] [--resume <session-id>] [--account <name>]
pitwall agent new --kind claude --project ~/code/api --name api --prompt "fix the failing tests"   # start an agent to do X
pitwall agent new --kind claude --project . --name review --prompt - < task.md   # or --prompt-file task.md
pitwall agent new --kind shell --project .            # a terminal
pitwall session add agw <vm> <session>                # track an existing session (no approval)
pitwall session add agw <vm> <session> --start        # …and start it if stopped (asks)
pitwall agent new --machine <vm> --name <session> --workspace <ws> [--template <t>] [--as admin|agent:<name>|new-agent[:<name>]]
pitwall agent new --machine <vm> --name <session> --new-workspace [<name>] [--workspace-template <t>]
```

- Kind ids: `claude`, `codex`, `shell`, … (Pitwall's New-agent list).
  `--project` defaults to the current folder.
- To start an agent to do something, give it the task with `--prompt` in the
  same command: it is queued in its Next up as it is created (as `queue add`
  does) and goes in when the agent is first free. The JSON has `queued`
  (`{id, position}`); if Auto-send is off it waits there for the user's Send.
  Should queueing fail, the agent still exists: the exit status is 1 (code
  `queue_failed`), stdout has the agent (with `queueError`), so retry with
  `queue add <id>`. An empty prompt creates nothing.
- `--account` picks the login (an account's name or id from Settings →
  Agents → Accounts, or `default` for the user's own); without it the
  project's account is used.
- `--machine` creates a session on an agw VM (asks the user; asked every time
  when it also creates a workspace or agent user). Read `machine form <vm>`
  first; `--option FIELD=VALUE` sets any field. Names: lowercase letters,
  digits, `-`, `_`, max 34. Removing it from Pitwall later leaves it on the VM.

## Manage agents

```bash
pitwall agent rename a1b2 api              # kept from now on
pitwall agent rename api --auto            # follow its conversation's title again
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

Example: spawn a reviewer with its task, wait, read its changes:

```bash
id=$(pitwall agent new --kind claude --project . --name reviewer \
  --prompt "Review the diff on this branch; list problems only." | jq -r .id)
pitwall wait "$id" --for done,blocked --fresh --timeout 1800
pitwall review changes "$id"
```

To watch every agent instead of one, `pitwall events` prints one JSON line
per event until stopped: `agents.changed` (the whole list, as `agent list`),
`attention` (`{agentId,name,reason:"blocked"|"done",detail?}`) and
`blocked.changed` (`{count}`). `--topics attention` narrows it; `--count N`
stops after N lines.

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

## Showing the user files

```bash
pitwall open src/main.rs:42 README.md --agent api   # tabs in a files tile right of api's terminal (at a line, marked)
pitwall open --folder src --agent api               # a folder as a tree, with git status
pitwall open --root fix/login --agent api           # a whole worktree (by folder name or branch)
pitwall open --root ../api-fix --agent api          # the same by its path: the tree is rooted there
```

A files tile is a slot of its own in the space's layout, next to the
terminal slots: right of the agent's terminal unless `--where left|above|below`
says otherwise (`--of web`: next to another agent's terminal), in that
terminal's slot with `--where replace` (only when the user wants the files
instead of the terminal: it leaves the layout and keeps running), or a window
of its own with `--where window`. Later calls add tabs wherever it is (the user
may have dragged it elsewhere or out to a window); naming a place moves it.
Pitwall doesn't come forward over another app. Any folder on this
computer can be shown. Agents Pitwall starts (Claude Code, Codex) have the
same as MCP tools (`show_files`, `show_folder`). Off with
`agents.showFiles false` (then `open` refuses).

## Showing the user a web page

```bash
pitwall open --web https://example.com --agent api   # a web tile right of api's terminal
pitwall open --web localhost:5173 --where left --agent api
pitwall open --web --where window --agent api        # an empty page in a window, its address field for the user
```

Only when the user asks for a page in Pitwall. It only shows the page: you
get nothing back from it and can't click, type or read anything in it. Only
http(s) addresses. Places as for files (`--where`, `--of`); a later call goes
to the page you opened before, wherever the user moved it. It never takes the
keyboard or brings Pitwall forward. MCP: `show_web`. Off with
`agents.showWeb false` or `general.webPages false`.

## Showing the user Pitwall's Settings

```bash
pitwall open --settings extensions --agent api   # Settings at its Extensions page, in the Settings tile
pitwall open --settings extensions --extension my-inbox --agent api   # with that extension's card unfolded
pitwall open --settings --agent api              # the page shown last
```

When the user asks where a Pitwall setting is. Pages: general, agents,
profiles, accounts, extensions, rules, appearance, folderAccess, peers,
about. It only shows Settings: you get nothing back from it (no values) and
can't change, click or type anything in it; to change a setting, the user
does it there (or you run `pitwall settings set`, when they ask you to). The
Settings tile opens right of your terminal, or stays where the user keeps
it and switches page. It never takes the keyboard or brings Pitwall
forward. MCP: `show_settings`. Off with `agents.showSettings false`.

Without `--root`, files come from the folder the agent works in now (where
its hooks last said it was, so a worktree it entered mid-session), shown in
the git worktree that holds it, with that worktree's branch and status. When
you work in another worktree than the one you started in, give `--root`
(its path) to be sure.

To show the user another worktree or folder, give it as `--root` (MCP:
`root`): its path, or a worktree's folder name or branch (of your
repository, or of one directly inside your folder when that isn't a
repository). The viewer's tree is rooted exactly there, also when it lies
inside your own folder, with that worktree's git status. The answer names
the root shown (and says how a name was read); an unknown name lists the
worktrees there are, an ambiguous one their paths. A `--folder` given by
its path in another worktree roots the viewer at that worktree too. The
user can switch the root themselves (the folder in the viewer's bar:
Worktrees, Open folder…, Up to parent folder; a folder's "Open as root"),
so never tell them to collapse the tree or start another agent to see a
worktree.

## Extensions (setting one up for the user)

```bash
pitwall extension list                     # installed (on/off, version, update known) and available
pitwall extension status inbox             # its checklist as data, and `next`: the one step to take
pitwall extension log inbox --lines 50     # its card's Log (secrets already taken out)
pitwall extension setup inbox              # e.g. what `next.command` says; asks the user
```

The loop: `status` → do `next` → `status` again. `next.who` says whose
step it is: `agent` (run `next.command`), `user` (tell them `next.text`;
only they can do it), `wait` (run `status` again in a few seconds) or
`nobody` (all set). Every change (`install`, `on`, `off`, `update`
after `check-updates`, `remove`, `setup [--action ID]`, `service ID
confirm|start|stop|restart|update`, `first-run`, `set ID KEY VALUE`,
`connection add|rename|remove`) shows the user an approval in Pitwall
with exactly what will run, and waits; one request per extension at a
time (another is refused `busy`). Declined (exit 3): nothing ran; don't
ask again unless the user asks.

- **Secrets**: never ask the user to paste a key into the chat, and never
  pass one on the command line. `pitwall extension secret ID KEY` opens
  Settings next to your terminal at that field; the user enters it there.
  You only ever see whether it is set.
- **Only the user** grants permissions beyond what turning on shows,
  deletes an extension's data, allows "send without asking", enters
  secrets and signs in.
- **Service "running elsewhere"**: something else already listens on its
  address (the user runs it). Use it; never start or stop it.
- **"Version too old"** (both versions in `service`): `pitwall extension
  service ID update` runs its install action, then starts it again.
- **Signed out** (`adapter.state`): its key was refused or expired. The
  user enters a new one (`secret`) or signs in again (`first-run`); read
  `log` for the reason.
- **First run**: opens a one-time sign-in link in the user's browser. You
  never get the link, only that it was opened.

## Projects

```bash
pitwall project add ~/code/api             # Pitwall's project list (sidebar)
pitwall project remove ~/code/api          # off the list; never deletes files
```

## Rules (rulesync)

**Skills first.** Reusable know-how belongs in a **skill** (a folder with
SKILL.md, loaded only when an agent finds it relevant). A **rule** is in
every session's context: before making something a rule (or turning a
skill into one with `--as-rule`), tell the user it costs context in every
session and prefer a skill unless they need it always on. A project's
CLAUDE.md / AGENTS.md is the project's own: Pitwall only generates it once
rules are copied into the project.

```bash
pitwall rules list                                   # skills and rules, with a type column
pitwall rules skill new review --description "Review a diff for bugs and missing tests"
pitwall rules skill new deploy --from ./tools/deploy   # a folder with SKILL.md (+ its files)
pitwall rules new tabs --description "Tabs, not spaces" [--from <file>] [--targets claudecode]
pitwall rules new deploy-rule --from library:skills/deploy --as-rule   # only when asked: always-on now
pitwall rules skill new review --overwrite ...       # replacing one asks
pitwall rules project assign ~/code/api --skill review   # copy into .rulesync/skills (asks)
```

Two scopes. **Project**: library skills and rules (the catalog) copied into the repo's
`.rulesync/rules/` and committed with it, plus the files rulesync generates
from them (CLAUDE.md, AGENTS.md, …). **Agent**: an agent's own set on top,
personal and local-only. No account- or team-wide rules. Writing into a
project asks the user; the files then show in Review for them to commit.

```bash
pitwall rules project status ~/code/api              # its rules, updates, generated files out of date?, what to do first
pitwall rules project import-existing ~/code/api     # hand-written CLAUDE.md/AGENTS.md → .rulesync/ first (asks)
pitwall rules project assign ~/code/api --set Web [--rule testing.md]   # copy into .rulesync/rules (asks)
pitwall rules project unassign ~/code/api style.md   # only if unchanged there (--force otherwise; asks)
pitwall rules project updates ~/code/api             # library changes, with diffs
pitwall rules project update ~/code/api [style.md] [--take library|project]   # opt-in (asks)
pitwall rules project generate ~/code/api [--targets claudecode,codexcli]     # CLAUDE.md, AGENTS.md… (asks)
pitwall rules project move-default ~/code/api        # old default set kept in Pitwall → the project (asks)
```

Never pass `--overwrite`, `--force` or `--take library` unless the user
chose to replace their file: those discard their text. The old default
set (`rules default`) only applies while a project has no `.rulesync/`.

```bash
pitwall rules list                                   # library rules (ids) and each agent's state
pitwall rules sets                                   # ids and names
pitwall rules show ~/code/api                        # default set, each agent's own set and effective rules
pitwall rules set create Web --rule style.md --rule testing.md   # a new set (rule ids or file names)
pitwall rules set add Web library:terse.md           # more rules
pitwall rules set remove Web style.md                # fewer
pitwall rules set rename Web "Web defaults"
pitwall rules set delete Web                         # asks first
pitwall rules import project ~/code/api              # a project's own .rulesync/ (read in place)
pitwall rules import file ~/code/api/CLAUDE.md       # a CLAUDE.md / AGENTS.md / GEMINI.md: asks first
pitwall rules import git https://github.com/acme/ai-rules.git   # cloned: asks first (only .claude/skills: copied into the library)
pitwall rules default ~/code/api Web                 # a project's default set (`none` clears)
pitwall rules apply api [--set strict|none]          # write the agent's rule files now
pitwall rules apply api --main-checkout              # it works in the main checkout: asks first
pitwall rules apply --stale [--project ~/code/api] [--restart]   # every agent whose rules changed (restarts ask)
```

Rule files take effect in the agent's next session (restart it). Without
`--main-checkout`, an agent in the main checkout gives `conflict`. To turn an
existing CLAUDE.md into a set: `rules import file <path>`, then
`rules set create <name> --rule <added id>`. Rules are referred to by id
(`library:style.md`) or, when only one rule has it, by file name. Gemini CLI
and Aider get rules too (Pitwall writes their file itself). `rules list`
shows whether rulesync is available (`settings set rules.allowNpx true`
allows npx; asks); importing a file needs it. The user does the same in Pitwall's
Settings → Rules & skills.

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
