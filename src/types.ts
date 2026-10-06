// Shapes from docs/CONTRACT.md. Keep in sync with the backend.

export type Status =
  | "working"
  | "blocked"
  | "idle"
  | "done"
  | "unknown"
  | "exited"
  | "stopped";

export interface KindView {
  id: string;
  name: string;
  installed: boolean;
  path?: string;
  /** The agent can work in its own git worktree (its native flag). May be missing on older backends. */
  worktree?: boolean;
}

export interface RecentProject {
  path: string;
  display: string;
  lastUsed: number;
}

export interface QueueItem {
  id: string;
  text: string;
}

export interface AgentView {
  id: string;
  name: string;
  /** What it is now; for a terminal running an agent the user started in it, that agent's kind. */
  kind: string;
  kindName: string;
  /** A plain terminal (kind "shell" when nothing else runs in it; docs/spec/terminals.md). May be missing on older backends. */
  terminal?: boolean;
  /** The conversation it is in, when known (a terminal: its agent's). May be missing on older backends. */
  sessionId?: string | null;
  cwd: string;
  cwdDisplay: string;
  project: string;
  projectDisplay: string;
  branch: string | null;
  /** Works in its own git worktree (`cwd` is inside it). */
  worktree: boolean;
  /** Asked for its own worktree; Pitwall hasn't seen where the agent made it yet. May be missing on older backends. */
  worktreePending?: boolean;
  location: "local";
  status: Status;
  statusSource: "hooks" | "screen" | "activity";
  statusDetail: string | null;
  running: boolean;
  added: number;
  removed: number;
  filesChanged: number;
  queue: QueueItem[];
  autoSend: boolean;
  lastSent: string | null;
  lastSentAt: number | null;
  createdAt: number;
  /** Open task (review.md); null when idle or unknown. May be missing on older backends. */
  currentTaskId?: string | null;
  /** Current PTY size (may be missing on older backends). */
  cols: number;
  rows: number;
}

export interface CreateAgentRequest {
  name: string;
  kind: string;
  projectPath: string;
  customCommand?: string;
  /** Launch with the agent's own worktree flag (kinds with `worktree: true` only). */
  worktree: boolean;
  /** Continue this existing conversation instead of starting fresh. */
  resumeSessionId?: string;
  /** Rules (docs/spec/rules.md): the agent's own extra rule set. */
  ruleSetId?: string;
  /** Rules: OK to write rule files into the main checkout (no worktree). */
  applyToMainCheckout?: boolean;
  /** Group under this project while running in `projectPath` (onboarding: `~` conversations). */
  displayProject?: string;
  /** Terminal size it will be shown at; the process starts at it (else the backend default). */
  cols?: number;
  rows?: number;
}

/** Git status letter as in VS Code's SCM view (from `git diff --name-status`). */
export type FileStatus = "M" | "A" | "D" | "R" | "U";

export interface FileChange {
  path: string;
  added: number;
  removed: number;
  untracked: boolean;
  binary: boolean;
  /**
   * Optional: not reported by the backend yet (TODO(diff-ui): emit it from
   * pitwall-core's vcs once the Exec migration lands). The UI degrades to U/M
   * via `fileStatus()` in lib/fileTree.ts.
   */
  status?: FileStatus;
}

export interface HooksStatus {
  installed: boolean;
  path: string;
}

export interface AttentionEvent {
  agentId: string;
  name: string;
  reason: "blocked" | "done";
  detail?: string;
}

export interface UiStateChanged {
  state: unknown;
  sourceWindow: string;
}

// ── Projects & first-launch scan (docs/spec/onboarding.md) ──────────────────

export interface Project {
  path: string;
  display: string;
  isGit: boolean;
  addedAt: number;
}

/** agw is reported inside "agents" (and its sessions inside "running"). */
export type ScanStep = "agents" | "projects" | "conversations" | "running" | "rules" | "hooks";
export type ScanStepStatus = "running" | "done" | "skipped" | "error";

export interface ScanProgress {
  step: ScanStep;
  status: ScanStepStatus;
  summary?: string;
}

export type ProjectSource = "claude" | "codex" | "vscode" | "cursor" | "folder";

export interface ScannedAgent {
  kind: string;
  name: string;
  installed: boolean;
  path: string | null;
  version: string | null;
}

export interface ScannedProject {
  path: string;
  display: string;
  isGit: boolean;
  /** Unix ms; null when the source has no timestamp (editor recents). */
  lastUsed: number | null;
  sources: ProjectSource[];
  /** Already in Pitwall's project list. */
  added: boolean;
  rules: { rulesync: boolean; claudeMd: boolean; agentsMd: boolean };
}

export interface ScannedConversation {
  kind: string;
  kindName: string;
  sessionId: string;
  projectPath: string;
  projectDisplay: string;
  title: string;
  lastUsed: number;
  /** A Pitwall agent already uses this session. */
  inPitwall: boolean;
  /** Started outside a project (`~` etc.); `projectPath` is still where it resumes. */
  outsideProject: boolean;
  /** Project the user chose to show it under last time (by session id). */
  displayProject: string | null;
  /** A process outside Pitwall runs this session right now. */
  runningElsewhere: boolean;
}

export interface RunningElsewhere {
  pid: number;
  kind: string;
  kindName: string;
  cwd: string | null;
  cwdDisplay: string | null;
  /** From the process args, else the newest transcript for that cwd. */
  sessionId: string | null;
  title: string | null;
  inPitwall: boolean;
  /** cwd isn't a project (`~` etc.); offer "Show under project…". */
  outsideProject: boolean;
  displayProject: string | null;
}

/** One `agw session list` row. Info only until the agw provider (wave 2),
 * which attaches by `machine` + `name`. */
export interface AgwSession {
  /** VM name (`AgwMachine.id`). */
  machine: string;
  /** Session name, unique across agw. */
  name: string;
  /** Pitwall kind id: "claude" | "codex" | "shell", else the harness id. */
  kind: string;
  kindName: string;
  /** agw harness integration ("claude-code", …). */
  harness: string | null;
  workspace: string | null;
  /** Agent (Linux user) it runs as; null = the VM's admin user. */
  agent: string | null;
  status: "running" | "stopped" | "unknown";
}

/** An agw VM and its sessions. */
export interface AgwMachine {
  /** VM name: the id agw commands take (`--vm <id>`). */
  id: string;
  /** vm-site ("lima-local", …). */
  site: string | null;
  /** Live VM state (e.g. "running"); null when unknown. */
  status: string | null;
  sessions: AgwSession[];
}

export interface AgwInfo {
  path: string;
  version: string | null;
  /** null when agw couldn't list its VMs/sessions. */
  machines: AgwMachine[] | null;
}

export interface ScanResult {
  agents: ScannedAgent[];
  projects: ScannedProject[];
  conversations: ScannedConversation[];
  running: RunningElsewhere[];
  agw: AgwInfo | null;
  codexHooks: HooksStatus | null;
}

export interface ContinueConversationRequest {
  kind: string;
  sessionId: string;
  /** Where the agent runs and resumes (the conversation's own cwd). */
  projectPath: string;
  name: string;
  /** Show it under this project in the sidebar instead of `projectPath`. */
  displayProject?: string;
  /** Terminal size it will be shown at; the process starts at it. */
  cols?: number;
  rows?: number;
}
