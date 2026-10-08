// Rules (via rulesync): the rules interface of docs/spec/rules.md, backed
// by an in-memory mock (the web demo has no backend; src/README.md).
// Kept apart from ../api.ts so the rules feature stays self-contained.

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

let promise: Promise<RulesApi> | null = null;
function get(): Promise<RulesApi> {
  if (!promise) promise = import("./mock").then((m) => m.createMockRulesApi());
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
