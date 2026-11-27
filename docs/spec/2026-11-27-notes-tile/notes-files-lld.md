# Notes tile: project notes as files (LLD)

How the Notes tile keeps project notes as Markdown files in the
project, with the SDK's `files` service
([platform services §14.3](../tile-sdk/platform-services.md#143-later-on-demand)).
It covers [frd.md](frd.md) FR-1 to FR-9 and FR-17 to FR-19, and
[hla.md §4.1](hla.md#41-saving-a-project-note-pseudocode).

## 1. Which project

A view's project is the one Pitwall says it is open in
(`files::project(view)`): the project of its space, or of the agent it
was opened next to, or the project whose sidebar menu opened it. The
tile never picks a folder. A view moved to another project gets
`project-changed`, and the tile reads that project's notes instead.
Views in different projects show different Project groups; private
notes are the same in every view.

## 2. The notes folder

- The folder is the setting `folder`, relative to the project: `notes`
  by default. The tile lists the `*.md` files directly in it (FR-3);
  sub-folders and other files are not shown.
- The folder is made on the first save of a project note
  (`files::make_folder`), not when the tile opens.
- `.gitignore`d files are not listed (the host's `list` leaves them
  out), so a team can keep `notes/` out of Git or in it.

## 3. File names

- A new project note has no file until it is first saved. Its file name
  then comes from its title (FR-8): lower case, letters and digits kept,
  every other run of characters one `-`, at most 60 characters,
  `.md` added (`Login page flickers!` → `login-page-flickers.md`). An
  empty title gives `note.md`.
- The name is taken with `files::create`, which refuses a name that
  exists; the tile then tries `-2`, `-3`, … up to `-99`.
- After that the name is kept when the title changes. "Rename to the
  title" in the note's bar renames the file to the title's name
  (`files::rename`), with the same numbering when taken.
- A project note's id in the tile is a hash of its project folder and
  path, so a view's open note comes back after a reload.

## 4. Versions and saving

A project note keeps the version it was last read or written at: the
file's size and modified time from `files::stat` (milliseconds).

Saving follows [hla.md §4.1](hla.md#41-saving-a-project-note-pseudocode):

1. `stat` the file.
2. Missing, and the note was read before: **Deleted on disk** (§5).
3. A different version: read the file. The same text as the edit: take
   the new version and stop. Else **Changed on disk** (§5).
4. The same version: `files::write` the text, then `stat` for the new
   version.

Saves come about a second after typing stops, on blur, on hide, on
leaving the note, as for private notes (FR-6). A failed write keeps the
edit unsaved and tries again a second later; the user is told once.

The check and the write are two calls; a change between them is lost to
the write (accepted, as in the HLA).

## 5. Changes on disk

The tile reads the folder again, and each file whose version changed,
when (FR-17, FR-19):

- the watch on the notes folder (`files::watch`) reports a change;
- the tile is shown again (`visibility(true)`) or gets focus;
- a view opens a note.

For each file read again:

| The note in the tile | What happens |
|---|---|
| No unsaved edits | Its text and version are the file's |
| Unsaved edits, the same text as the file | The new version is taken |
| Unsaved edits, different text | **Changed on disk** |
| The file is gone, no unsaved edits | The note leaves the list |
| The file is gone, unsaved edits | **Deleted on disk** |

**Changed on disk** is a banner over the open note, and the note isn't
saved while it shows:

- *Reload*: drop the edits, show the file's text.
- *Keep mine*: write the edits over the file.
- *Save as a copy*: write the edits to a new file named from the title
  with `-copy` (numbered when taken), and show the file's text in the
  original.

**Deleted on disk** offers *Save again* (create the file again with the
edits) and *Discard* (drop the note).

## 6. Permissions

| Without | The tile |
|---|---|
| `files.read` | The Project group says "Keep notes in this project's notes/ folder" with *Allow in Settings* (FR-4); private notes work |
| `files.write` | Project notes are read and shown; a new note is private; editing a project note shows it read-only with a line asking for "Change files in your projects" |
| a project | No Project group: the view isn't in one |

## 7. Private notes per project (left for later)

Private notes stay under `p/_/<id>` in the tile's storage, the same in
every view. Moving them to `p/<project key>/<id>`, with a one-time
migration that would have to guess a project for each old note, is left
open in the plan: with project notes as files, a note that belongs to a
project now goes there.

## 8. Send

The Send picker lists the agents of the note's project first: for a
project note its project, for a private note the view's (G5, G5b). The
rest follow, as before.
