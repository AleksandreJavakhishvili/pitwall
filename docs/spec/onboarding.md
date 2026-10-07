# First launch: auto-detect (read-only scan)

Shown on first launch (state flag `onboarded == false`) and from Settings →
"Scan again". Nothing on the machine changes unless the user ticks a box.

## Step 1 (welcome only, macOS): Folder access
Before the scan, because the scan's first read of a project in Desktop,
Documents or Downloads is what makes macOS ask. Explains why, "Open Settings"
(Privacy & Security → Full Disk Access) with three steps, a live ✓ (re-reads
`permissions_status` every ~2 s and on window focus), "Skip for now" (macOS
then asks per folder, with the Info.plist usage text). Skipped automatically
when already granted, not macOS, or the check fails. Settings → Permissions
shows the same status and button; an "Operation not permitted" error later
gives a one-time hint toast. Nothing reads other agents' folders (the
"Elsewhere" group) before onboarding is finished.

## What the scan finds (backend `scan.rs`, each step independent, failures non-fatal)
1. **Agents**: every known kind on the login-shell PATH, with `--version`.
   Never run an agent beyond `--version`; never answer prompts/dialogs.
2. **Projects** (no full-disk scan), each with sources + last used + isGit:
   - Claude transcripts `~/.claude/projects/*/*.jsonl` (`cwd` field) — exists in `projects.rs`.
   - Codex sessions under `~/.codex/sessions/**.jsonl` (inspect format read-only; take cwd from session meta).
   - VS Code / Cursor recently opened folders (`~/Library/Application Support/{Code,Cursor}/User/globalStorage/` — `storage.json` or `state.vscdb` via the system `sqlite3` CLI, read-only).
   - One level deep in `~/projects ~/code ~/Developer ~/src ~/dev ~/work` if they exist.
   - Do NOT enumerate `~/Desktop ~/Documents ~/Downloads` (macOS permission prompts); paths there that come from the sources above are fine, and "Add folder…" lets the user pick any folder.
3. **Conversations you can continue**: recent Claude sessions per project (session id, first user message as title truncated to ~80 chars, last used); Codex likewise if ids are available.
4. **Agents running elsewhere**: `claude`/`codex` processes not started by Pitwall (pid, cwd via `lsof -a -d cwd -p <pid> -Fn`). Info only — Pitwall can't adopt another app's terminal; offer "Continue in Pitwall" for their session later.
5. **agw**: if `agw` is on PATH → version + `agw session list --output json` count (info only; agw provider comes later). Time-box to ~5s.
6. **Rules**: per selected project, presence of `.rulesync/`, `CLAUDE.md`, `AGENTS.md`.
7. **Codex hooks**: existing `codex_hooks_status`.

## API
- `scan_environment` → `ScanResult` (all of the above). Emits `scan-progress` `{ step: "agents"|"projects"|"conversations"|"running"|"agw"|"rules"|"hooks", status: "running"|"done"|"skipped"|"error", summary?: string }` so the UI checklist fills live.
- Projects become a first-class list (sidebar shows them even with no agents):
  `list_projects` → `Project[] {path, display, isGit, addedAt}`, `add_project(path)`, `remove_project(path)` (never deletes files).
- `complete_onboarding({ projects: string[], installCodexHooks: boolean })` → sets `onboarded = true`, adds projects, installs Codex hooks only if true.
- `continue_conversation({ kind, sessionId, projectPath, name })` → creates an agent that starts with the kind's resume args.
- `get_onboarded` → boolean.

## UI
Welcome screen (full window, Pitwall branding): animated live checklist driven
by `scan-progress`; then project list with checkboxes (recent first, sources as
small chips, "Add folder…"), conversations list with "Continue" buttons,
info rows for running-elsewhere and agw, Codex hooks checkbox with "show
changes" disclosure (explains ~/.codex/hooks.json edit + backup). Buttons: Skip
/ Start Pitwall →. Settings gets "Scan again" (opens the same screen, minus
welcome copy). Mock mode must simulate the scan with staggered progress.

## Revision: "Start Pitwall" sets up agents

Clicking Start Pitwall must leave the user with agents attached to projects,
running and visible — not just a saved project list.
- **Conversations become checkboxes** (no separate Continue buttons). Default:
  the most recent conversation of each ticked project is ticked if it was used
  in the last 3 days. Each ticked conversation → an agent in that project,
  started with the kind's resume args (`continue_conversation`), running in the
  conversation's own cwd (no worktree).
- **Agents running elsewhere** are listed with a "Bring into Pitwall" checkbox,
  default OFF, with a clear note: Pitwall resumes the same conversation in its
  own terminal; close the old terminal first so two copies don't run. Session id
  from the process args (`--resume <id>` / `--session-id <id>`) or the newest
  transcript for that cwd.
- **Projects with nothing ticked** get an optional per-project "Start a new
  agent" toggle with a kind picker (default off).
- Agent names derived from the project (+ short suffix when needed), valid
  `[a-z][a-z0-9_-]{0,31}` and unique.
- On Start: show progress ("Starting 4 agents…"), create them one by one,
  report per-agent failures as toasts but keep going, then open the main
  screen with the new agents in the All space (tiled; overflow as chips),
  grouped under their projects in the sidebar, first one focused.
- Settings → "Scan again" uses the same flow and must skip conversations that
  already have a Pitwall agent (match by session id).
- API additions: `ScannedConversation.inPitwall`, `RunningElsewhere.{sessionId,
  title, inPitwall}` (`inPitwall` = a Pitwall agent already has that session id;
  set by `scan_environment`). Agents are created with the existing
  `continue_conversation` / `create_agent` after `complete_onboarding`.

## Revision: conversations started in the home folder

Conversations whose cwd is `~` (or another non-project folder that the scan
excludes) must not disappear.
- Show them in a "Started in ~" group (same checkbox rows; default unticked).
- Each row has a "Show under project…" picker (existing + detected projects,
  "Add folder…"). The agent appears under the chosen project in the sidebar,
  but it RUNS in the conversation's original cwd — `claude --resume <id>` only
  finds a conversation from the folder it started in. So an agent's display
  project and its cwd can differ: persist both (`project` for grouping, `cwd`
  for launch). Same for Codex.
- The same picker is available for running-elsewhere agents whose cwd is `~`.
- A conversation that is currently running elsewhere (matching session id)
  shows the "two copies" warning and is never ticked by default.
- Remember the user's project choice per session id so Scan again keeps it.
- API: `ScannedConversation.{outsideProject, displayProject, runningElsewhere}`,
  `RunningElsewhere.{outsideProject, displayProject}` (`displayProject` = the
  remembered choice); `continue_conversation({ …, displayProject? })` and
  `CreateAgentRequest.displayProject?` set the agent's `project` while `cwd`
  stays the conversation's folder. Choices live in `~/.pitwall/projects.json`
  (`conversationProjects`: session id → project). `~` itself is never added
  to the project list.

## Revision: agw is part of "agents", not a separate step

agw is where agents run, and agw sessions ARE agents (Claude/Codex/shell on a VM).
- Drop the separate "agw" checklist step. The Agents step summary includes it:
  `Claude 2.1 · Codex 0.160 · agw 0.19 (2 VMs)`.
- "Running now" becomes one list grouped by machine: "This Mac" (today's
  running-elsewhere processes) and one group per agw VM with its sessions
  (session name, harness/kind, workspace, user/agent, running/stopped) from
  `agw session list --output json` (+ `agw vm list --output json` if needed;
  read-only, time-boxed).
- agw rows are shown but not tickable yet: "available with agw support"
  (wave 2 provider). Keep the data shape ready for that (machine id, session
  name, kind) so the provider can reuse it.

## Revision: agw sessions can be added (Wave 2 step 8a)

The agw rows are tickable now: **"Add to Pitwall"** (default off) adopts the
session — Pitwall starts tracking it, attaches to its terminal when it runs
(`agw session attach`), and reads its status from the screen. Nothing changes
on agw. The scan no longer calls agw itself: `ScanResult.places` lists every
provider that can adopt sessions (`caps.attachExisting`), with its machines
and `discover`ed sessions (`inPitwall` by locator), and "Start Pitwall" calls
`adopt_session` for each ticked one after the planned agents. The Agents step
summary reads `agw 0.19.0 (1 machine)`; "Running now" `… · 2 sessions on agw`.
