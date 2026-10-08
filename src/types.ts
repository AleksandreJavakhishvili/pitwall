// Shapes from docs/CONTRACT.md. Keep in sync with the backend.

// Generated from pitwall-proto (ts-rs, `cargo test -p pitwall-proto`):
// these replace the hand-written copies, step by step (architecture.md §6 step 5).
export type { Status } from "./gen/Status";
export type { AgentCaps } from "./gen/AgentCaps";
export type { QueueItem } from "./gen/QueueItem";
export type { ApprovalView } from "./gen/ApprovalView";
export type { Requester } from "./gen/Requester";
export type { CreateForm } from "./gen/CreateForm";
export type { CreateField } from "./gen/CreateField";
export type { CreateChoice } from "./gen/CreateChoice";
export type { NameRule } from "./gen/NameRule";
export type { ProviderMachines } from "./gen/ProviderMachines";
import type { Status } from "./gen/Status";
import type { AgentCaps } from "./gen/AgentCaps";
import type { QueueItem } from "./gen/QueueItem";

/** What a kind can do on the machine new agents go to (architecture.md §3). */
export interface KindCaps {
  /** Its own worktree flag, and the provider can find where it went. */
  worktree: boolean;
  resume: boolean;
  /** Rule files can be generated for it. */
  rules: boolean;
  hooks: boolean;
  /** Runs the command the user types ("Custom command"). */
  customCommand: boolean;
}

export interface KindView {
  id: string;
  name: string;
  installed: boolean;
  path?: string;
  /** Same as `caps.worktree` (kept for older UIs). */
  worktree?: boolean;
  caps: KindCaps;
}

/** Where an agent runs; display and grouping. */
export interface MachineView {
  provider: string;
  id: string;
  label: string;
  /** New agents and terminals can be started there. */
  canCreate: boolean;
}

export interface RecentProject {
  path: string;
  display: string;
  lastUsed: number;
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
  /** The provider it runs on ("local"); display only. */
  location: string;
  machine?: MachineView;
  /** A terminal running an agent the user started in it by hand. */
  agentInTerminal?: boolean;
  /**
   * A terminal that restarts as the agent last started in it (that agent's
   * name): the shell starts and continues it inside. May be missing on older backends.
   */
  restartAs?: string | null;
  caps: AgentCaps;
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
  /** Not needed on a machine whose form has no folder (the platform decides what runs). */
  kind: string;
  projectPath: string;
  /** Where to make it (`listMachines`); omitted → this Mac. */
  provider?: string;
  machine?: string;
  /** That machine's form values (`CreateForm.fields`: id → value). */
  options?: Record<string, string>;
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
  /** From git (`--raw` / `--name-status`); older backends omit it and the UI
   * falls back to U/M via `fileStatus()` in lib/fileTree.ts. */
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

/** Other places (agw) are reported inside "agents" (and their sessions inside "running"). */
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
  /** Agents have worked here (some source is an agent's conversations). */
  agentHistory: boolean;
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

/** A session found on another machine (an agw VM, …) that can be added to
 * Pitwall: `provider` + `machine` + `native` is what `adoptSession` takes. */
export interface ScannedSession {
  provider: string;
  machine: string;
  /** The provider's own handle (an agw session name). */
  native: string;
  name: string;
  /** Pitwall kind it was matched to, else the platform's name. */
  kind: string;
  kindName: string;
  /** The platform's name for what runs there ("claude-code"). */
  program: string;
  workspace: string | null;
  /** The user it runs as when that isn't the machine's main user. */
  user: string | null;
  cwd: string | null;
  status: "running" | "stopped" | "unknown";
  /** A Pitwall agent already tracks it. */
  inPitwall: boolean;
}

export interface ScannedMachine {
  id: string;
  label: string;
  /** Where it lives (an agw vm-site), if known. */
  detail: string | null;
  sessions: ScannedSession[];
}

/** Another place agents run (one provider) and its machines. */
export interface ScannedPlace {
  provider: string;
  label: string;
  version: string | null;
  /** null when its machines couldn't be listed. */
  machines: ScannedMachine[] | null;
}

/** "Add to Pitwall" on a `ScannedSession`. */
export interface AdoptSessionRequest {
  provider: string;
  machine: string;
  native: string;
  /** Terminal size it will be shown at. */
  cols?: number;
  rows?: number;
}

export interface ScanResult {
  agents: ScannedAgent[];
  projects: ScannedProject[];
  conversations: ScannedConversation[];
  running: RunningElsewhere[];
  /** Other places agents run (agw VMs, …) with sessions that can be added. */
  places: ScannedPlace[];
  /** Only when an installed agent takes its hooks from Codex's global file. */
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

/** The `pitwall` command-line tool (Settings → Install command-line tool). */
export interface CliStatus {
  /** The tool shipped with the app; null when this build lacks it. */
  bin: string | null;
  /** An existing `pitwall` link to it. */
  installed: string | null;
  /** Where it can be linked, best first. */
  dirs: { path: string; onPath: boolean; exists: boolean }[];
}

/** macOS privacy permissions (`permissions_status`). Checking never makes macOS ask. */
export type Access = "granted" | "denied" | "unknown";
export interface PermissionsStatus {
  /** This OS guards folders per app (macOS); elsewhere everything is "unknown". */
  applies: boolean;
  fullDiskAccess: Access;
  /** "granted" with Full Disk Access; otherwise "unknown" (macOS asks on first use). */
  desktop: Access;
  documents: Access;
  downloads: Access;
}
/** System Settings panes `open_privacy_settings` can show. */
export type PrivacyPane = "fullDiskAccess" | "filesAndFolders";

/** The modifier of Pitwall's own shortcuts (`host_info`): ⌘, or Ctrl+Shift where Ctrl belongs to the terminal. */
export type ShortcutModifier = "meta" | "ctrlShift";
/** What this desktop offers (`host_info`); the UI decides by these, never by OS. */
export interface HostInfo {
  /** How the UI names this machine ("This Mac", "This computer", "This PC"). */
  machineLabel: string;
  shortcuts: ShortcutModifier;
  /** Pitwall's data folder as shown to the user ("~/.pitwall"). */
  dataDir: string;
  /** A Dock brings hidden windows back. */
  dock: boolean;
  /** A tray icon brings hidden windows back. Without a Dock or a tray, closing the main window quits. */
  tray: boolean;
  /** The native menu: the macOS app menu, a File menu (no Edit accelerators), or none (Settings and Quit in the palette). */
  menu: "app" | "file" | "none";
  /** Where the "agents need you" count shows: Dock badge, or taskbar overlay + tray tooltip. */
  badge: "dock" | "taskbar";
  /** Local sockets: Unix socket files, or per-user named pipes. */
  localSockets: "unix" | "namedPipe";
  /** The Glass look's native window material: macOS vibrancy, Windows 11 Mica, or none ("Glass lite", drawn by the UI). */
  glass: WindowGlass;
}
export type WindowGlass = "vibrancy" | "mica" | "none";
/** `set_window_glass`: whether the native material is on, and the OS's accessibility display options. */
export interface GlassState {
  native: boolean;
  reduceTransparency: boolean;
  increaseContrast: boolean;
}
