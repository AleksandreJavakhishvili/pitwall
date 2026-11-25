# The text editor

A text editor in a tile of a space's layout, beside the terminals, the
files tiles, web pages and Settings: any text file on this computer, by
its full path, in tabs. It edits with the same code view as the files
viewer's quick fixes (`crates/pitwall-app/src/code_view`), in an editor's
way: several cursors, comments, bracket pairs, indenting Enter.

Status: built (GPUI). Code: `crates/pitwall-app/src/editor/` (the editor),
`crates/pitwall-app/src/main_screen/editor_tile.rs` (its tile and window),
`crates/pitwall-core/src/explorer/anywhere.rs` (saving, creating and git,
by full path).

## Opening it

- **New file** in the command palette (⌘K): the editor with a new, empty
  file, named on its first save.
- **Open file in editor…** in the palette: the system's open panel, then
  the files picked, each in a tab.
- **Open in editor** in the files viewer's header, for a text file of an
  agent on this computer (not one mirrored from another machine): the same
  file in the editor, at the line the viewer's cursor is on.
- **Show the editor** in the palette while it is open somewhere.
- The editor opens right of the focused tile (in it when that tile is
  empty), as Settings' tile does. There is one editor; opening a file
  again shows its tab.

Agents don't open the editor. `show_files` keeps opening the viewer:
the editor adds nothing an agent can do or read, and the viewer's "Open in
editor" is one click away.

## The tile

The tiles' header: grip (drag to move, or out of the window for a window
of its own), the file shown's path, New file, Open file…, Save, Move…,
Maximise (⌘⏎), a window of its own, Close. In a window of its own: grip,
Place in a layout, Keep on top.

Under it, the tabs: the file's icon and name, its git status letter (as
in the files tree), a dot while it has unsaved edits (the close button
otherwise). A click shows it; the close button or a middle click closes
it; ⌃Tab and ⌃⇧Tab go through them. A tab's tooltip is its full path.

Under the tabs, the file; at the bottom a status line: line and column,
the number of cursors when there are several, the file's indentation
(tabs, or spaces and how many), LF or CRLF, and "Read-only: why" when it
isn't edited. With no file open: "No files open", New file and Open file….

It moves, maximises, goes to a window of its own and back like the other
tiles, and is back after a restart with its files: `ui.json` keeps the
pane's `editor: true` (or `editorWindow: true`) and `editor.tabs` (full
paths) and `editor.active`. Unsaved text is not kept; quitting asks about
it first. A file that is gone at the next start says so where its text
would be.

## Editing

Everything the viewer's quick fixes do (typing, input methods, undo and
redo with typing grouped, Find and Replace, cut, copy and paste, Tab and
⇧Tab on blocks of lines, a file's line breaks and final newline kept),
and:

| Keys (macOS; Ctrl for ⌘ elsewhere) | What |
|---|---|
| ⌘-click | A cursor there too (on one: that one goes) |
| ⌘D | The word at the cursor selected; again, a cursor at its next match |
| ⌘⇧L | A cursor at every match of the selection (or of the word) |
| ⌥⌘↑ / ⌥⌘↓ (Ctrl+Alt elsewhere) | A cursor on the line above or below too |
| Esc | Back to one cursor |
| ⌘/ | The lines commented out, or back in, by the file's language |
| ⌃G (Ctrl+L elsewhere) | Go to a line (`120`, or `120:8` for a column) |
| ⌘S, ⇧⌘S, ⌥⌘S | Save, Save As, Save all |
| ⌘W | Close the file (asks about unsaved edits) |

- **Several cursors.** Moving and selecting keys move every cursor;
  typing, Backspace, Delete, Enter, Tab, cut and paste act at each, as one
  undo step. Cursors that meet become one. Pasting as many lines as there
  are cursors puts one at each. Page Up/Down and ⌘↑/↓ go back to one
  cursor. Undo returns to the main cursor. Diffs and read-only files keep
  one cursor.
- **Comments** use the language's line comment (`//`, `#`, `--`, `;`, `%`)
  at the lines' smallest indentation; languages with block comments only
  (CSS, HTML, XML, Markdown, OCaml) wrap each line. A file Pitwall doesn't know
  the comment of is left alone.
- **Pairs.** `(`, `[`, `{`, quotes and backticks type their closing one
  when the next character is free; typing the closing one steps over it;
  with text selected they wrap it; Backspace in an empty pair removes
  both. Quotes aren't paired after a letter or digit; in Rust, Markdown,
  OCaml and plain text `'` isn't paired at all.
- **Indenting.** Enter keeps the line's indentation, adds a step after an
  opening bracket (after `:` in Python and YAML), and between a pair of
  brackets puts the closing one on a line of its own. Home goes to the
  line's text first, then to its very start. Indentation steps follow the
  file (tabs, or the spaces it uses).
- **The cursor's line** is marked; line numbers are on the left.

The viewer's quick fixes keep the plain behaviour (no pairs, no indenting
Enter, Home to the line's start); several cursors and ⌘/ work there too.

## Saving

- ⌘S saves the file shown, ⌥⌘S every file with unsaved edits. A new file
  asks where (the system's save panel) on its first save; ⇧⌘S asks for
  any file, and the tab then shows the new file (another tab with that
  file closes). The save panel asks before replacing a file.
- A save writes atomically (a temporary file in the same folder, renamed
  over the file, its permissions kept) and only over the file as it was
  read: if it changed since, nothing is written and the bar asks (below).
- Closing a file, the tile, its window, or Pitwall with unsaved edits
  asks: Save / Don't Save / Cancel (a new file's Save asks where).

## When an agent changes the file

Open files are looked at every two seconds (their time and size), and
read again when they changed.

- Nothing unsaved: the new text is shown at once; the cursor and the
  scroll stay.
- Unsaved edits: they are never replaced. A bar says "Changed on disk. You
  have unsaved changes." with **Keep mine** (your edits stay; a save
  writes them over the file), **Take theirs** (the file on disk replaces
  your edits) and **Show difference** (the file on disk against your
  edits, in place of the editor, until pressed again).
- A save that finds the file changed says "Changed on disk since it was
  read: saving would overwrite it." with **Save mine** in place of Keep
  mine.

This is the viewer's flow, with the same words.

## Git

- The tab shows the file's status in its repository (M, A, D, R, U), from
  `git status` on the file.
- The margin marks lines changed since HEAD: green for added, blue for
  changed, a red notch where lines were deleted. A file git doesn't track
  yet is all added; a file outside a repository has no marks. They follow
  the edits (worked out again a quarter of a second after typing stops,
  in the background) and HEAD (after a save, and when the file is read
  again).
- Git runs as all of Pitwall's git does: no locks, no prompts.

## Limits

- Text files up to 2 MiB are edited; from 2 to 10 MiB they are shown
  read-only ("Read-only: too large to edit (over 2 MiB)"). Larger files,
  binary files and pictures aren't opened: the tab says why.
- A file that isn't valid UTF-8 is shown read-only.
- Lines longer than 10,000 bytes are cut where they are shown; the file
  keeps them and saving writes them back unchanged.
- Typing in a large file touches only its line; highlighting is redone in
  the background once typing pauses.
- Long lines don't wrap yet (no soft wrap).

## Not in the editor

- Completion, hover, go to definition and other language-server help.
- Debugging.
- Editor extensions.
- A terminal inside the editor (the terminals are tiles beside it).
- Soft wrap.

A language server would sit beside a tab (`editor::Tab`: the file's path,
its text by version, the cursors) and draw through the code view's marks,
as git's changed lines do (`CodeView::set_line_marks`).

## Bench

`editor …` through the bench's command hook (nothing is clicked or typed
on the screen): `open <full path> [line]`, `new`, `type <text>` (at every
cursor; `\n` is Enter), `key <name>` (`save`, `save-all`, `close`, `goto`,
`undo`, `redo`, `comment`, `next-match`, `all-matches`, `cursor-above`,
`cursor-below`, `escape`, `tab`, `backspace`, `select-all`), `goto
<line[:col]>`, `save-to <full path>`, `keep-mine`, `take-theirs`,
`difference`, `max`, `close`, `state` (on stderr).
