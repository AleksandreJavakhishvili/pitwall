# Rules (via rulesync)

Pitwall never reimplements rulesync; it calls the `rulesync` CLI (detect
`rulesync` on PATH, else `npx -y rulesync` only if the user enabled it in
Settings; show install hint otherwise — never install automatically).
Reference docs: the `docs/` folder of a local clone of rulesync.

## Model
- **Skills first.** Reusable know-how lives as rulesync **skills**
  (loaded by an agent only when relevant); **rules** (always in context)
  are supported but secondary. A project's main instruction file
  (CLAUDE.md / AGENTS.md) is the project's own: Pitwall only generates it
  once rules were copied into the project (or its instructions imported).
- **Library**: a catalog of every skill and rule.
  Skills: `~/.pitwall/rules/skills/<name>/SKILL.md` plus any other files in
  that folder (scripts, references); listed by folder name with the
  frontmatter's `description` and `targets`; id `<source>:skills/<name>`.
  Rules: `~/.pitwall/rules/` is a rulesync-format input root
  (`rules/*.md`, optionally `skills/`, `subagents/`, `commands/`). Plus
  imported sources: a project's own `.rulesync/`, an existing CLAUDE.md /
  AGENTS.md (via `rulesync import`), or a git repo (clone into
  `~/.pitwall/rules-sources/<name>`, pull on demand).
- **Rule sets**: named selections of library rules (stored in state).
- **Assignment**: two scopes only. **Project**: rules copied into the
  repository and committed (see "Project rules"); the old project default
  set kept in Pitwall is only a fallback while the working folder has no
  `.rulesync/rules/`. **Agent**: an optional per-agent extra set, personal
  and local-only (New agent dialog: "Rules" dropdown). There are no
  account-wide or team-wide rules.
- **Effective rules**: what an agent gets is the project default set (only
  as the fallback above), then
  its own set (a rule in both counts once, as the project's), then the
  working folder's own `.rulesync/` rules, keeping only rules whose
  `targets` include the agent's target (`*` or no `targets`: every agent).
  Rules picked in a set but gone from the library are listed as missing.
  Settings → Rules → Project defaults → "Agents" shows this per project;
  `pitwall rules show <project>` / `--agent <agent>` prints it.

## Skills (rulesync 27 format)
Checked against rulesync 27.0.0's own source and bundled docs, and run
end to end with rulesync 33.0.0 (`cargo test -p pitwall-core real_rulesync
-- --ignored`; rulesync drops a skill script's executable bit, which
Pitwall restores from the source)
(`src/features/skills/*`, `docs/reference/file-formats.md`,
`docs/reference/cli-commands.md`):
- Source: `.rulesync/skills/<dir>/SKILL.md` + other files in the folder.
  Frontmatter: `name` and `description` required (name is used as a
  directory name: no separators, not `.`/`..`, no trailing dot/space),
  `targets` default `["*"]`, optional per-tool sections (`claudecode:`,
  `codexcli:`, …), `disable-model-invocation`, `user-invocable`, …
- Output folder is the source folder name: claudecode →
  `.claude/skills/<dir>/`, codexcli → `.agents/skills/<dir>/` (plus an
  `agents/openai.yaml` sidecar when its section asks), copilot →
  `.github/skills/`, cursor → `.cursor/skills/`, agentsmd/agentsskills →
  `.agents/skills/`, …; Gemini CLI and Aider have no skills support (Pitwall's
  own file for them holds rules only).
- `rulesync generate --targets a,b --features skills` generates skills only
  (no CLAUDE.md / AGENTS.md); the `--json` plan lists every written file of
  a skill folder as `{action: "write", kind: "file", path}`.
- rulesync has no local-only skills output (no `localRoot` for skills).
  Agent scope therefore uses the same mechanism as its other files: an
  agent's own set may hold skills, generated in its working folder with
  `--features rules,skills` and listed in `.git/info/exclude`; a tracked
  (project) skill file is never written over.
- New entries: `pitwall rules skill new <name> [--from <id|file|folder>]
  [--description] [--targets]` and `pitwall rules new <name> …` write
  proper files (a skill gets `name`/`description`/`targets`; a folder's
  other files and frontmatter keys come along). New: direct; `--overwrite`
  asks. A skill becomes a rule (or the reverse) only with `--as-rule` /
  `--as-skill`.

## Project rules (the project scope)
Rules end up in the project: copied into `<repo>/.rulesync/rules/`, where the
user commits them (they show up in Review; nothing here goes in
`.git/info/exclude`). Code: `crates/pitwall-core/src/rules/project.rs`.
- **Skills** are assigned the same way: the whole folder is copied to
  `.rulesync/skills/<name>/` and hashed over every file (path + content);
  the same states, approvals and no-overwrite guarantees; diffs per file.
- **Manifest** `<repo>/.rulesync/.pitwall.json`: for each copied file (and
  skill folder), the library entry it came from and its content hash at
  copy time; the targets and features the project generates; instruction files imported; the last
  generation (inputs fingerprint, output path → hash). The rule files stay
  plain rulesync files.
- **Assign** a whole set or single rules (Settings → Rules → a project's
  Details: checkboxes grouped by set; `pitwall rules project assign`).
  A rule goes to its path in its source (under `<source>/` when another
  source's rule has the name). A different file already there is never
  replaced unless the caller names it (`overwrite`). **Unassign** removes a
  copy only if unchanged since copied, or when forced.
- **Updates are opt-in** (never synced on their own). Per rule: `current`;
  `update` (library changed since the copy; shown with a unified diff);
  `modified` (changed in the project; never overwritten); `conflict` (both:
  shown with both texts, and the user must take the library's or keep the
  project's; keeping marks the library version seen). Accept per rule or
  all (all skips conflicts unless a choice is given).
- **Existing instructions**: once rules (not just skills) are assigned, a
  repo with a hand-written, tracked CLAUDE.md / AGENTS.md / GEMINI.md and no
  rules of its own in `.rulesync/` is offered
  "import existing instructions" first: `rulesync import` runs in a scratch
  copy and only new files are added to `.rulesync/rules/` (named
  `<file>-<rule>.md`). Generation never replaces such a file before it is
  imported (or explicitly named). `rulesync import` makes each file a root
  rule for every tool (`targets: ['*']`), which would merge CLAUDE.md and
  AGENTS.md into each other on generation, so Pitwall then sets each
  imported rule's `targets` (rest of the file byte for byte): CLAUDE.md's →
  `claudecode`; GEMINI.md's → `geminicli`; AGENTS.md's → the targets in use
  and the tools that read AGENTS.md except Claude Code when a CLAUDE.md was
  imported too (otherwise `*`, so Claude Code still gets it). An import in
  a later run adjusts earlier imported rules the same way while they are
  unchanged since (per the manifest's `importedRules`), else reports them.
- **Features**: `skills` when `.rulesync/skills/` has any; `rules` only when
  Pitwall copied rules in or imported the instruction files. A skills-only
  project never gets CLAUDE.md / AGENTS.md / GEMINI.md written (also
  filtered defensively). The features used are kept in the manifest.
- **Generation**: `rulesync generate --targets <in use> --features <those>`
  into a staging folder with `<repo>/.rulesync` as the input root (the
  project's rulesync.jsonc doesn't apply), then the outputs (CLAUDE.md,
  AGENTS.md, …) are placed in the repo to be committed. Targets: given, else
  the ones used last, else those of the project's agents' kinds, else
  `claudecode,codexcli`. A file is replaced only when it is Pitwall's last
  output unchanged, an imported instruction file not generated yet, an
  agent's own local copy (released, below), or named. Outputs no longer
  generated are removed when unchanged. **Out of date** when `.rulesync/rules/`
  or the targets changed since, or a generated file was edited or deleted
  (also when rules were copied but never generated).
- **Agent scope never clobbers shared files**: agent-scope generation never
  writes over a git-tracked file or one it didn't write. When the project
  generates a path an agent wrote for itself (untracked, excluded), the
  agent's record and `.git/info/exclude` block let go of it, so it shows in
  Review and the agent's cleanup never deletes it.
- **Migration**: rules.json keeps loading as before. The project default set
  is a fallback while the working folder has no `.rulesync/rules/`; "Move
  into project" (`pitwall rules project move-default`) copies its rules in
  through Assign, then drops the default (kept when files need a decision).
- **Approvals**: socket `rules.project.status|updates` read freely;
  `rules.project.assign|unassign|update|import|generate|move` write into
  the user's repository and ask first (Low: "remember" allowed, as the
  result is reviewed before commit). Pitwall's own window asks nobody.

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
- Bulk: "Re-apply & restart all" (Settings → Rules, shown while any agent's
  rules changed or failed since its session started) re-applies every such
  agent, then restarts the running ones (stopped ones use the rules when
  they start). Only agents Pitwall may write for: own worktree, or a main
  checkout the user confirmed before. CLI: `pitwall rules apply --stale
  [--project <p>] [--set <s>] [--restart]` (socket `rules.reapply`, no
  approval; each restart is `agent.restart`, which asks).

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
