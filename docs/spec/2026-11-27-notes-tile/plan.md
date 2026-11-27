# Notes tile: plan and progress

This is where the Notes tile's progress is tracked (see the `sdd` skill in
`.claude/skills/sdd/`). Requirements are in [frd.md](frd.md), the
architecture in [hla.md](hla.md). Tick an item in the pull request that
finishes it, once its definition of done holds on `main`, and put the
pull request number after it. When a pull request changes the plan, it
updates this file too.

The Tile SDK's own phases are tracked in
[tile-sdk/plan.md](../tile-sdk/plan.md). This plan only says which of
them each step waits for, and it doesn't tick their items.

**Status:** not started. Waits for SDK phase 1.

## SDK requests

Gaps from [frd.md §7](frd.md#7-sdk-gaps). Each is done when the Tile SDK
design records a decision on it (in its section or in
[README §10.3](../tile-sdk/README.md#103-decisions)), made by whoever owns
that design. The step it blocks is named.

- [ ] G1, G1b: the `input` element's record (multi-line, rows, font, who owns the value) and which of ⌘⏎ or submit wins in a tile. Blocks step 0
- [ ] G1c: input value size is measured in the phase 1 work, with a number recorded for a 256 KiB note. Blocks step 0's FR-10 limit
- [ ] G4b: `files` gains remove, rename and create-folder, or the design states that the WASI route under `/projects/<n>` does them under the same grants and path checks. Blocks v1
- [ ] G5, G5b: a tile learns its space's project, and grants and agents name projects by the same id. Blocks v1's per-project lists and agent picker
- [ ] G7: what happens to host calls in flight at suspension. Blocks v1's save on hide
- [ ] G4: `files.*` limited to `paths` in the manifest. Not blocking; without it v1 asks for the whole project
- [ ] G2: the selection of an input. Not blocking; without it v1 sends the whole note or a section
- [ ] G3: a file-change event. Not blocking v1
- [ ] G6: a link event from `markdown`. Not blocking v1

## Step 0: preview on phase 1

Waits for SDK phase 1 (WIT 0.1, `storage`, `tasks`, folder install).
Done when the preview installs from a folder on macOS, Linux and Windows,
and an SDK-preview author other than its writer has used it for a day.

- [ ] `tiles/notes/` scaffolded with `pitwall extension new`, its own Cargo project, not a workspace member; `pitwall extension check` passes
- [ ] CI job: builds `tiles/notes` for `wasm32-wasip2`, runs its harness tests and `check`, on every pull request that touches `tiles/` or `sdk/`
- [ ] CI dependency check: fails when `tiles/notes` depends on any `pitwall-*` crate other than `pitwall-extension` (and `pitwall-extension-test` as a dev-dependency) or on a path under `crates/`; a test branch that adds `pitwall-core` fails it
- [ ] Private notes: new, edit, delete, list by last change, title from the first line, kept in `storage` under `p/_/…`; harness test: create, edit, restart the tile, the text is there
- [ ] Multi-line editing with the kit's input (per G1); harness test types three lines and reads them back
- [ ] Autosave about 1 s after typing stops and on `visibility(false)`; unsaved text kept through `save-state`; harness tests for both, with the virtual clock
- [ ] Filter field over titles and text; harness test
- [ ] Each SDK gap found while building is added to [frd.md §7](frd.md#7-sdk-gaps) and the list above, in the same pull request

## v1: project notes, preview, send

Waits for SDK phase 2 and the on-demand `files.read/write` service
([14.3](../tile-sdk/platform-services.md#143-later-on-demand)), which this
step triggers. Done when every FR in [frd.md §5](frd.md#5-functional-requirements)
holds, with a harness test for each, and the README below is published.

- [ ] LLD `notes-files-lld.md` in this folder: file naming from titles, the notes folder setting, versions and the conflict flow of [hla.md §4.1](hla.md#41-saving-a-project-note-pseudocode), private keys per project; reviewed in its pull request
- [ ] Project notes: list `*.md` in the notes folder, open, edit, new, rename, delete (FR-1–FR-9); harness tests against a temp project folder
- [ ] No-grant state: the Project group explains and asks for `files.read`; private notes still work (FR-4); harness test with the grant refused
- [ ] Changes on disk: reload on show, focus and open; the version check before each save with Reload, Keep mine, Save as a copy (FR-17, FR-18); harness test that changes the file between read and save
- [ ] Preview with the `markdown` element, switched by a segmented control and a key (FR-11, FR-12); text snapshot test
- [ ] Send to agent: agent picker from `agent.list`, the note's project first; whole note, section in Preview, selection once G2 exists; 4 000-character check before the call (FR-13–FR-16); harness test with a fake method table that records `queue.add`
- [ ] Open in editor through `viewer.open` for notes over the edit limit or on request (FR-10); harness test records the call
- [ ] Private notes moved from `p/_/` to per-project keys once G5 exists, with a one-time migration; harness test
- [ ] Manifest permissions and reasons as in [frd.md §5.6](frd.md#56-permissions-as-the-user-sees-them); `check` passes; each permission refused in turn leaves the tile working, tested
- [ ] `tiles/notes/README.md` with what it does, each permission and why, and screenshots in Dark and Light
- [ ] Docs examples: marked regions in the source (`// docs:start …`) for `input`, `markdown`, `storage` and `files`, used by the Tiles and UI components and Platform services pages ([9.6](../tile-sdk/authoring.md#96-website-docs))
- [ ] Used daily by the owner for a week with three or more agents running; the problems found are fixed or listed here

## v2: agent tools

Waits for SDK phase 3. Done when an agent Pitwall started can open the
tile, draft a note and, once allowed, read one, and the README says what
agents can do.

- [ ] Tools `open` (`show`), `draft_note` (`present`, with `mode: append`), `read_note` and `list_notes` (`read`) with `#[tools]`, as in [hla.md §6](hla.md#6-v2-tools-for-agents); harness tests call each as an agent
- [ ] Drafts with `Presented<Draft>`: marked "from <agent>", Undo, Keep as project note, Keep as private note, Drop; harness test for Undo after an append
- [ ] `agents/SKILL.md`: when to use `draft_note`, that project notes are files in `notes/`; only Notes' own tools; `check` passes
- [ ] Command `new_note` in the palette and the CLI; `pitwall extension run notes new_note` creates a note
- [ ] README gains "What agents can do with Notes"

## Later

- [ ] Moved to its own repository when SDK phase 5 opens the marketplace: history kept, a published `pitwall-extension` version pinned, signed `.pwtile` releases, listed in the index; `tiles/notes/` removed from this repository and the docs build reads examples from a pinned tag
- [ ] Toolbar button for capture from anywhere ([3.9](../tile-sdk/ui.md#39-toolbar-items)), once G5 says which project a click is for; decided after v1 is in use
- [ ] Live updates while shown, once G3 exists
