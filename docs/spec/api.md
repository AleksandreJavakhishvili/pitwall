# Tauri API: commands, events, types

> **History.** This was the Tauri shell's interface (up to v0.1.x). The GPUI
> app (v0.2.0+) has no IPC layer: it calls the same `pitwall-core` functions
> directly (docs/spec/gpui/README.md "Commands and events"). The web demo's
> mock (`src/mock.ts`) still implements these shapes.

## Commands (JS names; Rust snake_case args become camelCase in JS)

| Command | Args | Returns |
|---|---|---|
| `list_kinds` | – | `KindView[]` |
| `recent_projects` | – | `RecentProject[]` |
| `list_agents` | – | `AgentView[]` |
| `create_agent` | `req: CreateAgentRequest` | `AgentView` (its project joins the project list if new → `projects-changed`) |
| `attach_output` | `agentId, onData: Channel<ArrayBuffer>` | `number` subscription id (replays buffered output, then streams raw PTY bytes) |
| `detach_output` | `agentId, subscriptionId` | `void` (unknown/restarted agent is not an error) |
| `watch_screen` | `agentId, onFrame: Channel<ScreenFrame>` | `number` watch id (the agent's screen as styled text for Wall tiles: whole screen first, then changed rows, ≤ 10 frames/s; wall.md) |
| `unwatch_screen` | `agentId, watchId` | `void` (unknown/restarted agent is not an error) |
| `write_input` | `agentId, data: string` | `void` |
| `resize` | `agentId, cols, rows` | `void` (also remembered on the agent, even while it isn't running, for its next start) |
| `send_prompt` | `agentId, text` | `void` (bracketed paste + Enter) |
| `queue_add` | `agentId, text` | `AgentView` |
| `queue_remove` | `agentId, itemId` | `AgentView` |
| `queue_send_now` | `agentId, itemId` | `AgentView` |
| `set_auto_send` | `agentId, enabled` | `AgentView` |
| `mark_seen` | `agentId` | `void` |
| `get_changes` | `agentId` | `FileChange[]` |
| `refresh_changes` | `agentId` | `FileChange[]`. Reads the agent's changes and branch now, bypassing the ticker's polling pace (back-off, a slow provider's `git_poll_ms`, idle agents); a refresh already running for the agent is awaited, not repeated. Through the agent's machine (`Exec`); the agent's numbers update via `agents-changed`. The UI calls it when Changes or Review opens, an agent is picked in Review, or the user refreshes (↻, ⌘⇧R) |
| `get_file_diff` | `agentId, path, untracked` | `string` (unified diff) |
| `stop_agent` | `agentId` | `void` |
| `restart_agent` | `agentId, cols?, rows?` | `AgentView` (resumes the session when the kind supports it; starts at `cols`×`rows`, else the agent's last known size) |
| `continue_conversation` | `kind, sessionId, projectPath, name, displayProject?, cols?, rows?` | `AgentView` (see onboarding.md) |
| `remove_agent` | `agentId, deleteWorktree` | `void` (`deleteWorktree`: `git worktree remove` of the agent's worktree, no `--force`; the branch is kept. Only sent after the user ticks "Also remove worktree") |
| `codex_hooks_status` | – | `{ installed: boolean, path: string }` |
| `install_codex_hooks` | – | `{ installed: boolean, path: string }` (only called after explicit user approval in UI) |

Command-line tool and approvals (architecture.md §4; the `pitwall` CLI talks to the app's socket `~/.pitwall/run/pitwalld.sock`, see `crates/pitwall-proto`):

| Command | Args | Returns |
|---|---|---|
| `list_approvals` | – | `ApprovalView[]` (requests from the CLI waiting for the user; `src/gen/ApprovalView.ts`) |
| `answer_approval` | `id, allow, remember` | `void` (only Pitwall's own window answers; `remember` only counts when `rememberable`) |
| `cli_status` | – | `CliStatus` (`bin`, `installed` link, candidate `dirs`) |
| `install_cli` | `dir` | `CliStatus` (links `<dir>/pitwall` → the shipped `pitwall-cli`; only after the user agreed in Settings; never replaces a `pitwall` that isn't Pitwall's) |

Windows and shared UI state ([layout.md](layout.md)):

| Command | Args | Returns |
|---|---|---|
| `get_ui_state` | – | `unknown` (shared layout blob, `null` before first save) |
| `set_ui_state` | `state` | `void` (→ `ui-state-changed`) |
| `open_window` | `spaceId` | `string` new window label (`pitwall-*`) |
| `focus_window` | `label` | `void` |
| `list_windows` | – | `string[]` open window labels |

macOS privacy (onboarding.md "Folder access"; core `permissions.rs`, probe in `platform`):

| Command | Args | Returns |
|---|---|---|
| `permissions_status` | – | `PermissionsStatus { applies, fullDiskAccess, desktop, documents, downloads }`, each `"granted"\|"denied"\|"unknown"`. Read-only and never makes macOS ask: it opens items only Full Disk Access unlocks (`~/Library/Application Support/com.apple.TCC/TCC.db`, `~/Library/Safari`), which have no consent prompt. Desktop/Documents/Downloads are never probed (that would prompt): `granted` with Full Disk Access, else `unknown` |
| `open_privacy_settings` | `kind: "fullDiskAccess"\|"filesAndFolders"` | `void` (shows that System Settings pane; changes nothing) |
| `host_info` | – | `HostInfo { machineLabel, shortcuts: "meta"\|"ctrlShift", dataDir, dock, tray, menu: "app"\|"file"\|"none", badge: "dock"\|"taskbar", localSockets: "unix"\|"namedPipe" }`: what this desktop offers (macOS: ⌘, Dock, app menu; Linux: Ctrl+Shift, no menu, no Dock/tray; Windows: Ctrl+Shift, tray, File menu, taskbar overlay badge, named pipes). The UI and the app shell decide by these, never by OS: app shortcuts are ⌘ or Ctrl+Shift (⌘⇧ → Ctrl+Shift+Alt; terminals copy/paste with Ctrl+Shift+C/V there), labels follow; the menu bar follows `menu` (no Edit accelerators off ⌘); closing main hides it when a Dock or tray brings it back (else quits). Loaded once before the first render; macOS values if it fails |
| `quit_app` | `stopAgents: bool` | `void` — quits (command palette); agents keep running in their holders unless `stopAgents` |

Onboarding and project list ([onboarding.md](onboarding.md)):

| Command | Args | Returns |
|---|---|---|
| `scan_environment` | – | `ScanResult` (read-only; emits `scan-progress`). `projects[].agentHistory`: some source is an agent's conversations; `codexHooks` is set only when an installed agent takes its hooks from Codex's global file; `places`: other places agents run (agw), from each adopting provider's `discover` (below) |
| `get_onboarded` | – | `boolean` |
| `list_projects` | – | `Project[]` |
| `add_project` | `path` | `Project[]` (→ `projects-changed`) |
| `remove_project` | `path` | `Project[]` (drops it from the list, never deletes files) |
| `complete_onboarding` | `projects: string[], installCodexHooks` | `Project[]` |
| `continue_conversation` | `kind, sessionId, projectPath, name, displayProject?, cols?, rows?` | `AgentView` (resumes that session; adds its project like `create_agent`) |
| `adopt_session` | `req: { provider, machine, native, cols?, rows? }` (a `ScannedSession`'s ids) | `AgentView`. "Add to Pitwall": tracks an existing session (agw) — attaches if it runs, else shows it stopped. Nothing changes on that machine. Idempotent: the same session again returns the same agent. Error if the provider can't adopt or the session is gone |

```ts
interface ScannedPlace { provider: string; label: string; version: string | null;
  machines: ScannedMachine[] | null } // null: couldn't be listed
interface ScannedMachine { id: string; label: string; detail: string | null; sessions: ScannedSession[] }
interface ScannedSession { provider: string; machine: string; native: string; // what adopt_session takes
  name: string; kind: string; kindName: string; // matched Pitwall kind (id or alias), else the platform's name
  program: string; workspace: string | null; user: string | null; cwd: string | null;
  status: "running"|"stopped"|"unknown"; inPitwall: boolean }
```

Review ([review.md](review.md)):

| Command | Args | Returns |
|---|---|---|
| `list_tasks` | `agentId` | `Task[]` |
| `get_task_changes` | `agentId, taskId?` | `FileChange[]` (no `taskId`: whole agent diff) |
| `get_file_versions` | `agentId, path, taskId?` | `FileVersions` (before/after text for the diff editor) |
| `discard_file` | `agentId, path` | `void` |
| `commit_agent` | `agentId, message` | `string` commit hash |
| `merge_agent` | `agentId` | `MergeResult` |
| `get_merge_status` | `agentId` | `MergeStatus` |

Worktrees in source control ([worktrees-view.md](worktrees-view.md)); a worktree is named by its project's `id` and its `path` from `list_worktrees` (any other path is refused):

| Command | Args | Returns |
|---|---|---|
| `list_worktrees` | – | `ProjectWorktrees[]` (`src/gen/ProjectWorktrees.ts`). One `git worktree list --porcelain` per project, and only when due: something about its agents changed (at most every 2 s, or the provider's `git_poll_ms`) or the last list is ~30 s old; otherwise the cached list, re-attributed |
| `refresh_worktrees` | `projectId?` | `ProjectWorktrees[]`, with that project (all, when omitted) listed again now whatever its age or its machine's pace; asked again while one runs, it waits for that one |
| `get_worktree_changes` | `projectId, path` | `FileChange[]` against the merge-base with the project's current branch (committed, uncommitted, untracked) |
| `get_worktree_file_versions` | `projectId, path, file` | `FileVersions` |
| `get_worktree_merge_status` | `projectId, path` | `MergeStatus` |
| `commit_worktree` | `projectId, path, message` | `string` short commit id (`git add -A && git commit`) |
| `merge_worktree` | `projectId, path` | `MergeResult` (Review merge rules: dirty main checkout refused, conflicts aborted; detached refused) |
| `remove_worktree` | `projectId, path` | `void` (`git worktree remove`, never `--force`; locked worktrees and agents' own folders refused; branch kept) |

```ts
interface ProjectWorktrees { id: string; repo: string; repoDisplay: string;
  branch: string | null; // main checkout's branch (merge target)
  machine: MachineView; agentIds: string[]; worktrees: WorktreeView[];
  error: string | null } // last list failed (the previous one is kept)
interface WorktreeView { path: string; pathDisplay: string; name: string;
  branch: string | null; head: string | null; locked: boolean; lockReason: string | null;
  prunable: boolean; agentId: string | null;
  via: "own" | "toolDir" | "process" | "other"; caps: WorktreeCaps }
interface WorktreeCaps { diff: boolean; commit: boolean; merge: boolean; remove: boolean; terminal: boolean }
```

Explorer, read-only ([explorer.md](explorer.md); `src/explorerApi.ts`, types in `src/gen/`). Paths are relative to the agent's folder and `/`-separated. Absolute paths, `..` and symlinks leading outside the folder are refused. Everything runs on the agent's machine (`Exec`):

| Command | Args | Returns |
|---|---|---|
| `list_files` | `agentId, dir?, ignored?` | `DirListing { dir, entries: FileEntry[], truncated, git }`. One folder (lazy tree). In a repository this is git's view (`ls-files` cached + others, ignore files respected), with the Changes panel's letters (`status`) and per-folder `changes` counts. `ignored: true` ("Show ignored files") lists what git ignores too, marked `ignored` (inside an ignored folder everything is). Otherwise a plain listing without `.git`, `.DS_Store` and the like. Folders first, max 5 000 |
| `list_all_files` | `agentId` | `FileIndex { files, truncated, git }` for quick open (git, else `rg --files`, else a 3 s walk; max 100 000) |
| `read_file` | `agentId, path, large?` | `FileView { path, size, kind: "text"\|"binary"\|"tooLarge", text, lang }` (text ≤ 2 MiB, ≤ 10 MiB with `large` — "Load anyway"; binary = NUL in the first 8 000 bytes or a binary extension) |
| `search_files` | `agentId, query: SearchQuery` | `SearchResult { matches: SearchMatch[], files, truncated, engine: "ripgrep"\|"gitGrep" }`. `rg --json` if installed there, else `git grep`. 100 matches per file, 2 000 total by default (`maxResults` ≤ 10 000). `node_modules` and `bower_components` are left out unless `defaultExcludes: false`. Ranges are in UTF-16 units of `text`. A newer search for the agent cancels this one (error `"cancelled"`) |
| `cancel_search` | `agentId` | `void` |

Rules ([rules.md](rules.md)):

| Command | Args | Returns |
|---|---|---|
| `rules_status` | – | `RulesStatus` |
| `set_rules_npx` | `enabled` | `RulesStatus` |
| `list_rule_library` | – | `RuleFile[]` |
| `reveal_rule_library` | – | `string` library path |
| `list_rule_sets` | – | `RuleSet[]` |
| `save_rule_set` | `set: SaveRuleSet` | `RuleSet` |
| `delete_rule_set` | `id` | `void` |
| `import_rules` | `req: ImportRequest` | `ImportResult` |
| `list_rule_sources` | – | `RuleSource[]` |
| `pull_rule_source` | `name` | `string` |
| `remove_rule_source` | `name` | `void` |
| `set_project_rules` | `projectPath, ruleSetId?` | `void` |
| `list_project_rules` | – | `Record<projectPath, ruleSetId>` |
| `get_project_rules` | `projectPath` | `string \| null` |
| `apply_rules` | `agentId, confirmMainCheckout?` | `ApplyResult` |
| `set_agent_rules` | `agentId, ruleSetId?` | `void` |
| `agent_rules` | – | `AgentRulesView[]` |

Plugins used from JS: `@tauri-apps/plugin-dialog` `open({ directory: true })`
(native folder picker; permission `dialog:allow-open`), notification, opener.

Events (Tauri `listen`):

| Event | Payload | When |
|---|---|---|
| `agents-changed` | `AgentView[]` | full list on any change, max ~4/s |
| `attention` | `{ agentId, name, reason: "blocked" \| "done", detail?: string }` | an agent needs you / finished |
| `ui-state-changed` | `{ state, sourceWindow }` | after `set_ui_state` |
| `window-closed` | `{ label }` | a secondary window closed |
| `scan-progress` | `ScanProgress` | each step of `scan_environment` |
| `projects-changed` | `Project[]` | project list changed |
| `approvals-changed` | `ApprovalView[]` | a CLI request started or stopped waiting for the user |
| `open-settings` | – | native menu "Pitwall → Settings… ⌘," (sent only to the focused window, else main) |

## TypeScript shapes

```ts
type Status = "working"|"blocked"|"idle"|"done"|"unknown"|"exited"|"stopped";
interface KindView { id: string; name: string; installed: boolean; path?: string;
  worktree: boolean; // = caps.worktree (kept for older UIs)
  caps: KindCaps }
// What a kind can do on the machine new agents go to: its definition combined
// with the provider's capabilities (architecture.md §3). "Custom command" is
// listed only when the provider runs typed commands (caps.customCommand).
interface KindCaps { worktree: boolean; resume: boolean; rules: boolean; hooks: boolean; customCommand: boolean }
interface RecentProject { path: string; display: string; lastUsed: number }
interface QueueItem { id: string; text: string }
interface AgentView {
  id: string; name: string; kind: string; kindName: string;
  cwd: string; cwdDisplay: string; project: string; projectDisplay: string;
  branch: string | null; worktree: boolean;
  location: string; // provider id ("local"); display only
  machine: { provider: string; id: string; label: string; canCreate: boolean }; // display and grouping;
                           // canCreate: "New agent/terminal here" is offered for its projects
  terminal: boolean; agentInTerminal: boolean; // a terminal, running an agent started by hand
  restartAs: string | null; // a terminal that restarts as the agent last started in it (its name; terminals.md §3)
  caps: AgentCaps; // the only thing the UI decides features by (below)
  worktreePending: boolean; // asked for its own worktree, not found yet (worktrees.md)
  status: Status; statusSource: "hooks"|"screen"|"activity"; statusDetail: string | null;
  running: boolean; added: number; removed: number; filesChanged: number;
  queue: QueueItem[]; autoSend: boolean; lastSent: string | null; lastSentAt: number | null;
  createdAt: number;
}
// architecture.md §3. The UI never asks which kind or provider an agent has
// (src/lib/noSpecialCases.test.ts enforces it).
interface AgentCaps {
  input: boolean;          // running and attached
  restart: boolean;        // provider can start it again (caps.start; agw: `agw session start`)
  resume: boolean;         // a restart continues its conversation (provider + kind can, and it has one)
  stop: boolean;           // running
  removeWorktree: boolean; // provider runs commands there && it has a worktree
  diff: boolean;           // provider runs commands there && cwd is a git repo (unknown counts as yes);
                           // false also stops git polling for the agent
  review: boolean;         // = diff
  merge: boolean;          // diff && worktree && on a branch
  rules: boolean;          // provider.rules && kind has a rulesync target
  hooks: boolean;          // provider delivers hooks && kind has hooks
  worktrees: boolean;      // its project's worktrees can be listed (= diff); missing from older servers = false
  explorer: boolean;       // provider runs commands there: the read-only code explorer (explorer.md); missing = false
  removeKeepsSession: boolean; // adopted (agw): remove only stops tracking, the session keeps running
}
interface CreateAgentRequest {
  name: string; kind: string; projectPath: string;
  customCommand?: string;
  worktree: boolean; // launch with the kind's own worktree flag (worktrees.md); error if unsupported
  resumeSessionId?: string; ruleSetId?: string; applyToMainCheckout?: boolean; displayProject?: string;
  cols?: number; rows?: number; // size it will be shown at
}
interface FileChange { path: string; added: number; removed: number; untracked: boolean; binary: boolean;
  status?: "M"|"A"|"D"|"R"|"U" } // git's letter (`diff --raw -M` / `--name-status`); renames carry the new path
```
All Rust structs crossing the boundary use `#[serde(rename_all = "camelCase")]`.

**Spawn size.** A process starts at the terminal size it will be shown at, so
its first frames (replayed into xterm on attach) aren't wrapped for a wider
terminal. `create_agent`, `continue_conversation` and `restart_agent` take
optional `cols`/`rows` (both or neither; clamped to 2–1000). Without them a
new agent starts at 120×40 and a restart at the agent's last size from
`resize` (persisted on the agent record). The UI passes: on restart, the
terminal's fitted size; for a new agent, the size of the pane it will land in
(first empty pane of the active space, else the focused one), falling back to
the focused terminal's size, then the last fitted size of any terminal. After
attach the backend still nudges a redraw (resize rows−1, then back).

