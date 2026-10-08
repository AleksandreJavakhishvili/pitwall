# Code explorer (read-only)

Browse, read and search the files of the folder an agent works in, the way
VS Code's Explorer and Search do, without leaving Pitwall. **Read-only**:
Pitwall is a tool, editing is what agents (and the user's editor) do. There
is no save, create, rename, move or delete. "Open in editor" hands a file to
the user's own editor.

Status: spec + backend + API (this round). The viewer UI comes after Review's
move to CodeMirror 6 and reuses its read-only editor.

## Where it lives (UI proposal)
- **Files tab** in the right panel, beside Changes (Next up / Changes /
  Files / Last sent). It belongs to the focused pane's agent, like the rest
  of the panel. It shows a lazy tree of the agent's folder (its worktree when
  it has one), with VS Code file icons (diff-ui.md) and change letters.
- **Viewer** in the main area, opened from the tree, ⌘P or a search hit. It
  is a full-window mode like Review (Esc or ⌘W closes it). Left: the same
  tree plus a Search pane. Center: one read-only CodeMirror view per open
  file, with tabs, syntax highlighting, line numbers and go-to-line. A
  header shows icon, path, change letter, size and "Open in editor" /
  "Copy path". A file with changes has "Show diff", which jumps to Review
  for that file.
- **⌘P quick open**: a fuzzy file picker over the agent's files (ignores
  respected), filtered client-side.
- **⇧⌘F search**: search in the agent's folder. Options: case, whole word,
  regex, include/exclude globs. Results are grouped by file, and clicking a
  line opens it in the viewer at that line.
- Shortcuts follow `HostInfo.shortcuts` (⌘ or Ctrl+Shift; ⇧⌘F becomes
  Ctrl+Shift+Alt+F), as all app shortcuts do.
- Offered only when `AgentView.caps.explorer` is true. "Open in editor" is
  offered only when `caps.openInEditor` is true; otherwise only "Copy path"
  (the path on the agent's machine) is offered.

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
- **Order and limits**: folders first, then files, by name, case-insensitive.
  At most 5 000 entries per folder (`truncated` set beyond that).
- **Change markers**: the same letters and base as the Changes panel
  (`git diff --raw -M <base_commit>` + untracked = `U`), run in the same
  batch as the listing. A folder carries the number of changed paths under
  it (`changes`), which the UI shows as a dot. An untracked folder is `U`.
- **Reading a file**: text up to 2 MiB is returned whole. A file is binary
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
  `bower_components` excluded. At most 100 matches per file and 2 000 in
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

## Open in editor (local only)
- Only for agents whose files are on this computer
  (`ProviderCaps.local_files` → `AgentCaps.openInEditor`).
- The command is a user setting (Settings → Editor), stored by the backend
  in `<data dir>/editor.json`. It is a command line with `{path}`, `{line}`
  and `{column}` placeholders (no `{path}`: the path is appended), split like
  a shell would split it but never run through a shell.
- Default when unset: the first of these found on the login PATH: VS Code
  (`code -g {path}:{line}:{column}`), Cursor (same), Zed (`zed
  {path}:{line}:{column}`), Sublime Text (`subl {path}:{line}:{column}`). If
  none is found, the system opener is used (macOS `open -t`, Linux
  `xdg-open`, Windows `explorer.exe`). The list is a convenience only, and any
  command works.
- The editor is started detached (platform layer). It is never waited for
  and never killed, and it gets the app's login PATH.
- **Remote** (agw, later SSH): "Copy path" only, for now. A provider that
  can name an ssh host the user's own ssh config knows could later offer
  `code --remote ssh-remote+<host> <path>`. That is not done yet: agw's
  exec doesn't go through the user's ssh config (architecture.md §2.7).

## Security
- Paths from the UI must be relative: an absolute path (`/x`, `\x`, `C:…`), a
  `..` segment, NUL, or a too-long path (> 4 096 bytes) is refused before
  anything runs.
- Every read, listed folder and editor path is resolved on the agent's
  machine (`Exec::real_path`, which follows every symlink in the path) and
  must still be inside the resolved root. So a symlink that points outside
  the root (`link -> /etc`) can be listed as an entry but never followed,
  listed into, read or opened.
- Size caps on every read (2 MiB text), listing (5 000 entries per folder,
  100 000 paths for quick open) and search (above). Search arguments are
  passed as argv after `--` / `-e`, never through a shell. Globs are passed
  to the tool (rg `-g`, git `:(glob)` pathspecs).
- Read-only. The explorer calls only `ls-files`, `diff`, `grep`, `rg`,
  `real_path`, `stat`, `list_dir` and `read_file`, and starts the user's
  editor locally.

## Capabilities
- `AgentCaps.explorer` = provider `exec` (it works outside git repos too).
- `AgentCaps.openInEditor` = `explorer` && provider `local_files`.
- `ProviderCaps.local_files`: local `true`, agw `false`.

## Performance budgets
| Operation | Local | agw (≈1 s per agw call) |
|---|---|---|
| Expand a folder | < 150 ms in a 100 000-file repo; 1 git round trip (+1 `real_path` below the root) | 1–2 agw calls |
| Read a file | < 50 ms for 2 MiB; `real_path` + read | 2 calls (+1 `stat` when too large) |
| Quick-open index | < 500 ms for 100 000 files | 1 call |
| Search | rg speed; first results ≤ 1 s on a 100 000-file repo | 1 call (+1 on the first search: rg probe) |
- Independent git commands go through `Exec::run_all` (one round trip), as
  `changes_and_branch` does. No polling on the backend: the UI asks.
- UI: the viewer is a lazy chunk shared with Review's CodeMirror. Trees
  render virtualised beyond ~500 rows. Closed files drop their documents.

## API
See [api.md](api.md) "Explorer". Types are generated from
`pitwall-proto/src/explorer.rs` into `src/gen/`.

## As built (backend)
- Core: `pitwall-core/src/explorer/` (`mod.rs` services and path checks,
  `tree.rs` listing and quick-open, `read.rs`, `search.rs`, `editor.rs`).
  `Exec` gained `list_dir` (local: `std::fs`, agw: a POSIX `sh` loop, fake:
  in memory) and a `Cmd::cancel` flag (honoured by `LocalExec`).
- The editor is started through `platform::spawn_detached` (the only new
  process start outside `LocalExec`).
