# Worktrees: use the agent's own feature, nothing tied to Pitwall

Problem: "Own git worktree" created folders in `~/.pitwall/worktrees/`
and branches named `pitwall/<agent>`, tying the user's work to Pitwall.
Pitwall is a tool: it must not invent its own scheme.

- New agent dialog: "Separate worktree" checkbox, DEFAULT OFF, with the hint
  "Separate copy of the repo, so this agent doesn't clash with others in the
  same project." Shown only for kinds whose CLI supports it.
- Implemented by passing the agent's native flag, declared in the agent
  definition file (new optional field, e.g. `worktree_args = ["--worktree", "{name}"]`):
  Claude `claude --worktree <name>` (`-w`), Codex `codex --worktree` (check
  `codex --help` for whether it takes a name). Verify both via `--help` only.
- Pitwall discovers where the agent actually works (the worktree path): from
  the hook payload `cwd` when hooks are on, else the process's cwd (`lsof -a -d cwd -p <pid> -Fn`,
  already used in scan.rs) or a new entry in `git worktree list --porcelain`
  appearing after launch. Store it as the agent's working dir for status,
  diffs, Review and resume (resume must run where the session lives).
- Review/merge: use the worktree's actual branch (`git -C <wt> branch --show-current`),
  never assume `pitwall/<name>`. Base commit = merge-base with the project's
  current branch when the worktree appears.
- Remove agent: never delete the agent's worktree or branch silently; offer
  "Also remove worktree" (git worktree remove, no --force) with confirmation.
- Remove Pitwall's own worktree creation code (`git::worktree_add`,
  `~/.pitwall/worktrees`, `pitwall/` branches). Existing agents created the
  old way keep working (their stored path stays valid); no migration of
  folders.
- Rules: generated files still go to `.git/info/exclude` of wherever the agent works.

## As built
- Flags (checked with `--help`): Claude `-w, --worktree [name]` (optional
  name) → `worktree_args = ["--worktree", "{name}"]`; Codex `--worktree`
  ("Run the session in a new managed Git worktree", no name) →
  `worktree_args = ["--worktree"]`. Other kinds: no field, no checkbox.
- `KindView.worktree` says whether the checkbox is shown; `AgentView.worktreePending`
  is true until the worktree is found (see backend.md "Worktrees").
- Merge status `branch` is `null` when the worktree is detached; merge is refused.
