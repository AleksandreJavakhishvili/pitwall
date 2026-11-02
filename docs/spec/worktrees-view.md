# Worktrees in source control

Agents often create git worktrees while they work (Claude Code runs
sub-agents in `.claude/worktrees/<name>` on branches like
`worktree-<name>`; other tools do similar). Pitwall must show those, not only
each agent's own working folder.

## Model
- Per project (git repo), list every worktree: `git worktree list --porcelain`
  (path, HEAD, branch, locked/prunable flags) through `Exec` (works for agw too).
- Attribute each worktree to an agent when possible, in this order:
  1. it is the agent's own working folder (today's behaviour);
  2. the agent worked in it: its hooks reported that folder. Every hook
     payload carries the session's `cwd` (the documented hook input, the
     same for every agent with hooks), and Pitwall's relay names the agent
     (`PITWALL_AGENT_ID`), so no agent's files or log formats are read.
     A tool hook's `tool_input` counts too: any path in any of its strings
     (`cd ../repo-feature && …`, a file edited by its full path; relative
     ones against `cwd`) that falls in a listed linked worktree counts as
     working in that worktree, once per hook. Agents often work in sibling
     worktrees while their `cwd` stays the main checkout (Claude Code's
     `cd` outside the project lasts one command).
     Kept on the agent (`AgentRecord::workedIn`: folder, score, last; most
     recent first, at most 40) across restarts. Each hook event adds 1 to
     the folder's score, and scores halve every 2 days, so this week's work
     outweighs last month's. Highest score wins; close scores (a visit vs.
     a visit) go to whoever was there last. The main checkout is everyone's
     and doesn't count. A folder that was one of the repository's
     worktrees in the last listing and isn't now (a temporary worktree,
     removed) is forgotten; other folders age out of the 40;
  3. a process inside the agent's terminal has (or had, this run) its cwd
     in it (local only, via the existing process helpers);
  4. otherwise it's an "other worktree" of the project (made by a session
     outside Pitwall, an earlier conversation, or by hand).
- Each worktree gets its own changes (+/−, files) against its merge-base with
  the project's current branch, refreshed cheaply: one `worktree list` per
  project when anything in the project changes or every 30 s while visible;
  per-worktree diff only when shown/expanded.

## UI
- Sidebar: an agent row shows a small "2 worktrees" chip; expanding lists
  them (branch, +/−, locked badge).
- Review (⌘R): left tree groups by agent → its folder + its worktrees, plus
  "Other worktrees" per project; selecting one shows its files and diff,
  per-task diffs only for the agent's own folder.
- Actions per worktree (confirmations as today): open a terminal there,
  commit, merge into the project's current branch (same rules as Review merge:
  refuse dirty main checkout, abort on conflicts), remove worktree (never
  --force; locked worktrees can't be removed from Pitwall).
- Worktrees that disappear (agent cleaned up after merging) leave the list.

## Capabilities
`caps.worktrees` (provider can list) and per-worktree caps (merge/remove)
drive the UI; no provider/kind special cases.

## As built
- Core: `pitwall-core/src/worktrees.rs` (cache, attribution, services);
  `vcs::git::parse_worktree_list` reads HEAD, branch, `bare`, `detached`,
  `locked [reason]`, `prunable`. Where a worktree lives never decides whose
  it is (git lists them wherever they are: `.claude/worktrees`, Codex's
  `~/.codex/worktrees`, `../anything`), and no folder name is special.
  Rule 2 ties: the agent with a process there, then
  the oldest agent. The main checkout is the project
  itself and isn't listed; bare entries are skipped.
- Agents are grouped per provider + machine + project folder; one list per
  group, run in the first agent's folder through its `Exec` (agw: AgwExec).
  Agent folders are resolved once (`real_path`, cached on the agent) so
  `/tmp` vs `/private/tmp` and letter case match git's paths.
- Rule 3 asks processes only when some linked worktree is still unclaimed,
  in one batched call per provider (`Provider::process_cwds`; local: one
  `lsof`); agw has no `process_cwd`, so it never asks. What it finds is
  remembered for the run (newest first), so a worktree stays the agent's
  after its build or test process ends.
- Refresh: UI-driven. App calls `useWorktrees` once per window: on first
  use, every 30 s while the document is visible, and ~1 s after agents'
  numbers move. The backend only lists a project when due (see api.md), so
  extra calls are free.
- Per-worktree diffs (`get_worktree_changes`) only for rows that are shown:
  the sidebar/Changes-panel lists once expanded (every 15 s), Review sections
  when open or selected (every 10 s and on HEAD changes).
- `caps.worktrees` (AgentCaps) gates listing; `WorktreeCaps` gates actions.
  `remove` is false for locked, gone (prunable) and agents' own folders;
  `merge` needs a branch different from the main checkout's;
  `terminal` = the project's machine can start terminals.
- UI: sidebar chip "N worktrees" under the agent row and "N other
  worktrees" per project, rows show branch, +/−, locked; right-click: Review,
  Open terminal here, Remove worktree…. Changes panel: the same chip. Review:
  worktrees nested under their agent and "Other worktrees · project";
  selecting one shows its files and diff (no task scope, no comments),
  with Open terminal / Remove worktree… / Commit (& merge) in the footer.
  Commit & merge reuses the Review dialog; removal has its own confirmation.
- Each worktree's view has `activeAt`: when an agent last worked there (its
  hooks, or a process seen there). Lists show the latest first; never seen
  ones last, by name.
- Folding (`review::model::fold`, the sidebar and Review alike): the
  window's worktree list reads whether each listed worktree has changes
  (again when its HEAD moves, or after 60 s). Folded away:
  - deleted (prunable: the folder is gone, git still lists it);
  - without changes against its branch (merged, untouched, a throwaway
    build checkout);
  - Review only: not worked in for 7 days (may still hold work: the line
    says how many have changes).
  The sidebar's chips count and list only the rest; Review ends with
  "Show N worktrees without changes", "Show N worktrees not worked in for
  a week · M with changes", and "Clean up N deleted worktrees" (`git
  worktree prune` in those projects: nothing on disk changes). The shown
  worktree always stays listed. No names are special (`agent-*` or
  otherwise): what's in a worktree and when it was used decide.
