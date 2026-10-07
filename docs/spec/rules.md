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
- **Assignment**: project default set + optional per-agent extra set
  (New agent dialog: "Rules" dropdown; Settings → Rules to manage sets).

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

## API
`rules_status` → `{ available: boolean, via: "rulesync"|"npx"|null, version?: string }`,
`list_rule_library` → `RuleFile[] {id, path, description, targets, root}`,
`list_rule_sets` / `save_rule_set({id?, name, ruleIds})` / `delete_rule_set(id)`,
`import_rules({ kind: "project"|"file"|"git", source })`,
`set_project_rules(projectPath, ruleSetId|null)`,
`apply_rules(agentId)` → `{ generated: string[], log: string }`.
`CreateAgentRequest` gains `ruleSetId?: string`, `applyToMainCheckout?: boolean`.
