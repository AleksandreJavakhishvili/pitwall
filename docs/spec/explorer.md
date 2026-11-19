# Code explorer

Browse, read and search the files of the folder an agent works in, the way
VS Code's Explorer and Search do, without leaving Pitwall. Agents do the
work; the user can make **quick fixes while reading** (see "Editing"
below): type in a text file and save it. It is not an IDE: there is no
create, rename, move or delete, no "Open in editor", and agents never get a
way to write through it. "Copy path" is the one way out.

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
  10 MiB). An open file that changed on disk is read again quietly (the
  view keeps its scroll position and caret) unless it has unsaved changes
  (see "Editing").
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
  highlighter. The viewer may still override it. An SVG is `image` and
  carries its text (≤ 2 MiB); a PDF is `binary`, and the viewer reads its
  bytes with `read_image` (≤ 64 MiB) where it can draw it.
- **Markdown**: the viewer's header switches a Markdown file between
  **Code** and **Preview** (formatted, as Review's Markdown preview, without
  change bars; relative images from the file's folder, remote ones never
  fetched). Its text selects and copies the same way (drag, double- and
  triple-click, ⌘A, ⌘C, the right-click menu). Code is the default; the
  choice holds for the app session. Files over 1 MB stay Code. ⌘⇧V
  (Ctrl+Shift+Alt+V) switches between them, as for an SVG.
- **Images**: PNG, JPEG, GIF, WebP and SVG up to 10 MiB are drawn (their
  bytes read separately, `read_image`), fitted to the pane or at a zoom
  (−/+, Fit, 100 %, a double click), over a checkerboard, with their pixel
  size in the header. Larger ones and other binary files say what they
  are and, on this computer, offer "Open in default app".
- **Moving and zooming a picture** (an image, an SVG's picture, a PDF; in
  a files tile, a files window and the full viewer alike): larger than its
  pane either way, it is pressed and dragged any way (an open hand over
  it, a closed one while it moves, wherever the pointer goes until the
  release), and a scroll (a trackpad's two fingers, a wheel; Shift + a
  wheel goes sideways) moves it both ways at once. It never leaves the
  pane: its edge stops 16 pt in, and while it fits that way it is
  centred and doesn't move. A press that travels 3 pt or less is a
  click, so a double click still zooms; a double click that ends a drag
  doesn't. A pinch on a trackpad zooms smoothly, ⌘ + scroll (Ctrl +
  scroll on Windows and Linux) too, about 10 % a wheel's notch; both keep
  the point under the pointer where it is (as far as the picture can
  move), within the zoom's range (an image 10 % to 1600 %, or down to its
  fitted scale; a PDF 25 % to 400 %). A double click keeps the point
  clicked under the pointer; −/+, Fit and 100 % keep the view's centre;
  the header's percentage follows. Pinches are GPUI's own `PinchEvent`
  (macOS `magnifyWithEvent:`, Windows' DirectManipulation, Wayland's and
  X11's pinch gestures; a Windows touchpad without it sends Ctrl + wheel),
  heard by what is under the pointer as a scroll is, also right after a
  key. A PDF's pages are scaled as drawn while a pinch or ⌘-scroll goes
  on, pages not drawn yet drawn at once, and drawn again at the new scale
  when the pinch ends or 200 ms after the last event. Where an image is
  moved is kept per open file: another file starts centred, and the same
  image read again (changed on disk) stays where it was unless its size
  changed, when it is centred again. A files tile is still dragged only by
  its bar: a press on the picture moves the picture, never the tile, and
  a drag from the bar moves the tile, never the picture. The PDF view has
  no text selection, so a press there moves the pages too.
- **SVG**: drawn as a picture by default; the header's **Picture /
  Source** switch (⌘⇧V, Ctrl+Shift+Alt+V on Windows and Linux) shows its
  XML instead, highlighted, and edited and saved like any text file (the
  dirty dot, Save / Don't Save, "Changed on disk", Keep mine / Take
  theirs). The picture follows the edits and, after a save, is the saved
  file. `read_file` sends an SVG as `image` with its text (up to 2 MiB;
  larger ones up to 10 MiB are pictures only). The choice is kept per
  file (agent and path) for the app session.
- **PDF** (macOS only for now): drawn with the system's PDF engine
  (PDFKit, `platform/pdf_macos.rs`): a document lives on a thread of its
  own; pages are drawn there into bitmaps the viewer shows as images, so
  menus, pop-ups, drag previews and drop zones stay above them (no native
  `PDFView` over the window). Pages one under the other as white paper on
  the pane's ground in either theme; **Fit** (the width), **100 %**, −/+
  (25 % to 400 %; the place at the view's centre stays there), a pinch
  and ⌘-scroll (above); wider than the pane, the pages scroll sideways
  too; "Page 3 of 120" in the
  header. Only the pages in view and about a screen around them are
  drawn, for the display's scale, at most three at a time, nearest first;
  drawn pages are kept up to 192 MiB per document (the farthest let go
  first), a page is never drawn larger than 16 M pixels (beyond that it
  is scaled up), and the other open PDFs keep none. **Find** (⌘F /
  Ctrl+F, or the header's button): case and accents ignored, every hit
  marked on its page, the one shown stronger, ↵ / ⇧↵ (or the arrows) the
  next and previous round the ends, "3 of 12", Esc closes it; at most
  5 000 hits. "Open in default app" stays in the header. A PDF needing a
  password says so (no prompt yet); one the engine can't read says it
  may be damaged; one over 64 MiB says it isn't shown. Its bytes are read
  through the agent's `Exec` like an image's (`read_image`, up to
  64 MiB), so another machine's agent's PDFs are shown too ("Open in
  default app" only for files on this computer). A PDF that changes on
  disk is read again as an image is (when its size changes), keeping its
  zoom and place. **Windows and Linux: not supported yet.** A PDF there
  says "Viewing PDFs in Pitwall isn't available on Windows and Linux yet"
  and offers "Open in default app" (`platform::can_view_pdf`).
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
- The explorer calls only `ls-files`, `diff`, `grep`, `rg`, `real_path`,
  `stat`, `list_dir` and `read_file`. It starts nothing else. The one write
  is the user's own save (`write_file`, below).

## Editing

On by default (`agents.editFiles`; off: the viewer is read-only). Quick
fixes, not an IDE.

- **What can be edited**: a text file of an agent on this computer
  (`ProviderCaps::local_process`; never another machine's agent, nor one
  shown here from a peer) or of a folder on this computer (a `path:` root),
  valid UTF-8, up to 2 MiB, inside the folder; an SVG's source too (in
  Source). Other images, binary files (PDFs too), files
  over 2 MiB (also when loaded anyway), files that aren't UTF-8 and ⌘-clicked
  files outside the folder stay read-only, and the header says why
  ("Read-only · on another machine"); typing says so too.
- **The editor** (`code_view/edit.rs`, `buffer.rs`): caret, typing and input
  methods (composition, any script; measured by the shaper), selection by
  mouse (double and triple click) and keys, cut/copy/paste, undo and redo in
  steps (a typing run, a composition, a paste, a whole Replace All), Tab and
  Shift-Tab in the file's own indentation (over every selected line), Enter
  keeping the line's indentation, Find plus Replace and Replace All (⌥⌘F /
  Ctrl+H; `$1` in a regular expression). Each line keeps its own line break
  and the final newline stays as it was; new line breaks take the file's
  own. An edit re-lays out only the lines it touched; highlighting is redone
  in the background 150 ms after typing stops. Diffs stay read-only.
- **Saving**: ⌘S / Ctrl+S. `write_file(agent, path, text, base)` writes only
  an existing file inside the resolved root (symlinks followed and checked,
  as reads), atomically: a temporary file in the same folder, the file's
  permissions copied, renamed over it. `base` is the text as read: when the
  file on disk differs, nothing is written and the viewer asks.
- **Unsaved changes**: a dot on the tab. Closing the tab, a files tile or
  the window and quitting Pitwall ask first: Save / Don't Save / Cancel.
  Moving the files (a drag, "Open in window", "Dock…", "Move…") asks
  nothing: the same viewer moves, its edits with it.
- **Changed on disk** (the agent edited it): with nothing unsaved it is read
  again quietly. With unsaved changes neither side is overwritten: a bar
  offers **Keep mine** (a save then writes over what is on disk),
  **Take theirs** (the edits are dropped) and **Show difference** (the file
  on disk against the edits). A save that finds the file changed since it
  was read writes nothing and shows the same bar ("Save mine").
- **Markdown**: edited in Code; Preview follows the edits.
- **SVG**: edited in Source; the picture follows the edits.
- **Where agents' files show** (`show_files` / `show_folder`, `pitwall
  open`): one viewer per agent, in one of two places. A **files tile** is a
  slot of its own in the space's layout (a pane with `files` in `ui.json`,
  a peer of the terminal tiles; never a split inside a terminal's tile),
  on the side of a terminal's slot `where` names (right by default; `of`:
  another agent's terminal), or in that slot's place (`where: replace`). It resizes with the dividers, folds after
  terminals when room runs out (it is drawn down to 240×160), and is kept
  with what it had open (`dockedFiles`), opened again after a restart. A
  **window** of its own (`where: window`). The bar drags either (anywhere
  on it but its controls, and the tab strip's empty end; the grip is its
  hint): a press that travels 4 px is a drag, one that doesn't a click, a
  double click nothing; in a window the window moves with it. Into any
  Pitwall window's layout: a band along a side of a tile (a fifth of it,
  28 to 64 px: a new slot on that side), an empty tile, the rest of a
  terminal's tile (its place: the files take its slot as big and where it
  is, and the terminal leaves the layout as when its tile is closed, its
  agent running, shown again from the sidebar or a chip; the preview
  fills the whole tile, more strongly, and says "Files replace api's
  terminal (it keeps running)"), the middle of another agent's files tile
  (the two swap places when both are in that space; else only its side
  bands) or an edge of the main area (a column or row); a tile let go
  outside every Pitwall window becomes a window there. The pointer is followed on the screen (macOS); elsewhere
  "Move…" / "Dock…" list the same places, with "Replace …'s terminal" for
  each terminal in the files' space (their own first). A later call adds tabs wherever
  the files are; a place named that isn't where they are moves them.
  Both draw the viewer's one bar (tree toggle, agent, folder, branch); the
  grip and "Move…" / "Dock…" sit in its slot for a place's controls, then
  "Open in window" and Close (a tile) or Dock and Pin (a window). Each
  keeps its own tree column (`filesTree.tile`, `filesTree.window`; the
  tile's was `beside` and is still read). ⌘P (Go to file) and ⇧⌘F
  (Search) work in a files window too, on the folder it shows.
  Where a tile is, is said one way everywhere (the tool's answer, the drop
  preview, `files-place state`): next to the terminal it was placed
  against while it still touches it ("above demo's terminal"); else from
  the tiles as laid out, never their order in the tree: a column or row
  along an edge of the space beside several tiles ("a column on the right
  of the space"), between two terminals that each have a whole side of it,
  or next to the terminal it shares most of an edge with.
- **An agent removed** from Pitwall takes its files with it: its files
  tile leaves the layout (the gap closes as for a terminal), its window
  closes, and `dockedFiles` forgets it; a tile saved in `ui.json` for an
  agent that no longer exists isn't drawn again after a restart. Files
  with unsaved edits stay instead, reading the agent's folder on this
  computer (a `path:` key), under a note that the agent was removed, with
  Save and Discard; once saved or discarded they close the same way. An
  agent on another machine has nothing editable here: its files just
  close. A restart or resume keeps the agent's id: nothing happens.
- **Which folder** an agent's files are shown from: the `root` it gives
  (a path, or a worktree of its repository by folder name or branch), else
  where it works **now**, else where it started. "Now" is the `cwd` of its
  latest hook (it follows Claude Code into a worktree it enters
  mid-session), else the newest folder it said it works in (`pitwall agent
  track`), else its process's folder where the provider can tell (a shell
  the user `cd`s in); the hook's is kept in memory only, so after a
  restart it is known again from the next hook. Relative paths are taken
  from that folder itself; the tree shows the git worktree holding it
  (its own branch, index and status, read with `git -C` there), unless that
  is the worktree the agent started in (then its own folder, as always). A
  folder that is gone (a removed worktree) is an error that says so,
  never a quiet fallback to the start folder. A files view already showing
  another root for that agent is replaced by the new one: tabs whose file
  is there under the new root by the same path stay open (the same file in
  the other worktree), the rest close; while it has unsaved edits the call
  is refused, telling the agent the user has to save or discard first.
  Agents are told (tool descriptions, server instructions) to give `root`
  when they work in another worktree than the one they started in.
- **Agents** can't write through Pitwall: `show_files` / `show_folder` and
  `pitwall open` only display, and nothing is sent to an agent when the user
  saves.

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
- GPUI viewer's images and PDFs: `pitwall-app/src/explorer/viewer.rs`,
  `image_view.rs` (fit and zoom steps), `pan_zoom.rs` (moving a picture,
  keeping it in view, zoom about the pointer, the drag threshold; tested
  without a window), `pdf_view.rs` (page layout, the
  pages in view, zoom steps, what to draw and let go; tested without a
  window); the PDF engine in `pitwall-app/src/platform/pdf.rs` (the API,
  `can_view_pdf`) and `pdf_macos.rs` (PDFKit, tested on macOS with PDFs
  made by hand and a password-protected one in `platform/testdata`).
