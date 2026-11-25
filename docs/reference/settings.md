# Settings reference

Every setting Pitwall has, by Settings page. Change them in Settings (⌘,), with `pitwall settings set <key> <value>`, or by editing `~/.pitwall/ui.json`. This page is generated from the same list the app and `pitwall settings list` use.

## How settings apply

- **Live.** Pitwall watches `ui.json`: an edit applies at once, and a value that isn't allowed is reported while the last good one stays. The file also holds spaces, layouts and windows; its settings have a [JSON Schema](https://github.com/AleksandreJavakhishvili/pitwall/blob/main/docs/settings.schema.json) for editors.
- **From the CLI.** `pitwall settings list` shows every key with its value, allowed values and default; `get`, `set` and `reset` work on one. With Pitwall closed, `settings` edits `ui.json` directly (settings kept there only).
- **Approval.** Settings that change something outside Pitwall (installing the CLI link or Codex hooks, running rulesync through npx) ask for your OK in Pitwall when an agent changes them, and need Pitwall running.
- **Kept elsewhere.** A few settings are state Pitwall reads from outside `ui.json` (an installed link, hooks in another tool's file); only a running Pitwall changes them.

## General

### `general.showElsewhere`

Show agents running in other terminal apps as a sidebar group (read-only, checked every ~10 s).

- **Values:** true or false
- **Default:** `true`
- **Stored in:** `ui.json` → `hideElsewhere` (the opposite: `true` there means `false` here)
- **When an agent changes it:** applies directly

### `general.sidebarGroup`

How the sidebar groups agents: by project, or by account (the Claude Code or Codex login they run on; by profile once you have one), then project.

- **Values:** one of project, account
- **Default:** `project`
- **Stored in:** `ui.json` → `sidebarGroup`
- **When an agent changes it:** applies directly

### `general.floatingPrompts`

While Pitwall isn't in front and an agent needs you, show a small floating control above other apps, on every desktop: what each agent asks, with Accept and Reject for a plain yes/no prompt and Open for everything else, and the open questions in the inboxes of extensions that are on, answered right there. Resize it by any edge or corner, or pick Small, Medium or Large in its header.

- **Values:** true or false
- **Default:** `true`
- **Stored in:** `ui.json` → `floatingPrompts`
- **When an agent changes it:** applies directly

### `general.webPages`

Web pages in Pitwall: a page in a tile of the layout next to your terminals (Cmd+L / Ctrl+Shift+L, or Open web page… in the command palette), and an agent's Preview of the dev server its terminal printed, in the system's own web view. Pages keep their logins in Pitwall's own browser accounts, not your browser's. Agents can't see or use these pages; with agents.showWeb they can open one for you to see. false: no web view is made, a Preview opens in your browser instead, and agents can't open pages either.

- **Values:** true or false
- **Default:** `true`
- **Stored in:** `ui.json` → `webPages`
- **When an agent changes it:** applies directly

### `general.devicePreviews`

Device screens in Pitwall: an Android emulator's or phone's screen, or an iOS Simulator's, in a tile of the layout next to your terminals (Open device… in the command palette), where you can tap and type into it. Android needs adb and scrcpy-server; iOS Simulators need Xcode, and touch goes through Xcode's own simulator frameworks in a helper process. Agents can't see or use these screens. false: no device tile opens and none is offered.

- **Values:** true or false
- **Default:** `true`
- **Stored in:** `ui.json` → `devicePreviews`
- **When an agent changes it:** applies directly

### `general.cliTool`

The `pitwall` command-line tool is installed (a link in a folder on PATH). true installs it; delete the link to remove it.

- **Values:** true (turning it off isn't done from Pitwall)
- **Default:** `false`
- **Stored in:** kept by Pitwall, outside `ui.json`
- **When an agent changes it:** asks you in Pitwall first

## Agents

### `agents.codexHooks`

Codex hooks are installed in ~/.codex/hooks.json for exact Codex status (backed up first). true installs them; remove them by hand.

- **Values:** true (turning it off isn't done from Pitwall)
- **Default:** `false`
- **Stored in:** kept by Pitwall, outside `ui.json`
- **When an agent changes it:** asks you in Pitwall first

### `agents.updateTools`

Pitwall keeps agent tools up to date itself (claude update, at startup and every 6 hours, one at a time) and turns off their own updaters in what it starts, so many sessions don't update at once and break the install. false: each tool updates itself.

- **Values:** true or false
- **Default:** `true`
- **Stored in:** `ui.json` → `noToolUpdates` (the opposite: `true` there means `false` here)
- **When an agent changes it:** applies directly

### `agents.showFiles`

Agents can show you files, folders and images in Pitwall's file viewer when you ask them to: Pitwall gives the agents it starts its `pitwall` MCP server per launch where the tool has a way (Claude Code, Codex, Copilot CLI, Qwen Code, Amp, opencode; your own config files are never edited), Aider gets a rule to run `pitwall-cli open`, and `pitwall open` does the same from a shell. Gemini CLI and Cursor Agent only read their own MCP files: see agents.showFilesGemini and agents.showFilesCursor. false: agents start without it and `pitwall open` refuses. Applies when an agent starts.

- **Values:** true or false
- **Default:** `true`
- **Stored in:** `ui.json` → `agentsShowFiles`
- **When an agent changes it:** applies directly

### `agents.showWeb`

Agents can show you a web page in Pitwall when you ask them to ("open the docs on the left", "open a browser window"): the `show_web` tool of Pitwall's `pitwall` MCP server (the agents that get it for files get this too), and `pitwall open --web` from a shell. The page opens in a web tile next to the agent's terminal or a window of its own, in the project's browser account in Pitwall, where you may be signed in; only http and https addresses, shown in the address field. The agent gets nothing back from the page and can't click or type in it. Needs general.webPages. false: the tool is gone from the agents' list and `pitwall open --web` refuses.

- **Values:** true or false
- **Default:** `true`
- **Stored in:** `ui.json` → `agentsShowWeb`
- **When an agent changes it:** applies directly

### `agents.showSettings`

Agents can show you Pitwall's Settings at a page when you ask them to ("where do I turn that on", "show me that extension's permissions"): the `show_settings` tool of Pitwall's `pitwall` MCP server (the agents that get it for files get this too), and `pitwall open --settings` from a shell. Settings opens in its tile next to the agent's terminal, or wherever you keep that tile; it never takes the keyboard or brings Pitwall forward. The agent gets nothing back about what Settings holds and can't change, click or type anything in it. false: the tool is gone from the agents' list and `pitwall open --settings` refuses.

- **Values:** true or false
- **Default:** `true`
- **Stored in:** `ui.json` → `agentsShowSettings`
- **When an agent changes it:** applies directly

### `agents.editFiles`

You can edit text files in Pitwall's file viewer for quick fixes and save them with Cmd+S / Ctrl+S: files of agents on this computer and folders on it, UTF-8 and up to 2 MiB, and an SVG's source. Files of agents on other machines, other images, binary files (PDFs too) and larger files stay read-only. Only you save: agents can't write through Pitwall, and nothing is sent to them. false: the viewer is read-only.

- **Values:** true or false
- **Default:** `true`
- **Stored in:** `ui.json` → `agentsEditFiles`
- **When an agent changes it:** applies directly

### `agents.showFilesGemini`

Pitwall's `pitwall` MCP server is in Gemini CLI's ~/.gemini/settings.json, so Gemini agents can show you files (Gemini has no per-launch way). Only Pitwall's entry is added or taken out; the rest of the file stays as it is, and a backup is kept next to it. true adds it, false takes it out.

- **Values:** true or false
- **Default:** `false`
- **Stored in:** kept by Pitwall, outside `ui.json`
- **When an agent changes it:** asks you in Pitwall first

### `agents.showFilesCursor`

Pitwall's `pitwall` MCP server is in Cursor's ~/.cursor/mcp.json, so Cursor Agent agents can show you files (Cursor Agent has no per-launch way). Only Pitwall's entry is added or taken out; the rest of the file stays as it is, and a backup is kept next to it. true adds it, false takes it out.

- **Values:** true or false
- **Default:** `false`
- **Stored in:** kept by Pitwall, outside `ui.json`
- **When an agent changes it:** asks you in Pitwall first

### `engineer.agent`

What the Race Engineer (Pitwall's optional assistant) runs on: auto (Claude Code if installed, else the first installed agent), an agent id from New agent (claude, codex, gemini, opencode, aider, …) or custom (engineer.command). Applies the next time it starts.

- **Values:** one line of text, up to 64 characters
- **Default:** `auto`
- **Stored in:** `ui.json` → `engineerAgent`
- **When an agent changes it:** applies directly

### `engineer.command`

The command the Race Engineer runs when engineer.agent is custom (a command line, run by your login shell).

- **Values:** one line of text, up to 500 characters
- **Default:** empty
- **Stored in:** `ui.json` → `engineerCommand`
- **When an agent changes it:** asks you in Pitwall first

### `engineer.greeting`

The prompt queued for a new Race Engineer when it first opens (empty: none).

- **Values:** one line of text, up to 1000 characters
- **Default:** `Introduce yourself in two lines and offer: set up agw, arrange spaces, create agents per project, set up rules, set up an extension, tune settings.`
- **Stored in:** `ui.json` → `engineerGreeting`
- **When an agent changes it:** applies directly

## Extensions

### `extensions.restartAfterUpdate`

After an extension's update, what Pitwall does with running agents that use its previous setup (docs/extensions/updates.md): ask (restart nothing; their rows say so, with Restart), whenIdle (restart each once it has waited for you for 30 s, one at a time, continuing its conversation; never one that is working, waiting on an approval or can't continue its conversation) or never. An extension's card can override it.

- **Values:** one of ask, whenIdle, never
- **Default:** `ask`
- **Stored in:** `ui.json` → `extensionsRestartAfterUpdate`
- **When an agent changes it:** applies directly

## Rules & skills

### `rules.allowNpx`

When rulesync isn't installed, run it with `npx -y rulesync` (downloads and runs it from npm).

- **Values:** true or false
- **Default:** `false`
- **Stored in:** kept by Pitwall, outside `ui.json`
- **When an agent changes it:** asks you in Pitwall first

## Appearance

### `appearance.theme`

Light or dark; system follows the OS.

- **Values:** one of system, dark, light
- **Default:** `system`
- **Stored in:** `ui.json` → `theme`
- **When an agent changes it:** applies directly

### `appearance.look`

Flat, or Glass: translucent chrome over the window material (Liquid Glass where the OS has it). Terminals stay solid.

- **Values:** one of flat, glass
- **Default:** `flat`
- **Stored in:** `ui.json` → `look`
- **When an agent changes it:** applies directly

### `appearance.reduceMotion`

Fewer animations (the OS setting also applies).

- **Values:** true or false
- **Default:** `false`
- **Stored in:** `ui.json` → `reduceMotion`
- **When an agent changes it:** applies directly

### `appearance.density`

Smallest tile before extra agents fold into chips (spaces may override it).

- **Values:** one of comfortable, compact, dense
- **Default:** `compact`
- **Stored in:** `ui.json` → `density`
- **When an agent changes it:** applies directly

### `appearance.terminalFontSize`

Base terminal font size in px; tiles may shrink below it to fit.

- **Values:** a whole number 9–22
- **Default:** `13`
- **Stored in:** `ui.json` → `fontSize`
- **When an agent changes it:** applies directly

## Extensions

Extensions (Settings → Extensions): one entry per extension id, with `enabled` (default false: an extension found in the extensions folder of Pitwall's data folder is off until you turn it on). An extension that is on runs an adapter per connection; `selected` is the thread its inbox tile shows, `actionsDone` its setup actions that ran without a failure. An extension's settings live in its connections (accounts of kind `ext:<id>` in `accounts.json`, one per server or key; secrets in the system's secure store, never in a file); `values` and `secretsSet` here are a copy of the connection carried over from before connections, kept for an older Pitwall. Not a `pitwall settings` key: turn extensions on and off in Settings or here.

- **Stored in:** `ui.json` → `extensions.<id>.enabled`
- **More:** [extensions](../spec/extensions.md)
