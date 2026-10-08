# Code explorer (read-only)

Browse, read and search the files of the folder an agent works in, the way
VS Code's Explorer and Search do, without leaving Pitwall. **Strictly
read-only**: the user never edits files here (agents do). There is no typing
into a file, save, create, rename, move or delete, and no "Open in editor"
(dropped: the user doesn't edit files). "Copy path" is the one way out.

Status: built (backend, API and UI).

## Where it lives
- **Files tab** in the right panel. Changes and Files are two tabs of one
  section (Next up above and Last sent below stay where they were; the tree
  scrolls inside the section so Last sent stays in reach). It belongs to the
  focused pane's agent, like the rest of the panel, and is offered only when
  `AgentView.caps.explorer`. It shows the agent's folder (its worktree when
  it has one) as a lazy tree with VS Code file icons (diff-ui.md), the
  Changes panel's letters on files and a dot on folders with changes below
  them. Above the tree: "Go to file ⌘P", "Search ⇧⌘F" and an "Ignored"
  checkbox ("Show ignored files", off by default; ignored entries are
  dimmed). The header has ↻ (with "updated … ago") and ⤢ to open the viewer.
- **Viewer** in the main area, opened from the tree, ⌘P or a search hit.
  A full-window mode like Review (Esc or "Back" closes it; the right panel
  hides while it is open), lazy-loaded with its own error boundary. Left:
  Explorer (the same tree, same open folders as the panel) or Search,
  switched with a segmented control. Center: one tab per open file, each a
  read-only CodeMirror view with Review's setup (`review/editorSetup.ts`,
  `FileViewer` in `review/diffView.ts`: same colours, line numbers,
  scrollbars, Find ⌘F, bracket colours). The header shows icon, path,
  change letter, size, "Copy path" and, for a changed file, "Show diff",
  which opens Review on that agent and file. Binary files say so; files over
  2 MiB say "Large file · 3.3 MB · not opened" with **Load anyway** (up to
  10 MiB). An open file that changed on disk shows "Changed on disk since it
  was opened · Reload".
- **⌘P quick open**: a palette-style fuzzy file picker over the agent's files
  (`list_all_files`, ignores respected), filtered in the UI (`lib/explorer.ts`
  `quickOpen`: file-name substrings first, then word starts and runs; an
  empty query lists recently opened files first). ↵ opens in the viewer.
- **⇧⌘F search**: the viewer's Search pane, searching as you type (debounced).
  Options: match case, whole word, regex; "files to include / exclude" globs
  (comma-separated) and "Leave out node_modules and bower_components" (on by
  default, as VS Code's `search.exclude`). Results are grouped by file and all
  shown at once (v1, up to the 2 000 cap), with the matched text marked;
  clicking a line opens the file there with the match selected.
- Shortcuts follow `HostInfo.shortcuts` (⌘ or Ctrl+Shift): ⌘P is
  Ctrl+Shift+P, ⇧⌘F is Ctrl+Shift+Alt+F. Neither clashed with an existing
  app shortcut. Both, and "Browse files", are in the command palette (⌘K)
  when the focused agent's files can be read.
- Works for agw agents: paths are the VM's, and "Copy path" copies the path
  on the agent's machine.

## Behaviour
- **Root** = the agent's `cwd` (its worktree folder when it has one). Every
  path in the API is relative to the root, uses `/` separators, and is never
  absolute.
- **Tree, git repo**: one directory per call (lazy; folders expand on
  demand). Its children come from `git ls-files -z --stage` +
  `git ls-files -z --others --exclude-standard --directory
  --no-empty-directory`, run in that folder. So `.gitignore`, `.git/info/exclude` and the
  global excludes apply exactly as git applies them. Tracked symlinks
  (mode 120000) are `symlink` entries. Submodules (160000) and nested repos
  are folders, listed by their own git when expanded. Files deleted from
  the working tree are not shown (Review shows them). An untracked folder
  is listed with a second `ls-files --others` (without `--directory`).
- **Tree, not a repo** (or no git there): a plain directory listing
  (`Exec::list_dir`) without VS Code's default `files.exclude` names
  (`.git`, `.svn`, `.hg`, `CVS`, `.DS_Store`, `Thumbs.db`).
- **Ignored files** ("Show ignored files"): one more `ls-files --others
  --ignored --exclude-standard --directory` in the same batch; ignored
  entries are marked `ignored`. An ignored folder, opened, is listed plainly
  with everything in it marked.
- **Order and limits**: folders first, then files, by name, case-insensitive.
  At most 5 000 entries per folder (`truncated` set beyond that).
- **Change markers**: the same letters and base as the Changes panel
  (`git diff --raw -M <base_commit>` + untracked = `U`), run in the same
  batch as the listing. A folder carries the number of changed paths under
  it (`changes`), which the UI shows as a dot. An untracked folder is `U`.
- **Reading a file**: text up to 2 MiB is returned whole (up to 10 MiB when
  asked for: "Load anyway"). A file is binary
  when its first 8 000 bytes contain a NUL (git's and VS Code's rule) or
  its extension is a known binary one (images, archives, fonts, media,
  executables). Binary files are never transferred. Larger files are
  `tooLarge`, with their size. Invalid UTF-8 is shown with replacement
  characters, as VS Code does by default. Line endings are kept. A language
  hint (`lang`, from the file name or extension) helps the viewer pick a
  highlighter. The viewer may still override it.
- **Quick-open index** (`list_all_files`): `git ls-files` (cached + others,
  minus deleted) in a repo. Elsewhere `rg --files` when ripgrep is on that
  machine. Otherwise a breadth-first listing within a 3 s budget. At most
  100 000 paths.
- **Search**: `rg --json` when ripgrep is installed on the agent's machine
  (probed once per machine, remembered while Pitwall runs). Otherwise
  `git grep -n --column -I --untracked` in a repo. Neither available: a clear
  message ("Search needs ripgrep (rg) or a git repository on <machine>").
  Defaults match VS Code: fixed string, case-insensitive, hidden files
  included, ignores respected, and `.git`, `node_modules` and
  `bower_components` excluded (`defaultExcludes: false` searches them). At most 100 matches per file and 2 000 in
  total (`truncated` beyond that, up to a requested 10 000). Files over
  2 MiB are skipped. Lines are cut to 400 characters around the first match.
  Match ranges are in UTF-16 code units of the returned text, ready for JS.
  **Cancellable**: `cancel_search(agentId)` stops it, and a new search for the
  same agent cancels the previous one. Locally the process is killed. On a
  remote machine the answer is dropped (and the VM-side `timeout` ends it,
  30 s).
- **Freshness**: nothing is cached on the backend. The tree reads on open, on
  expand and on ↻ / ⌘⇧R (`lib/freshness.ts`, like Changes). While visible,
  it re-reads expanded folders when the agent's `filesChanged`/`added`/
  `removed` move, or at most every 10 s. An open file shows "changed on
  disk, reload" when its size or text differs on the next read.

## Security
- Paths from the UI must be relative: an absolute path (`/x`, `\x`, `C:…`), a
  `..` segment, NUL, or a too-long path (> 4 096 bytes) is refused before
  anything runs.
- Every read and listed folder is resolved on the agent's
  machine (`Exec::real_path`, which follows every symlink in the path) and
  must still be inside the resolved root. So a symlink that points outside
  the root (`link -> /etc`) can be listed as an entry but never followed,
  listed into or read.
- Size caps on every read (2 MiB text), listing (5 000 entries per folder,
  100 000 paths for quick open) and search (above). Search arguments are
  passed as argv after `--` / `-e`, never through a shell. Globs are passed
  to the tool (rg `-g`, git `:(glob)` pathspecs).
- Read-only. The explorer calls only `ls-files`, `diff`, `grep`, `rg`,
  `real_path`, `stat`, `list_dir` and `read_file`. It starts nothing else.

## Capabilities
- `AgentCaps.explorer` = provider `exec` (it works outside git repos too).

## Performance budgets
| Operation | Local | agw (≈1 s per agw call) |
|---|---|---|
| Expand a folder | < 150 ms in a 100 000-file repo; 1 git round trip (+1 `real_path` below the root) | 1–2 agw calls |
| Read a file | < 50 ms for 2 MiB; `real_path` + read | 2 calls (+1 `stat` when too large) |
| Quick-open index | < 500 ms for 100 000 files | 1 call |
| Search | rg speed; first results ≤ 1 s on a 100 000-file repo | 1 call (+1 on the first search: rg probe) |
- Independent git commands go through `Exec::run_all` (one round trip), as
  `changes_and_branch` does. No polling on the backend: the UI asks.
- UI: the viewer is a lazy chunk and its editor shares Review's CodeMirror
  chunk. Tree rows use `content-visibility: auto`, so long folders cost
  little to draw. Closed tabs drop their documents. The tree re-reads open
  folders on open, ↻ / ⌘⇧R, when the agent's change totals move and every
  10 s while the window is visible (`components/explorer/useTree.ts`).

## API
See [api.md](api.md) "Explorer". Types are generated from
`pitwall-proto/src/explorer.rs` into `src/gen/`.

## As built
- Core: `pitwall-core/src/explorer/` (`mod.rs` services and path checks,
  `tree.rs` listing and quick-open, `read.rs`, `search.rs`). `Exec` gained
  `list_dir` (local: `std::fs`, agw: a POSIX `sh` loop, fake: in memory) and
  a `Cmd::cancel` flag (honoured by `LocalExec`).
- UI: `src/components/explorer/` (`FilesPanel`, `ExplorerTree`, `Viewer`,
  `FileEditor`, `SearchPane`, `QuickOpen`, `useTree`), logic in
  `src/lib/explorer.ts` (tested), mock in `src/mockExplorer.ts`.
