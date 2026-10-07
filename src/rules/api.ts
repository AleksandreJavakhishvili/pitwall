// Rules (via rulesync): typed wrapper over the rules commands in
// docs/spec/rules.md, with an in-memory mock for the browser (`pnpm dev`).
// Kept apart from ../api.ts so the rules feature stays self-contained.
import { inTauri } from "../api";

export interface RulesStatus {
  available: boolean;
  via: "rulesync" | "npx" | null;
  version?: string;
  /** User allowed `npx -y rulesync` (Settings). */
  npxAllowed: boolean;
  npxFound: boolean;
}

export interface RuleFile {
  /** `<source>:<path under rules/>` */
  id: string;
  path: string;
  description: string | null;
  targets: string[];
  /** rulesync `root: true` — the tool's main instruction file. */
  root: boolean;
  localRoot: boolean;
  source: string;
}

export interface RuleSet {
  id: string;
  name: string;
  ruleIds: string[];
}

export interface RuleSource {
  name: string;
  kind: "project" | "git";
  origin: string;
  root: string;
}

export type ImportKind = "project" | "file" | "git";
export interface ImportResult {
  added: string[];
  log: string;
}
export interface ApplyResult {
  generated: string[];
  log: string;
}

export interface AgentRules {
  agentId: string;
  ruleSetId: string | null;
  projectRuleSetId: string | null;
  appliedAt: number | null;
  generated: string[];
  error: string | null;
  stale: boolean;
  mainCheckout: boolean;
  mainCheckoutConfirmed: boolean;
}

export interface RulesApi {
  status(): Promise<RulesStatus>;
  setNpx(enabled: boolean): Promise<RulesStatus>;
  library(): Promise<RuleFile[]>;
  revealLibrary(): Promise<string>;
  sets(): Promise<RuleSet[]>;
  saveSet(set: { id?: string; name: string; ruleIds: string[] }): Promise<RuleSet>;
  deleteSet(id: string): Promise<void>;
  importRules(kind: ImportKind, source: string): Promise<ImportResult>;
  sources(): Promise<RuleSource[]>;
  pullSource(name: string): Promise<string>;
  removeSource(name: string): Promise<void>;
  setProjectRules(projectPath: string, ruleSetId: string | null): Promise<void>;
  projectRules(): Promise<Record<string, string>>;
  getProjectRules(projectPath: string): Promise<string | null>;
  apply(agentId: string, confirmMainCheckout?: boolean): Promise<ApplyResult>;
  setAgentRules(agentId: string, ruleSetId: string | null): Promise<void>;
  agentRules(): Promise<AgentRules[]>;
}

async function tauriRulesApi(): Promise<RulesApi> {
  const { invoke } = await import("@tauri-apps/api/core");
  return {
    status: () => invoke("rules_status"),
    setNpx: (enabled) => invoke("set_rules_npx", { enabled }),
    library: () => invoke("list_rule_library"),
    revealLibrary: () => invoke("reveal_rule_library"),
    sets: () => invoke("list_rule_sets"),
    saveSet: (set) => invoke("save_rule_set", { set }),
    deleteSet: (id) => invoke("delete_rule_set", { id }),
    importRules: (kind, source) => invoke("import_rules", { req: { kind, source } }),
    sources: () => invoke("list_rule_sources"),
    pullSource: (name) => invoke("pull_rule_source", { name }),
    removeSource: (name) => invoke("remove_rule_source", { name }),
    setProjectRules: (projectPath, ruleSetId) => invoke("set_project_rules", { projectPath, ruleSetId }),
    projectRules: () => invoke("list_project_rules"),
    getProjectRules: (projectPath) => invoke("get_project_rules", { projectPath }),
    apply: (agentId, confirmMainCheckout) => invoke("apply_rules", { agentId, confirmMainCheckout: confirmMainCheckout ?? null }),
    setAgentRules: (agentId, ruleSetId) => invoke("set_agent_rules", { agentId, ruleSetId }),
    agentRules: () => invoke("agent_rules"),
  };
}

let promise: Promise<RulesApi> | null = null;
function get(): Promise<RulesApi> {
  if (!promise) promise = inTauri ? tauriRulesApi() : import("./mock").then((m) => m.createMockRulesApi());
  return promise;
}

/** Call without awaiting the import first; every method returns a promise. */
export const rulesApi: RulesApi = new Proxy({} as RulesApi, {
  get(_t, prop: string) {
    return (...args: unknown[]) =>
      get().then((a) => (a as unknown as Record<string, (...x: unknown[]) => unknown>)[prop](...args));
  },
});

/** "library:web/style.md" → "web/style.md" */
export function ruleLabel(id: string): string {
  const i = id.indexOf(":");
  return i >= 0 ? id.slice(i + 1) : id;
}
