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
| `rulesync_target` | string | none | The [rulesync](https://github.com/dyoshikawa/rulesync) target written when rules are applied for this kind (`claudecode`, `codexcli`, `cursor`, …). None: New agent doesn't offer rules for it. |
| `worktree_args` | list of strings | `[]` | The agent's own "work in a new git worktree" flag, added when New agent's **Separate worktree** is ticked. `{name}` becomes a name Pitwall picks. Empty: no worktree option for this kind. Only for agents on this computer. |
| `worktree_dirs` | list of strings | `[]` | Folders, relative to the agent's folder, where the tool makes worktrees of its own (`.claude/worktrees`). Worktrees found there are shown under the agent. |
| `process_names` | list of strings | the program of `command` | Program names the agent runs as in the process table. Used to notice the agent when you start it by hand in a Pitwall terminal. |
| `aliases` | list of strings | `[]` | Other names for this kind, matched like `id` when adopting sessions from other platforms (agw calls Claude Code `claude-code`). Matching ignores case. |
| `account_env` | string | none | The environment variable that names the folder the tool keeps its login in (`CLAUDE_CONFIG_DIR`). Set, the kind can run on more than one account (see [Accounts](#accounts)). |
| `account_home` | string | none | That folder when the variable isn't set (`~/.claude`). It's the Default account. `~/` means your home folder. |
| `icon` | string | none | The logo shown before the kind's name in the sidebar, tile headers, New agent and the strip of hidden agents: a bundled logo (`claude`, `codex`, `gemini`, `copilot`, `cursor`, `qwen`, or `terminal` for the terminal icon) or the full path of a one-colour SVG file (`~/` means your home folder; it's tinted to the theme). None: the kind's first letter on a badge. |
| `update_command` | string | none | The command that updates the tool (`claude update`). With **Keep agent tools up to date** on (Settings → Agents, `agents.updateTools`), Pitwall runs it through your login shell when it starts and every 6 hours, one tool at a time, and only for installed tools. |
| `update_off_env` | table | `{}` | Environment that turns the tool's own updater off (`{ DISABLE_AUTOUPDATER = "1" }`). While Pitwall updates the tool, every agent and terminal it starts gets it, so many sessions don't update themselves at once (they race and can leave the tool's command missing). |
| `share` | table | none | What hosting a shared session of this kind needs (see [Sharing](#sharing)). None: the kind can still be shared, with replies from its terminal output, no fork, and no model hosts allowed until the owner adds some. |

Unknown keys are ignored.

## Placeholders

| Placeholder | In | Becomes |
|---|---|---|
| `{session_id}` | `new_args`, `resume_args` | The conversation id. In `new_args` it's only filled when `assign_session_id = true`; when there is no id, the argument holding it is left out. |
| `{name}` | `worktree_args` | The worktree name Pitwall picked for this agent. |

Every argument is shell-quoted before it's added to `command`, so an argument with spaces stays one argument. `command` itself is used as written.

## How an agent is started

Pitwall builds one command line and runs it with your login shell (`$SHELL -l -i -c`; PowerShell on Windows), so the agent sees the same PATH, logins and config as your terminal.

1. `command`.
2. When resuming a known conversation and `resume_args` isn't empty: `resume_args`. Otherwise `new_args` (with a new id when `assign_session_id = true`).
3. On a fresh start with **Separate worktree** ticked: `worktree_args`.
4. With `hooks = "claude-settings"`: `--settings '<json>'` carrying Pitwall's hooks.

For example, the bundled Claude Code definition starts `claude --session-id <uuid> --settings '…'` and resumes with `claude --resume <uuid> --settings '…'`.

Pitwall knows a conversation's id when it chose it (`assign_session_id`), when a hook reports it (Codex), or when it's on the command line of an agent you started by hand in a Pitwall terminal, read through the kind's own `new_args` and `resume_args` (`--resume <id>`, `--resume=<id>`, `resume <id>`). Without an id, Restart starts a new conversation.

## Hooks

| Value | What it does |
|---|---|
| `claude-settings` | Hooks are passed on each launch with `--settings <json>`. Your own Claude Code settings files are never changed. |
| `codex-global` | Hooks live in the tool's `hooks.json` (`~/.codex/hooks.json`, or the account's folder), which Pitwall only edits after you approve it (Settings → Agents, or `pitwall settings set agents.codexHooks true`). Outside Pitwall the hook does nothing. |
| `none` | No hooks. Status comes from [detection rules](detection.md), then output activity. |

Both hook modes are tied to the Claude Code and Codex hook formats. A kind of your own normally uses `none`.

## Accounts

A kind with `account_env` can run on more than one login, say a work and a personal subscription. Each account is a login folder; the agent starts with `account_env` set to that folder. Default is your own login, in `account_home`.

- Add accounts in Settings → Agents → Accounts, or from a project's right-click menu → Accounts…. Choosing a folder you already use is fine.
- Pick one in New agent, set one per project, or move an agent with its right-click menu → Run on account.
- The CLI takes `pitwall agent new --account <name>`.

Moving a running agent carries its conversation along and showing who an account is logged in as both work for Claude Code and Codex only, since they need to know where each tool keeps its files. For another kind, `account_env` still lets each agent start on the account you chose.

## Sharing

A `[share]` table tells Pitwall what a shared session of the kind needs inside its sandbox (see the collaboration spec, `docs/spec/collaboration/sessions.md` §6.3 and §6.7). Every key is optional.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `model_hosts` | list of strings | `[]` | Hosts the agent reaches its model and login at. The share's egress proxy allows these, plus the hosts the owner adds. `host`, `host:port` or `*.domain`; without a port, 443 and 80. |
| `install_paths` | list of strings | `[]` | Where the agent's own install may be (`~/` means your home folder). They're readable inside the sandbox and their programs may run. Missing ones are skipped. |
| `programs` | list of strings | `[]` | Programs the agent needs besides its command, such as its runtime (`node`), looked up on PATH. |
| `keychain` | boolean | `false` | The agent keeps its login in the macOS Keychain, so the sandbox lets it reach the Keychain. |
| `own_sandbox_off_args` | list of strings | `[]` | Arguments that turn the agent's own sandbox off (Codex: `["--sandbox", "danger-full-access"]`). Its sandbox can't start inside Pitwall's (Seatbelt doesn't nest), and Pitwall's is the one that holds; the agent's own tool rules stay as a second layer. |
| `own_sandbox_off_env` | table | `{}` | The same as environment (Gemini CLI: `{ GEMINI_SANDBOX = "false" }`). |
| `own_sandbox_off_settings` | table | none | The same as settings, for kinds that take a settings object per launch (`hooks = "claude-settings"`): merged into the `--settings` JSON with the hooks, so it wins over the account's own settings files (Claude Code: `{ sandbox = { enabled = false } }`). |
| `reply` | string | `terminal` | Where a turn's reply is read: `hook` (the `Stop` hook's last message), `session-file`, `non-interactive` or `terminal` (the turn's terminal output). |
| `fork_args` | list of strings | `[]` | Arguments that start a session as a fork of a base conversation: `{base}` is its id, `{session_id}` a new one. Empty: it can't fork, and the approved summary is sent as the first message instead. |
| `instruction_files` | list of strings | `[]` | The files it reads its instructions from (`CLAUDE.md`); the owner's text goes into these in the session folder. Empty: `AGENTS.md`, `CLAUDE.md`, `GEMINI.md` and `QWEN.md`. |
| `limit_screen` | string | none | A regex matching its own usage-limit message; a group named `until` is the reset time it shows. |

```toml
[share]
model_hosts = ["api.example.com"]
install_paths = ["~/.local/share/drift"]
programs = ["node"]
```

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
# Where Claude makes worktrees (its own --worktree and sub-agents), relative to
# the folder it works in; shown under the agent in Pitwall.
worktree_dirs = [".claude/worktrees"]
# What other platforms call it (agw's harness integration), for adopting their sessions.
aliases = ["claude-code"]
# Accounts: each login lives in its own folder (Settings → Agents → Accounts).
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
