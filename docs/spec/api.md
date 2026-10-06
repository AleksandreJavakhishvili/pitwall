# Tauri API: commands, events, types

## Commands (JS names; Rust snake_case args become camelCase in JS)

| Command | Args | Returns |
|---|---|---|
| `list_kinds` | – | `KindView[]` |
| `recent_projects` | – | `RecentProject[]` |
| `list_agents` | – | `AgentView[]` |
| `create_agent` | `req: CreateAgentRequest` | `AgentView` (its project joins the project list if new → `projects-changed`) |
| `attach_output` | `agentId, onData: Channel<ArrayBuffer>` | `number` subscription id (replays buffered output, then streams raw PTY bytes) |
| `detach_output` | `agentId, subscriptionId` | `void` (unknown/restarted agent is not an error) |
| `write_input` | `agentId, data: string` | `void` |
| `resize` | `agentId, cols, rows` | `void` (also remembered on the agent, even while it isn't running, for its next start) |
| `send_prompt` | `agentId, text` | `void` (bracketed paste + Enter) |
| `queue_add` | `agentId, text` | `AgentView` |
| `queue_remove` | `agentId, itemId` | `AgentView` |
| `queue_send_now` | `agentId, itemId` | `AgentView` |
| `set_auto_send` | `agentId, enabled` | `AgentView` |
| `mark_seen` | `agentId` | `void` |
| `get_changes` | `agentId` | `FileChange[]` |
| `get_file_diff` | `agentId, path, untracked` | `string` (unified diff) |
| `stop_agent` | `agentId` | `void` |
| `restart_agent` | `agentId, cols?, rows?` | `AgentView` (resumes the session when the kind supports it; starts at `cols`×`rows`, else the agent's last known size) |
| `continue_conversation` | `kind, sessionId, projectPath, name, displayProject?, cols?, rows?` | `AgentView` (see onboarding.md) |
| `remove_agent` | `agentId, deleteWorktree` | `void` (`deleteWorktree`: `git worktree remove` of the agent's worktree, no `--force`; the branch is kept. Only sent after the user ticks "Also remove worktree") |
| `codex_hooks_status` | – | `{ installed: boolean, path: string }` |
| `install_codex_hooks` | – | `{ installed: boolean, path: string }` (only called after explicit user approval in UI) |

Windows and shared UI state ([layout.md](layout.md)):

| Command | Args | Returns |
|---|---|---|
| `get_ui_state` | – | `unknown` (shared layout blob, `null` before first save) |
| `set_ui_state` | `state` | `void` (→ `ui-state-changed`) |
| `open_window` | `spaceId` | `string` new window label (`pitwall-*`) |
| `focus_window` | `label` | `void` |
| `list_windows` | – | `string[]` open window labels |

Onboarding and project list ([onboarding.md](onboarding.md)):

| Command | Args | Returns |
|---|---|---|
| `scan_environment` | – | `ScanResult` (read-only; emits `scan-progress`) |
| `get_onboarded` | – | `boolean` |
| `list_projects` | – | `Project[]` |
| `add_project` | `path` | `Project[]` (→ `projects-changed`) |
| `remove_project` | `path` | `Project[]` (drops it from the list, never deletes files) |
| `complete_onboarding` | `projects: string[], installCodexHooks` | `Project[]` |
| `continue_conversation` | `kind, sessionId, projectPath, name, displayProject?, cols?, rows?` | `AgentView` (resumes that session; adds its project like `create_agent`) |

Review ([review.md](review.md)):

| Command | Args | Returns |
|---|---|---|
| `list_tasks` | `agentId` | `Task[]` |
| `get_task_changes` | `agentId, taskId?` | `FileChange[]` (no `taskId`: whole agent diff) |
| `get_file_versions` | `agentId, path, taskId?` | `FileVersions` (before/after text for Monaco) |
| `discard_file` | `agentId, path` | `void` |
| `commit_agent` | `agentId, message` | `string` commit hash |
| `merge_agent` | `agentId` | `MergeResult` |
| `get_merge_status` | `agentId` | `MergeStatus` |

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
| `open-settings` | – | native menu "Pitwall → Settings… ⌘," (sent only to the focused window, else main) |

## TypeScript shapes

```ts
type Status = "working"|"blocked"|"idle"|"done"|"unknown"|"exited"|"stopped";
interface KindView { id: string; name: string; installed: boolean; path?: string;
  worktree: boolean } // the kind's CLI has its own worktree flag (`worktree_args`)
interface RecentProject { path: string; display: string; lastUsed: number }
interface QueueItem { id: string; text: string }
interface AgentView {
  id: string; name: string; kind: string; kindName: string;
  cwd: string; cwdDisplay: string; project: string; projectDisplay: string;
  branch: string | null; worktree: boolean; location: "local";
  worktreePending: boolean; // asked for its own worktree, not found yet (worktrees.md)
  status: Status; statusSource: "hooks"|"screen"|"activity"; statusDetail: string | null;
  running: boolean; added: number; removed: number; filesChanged: number;
  queue: QueueItem[]; autoSend: boolean; lastSent: string | null; lastSentAt: number | null;
  createdAt: number;
}
interface CreateAgentRequest {
  name: string; kind: string; projectPath: string;
  customCommand?: string;
  worktree: boolean; // launch with the kind's own worktree flag (worktrees.md); error if unsupported
  resumeSessionId?: string; ruleSetId?: string; applyToMainCheckout?: boolean; displayProject?: string;
  cols?: number; rows?: number; // size it will be shown at
}
interface FileChange { path: string; added: number; removed: number; untracked: boolean; binary: boolean }
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

