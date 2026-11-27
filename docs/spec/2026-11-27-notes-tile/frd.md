Status: design, not built. A seed tile of the [Tile SDK](../tile-sdk/README.md)
([10.1](../tile-sdk/README.md#101-seed-tiles-first-party-public-sdk-only)).
Architecture in [hla.md](hla.md), progress in [plan.md](plan.md).

# Notes tile: functional requirements

A tile for notes, beside the agents. It is one of the first two seed
tiles, with CI and pull requests. Like every seed tile it is built on the
public SDK alone, so every gap it hits is a gap in the SDK
([10.1](../tile-sdk/README.md#101-seed-tiles-first-party-public-sdk-only)).

Marks: **[SDK]** is designed in `docs/spec/tile-sdk/`; **[gap]** is
something the SDK design lacks, listed in [SDK gaps](#7-sdk-gaps);
*guess* marks a claim not checked against the repository.

## 1. What notes mean in Pitwall

The user is one person supervising several coding agents per project.
While the agents run, the user:

- **catches thoughts** that come up while watching: "the api agent should
  also handle 429s", "ask web to drop the old route", a TODO for later;
- **keeps context** for the project: decisions, a plan, links, things to
  tell the next agent. Some of it is for agents and teammates to see;
  some of it is private;
- **turns a note into a prompt**: sends it, or part of it, to an agent
  as its next instruction.

Two places a note can live, and why both:

| | Project notes | Private notes |
|---|---|---|
| Where | Markdown files in the project, in a `notes/` folder (the folder name is a setting) | The tile's own storage, outside the project ([14.2](../tile-sdk/platform-services.md#142-the-v0x-minimum-built-in-phases-12)) |
| Who sees them | The user, the project's agents (with their own file tools), git and so teammates | Only the user, in this Pitwall |
| Good for | Plans, decisions, context agents should read | Scratch, half-thoughts, things not for the repo |
| Needs | `files.read`, `files.write` on the project (on demand, [14.3](../tile-sdk/platform-services.md#143-later-on-demand)) | Nothing: own data only |

Project notes are plain files on purpose. Agents already read and write
files in their project with their own tools. The tile adds no second way
for agents to reach them ([11.1](../tile-sdk/agent-tools.md#111-why-and-what)).

## 2. Personas

- **The supervisor.** Runs two to six agents on one or two projects.
  Watches their terminals, answers their questions, queues the next
  instruction. Wants capture in one step and no switching to another app.
- **The teammate** (indirect). Pulls the repository and reads
  `notes/` in their own editor. Never sees private notes.
- **The agent** (v2). Reads project notes with its own tools today. With
  the tile's tools (phase 3) it can put a draft in front of the user and
  read the private notes the user allows.

## 3. User stories

v1:
1. While three agents run, I press New in the Notes tile, type "web: the
   login page flickers on reload", and look back at the terminals. The
   note is saved without a Save button.
2. I write a plan in `notes/auth.md` and switch to Preview to read it
   with headings and lists. The api agent reads the same file with its
   own tools when I tell it to.
3. I select the "Next steps" section of a note and press Send to agent,
   pick "api", and the text lands in api's Next up, marked "From Notes",
   waiting for my Send.
4. A note is private: I make it in Private, and it never appears in the
   repository or in git status.
5. An agent edits `notes/auth.md` while I have it open. When I come back,
   the tile shows its new text; if I had unsaved edits, it asks before
   overwriting either side.
6. A note grows long. I press Open in editor and keep writing in
   Pitwall's editor ([editor.md](../editor.md)).

v2 (phase 3):
7. I ask an agent "summarise what you found as a note". A draft note
   appears in the tile marked "from api", with Undo. I keep it as a
   project note or a private note.
8. I allow agents to read Notes once. An agent asks for my private note
   "release checklist" and continues from it.

## 4. v1 scope and why

v1 is the smallest tile that uses what makes Notes a Pitwall tile rather
than a text editor: notes that live with the project and its agents, and
a path from a note to an agent's prompt.

**In v1:**
- **Project notes** as Markdown files in the project's `notes/` folder:
  list, open, edit, new, rename, delete.
- **Private notes** in the tile's storage, kept per project.
- **Quick capture**: New is one click or one key in the tile; the title
  is the first line; saving is automatic.
- **Edit and Preview**: a multi-line plain-text editor and an inert
  Markdown preview ([3.3](../tile-sdk/ui.md#33-the-ui-description)).
- **Send to agent**: the whole note, a selection or one section, into an
  agent's Next up through `queue.add` ([14.2](../tile-sdk/platform-services.md#142-the-v0x-minimum-built-in-phases-12)).
- **Open in editor** for long writing, through `viewer.open` (the `show`
  permission).
- **Changes on disk** are picked up and never silently overwritten.

**Why this cut:**
- Capture and send-to-agent are the jobs a supervisor does many times a
  day. Everything else (tags, links, search) is useful only once there
  are many notes.
- Files in `notes/` make agents and teammates part of it with no tile
  tools at all, so v1 needs no phase 3.
- It needs only phase 2 plus the files service. Notes is the named
  trigger for `files.read/write` ([14.3](../tile-sdk/platform-services.md#143-later-on-demand)),
  so v1 is where that service gets built.

**Before v1: a preview on phase 1 only.** Private, plain-text notes,
built as soon as phase 1 ships. It uses only core elements, `input`,
`storage`, `tasks` and `ui`. It has no project notes, no preview and no
send. Its purpose is to prove phase 1 with a real tile and find gaps in
the `input` element early. Details in [plan.md](plan.md#step-0-preview-on-phase-1).

## 5. Functional requirements

### 5.1 Notes list
- FR-1. The tile shows two groups, **Project** and **Private**, each a
  list of notes by title, newest change first.
- FR-2. A filter field narrows the list by title and text of the notes
  already loaded.
- FR-3. Project notes are the `*.md` files directly in the notes folder.
  Sub-folders are not shown in v1.
- FR-4. With no `files.read` grant, the Project group shows one line,
  "Keep notes in this project's notes/ folder", and a button that asks
  for the grant. Private notes work without it.

### 5.2 Editing
- FR-5. A note opens in a multi-line, soft-wrapped plain-text field with
  Pitwall's IME, selection, undo and clipboard (the kit's `TextInput`
  [today], [gap G1](#7-sdk-gaps)).
- FR-6. Changes are saved about one second after typing stops, when the
  note loses focus, and when the tile is hidden. No Save button.
- FR-7. Unsaved text survives the tile being dropped or reloaded
  (`save-state`, [2.3](../tile-sdk/runtime.md#23-lifecycle)).
- FR-8. A new note's title is its first line. A project note's file name
  is made from the title when first saved (`login-page-flicker.md`) and
  then kept; Rename changes it.
- FR-9. Delete asks once, then removes the file or the stored note.
- FR-10. Notes up to 256 KiB edit in the tile (*guess*; set by
  measurement). Above that the tile shows Preview and Open in editor
  only.

### 5.3 Preview
- FR-11. Edit and Preview switch with a segmented control and a key.
- FR-12. Preview draws the note with the host's inert Markdown view: no
  HTML, no remote images [today].

### 5.4 Send to an agent
- FR-13. Send to agent offers the agents of the note's project (by
  `agents.read`), then calls `queue.add` with the chosen text.
- FR-14. What is sent: the selection if there is one ([gap G2](#7-sdk-gaps)),
  otherwise the whole note. In Preview, each heading's section has its own
  Send, which works without G2.
- FR-15. The text lands in the agent's Next up marked "From Notes" and
  waits for the user's Send, unless the user turned on sending without
  asking [today's `agents.queue` rules].
- FR-16. Text over 4 000 characters, or a fourth waiting item for the
  same agent, is refused by Pitwall [today's `queue.add` limits]. The tile
  says so before sending ("Too long to send: 5 210 of 4 000 characters.
  Select a part.").

### 5.5 Changes on disk
- FR-17. When the tile is shown again, gets focus, or opens a note, it
  re-reads the folder and the open file.
- FR-18. Before each save it checks that the file is unchanged since it
  was read. If it changed and there are no unsaved edits, the tile loads
  the new text. If both changed, it shows "Changed on disk" with Reload,
  Keep mine and Save as a copy.
- FR-19. With file watching ([gap G3](#7-sdk-gaps)) the open note also updates
  while shown. Without it, FR-17 and FR-18 are enough for v1.

### 5.6 Permissions, as the user sees them
| Permission | Reason shown | Needed for |
|---|---|---|
| `files.read` | "To list and read notes in the projects you choose" | Project notes |
| `files.write` | "To save notes in the notes/ folder of the projects you choose" | Saving project notes |
| `agents.read` | "To list the agents you can send a note to" | Send to agent |
| `agents.queue` | "To put a note in an agent's Next up, waiting for your Send" | Send to agent |
| `show` | "To open a long note in Pitwall's editor" | Open in editor |

Each one is optional. Without it, the tile hides or explains the part it
gives. The tile never writes outside the notes folder; with today's
design the grant still covers the whole project ([gap G4](#7-sdk-gaps)).

## 6. Out of scope

Out of v1 and v2, with the reason:
- **Agent tools** (`show`, `present`, `read`): v2, phase 3.
- **An `act` tool that writes project notes**: agents already edit files
  in their project with their own tools.
- **Rich editing** (WYSIWYG, highlighting while typing, live
  side-by-side preview): the SDK's `code` view is read-only and `input` is
  plain text. Open in editor covers long writing.
- **Tags, wiki links, backlinks, full-text search across projects.**
- **Checkboxes you tick in Preview, clickable links**: the Markdown
  element sends no events ([gap G6](#7-sdk-gaps)).
- **Images and attachments.**
- **Sub-folders and notes outside `notes/`.**
- **Editing `.gitignore`** to keep `notes/` out of git: the user's choice,
  in their own editor.
- **Sync between devices or people, sharing, accounts**: Pitwall runs no
  server ([14.4](../tile-sdk/platform-services.md#144-not-provided)).
- **Notes on a peer's machine** (tailnet peers are read-only).
- **A toolbar button for capture from anywhere** ([3.9](../tile-sdk/ui.md#39-toolbar-items)):
  a good next step after v1, not in it ([gap G5](#7-sdk-gaps) first).
- **Notes per agent or per worktree**: notes belong to the project in v1.

## 7. SDK gaps

Found while specifying this tile. Each needs a decision in the Tile SDK
design before the step that depends on it. This spec doesn't edit that
design; [plan.md](plan.md#sdk-requests) tracks the requests.

| # | Gap | Why Notes needs it | Proposal (open) | Blocks |
|---|---|---|---|---|
| G1 | The `input` element's record is not defined: nothing says multi-line, visible rows, mono font, or who owns the value | A note is many lines. The kit's `TextInput` already has a multi-line mode [today] | `input` gains `multiline(rows)` and `font`; the host owns the text and sends changes, and the guest sends a value only to replace it (with a revision, [2.4](../tile-sdk/runtime.md#24-threading-with-gpui)) | Step 0 |
| G1b | ⌘⏎ is reserved by Pitwall to maximise a tile ([3.5](../tile-sdk/ui.md#35-keyboard-and-focus)), but the kit's multi-line input uses ⌘↵ to submit, and `submit` names "Enter / ⌘⏎ in an input" ([3.4](../tile-sdk/ui.md#34-events)) | Which one wins in a tile's multi-line input is undefined | Pitwall's ⌘⏎ wins; a multi-line input in a tile submits with another key, or not at all | Step 0 |
| G1c | Large values: the whole text crosses the boundary on each change (up to 2 per frame), and maybe in each frame | Notes can be tens of KiB; the frame limit is 2 MiB | Measure in step 0; an input whose value is not repeated in every frame (G1) solves most of it | Step 0 |
| G2 | `input-changed` carries the value, not the selection | "Send selection" | `input-changed` gains the selection range, or `ui` gains a call that returns it | v1, Send selection only |
| G3 | `files.watch` is in the host API table ([3.8](../tile-sdk/runtime.md#38-host-apis)), but no event in [3.4](../tile-sdk/ui.md#34-events) delivers a change, and nothing says what happens while suspended | Agents edit notes while the user has them open | A `files-changed(project, paths)` event, merged while suspended and delivered on show | Not v1 (FR-17/18 suffice) |
| G4 | `files.read/write` cover a whole project; `files.write` is High risk, "can change code agents and the user run" ([5.1](../tile-sdk/permissions.md#51-new-permissions-new)) | Notes only needs `notes/`. Asking for write access to all code is a hard sell for a notes tile | The manifest's object form takes `paths` (`"paths": ["notes/"]`), shown on the permission screen and enforced by the host's path check | v1 (works without it, at a higher risk) |
| G4b | The `files` interface has `read`, `write`, `list`, `stat`, `watch`, but no delete, rename or create-folder; WASI under `/projects/<n>` could do them, but how that path meets the grants and Pitwall's path checks is not stated | Rename, Delete, and making `notes/` the first time | Add `remove`, `rename`, `create-dir` to `files`, under `files.write` | v1 |
| G5 | A tile doesn't know which project its space belongs to. `init` and `render(view-id)` carry no project; projects are a `projects.read` read, which is on demand ([14.3](../tile-sdk/platform-services.md#143-later-on-demand)) | Private notes are kept per project, and the Project group must show the space's project | The view context carries the space's project as an opaque key with no permission, and its name and path with `projects.read` (or the `files` grant for that project) | Step 0 keeps one private list; v1 |
| G5b | `files` grants are per project picked, and `pick-project` returns a task; how a granted folder (`/projects/<n>`) maps to Pitwall's project, and to `agents.read`'s agents of that project, is not stated | Send to agent lists the note's project's agents | Grants and agents name projects by the same id | v1 |
| G6 | The `markdown` element sends no events: no link clicks, no checkbox toggles | Nice in Preview; not needed in v1 | A `link-clicked(url)` event; relative links to other notes open them | Not v1 |
| G7 | Whether a host call started at `visibility(false)` (the last save) finishes before the tile is suspended is not stated ([2.3](../tile-sdk/runtime.md#23-lifecycle)) | FR-6 saves when hidden | Calls in flight at suspension complete; `save-state` (FR-7) is the fallback | v1 |

Also noted: the SDK design names the crate `pitwall-extension`, but its
examples import `pitwall_tile::…` ([4.2](../tile-sdk/authoring.md#42-a-diagram-tile-as-an-author-writes-it)).
The Notes source will use whatever the published crate is called.
