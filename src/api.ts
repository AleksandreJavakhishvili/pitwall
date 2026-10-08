// Typed wrapper over the Tauri commands/events in docs/CONTRACT.md.
// Outside Tauri (plain `pnpm dev` in a browser) a mock backend is used instead.
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
import { tauriReviewApi, type ReviewApi } from "./reviewTypes";
import { tauriWorktreesApi, type WorktreesApi } from "./worktreesApi";
import { tauriExplorerApi, type ExplorerApi } from "./explorerApi";

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

export const inTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Channel payloads may arrive as ArrayBuffer, Uint8Array or number[]. */
export function toBytes(data: unknown): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  if (Array.isArray(data)) return Uint8Array.from(data as number[]);
  if (typeof data === "string") return new TextEncoder().encode(data);
  return new Uint8Array();
}

/** Turn whatever invoke() rejected with into a readable message. */
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

async function createTauriApi(): Promise<Api> {
  const { invoke, Channel } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
  const label = getCurrentWindow().label;
  return {
    isMock: false,
    ...tauriReviewApi(invoke),
    ...tauriWorktreesApi(invoke),
    ...tauriExplorerApi(invoke),
    listKinds: () => invoke("list_kinds"),
    recentProjects: () => invoke("recent_projects"),
    listAgents: () => invoke("list_agents"),
    createAgent: (req) => invoke("create_agent", { req }),
    listMachines: () => invoke("list_machines"),
    createForm: (provider, machine) => invoke("create_form", { provider, machine }),
    async attachOutput(agentId, onData) {
      let live = true;
      const channel = new Channel<unknown>((msg) => {
        if (live) onData(toBytes(msg));
      });
      const subscriptionId = await invoke<number | null>("attach_output", { agentId, onData: channel });
      return () => {
        live = false;
        if (typeof subscriptionId === "number") {
          invoke("detach_output", { agentId, subscriptionId }).catch(() => {});
        }
      };
    },
    async watchScreen(agentId, onFrame) {
      let live = true;
      const channel = new Channel<ScreenFrame>((frame) => {
        if (live) onFrame(frame);
      });
      const watchId = await invoke<number>("watch_screen", { agentId, onFrame: channel });
      return () => {
        live = false;
        invoke("unwatch_screen", { agentId, watchId }).catch(() => {});
      };
    },
    writeInput: (agentId, data) => invoke("write_input", { agentId, data }),
    resize: (agentId, cols, rows) => invoke("resize", { agentId, cols, rows }),
    sendPrompt: (agentId, text) => invoke("send_prompt", { agentId, text }),
    queueAdd: (agentId, text) => invoke("queue_add", { agentId, text }),
    queueRemove: (agentId, itemId) => invoke("queue_remove", { agentId, itemId }),
    queueSendNow: (agentId, itemId) => invoke("queue_send_now", { agentId, itemId }),
    setAutoSend: (agentId, enabled) => invoke("set_auto_send", { agentId, enabled }),
    markSeen: (agentId) => invoke("mark_seen", { agentId }),
    getChanges: (agentId) => invoke("get_changes", { agentId }),
    refreshChanges: (agentId) => invoke("refresh_changes", { agentId }),
    getFileDiff: (agentId, path, untracked) =>
      invoke("get_file_diff", { agentId, path, untracked }),
    stopAgent: (agentId) => invoke("stop_agent", { agentId }),
    restartAgent: (agentId, size) => invoke("restart_agent", { agentId, cols: size?.cols, rows: size?.rows }),
    removeAgent: (agentId, deleteWorktree) =>
      invoke("remove_agent", { agentId, deleteWorktree }),
    codexHooksStatus: () => invoke("codex_hooks_status"),
    installCodexHooks: () => invoke("install_codex_hooks"),
    listApprovals: () => invoke("list_approvals"),
    answerApproval: (id, allow, remember) => invoke("answer_approval", { id, allow, remember }),
    onApprovalsChanged: (cb) => listen<ApprovalView[]>("approvals-changed", (e) => cb(e.payload)),
    cliStatus: () => invoke("cli_status"),
    installCli: (dir) => invoke("install_cli", { dir }),
    getUiState: () => invoke("get_ui_state"),
    setUiState: (state) => invoke("set_ui_state", { state }),
    openWindow: (spaceId) => invoke("open_window", { spaceId }),
    focusWindow: (label) => invoke("focus_window", { label }),
    listWindows: () => invoke("list_windows"),
    windowLabel: () => label,
    onUiStateChanged: (cb) => listen<UiStateChanged>("ui-state-changed", (e) => cb(e.payload)),
    onWindowClosed: (cb) => listen<{ label: string }>("window-closed", (e) => cb(e.payload)),
    // Window-scoped: the menu targets one window, not every open one.
    onOpenSettings: (cb) => getCurrentWebviewWindow().listen("open-settings", () => cb()),
    onAgentsChanged: (cb) => listen<AgentView[]>("agents-changed", (e) => cb(e.payload)),
    onAttention: (cb) => listen<AttentionEvent>("attention", (e) => cb(e.payload)),
    scanEnvironment: () => invoke("scan_environment"),
    getOnboarded: () => invoke("get_onboarded"),
    listProjects: () => invoke("list_projects"),
    addProject: (path) => invoke("add_project", { path }),
    removeProject: (path) => invoke("remove_project", { path }),
    completeOnboarding: (projects, installCodexHooks) =>
      invoke("complete_onboarding", { projects, installCodexHooks }),
    continueConversation: (req) => invoke("continue_conversation", { ...req }),
    adoptSession: (req) => invoke("adopt_session", { req }),
    listElsewhere: () => invoke("list_elsewhere"),
    onScanProgress: (cb) => listen<ScanProgress>("scan-progress", (e) => cb(e.payload)),
    onProjectsChanged: (cb) => listen<Project[]>("projects-changed", (e) => cb(e.payload)),
    permissionsStatus: () => invoke("permissions_status"),
    openPrivacySettings: (kind) => invoke("open_privacy_settings", { kind }),
    hostInfo: () => invoke("host_info"),
    setWindowGlass: (on) => invoke("set_window_glass", { on }),
    quitApp: (stopAgents) => invoke("quit_app", { stopAgents }),
  };
}

let apiPromise: Promise<Api> | null = null;

export function getApi(): Promise<Api> {
  if (!apiPromise) {
    apiPromise = inTauri
      ? createTauriApi()
      : import("./mock").then((m) => m.createMockApi());
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
