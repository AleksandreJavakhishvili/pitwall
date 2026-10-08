// The UI's backend interface (docs/CONTRACT.md). This React UI is the
// website's live demo only (src/README.md): the backend is always the
// in-browser mock (`./mock`). The desktop app is crates/pitwall-app (GPUI).
import type {
  UiStateChanged,
  AdoptSessionRequest,
  ContinueConversationRequest,
  Project,
  ScanProgress,
  ScanResult,
  AgentView,
  ApprovalView,
  AttentionEvent,
  CliStatus,
  PermissionsStatus,
  PrivacyPane,
  HostInfo,
  GlassState,
  CreateAgentRequest,
  CreateForm,
  FileChange,
  HooksStatus,
  KindView,
  ProviderMachines,
  RecentProject,
  RunningElsewhere,
} from "./types";

import type { ScreenFrame } from "./gen/ScreenFrame";
import type { ReviewApi } from "./reviewTypes";
import type { WorktreesApi } from "./worktreesApi";
import type { ExplorerApi } from "./explorerApi";

export type Unlisten = () => void;

export interface Api extends ReviewApi, WorktreesApi, ExplorerApi {
  isMock: boolean;
  listKinds(): Promise<KindView[]>;
  recentProjects(): Promise<RecentProject[]>;
  listAgents(): Promise<AgentView[]>;
  createAgent(req: CreateAgentRequest): Promise<AgentView>;
  /** Providers and their machines (New agent → "Runs on": those with `canCreate`). */
  listMachines(): Promise<ProviderMachines[]>;
  /** How new agents are made on one machine: Pitwall's folder form, or a platform's own fields and choices. */
  createForm(provider: string, machine: string): Promise<CreateForm>;
  /** Replays buffered output, then streams raw PTY bytes. Returns a detach fn (local only). */
  attachOutput(agentId: string, onData: (bytes: Uint8Array) => void): Promise<Unlisten>;
  /** Styled frames of the agent's screen (the Wall's view-only tiles): the whole
   * screen first, then changed rows, ≤ ~10/s. Returns an unwatch fn. */
  watchScreen(agentId: string, onFrame: (frame: ScreenFrame) => void): Promise<Unlisten>;
  writeInput(agentId: string, data: string): Promise<void>;
  resize(agentId: string, cols: number, rows: number): Promise<void>;
  sendPrompt(agentId: string, text: string): Promise<void>;
  queueAdd(agentId: string, text: string): Promise<AgentView>;
  queueRemove(agentId: string, itemId: string): Promise<AgentView>;
  queueSendNow(agentId: string, itemId: string): Promise<AgentView>;
  setAutoSend(agentId: string, enabled: boolean): Promise<AgentView>;
  markSeen(agentId: string): Promise<void>;
  getChanges(agentId: string): Promise<FileChange[]>;
  /** Its changes and branch read now, bypassing the backend's polling pace (back-off,
   * slow machines); one already running is awaited. Totals follow via agents-changed. */
  refreshChanges(agentId: string): Promise<FileChange[]>;
  getFileDiff(agentId: string, path: string, untracked: boolean): Promise<string>;
  stopAgent(agentId: string): Promise<void>;
  /** `size`: what the terminal shows now; omitted → the agent's last known size. */
  restartAgent(agentId: string, size?: { cols: number; rows: number }): Promise<AgentView>;
  removeAgent(agentId: string, deleteWorktree: boolean): Promise<void>;
  codexHooksStatus(): Promise<HooksStatus>;
  installCodexHooks(): Promise<HooksStatus>;
  /** Requests from the `pitwall` CLI waiting for the user (Pitwall's approval dialog). */
  listApprovals(): Promise<ApprovalView[]>;
  /** The user's answer; only Pitwall's own window can give it. */
  answerApproval(id: string, allow: boolean, remember: boolean): Promise<void>;
  onApprovalsChanged(cb: (list: ApprovalView[]) => void): Promise<Unlisten>;
  /** The `pitwall` command-line tool: shipped binary and where it is linked. */
  cliStatus(): Promise<CliStatus>;
  /** Link `pitwall` into `dir` (one of `cliStatus().dirs`), after the user agreed. */
  installCli(dir: string): Promise<CliStatus>;
  getUiState(): Promise<unknown>;
  setUiState(state: unknown): Promise<void>;
  /** Opens a new OS window showing `spaceId`; returns its label. */
  openWindow(spaceId: string): Promise<string>;
  focusWindow(label: string): Promise<void>;
  listWindows(): Promise<string[]>;
  /** This window's label ("main" for the first window). */
  windowLabel(): string;
  onUiStateChanged(cb: (e: UiStateChanged) => void): Promise<Unlisten>;
  onWindowClosed(cb: (e: { label: string }) => void): Promise<Unlisten>;
  /** Native menu "Pitwall → Settings…" (sent to the focused window only). */
  onOpenSettings(cb: () => void): Promise<Unlisten>;
  onAgentsChanged(cb: (agents: AgentView[]) => void): Promise<Unlisten>;
  onAttention(cb: (e: AttentionEvent) => void): Promise<Unlisten>;

  // first-launch scan + project list (docs/spec/onboarding.md)
  /** Read-only scan; emits scan-progress as each step runs. */
  scanEnvironment(): Promise<ScanResult>;
  getOnboarded(): Promise<boolean>;
  listProjects(): Promise<Project[]>;
  addProject(path: string): Promise<Project[]>;
  /** Drops it from the list; never deletes files. */
  removeProject(path: string): Promise<Project[]>;
  completeOnboarding(projects: string[], installCodexHooks: boolean): Promise<Project[]>;
  continueConversation(req: ContinueConversationRequest): Promise<AgentView>;
  /** "Add to Pitwall" on a session found on another machine (`ScanResult.places`).
   * Nothing changes there; a running session is attached. Adopting twice returns the same agent. */
  adoptSession(req: AdoptSessionRequest): Promise<AgentView>;
  /** Agents running in other terminal apps (read-only), minus conversations Pitwall already has. */
  listElsewhere(): Promise<RunningElsewhere[]>;
  onScanProgress(cb: (e: ScanProgress) => void): Promise<Unlisten>;
  onProjectsChanged(cb: (projects: Project[]) => void): Promise<Unlisten>;

  // macOS privacy (welcome screen "Folder access", Settings → Permissions)
  /** Full Disk Access and folder access; read-only, never makes macOS ask. */
  permissionsStatus(): Promise<PermissionsStatus>;
  /** Shows that System Settings pane; the user changes it there. */
  openPrivacySettings(kind: PrivacyPane): Promise<void>;

  /** What this desktop offers: shortcut modifier, machine label, data folder (`lib/host.ts`). */
  hostInfo(): Promise<HostInfo>;
  /** Settings → Appearance → Look: the native window material behind this window (`lib/look.ts`). */
  setWindowGlass(on: boolean): Promise<GlassState>;
  /** Quit Pitwall; agents keep running in their holders unless `stopAgents`. */
  quitApp(stopAgents: boolean): Promise<void>;
}

/** Turn whatever a call rejected with into a readable message. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

let apiPromise: Promise<Api> | null = null;

export function getApi(): Promise<Api> {
  if (!apiPromise) {
    apiPromise = import("./mock").then((m) => m.createMockApi());
  }
  return apiPromise;
}

/**
 * Synchronous proxy so components can call `api.foo()` without awaiting the
 * dynamic import first. Every method returns a promise anyway.
 */
export const api: Omit<Api, "isMock" | "windowLabel"> = new Proxy({} as Omit<Api, "isMock" | "windowLabel">, {
  get(_t, prop: string) {
    return (...args: unknown[]) =>
      getApi().then((a) => (a as unknown as Record<string, (...x: unknown[]) => unknown>)[prop](...args));
  },
});
