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

How the sidebar groups agents: by project, or by account (the Claude Code or Codex login they run on), then project.

- **Values:** one of project, account
- **Default:** `project`
- **Stored in:** `ui.json` → `sidebarGroup`
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
- **Default:** `Introduce yourself in two lines and offer: set up agw, arrange spaces, create agents per project, set up rules, tune settings.`
- **Stored in:** `ui.json` → `engineerGreeting`
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
