// In-memory rules backend for the browser mock (`pnpm dev`).
import type { AgentRules, RuleFile, RuleSet, RuleSource, RulesApi, RulesStatus } from "./api";

const delay = <T,>(v: T, ms = 120) => new Promise<T>((r) => setTimeout(() => r(v), ms));

export function createMockRulesApi(): RulesApi {
  let status: RulesStatus = { available: false, via: null, npxAllowed: false, npxFound: true };
  const lib: RuleFile[] = [
    { id: "library:typescript.md", path: "~/.pitwall/rules/rules/typescript.md", description: "TypeScript style", targets: ["*"], root: false, localRoot: false, source: "library" },
    { id: "library:testing.md", path: "~/.pitwall/rules/rules/testing.md", description: "Run tests before saying done", targets: ["*"], root: false, localRoot: false, source: "library" },
    { id: "library:overview.md", path: "~/.pitwall/rules/rules/overview.md", description: "Team overview", targets: ["claudecode", "codexcli"], root: true, localRoot: false, source: "library" },
    { id: "library:terse.md", path: "~/.pitwall/rules/rules/terse.md", description: "Keep answers short", targets: ["*"], root: false, localRoot: false, source: "library" },
  ];
  let sets: RuleSet[] = [{ id: "s1", name: "Web defaults", ruleIds: ["library:typescript.md", "library:testing.md"] }];
  let sources: RuleSource[] = [];
  const projects: Record<string, string> = {};
  const agents: Record<string, Partial<AgentRules>> = {};

  return {
    status: () => delay(status),
    async setNpx(enabled) {
      status = { ...status, npxAllowed: enabled, available: enabled, via: enabled ? "npx" : null };
      return delay(status);
    },
    library: () => delay([...lib]),
    revealLibrary: () => delay("~/.pitwall/rules/rules"),
    sets: () => delay([...sets]),
    async saveSet(set) {
      const name = set.name.trim();
      if (!name) throw "name the rule set";
      if (sets.some((s) => s.name.toLowerCase() === name.toLowerCase() && s.id !== set.id)) throw `a rule set named "${name}" already exists`;
      const saved = { id: set.id ?? `s${Date.now()}`, name, ruleIds: [...new Set(set.ruleIds)] };
      sets = set.id ? sets.map((s) => (s.id === set.id ? saved : s)) : [...sets, saved];
      return delay(saved);
    },
    async deleteSet(id) {
      sets = sets.filter((s) => s.id !== id);
      for (const k of Object.keys(projects)) if (projects[k] === id) delete projects[k];
      return delay(undefined);
    },
    async importRules(kind, source) {
      if (!source.trim()) throw "enter a source";
      if (kind === "file") {
        if (!status.available) throw "rulesync isn't available, and importing a file needs `rulesync import`.";
        const name = source.split("/").filter(Boolean).slice(-2, -1)[0] ?? "imported";
        lib.push({ id: `library:${name}.md`, path: `~/.pitwall/rules/rules/${name}.md`, description: null, targets: ["*"], root: true, localRoot: false, source: "library" });
        return delay({ added: [`library:${name}.md`], log: "" }, 500);
      }
      const name = source.replace(/\.git$/, "").split(/[/:]/).filter(Boolean).pop() ?? "rules";
      sources = [...sources, { name, kind: kind === "git" ? "git" : "project", origin: source, root: `${source}/.rulesync` }];
      lib.push({ id: `${name}:shared.md`, path: `${source}/.rulesync/rules/shared.md`, description: "Shared rule", targets: ["*"], root: false, localRoot: false, source: name });
      return delay({ added: [`${name}:shared.md`], log: `Added source "${name}" (1 rules)` }, 600);
    },
    sources: () => delay([...sources]),
    pullSource: (name) => delay(`${name}: Already up to date.`, 400),
    async removeSource(name) {
      sources = sources.filter((s) => s.name !== name);
      return delay(undefined);
    },
    async setProjectRules(projectPath, ruleSetId) {
      if (ruleSetId) projects[projectPath] = ruleSetId;
      else delete projects[projectPath];
      return delay(undefined);
    },
    projectRules: () => delay({ ...projects }),
    getProjectRules: (projectPath) => delay(projects[projectPath] ?? null),
    async apply(agentId) {
      if (!status.available) throw "rulesync isn't available. Install it (npm install -g rulesync) or allow npx in Settings → Rules.";
      agents[agentId] = { ...agents[agentId], appliedAt: Date.now(), stale: false, error: null, generated: [".claude/rules/typescript.md"] };
      return delay({ generated: [".claude/rules/typescript.md"], log: "" }, 500);
    },
    async setAgentRules(agentId, ruleSetId) {
      agents[agentId] = { ...agents[agentId], ruleSetId };
      return delay(undefined);
    },
    agentRules: () =>
      delay(
        Object.entries(agents).map(([agentId, a]) => ({
          agentId,
          ruleSetId: a.ruleSetId ?? null,
          projectRuleSetId: null,
          appliedAt: a.appliedAt ?? null,
          generated: a.generated ?? [],
          error: a.error ?? null,
          stale: a.stale ?? false,
          mainCheckout: false,
          mainCheckoutConfirmed: false,
        })),
      ),
  };
}
