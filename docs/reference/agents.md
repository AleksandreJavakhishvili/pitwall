# Agent definitions

Every agent kind in New agent (Claude Code, Codex, Gemini CLI, Shell, …) is one small TOML file. Pitwall ships a file per kind, and you can add your own kinds or change the bundled ones without an app update.

## Where the files live

The bundled definitions are built into the app; their sources are in [`crates/pitwall-core/agents/`](../../crates/pitwall-core/agents/). Your own go in the `agents` folder inside Pitwall's data folder:

| System | Folder |
|---|---|
| macOS | `~/.pitwall/agents/` |
| Linux | `$XDG_DATA_HOME/pitwall/agents/` (default `~/.local/share/pitwall/agents/`) |
| Windows | `%APPDATA%\Pitwall\agents\` |

With `$PITWALL_HOME` set, the folder is `$PITWALL_HOME/agents/` on every system.

- Every `*.toml` file in the folder is read; other files are ignored.
- A file whose `id` matches a bundled kind replaces that kind completely (it doesn't merge key by key), so copy the bundled file and change what you need.
- A file with a new `id` adds a kind to New agent.
- A file that doesn't parse is skipped with a message on Pitwall's error output; the other kinds still load.
- Files are read when Pitwall starts. Restart Pitwall after editing one.

## A first example

A made-up CLI called `drift` that starts with `drift chat`, resumes with `drift chat --continue <id>`, and accepts a session id up front:

```toml
id = "drift"
name = "Drift"
command = "drift chat"
new_args = ["--session", "{session_id}"]
resume_args = ["--continue", "{session_id}"]
assign_session_id = true
hooks = "none"
```

Save it as `~/.pitwall/agents/drift.toml` (on macOS; see the table above for other systems), restart Pitwall, and Drift appears in New agent. Its status comes from output activity until you add [detection rules](detection.md) for it in `~/.pitwall/detect/drift.toml`.

## Keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | required | The kind's id: what `pitwall agent new --kind` takes, the name of its detection rule file, and what a user file overrides. |
| `name` | string | required | The name shown in New agent, menus and tiles. |
| `command` | string | required | The command line that starts the agent, run by your login shell. It may hold arguments and shell syntax (`$SHELL`, `npx foo --bar`). |
| `new_args` | list of strings | `[]` | Arguments added for a fresh start. |
| `resume_args` | list of strings | `[]` | Arguments added to continue a conversation. Empty: the kind can't resume, and Restart starts fresh. |
| `assign_session_id` | boolean | `false` | Pitwall picks a new conversation id (a UUID) for each fresh start and fills it into `{session_id}` in `new_args`. |
| `hooks` | string | required | How the agent reports its status: `claude-settings`, `codex-global` or `none` (see [Hooks](#hooks)). |
| `mcp` | string | `none` | How the agent gets Pitwall's `pitwall` MCP server at launch, which lets it show you files in Pitwall: `claude-config`, `codex-config`, `copilot-config`, `qwen-config`, `amp-config`, `opencode-env` or `none` (see [Showing files](#showing-files)). |
| `mcp_file` | table | none | For a tool with no launch-time way: the MCP file Pitwall may add its server to when you ask, `path` (a JSON file with `mcpServers`; `~/` is your home folder) and `entry` (fields added next to `command` and `args`, like `{ trust = true }`). See [Showing files](#showing-files). |
| `rulesync_target` | string | none | The [rulesync](https://github.com/dyoshikawa/rulesync) target written when rules are applied for this kind (`claudecode`, `codexcli`, `cursor`, …). None: New agent doesn't offer rules for it, unless `rules_fallback` is set. |
| `rules_fallback` | table | none | For a tool rulesync has no target for: Pitwall writes the rules into one file itself (see [Rules without rulesync](#rules-without-rulesync)). Ignored when `rulesync_target` is set. |
| `worktree_args` | list of strings | `[]` | The agent's own "work in a new git worktree" flag, added when New agent's **Separate worktree** is ticked. `{name}` becomes a name Pitwall picks. Empty: no worktree option for this kind. Only for agents on this computer. |
| `process_names` | list of strings | the program of `command` | Program names the agent runs as in the process table. Used to notice the agent when you start it by hand in a Pitwall terminal. |
| `aliases` | list of strings | `[]` | Other names for this kind, matched like `id` when adopting sessions from other platforms (agw calls Claude Code `claude-code`). Matching ignores case. |
| `account_env` | string | none | The environment variable that names the folder the tool keeps its login in (`CLAUDE_CONFIG_DIR`). Set, the kind can run on more than one account (see [Accounts](#accounts)). |
| `account_home` | string | none | That folder when the variable isn't set (`~/.claude`). It's the Default account. `~/` means your home folder. |
| `icon` | string | none | The logo shown before the kind's name in the sidebar, tile headers, New agent and the strip of hidden agents: a bundled logo (`claude`, `codex`, `gemini`, `copilot`, `cursor`, `qwen`, or `terminal` for the terminal icon) or the full path of a one-colour SVG file (`~/` means your home folder; it's tinted to the theme). None: the kind's first letter on a badge. |
| `update_command` | string | none | The command that updates the tool (`claude update`). With **Keep agent tools up to date** on (Settings → Agents, `agents.updateTools`), Pitwall runs it through your login shell when it starts and every 6 hours, one tool at a time, and only for installed tools. |
| `update_off_env` | table | `{}` | Environment that turns the tool's own updater off (`{ DISABLE_AUTOUPDATER = "1" }`). While Pitwall updates the tool, every agent and terminal it starts gets it, so many sessions don't update themselves at once (they race and can leave the tool's command missing). |

Unknown keys are ignored.

## Placeholders

| Placeholder | In | Becomes |
|---|---|---|
| `{session_id}` | `new_args`, `resume_args` | The conversation id. In `new_args` it's only filled when `assign_session_id = true`; when there is no id, the argument holding it is left out. |
| `{name}` | `worktree_args` | The worktree name Pitwall picked for this agent. |
| `{file}` | `rules_fallback.args`, `rules_fallback.alt_args` | The absolute path of the rules file Pitwall wrote. |

Every argument is shell-quoted before it's added to `command`, so an argument with spaces stays one argument. `command` itself is used as written.

## How an agent is started

Pitwall builds one command line and runs it with your login shell (`$SHELL -l -i -c`; PowerShell on Windows), so the agent sees the same PATH, logins and config as your terminal.

1. `command`.
2. When resuming a known conversation and `resume_args` isn't empty: `resume_args`. Otherwise `new_args` (with a new id when `assign_session_id = true`), then the `rules_fallback` arguments for the file that holds the agent's rules, if any.
3. On a fresh start with **Separate worktree** ticked: `worktree_args`.
4. With **Agents can show you files** on, the `mcp` way: `claude-config` adds `--mcp-config '<json>'` here; `codex-config`, `copilot-config`, `qwen-config` and `amp-config` put their flags right after `command` instead, before the kind's own arguments; `opencode-env` adds nothing to the line (it's in the environment).
5. With `hooks = "claude-settings"`: `--settings '<json>'` carrying Pitwall's hooks (and, with the MCP server, the rule that allows its two tools).

For example, the bundled Claude Code definition starts `claude --session-id <uuid> --settings '…'` and resumes with `claude --resume <uuid> --settings '…'`.

Pitwall knows a conversation's id when it chose it (`assign_session_id`), when a hook reports it (Codex), or when it's on the command line of an agent you started by hand in a Pitwall terminal, read through the kind's own `new_args` and `resume_args` (`--resume <id>`, `--resume=<id>`, `resume <id>`). Without an id, Restart starts a new conversation.

## Rules without rulesync

rulesync has no target for some tools (Gemini CLI, Aider). With `rules_fallback`, Pitwall still gives them rules: it joins the bodies of the rules the agent gets (frontmatter dropped; a rule counts when its `targets` include `target`, `*`, or it has no `targets`) into one file in the agent's folder. Like rulesync's output, the file is recorded for the agent, listed in `.git/info/exclude`, never written over a file git tracks or one Pitwall didn't write, and only written into a main checkout after you confirm.

| Key | Type | Default | What it does |
|---|---|---|---|
| `target` | string | required | The name rules' `targets` use for this tool (`geminicli`). |
| `file` | string | required | The file to write, relative to the agent's folder (`GEMINI.md`). |
| `args` | list of strings | `[]` | Added to fresh starts while `file` holds the rules (Aider: `["--read", "{file}"]`). |
| `alt_file` | string | none | Where the rules go when `file` is the project's own. None: they aren't written then. |
| `alt_args` | list of strings | `[]` | Added to fresh starts while `alt_file` holds the rules, to point the tool at it. |

The bundled Gemini CLI definition writes `GEMINI.md` (which Gemini reads by itself); when the project has its own, it writes `.gemini/pitwall-rules.md` and starts Gemini with `--prompt-interactive "Read <file> first: …"`. The bundled Aider definition writes `.aider.pitwall-rules.md` and starts Aider with `--read <file>`.

## Hooks

| Value | What it does |
|---|---|
| `claude-settings` | Hooks are passed on each launch with `--settings <json>`. Your own Claude Code settings files are never changed. |
| `codex-global` | Hooks live in the tool's `hooks.json` (`~/.codex/hooks.json`, or the account's folder), which Pitwall only edits after you approve it (Settings → Agents, or `pitwall settings set agents.codexHooks true`). Outside Pitwall the hook does nothing. |
| `none` | No hooks. Status comes from [detection rules](detection.md), then output activity. |

Both hook modes are tied to the Claude Code and Codex hook formats. A kind of your own normally uses `none`.

## Showing files

With **Agents can show you files** on (Settings → Agents, `agents.showFiles`, on by default), agents Pitwall starts on this computer can open files, folders and images in Pitwall's file viewer when you ask them to ("open that file for me"). Pitwall's MCP server, `pitwall-cli mcp`, has two tools, `show_files` and `show_folder`; `pitwall open` does the same from a shell. Each kind gets it the best way its tool has, in this order, and Settings → Agents lists which:

1. **MCP at launch** (`mcp`): added in the tool's own documented launch-time way, which adds to your own servers, so your config files are never edited.
2. **MCP via its config** (`mcp_file`): the tool only reads servers from its own file. Pitwall adds its entry there only when you ask, and takes it out again the same way.
3. **Shell command via a rule**: no MCP at all. A built-in rule tells the agent to run `pitwall-cli open`.
4. **Not available**: a plain terminal, or a kind of your own with none of the above.

Files are shown from the folder the agent works in **now**: the `root` it gives, else where its hooks last said it works (so a worktree Claude Code entered mid-session), else the folder it said it works in (`pitwall agent track`) or its process's, else where it started. The tree is the git worktree holding that folder, with that worktree's own branch and git status; a removed worktree is reported as gone. Agents are told to give `root` when they work in another worktree than the one they started in.

| `mcp` | What it does |
|---|---|
| `claude-config` | `claude --mcp-config '{"mcpServers":{"pitwall":…}}'`, added to your own servers, and the two tools allowed (`mcp__pitwall__show_files`, `mcp__pitwall__show_folder`) in the `--settings` Pitwall passes, so they don't ask each time. |
| `codex-config` | `codex -c 'mcp_servers.pitwall.command="…"' -c 'mcp_servers.pitwall.args=["mcp"]'`: config overrides for this run only. The tools say they are read-only, so Codex doesn't ask for them. |
| `copilot-config` | `copilot --additional-mcp-config '{"mcpServers":{"pitwall":{…,"type":"local","tools":["*"]}}}' --allow-tool pitwall`: augments `~/.copilot/mcp-config.json` for this run, and the server's tools are allowed. |
| `qwen-config` | `qwen --mcp-config '{"mcpServers":{"pitwall":…}}' --allowed-tools mcp__pitwall__show_files --allowed-tools mcp__pitwall__show_folder`: merged with your and the project's servers. |
| `amp-config` | `amp --mcp-config '{"pitwall":…}'`: merged with your servers; Amp doesn't ask to approve servers given this way. |
| `opencode-env` | `OPENCODE_CONFIG_CONTENT='{"mcp":{"pitwall":{"type":"local","command":[…],"enabled":true}},"permission":{"pitwall_*":"allow"}}'` in the agent's environment: opencode's inline config, merged with your and the project's. |
| `none` | No server at launch (the default). |

The bundled kinds:

| Kind | Way | What Pitwall uses | Checked against |
|---|---|---|---|
| Claude Code | MCP at launch | `--mcp-config`, tools allowed in `--settings` | the installed `claude`, run |
| Codex | MCP at launch | `-c mcp_servers.pitwall.…` | the installed `codex`, run |
| Copilot CLI | MCP at launch | `--additional-mcp-config`, `--allow-tool pitwall` | GitHub's CLI command reference (not run) |
| Qwen Code | MCP at launch | `--mcp-config`, `--allowed-tools` | Qwen Code's options and source (not run) |
| Amp | MCP at launch | `--mcp-config` | ampcode.com MCP docs (not run) |
| opencode | MCP at launch | `OPENCODE_CONFIG_CONTENT` | opencode's config docs (v1; not run) |
| Gemini CLI | MCP via its config | `~/.gemini/settings.json`, with `"trust": true` so its tools don't ask | gemini-cli docs (not run) |
| Cursor Agent | MCP via its config | `~/.cursor/mcp.json`, with `"type": "stdio"`; each tool call asks unless you allow `Mcp(pitwall:*)` in Cursor's CLI permissions | `cursor-agent --help` (it has no MCP flag) and cursor.com docs |
| Aider | Shell command via a rule | the rule, read with `--read` | Aider's options (it has no MCP) |
| Terminal | Not available | | |

**MCP via its config.** Settings → Agents shows the file next to the kind, with **Add Pitwall to it…**; `pitwall settings set agents.showFilesGemini true` (or `agents.showFilesCursor`) does the same and asks you in Pitwall first. Only Pitwall's `pitwall` entry is added (an older one is brought up to date): the change is made in the file's text, so everything else stays byte for byte, and a backup is kept next to it (`settings.json.pitwall-backup`). **Remove** (or `false`) takes the entry out again, and `mcpServers` too when Pitwall added it. Nothing is added until you ask. The tool then reads the server in every session, also outside Pitwall: its tools can't show anything there. The file is the one in your home folder, not an account's own folder (Settings → Accounts).

**Shell command via a rule.** While the setting is on, Pitwall keeps a short built-in rule (`~/.pitwall/run/show-files/pitwall-show-files.md`) that tells the agent to run `pitwall-cli open <file>[:line[:column]]` or `pitwall-cli open --folder <folder>` when you ask to see something. An agent with rules gets it with them, in its rules file (and its rules count as changed when the setting changes, to reapply); one without gets the rule itself (Aider: `--read <rule>`). Aider suggests the command and asks you before it runs it, as for any shell command. Kinds that can't run shell commands get nothing.

The setting applies when an agent starts: restart an agent to give it the server or take it away. The server's command is a copy of the CLI in Pitwall's data folder (`~/.pitwall/bin/pitwall-cli`), refreshed when Pitwall starts, so agents keep it when the app is moved or updated.

The server lives as long as its agent, so it often outlasts an update of Pitwall. It doesn't carry the tools itself: their list, the instructions and every call go to the running Pitwall, which checks and answers them, so an agent started before an update uses the updated tools. When Pitwall's tools change (updated and restarted), the server tells the agent to read them again (MCP's `tools/list_changed`; it notices on the next call, or within about 20 seconds). Pitwall not running, or older than the server: the server's own copy of the tools, and a call says Pitwall isn't running, or that Pitwall is older and should be restarted. An answer it can't fully read (from a newer Pitwall) still says what opened; a call is only reported as failed when nothing was opened. `pitwall open` reads answers the same way. A server from before this (it says no `proto` when it asks) is answered in the words it knows, `beside` or `window`, until its agent restarts.

Where it opens (the agent can name one with `where`, `pitwall open --where`):

| Place | What you get |
|---|---|
| `right` (the default), `left`, `above`, `below` | A files tile: a slot of its own in the space's layout, on that side of the agent's terminal slot (or of another agent's, named with `of` / `--of`), in the Pitwall window that shows it (its space comes up there if another one was shown). It never goes inside a terminal's tile. It resizes with the layout's dividers like any tile and is kept across restarts with what it had open. The terminal keeps the keyboard until you click into the files; Esc gives it back. When a squeezed row or column would fold tiles away, it is shared evenly instead. `beside` is read as `right`. |
| `replace` | A files tile in the agent's terminal slot (or the one `of` / `--of` names): the same place and size. The terminal leaves the layout as when you close its tile; the agent keeps running and is in the sidebar, to show again. |
| `window` | A normal window of its own, titled with the agent and the folder: move it to another screen, resize it (its place and size are kept for the next one). The pin keeps it above other apps' windows (macOS and Windows). |

Move it yourself: drag the tile's (or the window's) bar, anywhere but its buttons (the grip shows it can be dragged; the tab strip's empty end works too), into any Pitwall window's layout. A window moves with it. The tile under the pointer shows where it lands: the band along a side of any tile (splitting off a slot there), an empty tile, the rest of a terminal's tile (the files take its place, as big and where it is; the terminal leaves the layout and keeps running, and the preview says so), the middle of another agent's files tile (the two swap places), or along an edge of the main area (a column or row of its own). Drag a tile out of every Pitwall window and it becomes a window there. Without dragging: **Move…** on a tile and **Dock…** on a window list the same places (beside a terminal, along an edge, a slot where there is room, "Replace …'s terminal" for each terminal in the space), **Open in window** pops a tile out, **Dock right of the terminal** puts a window back in the slot right of the agent's terminal. Nothing asks about unsaved edits when it moves: the same viewer moves, edits and all. Dragging follows the pointer across windows on macOS; elsewhere use the menus.

Not named: where the agent's files are already (a tile or a window), else a tile right of its terminal. Naming a place that isn't where they are moves them there. Pitwall never comes forward over the app you are in, and the terminal keeps the keyboard. An agent no open Pitwall window shows gets a window, and its tool result says so. Later calls from the same agent add tabs to the same tile or window. A folder outside every agent's folder opens with the tree rooted at that folder (agents on this computer; another machine's agents can't show files).

## Accounts

A kind with `account_env` can run on more than one login, say a work and a personal subscription. Each account is a login folder; the agent starts with `account_env` set to that folder. Default is your own login, in `account_home`.

- Add accounts in Settings → Accounts, or from a project's right-click menu → Accounts…. Choosing a folder you already use is fine.
- Pick one in New agent, set one per project, or move an agent with its right-click menu → Run on account.
- The CLI takes `pitwall agent new --account <name>`.

Built in: Claude Code (`CLAUDE_CONFIG_DIR`), Codex (`CODEX_HOME`), Gemini CLI (`GEMINI_CLI_HOME`, the folder its `.gemini` goes in) and GitHub Copilot CLI (`COPILOT_HOME`). Cursor Agent, opencode, Amp, Aider and Qwen Code have no setting that moves their login on its own, so they run on one login. Settings → Accounts lists the tools that are installed or already have accounts.

Moving a running agent carries its conversation along for Claude Code and Codex only, since that needs to know where each tool keeps its conversations. Who an account is logged in as shows for all four where the tool's files say it (Gemini CLI and Copilot CLI may keep their login in the keychain instead). For another kind, `account_env` still lets each agent start on the account you chose.

### Usage

Settings → Accounts shows each account's usage, read from what the tool records on this Mac (nothing is sent anywhere):

- Codex: its plan's limits as Codex last saw them (the 5-hour and weekly windows: how much is used and when each starts over), and tokens.
- Claude Code: tokens in the last 5 hours, day and week, from its conversation logs (each reply counted once; the tooltip splits input, output and the prompt cache). Claude Code doesn't record its plan's limits on disk; `/usage` in Claude Code shows them.
- Other tools record neither, so they show none.

### Git identities

A git identity says who a project's agents and terminals are in git: the name and email on their commits, their GitHub and Azure DevOps logins, and optionally an SSH key. Use one when different projects belong to different accounts.

- Add them in Settings → Accounts → Git identities. Each one logs in to GitHub and Azure DevOps on its own: "Log in" next to a service opens a terminal where `gh auth login` or `az login` runs in the browser. Log in only to the services you use.
- Pick one per project from its right-click menu → Accounts…. Projects without one use your own git setup.
- Agents and terminals Pitwall starts in that project get `GIT_AUTHOR_NAME`/`EMAIL` and `GIT_COMMITTER_NAME`/`EMAIL` (and `user.name`/`user.email` as git settings, so `git config` shows them), and with a key `GIT_SSH_COMMAND` (works for any host, Azure DevOps' `ssh.dev.azure.com` too).
- For each service the identity is logged in to, they also get its CLI's folder (`GH_CONFIG_DIR`, `AZURE_CONFIG_DIR`), and git's HTTPS logins to its hosts come from it: `github.com` through `gh`, `dev.azure.com` and `*.visualstudio.com` with a token from `az`. A service it isn't logged in to stays on your own setup. Your `~/.gitconfig` isn't changed.
- It's read at each start: restart a running agent to switch it. Terminals you open outside Pitwall still use your own setup.

## The bundled Claude Code definition

```toml
# Claude Code. Status comes from hooks passed per launch with --settings,
# so the user's ~/.claude/settings.json is never modified.
id = "claude"
name = "Claude Code"
command = "claude"
# {session_id} is a UUID Pitwall picks up front, so resume works without a hook.
new_args = ["--session-id", "{session_id}"]
resume_args = ["--resume", "{session_id}"]
assign_session_id = true
hooks = "claude-settings"
# rulesync target used when applying rules (docs/spec/rules.md).
rulesync_target = "claudecode"
# `claude --worktree [name]` (-w): Claude creates and enters its own worktree.
worktree_args = ["--worktree", "{name}"]
# What other platforms call it (agw's harness integration), for adopting their sessions.
aliases = ["claude-code"]
# Accounts: each login lives in its own folder (Settings → Accounts).
account_env = "CLAUDE_CONFIG_DIR"
account_home = "~/.claude"
```

## Changing a bundled kind

To run Codex through a wrapper script, copy the bundled `codex.toml` to `~/.pitwall/agents/codex.toml` and change only `command`:

```toml
id = "codex"
name = "Codex"
command = "~/bin/codex-wrapper"
new_args = []
resume_args = ["resume", "{session_id}"]
assign_session_id = false
hooks = "codex-global"
rulesync_target = "codexcli"
worktree_args = ["--worktree"]
process_names = ["codex"]
account_env = "CODEX_HOME"
account_home = "~/.codex"
```

`process_names` keeps Pitwall recognising `codex` in the process table now that `command` starts something else.
