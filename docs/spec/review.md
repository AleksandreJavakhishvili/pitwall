# Review screen

Full-window mode (⌘R, top-bar button, ⌘K) for reviewing what agents changed.

## Layout
Left: changes grouped by agent → files (+/−, untracked badge). Center: Monaco
diff editor (side-by-side / inline toggle, syntax highlight, collapsed
unchanged regions), bundled locally — no CDN (desktop app works offline).
Above the diff: "All changes" or a specific task (see below). Bottom actions
per agent: Discard file, Send comments, Commit & merge.

## Per-task diffs (backend)
A task = one prompt delivered to an agent (send_prompt, queue send, typed Enter,
or UserPromptSubmit hook) until it is done/idle. Snapshot the working tree
WITHOUT side effects at task start and end: use a temporary index
(`GIT_INDEX_FILE=<tmp> git add -A && git write-tree`) to get a tree id that
includes untracked files; never touch the real index or working tree. Keep the
trees alive with refs under `refs/pitwall/<agent>/<task>` (cleaned up when the
agent is removed). Store tasks in state: `{ id, prompt, startedAt, endedAt?, startTree, endTree? }`.

## Comments
Click a line in the diff → add a comment. "Send comments" composes ONE prompt
(shown to the user, editable, before sending): `Review comments:\n- <path>:<line> — <comment>`
and sends it verbatim via send_prompt (or adds to Next up if the agent is busy).

## Commit & merge (explicit user actions only, with confirmations)
- Commit: in the agent's worktree, `git add -A && git commit -m <user message>`.
- Merge: into the project's current branch in the main checkout with
  `git merge --no-ff <branch>`, where `<branch>` is the worktree's current
  branch (`git -C <wt> branch --show-current`; refuse if detached); refuse with a clear message if the main
  checkout is dirty. On conflicts: `git merge --abort`, then offer a prefilled,
  editable prompt to the agent ("rebase onto <branch> and resolve conflicts").
- Discard file: restore from base / delete untracked file, after confirmation.
- Agents without a worktree: no merge (they work in the main checkout); commit only.

## API (new commands)
`list_tasks(agentId)`, `get_task_changes(agentId, taskId)`,
`get_file_versions(agentId, path, taskId?)` → `{ original: string|null, modified: string|null, binary: boolean }` (for Monaco),
`discard_file(agentId, path)`, `commit_agent(agentId, message)`,
`merge_agent(agentId)` → `{ merged: boolean, conflict: boolean, message: string }`.
`AgentView` gains `currentTaskId: string|null`. Mock mode covers all of it.
