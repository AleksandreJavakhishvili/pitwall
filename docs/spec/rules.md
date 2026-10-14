# Rules (via rulesync)

Pitwall never reimplements rulesync; it calls the `rulesync` CLI (detect
`rulesync` on PATH, else `npx -y rulesync` only if the user enabled it in
Settings; show install hint otherwise — never install automatically).
Reference docs: the `docs/` folder of a local clone of rulesync.

## Model
- **Library**: `~/.pitwall/rules/` is a rulesync-format input root
  (`rules/*.md`, optionally `skills/`, `subagents/`, `commands/`). Plus
  imported sources: a project's own `.rulesync/`, an existing CLAUDE.md /
  AGENTS.md (via `rulesync import`), or a git repo (clone into
  `~/.pitwall/rules-sources/<name>`, pull on demand).
- **Rule sets**: named selections of library rules (stored in state).
- **Assignment**: two scopes only: the project's default set + an optional
  per-agent extra set (New agent dialog: "Rules" dropdown; Settings → Rules
  to manage sets). There are no account-wide or team-wide rules.
- **Effective rules**: what an agent gets is the project default set, then
  its own set (a rule in both counts once, as the project's), then the
  working folder's own `.rulesync/` rules, keeping only rules whose
  `targets` include the agent's target (`*` or no `targets`: every agent).
  Rules picked in a set but gone from the library are listed as missing.
  Settings → Rules → Project defaults → "Agents" shows this per project;
  `pitwall rules show <project>` / `--agent <agent>` prints it.

## Applying (at agent create, and "Re-apply rules & restart")
- Map kind → rulesync target (claude→`claudecode`, codex→`codexcli`, others
  from the agent definition file: add an optional `rulesync_target` field).
- Generate into the agent's working directory using rulesync's
  separate/multiple input-root support (read docs/guide/separate-input-root.md)
  so the project's files aren't rewritten by Pitwall's library.
- Per-agent extra rules go to local-only files (rulesync `localRoot`, e.g.
  CLAUDE.local.md) and every generated path is added to the worktree's
  `.git/info/exclude` so generated files never show up in diffs or commits.
- Agents that make their own worktree (worktrees.md) get their rules once
  the worktree is found; they take effect from the next session.
- Agents without their own worktree run in the user's main checkout: only
  apply when the user explicitly confirms (checkbox in the dialog, default off).
- Rules only affect new sessions; after edits show "Re-apply & restart" on
  affected agents.

## Agents rulesync doesn't cover (Gemini CLI, Aider)
rulesync has no target for them, so Pitwall writes their rules itself, from
the agent definition's `rules_fallback` (`target`, `file`, `args`,
`alt_file`, `alt_args`; docs/reference/agents.md):
- The file is the effective rules' bodies (frontmatter stripped; a rule
  counts when its `targets` include the fallback's `target`, `*`, or it has
  none), in the same order as above, under a one-line "written by Pitwall"
  comment. rulesync isn't needed.
- It goes to `file` unless that is the project's own (tracked by git, or
  not written by Pitwall), else to `alt_file`; never over a tracked file or
  someone else's. Gemini CLI: `GEMINI.md`, else `.gemini/pitwall-rules.md`.
  Aider: `.aider.pitwall-rules.md`.
- Recorded in the agent's apply record (files, hashes, fingerprint) and
  listed in `.git/info/exclude` exactly like rulesync's output; removed when
  rules are unassigned; the main-checkout confirmation applies the same way.
- Agents that must be pointed at the file get `args` / `alt_args` (`{file}`
  = its absolute path) on fresh starts: Aider `--read <file>`; Gemini CLI,
  when the rules went to `alt_file`, `--prompt-interactive "Read <file>
  first: …"`. A resumed conversation already has them.

## API
`rules_status` → `{ available: boolean, via: "rulesync"|"npx"|null, version?: string }`,
`list_rule_library` → `RuleFile[] {id, path, description, targets, root}`,
`list_rule_sets` / `save_rule_set({id?, name, ruleIds})` / `delete_rule_set(id)`,
`import_rules({ kind: "project"|"file"|"git", source })`,
`set_project_rules(projectPath, ruleSetId|null)`,
`apply_rules(agentId)` → `{ generated: string[], log: string }`,
`project_rules_detail(projectPath)` → `{ project, ruleSetId, agents: [{ agentId,
ruleSetId, projectRuleSetId, appliedAt, generated, error, stale, mainCheckout,
mainCheckoutConfirmed, name, kindName, target, effective: [{ id, description,
from: "project"|"agent"|"folder", missing }] }] }` (socket: `rules.show
{ project | agentId }`).

Socket / CLI (`pitwall rules …`, for the Race Engineer and scripts):
`rules.set.save { id?, name, ruleIds }` (`rules set create|add|remove|rename`;
new rule ids must be in the library) applies directly, as sets are
Pitwall's own data. `rules.set.delete { set }` (`rules set delete`) and
`rules.import { kind: "file"|"git", source }` (`rules import file|git`) ask
the user first (High: every time); `rules.import` of a `project` reads its
`.rulesync/` in place and doesn't ask. Pitwall's own window never asks.
`CreateAgentRequest` gains `ruleSetId?: string`, `applyToMainCheckout?: boolean`.
