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

**Status:** step 0 built (#119) on SDK phase 1, its SDK gaps G8–G18
closed and seen in Pitwall headless on macOS (#121). v1's Preview,
Send and its permissions built (#131); project notes wait for `files.*`
and G5.

## SDK requests

Gaps from [frd.md §7](frd.md#7-sdk-gaps). Each is done when the Tile SDK
design records a decision on it (in its section or in
[README §10.3](../tile-sdk/README.md#103-decisions)), made by whoever owns
that design. The step it blocks is named.

- [x] G1, G1b: the `input` element's record (multi-line, rows, font, who owns the value) and which of ⌘⏎ or submit wins in a tile. Blocks step 0 (#111: WIT 0.1's `input` and [ui.md §3.5](../tile-sdk/ui.md#35-keyboard-and-focus): a focused multi-line field submits on ⌘⏎)
- [x] G1c: input value size is measured in the phase 1 work, with a number recorded for a 256 KiB note. Blocks step 0's FR-10 limit (#119: about 3 ms a change round trip in the harness, release tile, Apple-silicon Mac; the value isn't repeated in frames)
- [x] G8: `TileView` and folder install, then the `input` layout (`size_full` vs `rows`), icons and look checked in Pitwall. Blocks step 0's done (#121: `tiles/notes_shots.rs` installs Notes from its folder and draws it headless on macOS; a multi-line field given a height now fills it, icon buttons are square; not yet seen on Windows or Linux)
- [x] G9: the harness sends focus, resize and theme, restarts without state, reads a field's text, and has a virtual clock. Blocks tests of save on focus loss and of the side-by-side layout (#121: also `focus-changed`, stored values at start, `field()`, the text and focus in the snapshot, and keys the tile wouldn't get refused)
- [x] G10: a blur event for one text field. Not blocking; the 1 s timer covers it (#121: `focus-changed(node-focus)` in WIT; `on_focus`/`on_blur` in the SDK)
- [x] G11: reading many storage keys in one call. Blocks v1 with more than about 190 notes loading in one second (#121: `storage.get-many`, ≤ 1024 keys a call; the SDK's `entries_json`)
- [x] G12: the icon set listed in the SDK and checked. Not blocking (#121: `Icon` has all 51, `ICON_NAMES` in core; `check` and the harness warn)
- [x] G13: the reserved keys listed and checked; when bindings fire while a field has focus. Not blocking (#121: listed in ui.md; the host drops them with a Log line; plain keys don't fire while a field has focus; Notes binds N and /)
- [x] G14: focus a field from the element, not by its full key path. Not blocking (#121: the docs now agree, `cx.focus` and `scroll_to` take the key path without the root's; focusing from the element stays open)
- [x] G15: button size apart from its look. Not blocking (#121: `button-size`; `.small().danger()`)
- [x] G16: `extension new --sdk` writes relative paths and no skill copies inside Pitwall's checkout. Not blocking (#121: relative inside the checkout; `--no-skill`)
- [x] G17: which WASI interfaces (clocks, random) a tile has, documented. Not blocking (#121)
- [x] G18: `Div` and the other builders in the prelude; a show-on-hover style. Not blocking (#121: `group` and `visible_on_group_hover`; Notes shows a Delete on a row on hover)
- [ ] G4b: `files` gains remove, rename and create-folder, or the design states that the WASI route under `/projects/<n>` does them under the same grants and path checks. Blocks v1
- [ ] G5, G5b: a tile learns its space's project, and grants and agents name projects by the same id. Blocks v1's per-project lists and agent picker
- [ ] G7: what happens to host calls in flight at suspension. Blocks v1's save on hide
- [ ] G4: `files.*` limited to `paths` in the manifest. Not blocking; without it v1 asks for the whole project
- [x] G2: the selection of an input. Not blocking; without it v1 sends the whole note or a section (#135: in WIT 0.1 already, `input-changed` and `input-selected` carry it; the harness selects with `select_text`)
- [ ] G3: a file-change event. Not blocking v1
- [x] G6: a link event from `markdown`, and task boxes as `toggle` (#126)

## Step 0: preview on phase 1

Waits for SDK phase 1 (WIT 0.1, `storage`, `tasks`, folder install).
Done when the preview installs from a folder on macOS, Linux and Windows,
and an SDK-preview author other than its writer has used it for a day.

- [x] `tiles/notes/` scaffolded with `pitwall extension new`, its own Cargo project, not a workspace member; `pitwall extension check` passes (#119: from `--template list`; paths made relative and the skill copies dropped, G16)
- [x] CI job: builds `tiles/notes` for `wasm32-wasip2`, runs its harness tests and `check`, on every pull request that touches `tiles/` or `sdk/` (#119: `tile-notes`, gated like `tile-c4`; verified locally, since CI runs only on pull requests into `main`)
- [x] CI dependency check: fails when `tiles/notes` depends on any `pitwall-*` crate other than `pitwall-extension` (and `pitwall-extension-test` as a dev-dependency) or on a path under `crates/`; a test branch that adds `pitwall-core` fails it (#119: `tiles/notes/scripts/check-deps.sh`; also keeps the harness's own crates out of what `tile.wasm` is built from; checked locally with `pitwall-core` as a dependency and a dev-dependency, and the harness as a normal dependency)
- [x] Private notes: new, edit, delete, list by last change, title from the first line, kept in `storage` under `p/_/…`; harness test: create, edit, restart the tile, the text is there (#119: "restart" is the harness's `reload`, which also restores state, G9; empty notes are dropped when left; delete asks inline)
- [x] Multi-line editing with the kit's input (per G1); harness test types three lines and reads them back (#119: read back from storage and the list, since the harness can't read a field's text, G9; ⌘⏎ saves and goes back to the list)
- [x] Autosave about 1 s after typing stops and on `visibility(false)`; unsaved text kept through `save-state`; harness tests for both, with the virtual clock (#119: on the wall clock, the harness has no virtual one, G9; also saves on leaving a note and on the tile's focus loss, untested, G9, G10)
- [x] Filter field over titles and text; harness test (#119: every word must appear, case-insensitive)
- [x] Opened with a draft note (`input.schema.json`, `{ "title"?, "text" }`): unsaved and marked until Keep, Discard drops it, new content replaces it in place; harness tests (#119; moved here from v1's agent draft, since phase 1 already delivers content to `on_content`)
- [x] Each SDK gap found while building is added to [frd.md §7](frd.md#7-sdk-gaps) and the list above, in the same pull request (#119: G8–G18)
- [x] Seen in Pitwall, installed from its folder, in Dark and Light, narrow (one pane) and wide (list and note side by side); fixed where it doesn't look like Pitwall's own tiles. Waits for G8 (#121: headless on macOS, `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app notes_shots -- --ignored`; the editor's field fills the pane, icon buttons square, the bars' buttons small)

## v1: project notes, preview, send

Waits for SDK phase 2 and the on-demand `files.read/write` service
([14.3](../tile-sdk/platform-services.md#143-later-on-demand)), which this
step triggers. Done when every FR in [frd.md §5](frd.md#5-functional-requirements)
holds, with a harness test for each, and the README below is published.

- [ ] LLD `notes-files-lld.md` in this folder: file naming from titles, the notes folder setting, versions and the conflict flow of [hla.md §4.1](hla.md#41-saving-a-project-note-pseudocode), private keys per project; reviewed in its pull request
- [ ] Project notes: list `*.md` in the notes folder, open, edit, new, rename, delete (FR-1–FR-9); harness tests against a temp project folder
- [ ] No-grant state: the Project group explains and asks for `files.read`; private notes still work (FR-4); harness test with the grant refused
- [ ] Changes on disk: reload on show, focus and open; the version check before each save with Reload, Keep mine, Save as a copy (FR-17, FR-18); harness test that changes the file between read and save
- [x] Preview with the `markdown` element, switched by a segmented control and a key (FR-11, FR-12); text snapshot test. Preview, its tabs and ticking tasks there are in #126; the key is left (#131: ⌘/, which isn't reserved and works while the field has focus; saves before Preview)
- [ ] Send to agent: agent picker from `agent.list`, the note's project first; whole note, section in Preview, selection once G2 exists; 4 000-character check before the call (FR-13–FR-16); harness test with a fake method table that records `queue.add` (#131: a picker in a popover from the note's bar, Pitwall's plain terminals left out; the whole note or one section under a heading, chosen in the picker in Edit and Preview alike; the 4 000-character check; Busy, Denied, gone agents shown in the picker. #135: the selection, first when there is one, shown in the picker. Still open: the note's project first waits for G5)
- [ ] Open in editor through `viewer.open` for notes over the edit limit or on request (FR-10); harness test records the call (`show.view` and `show.file` exist, #135; waits for project notes, `files.*`, since private notes are no files)
- [ ] Private notes moved from `p/_/` to per-project keys once G5 exists, with a one-time migration; harness test
- [ ] Manifest permissions and reasons as in [frd.md §5.6](frd.md#56-permissions-as-the-user-sees-them); `check` passes; each permission refused in turn leaves the tile working, tested (#131: `agents.read`, `agents.queue`; harness tests with neither and with only `agents.read`. `files.*` come with project notes)
- [x] `tiles/notes/README.md` with what it does, each permission and why, and screenshots in Dark and Light (#131: the Send picker, dark and light, from `notes_send_shots`)
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
