# Review screen

Full-window mode (⌘R, top-bar button, ⌘K) for reviewing what agents changed.

## Which agents
With an agent focused, Review lists **that agent alone** (with its own
worktrees, not the project's other ones). The header's **Agent / <project> /
All** switch widens it: to the agent's project — the **active space**: a
project space → that project's agents, a custom space → its members (named
after the space), the "All" space → the focused agent's project — or to every
agent. The choice is remembered for the session (not persisted); focusing
another agent keeps it. With no agent focused there's no Agent choice: the
space's agents, everyone in "All" (no switch).
Opened for a specific agent — ⌘R / the top-bar button with an agent focused,
the Changes panel's Review button, or "Show diff" in the file viewer
(explorer.md) — that agent is listed even outside the space and selected
first (its first file, or the given file). Opened from a worktree, that
worktree's project agents are listed too, and the list starts at the project
(not the agent alone) until another agent is focused.

## What "All changes" compares with
A branch, never a commit remembered from when the agent started
(`compare_with`, `vcs/review.rs`): the agent's commits since its branch left
that branch (`git merge-base`), plus what isn't committed yet. What others
merged there since doesn't show; once merged, nothing does. The branch, in
order:
1. the one the user chose in Review's "Compared with" menu (local and
   `origin/` branches), remembered per checkout and branch in
   `~/.pitwall/projects.json`; "Automatic" goes back. Used as chosen.
2. the branch the agent's branch was created from, by git's own records:
   the oldest entry of its reflog, "branch: Created from develop"; when that
   says HEAD (`git checkout -b`, `git switch -c`), the HEAD reflog's oldest
   "checkout: moving from develop to <branch>". Not when that names the
   branch itself or its `origin/` copy, a commit, or a branch that no
   longer exists. This holds for worktree agents too.
3. a worktree agent: the main checkout's current branch (its merge target).
4. the project's main branch (`origin/HEAD`'s, else `main` / `master`):
   when there's no record (a fresh clone, an expired reflog).

For 2–4, of the local branch and its `origin/` copy, whichever has moved on
further (the newer merge base), so a local `main` never pulled doesn't put
months of history into Review. On the compared-with branch itself: what
isn't committed yet (as the Changes panel shows it); once committed,
nothing. Review's header shows "Compared with <branch>" (the ref used, e.g.
`origin/main`); `pitwall review changes` reports it as `comparedWith`, and
`--base <branch>` compares with another branch just once.
A task's diff (below) is always its own start and end snapshots.

## Layout
Left: changes grouped by agent → files (+/−, untracked badge). Center: diff
editor (side-by-side / inline toggle, syntax highlight, collapsed unchanged
regions), bundled locally — no CDN (desktop app works offline). It is
CodeMirror 6 + `@codemirror/merge` (`src/components/review/diffView.ts`), made
to look and behave like the Monaco diff editor it replaced (perf.md, pass 2).
Above the diff: "All changes" or a specific task (see below). Bottom actions
per agent: Discard file, Send comments, Commit & merge.

## Markdown preview
A Markdown file (`.md`, `.markdown`, `.mdx` read as plain Markdown) gets a
**Diff / Preview** switch in its header; Diff is the default and the choice
holds for the app session. Preview draws the file's new version formatted
(`crates/pitwall-app/src/markdown_view`, pulldown-cmark with GFM tables,
task lists, strikethrough, footnotes): headings, paragraphs, emphasis,
inline and fenced code (highlighted like the diff), quotes, nested and task
lists (display only), tables (scroll sideways when wide), rules, front
matter as code, raw HTML as text. Centred column, at most 760 px wide.
- Changes: the diff's hunks map to blocks by source line. A block wholly
  inside an insertion gets a green bar on its left, any other touched block
  an amber one (list items one by one); removed lines between blocks are a
  thin red "N lines removed" rule where they were.
- Links: http(s) and mailto open in the browser; relative links are only
  styled. Images next to the file (relative paths, in the worktree) are
  shown at most the column wide; remote images are never fetched: their alt
  text with an image icon instead.
- No Preview for deleted or binary files; over 1 MB the file is shown as a
  diff only (Preview disabled, says why). Text in Preview isn't selectable
  (use Diff to copy). Comments are added from the Diff.

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
- Discard file: restore from the compared-with commit / delete untracked file, after confirmation.
- Agents without a worktree: no merge (they work in the main checkout); commit only.

## API (new commands)
`list_tasks(agentId)`, `get_task_changes(agentId, taskId)`,
`get_file_versions(agentId, path, taskId?)` → `{ original: string|null, modified: string|null, binary: boolean }` (for the diff editor),
`discard_file(agentId, path)`, `commit_agent(agentId, message)`,
`merge_agent(agentId)` → `{ merged: boolean, conflict: boolean, message: string }`.
`AgentView` gains `currentTaskId: string|null`. Mock mode covers all of it.
