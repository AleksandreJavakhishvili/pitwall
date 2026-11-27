# C4 diagram tile: plan and progress

This is where the C4 tile's progress is tracked (see the `sdd` skill in
`.claude/skills/sdd/`). The requirements are in [frd.md](frd.md) and the
architecture in [hla.md](hla.md). Tick an item in the pull request that
finishes it, once its definition of done holds on `main`, and put the
pull request number after it. When a pull request changes the plan, it
updates this file too.

SDK phases are the Tile SDK's ([README §10.2](../tile-sdk/README.md#102-order-of-work),
[SDK plan](../tile-sdk/plan.md)). Each phase below names the SDK pieces
it waits for.

**Status:** phases A and B built in `tiles/c4/` (the crate `c4-tile`, its
own Cargo workspace; the tile is its `tile` feature, `tile.wasm` is
committed). Phase C waits for a `files.read` service. The owner's
answers to [frd.md §9](frd.md#9-open-questions-for-the-owner) are
recorded there.

Low-level designs: [mermaid-lld.md](mermaid-lld.md) (blocks, the Mermaid
subset, errors, C4 reading) and [layout-lld.md](layout-lld.md) (text
widths, sizes, the `Layout` trait, dagre, the comparison with
rust-sugiyama).

## Phase A: the core, as plain Rust

Waits for: nothing. Parsing, linking and layout don't need the SDK, so
they can be written and tested natively now, inside `tiles/c4/`.

- [x] `tiles/c4/` exists as its own Cargo workspace, not a member of the root workspace. Done when `cargo test --manifest-path tiles/c4/Cargo.toml` passes and the root `cargo test --workspace` doesn't build it (#113)
- [x] `mermaid-lld.md` written in this folder: the grammar of the subset in [frd.md §6.1](frd.md#61-supported-in-v1), the model, and every error message. Done when merged and linked from here (#113)
- [x] `layout-lld.md` written in this folder: sizing, the dagre options, edge routing, the sequence layout, the size limits. Done when merged and linked from here (#113)
- [x] Block finder: Mermaid fences in Markdown with line numbers, heading path and diagram ids ([hla §3.2](hla.md#32-finding-blocks)). Done when tests cover fences with `~~~`, indented fences, repeats under one heading, and CRLF files (#113)
- [x] Mermaid parser for the flowchart subset. Done when every row of [frd.md §6.1](frd.md#61-supported-in-v1) for flowcharts has a passing test, and each error test checks line, column and message (#113)
- [x] Mermaid parser for the sequence subset. Done when every sequence row of [frd.md §6.1](frd.md#61-supported-in-v1) has a passing test (#113)
- [x] Unsupported types and partial support: the fallback model and the "Not shown" list (FR-18, FR-20). Done when a test feeds each Mermaid type named in [frd.md §6.2](frd.md#62-not-supported-in-v1) and gets the fallback (#113)
- [x] C4 reading of labels: `Name [Kind]`, purpose line, classes, levels (FR-11). Done when tests cover each separator in [frd.md §3.2](frd.md#32-c4-structure-diagrams-context-container-component) and each level rule (#113)
- [x] Linker: zoom links and before/after pairs ([hla §3.4](hla.md#34-levels-zoom-links-and-beforeafter-pairs)). Done when, with this folder's `hla.md` as input, the "C4 tile" box links to the container diagram and "tile.wasm" to the component diagram, and a fixture with Before/After headings pairs (#113)
- [x] Changes view model (FR-17): added, removed and changed boxes and arrows. Done when fixture tests pass (#113)
- [x] Layout with the `dagre` crate behind a `Layout` trait. Done when every fixture lays out natively with no overlapping boxes and every box inside its boundary (a test checks both) (#113)
- [x] `rust-sugiyama` fallback compared on the same fixtures. Done when a short note in `layout-lld.md` records quality and time for both, and which one v1 uses (#113; dagre, [layout-lld §7](layout-lld.md#7-dagre-against-rust-sugiyama))
- [x] Inter advance-width table generated at build time ([hla §3.7](hla.md#37-text-sizes)). Done when a test compares its widths with the font's for 100 sample labels, within 1 % (#113; the table gained kerning pairs to get there: 0.36 %)
- [x] Fixtures: this folder's `hla.md`, and the SDK's own docs (`docs/spec/tile-sdk/runtime.md`'s diagrams), plus hand-written cases. Done when they live in `tiles/c4/tests/fixtures/` and all of the above use them (#113; copies, dated: `c4-tile/hla.md`, `tile-sdk/diagrams.md`, the `shop/` project, `markdown/fences.md`, `unsupported.md`)

## Phase B: v1, first part: shown diagrams

The headline: an agent calls `c4__show` with Mermaid and the diagram
opens next to its terminal ([frd.md §5.1](frd.md#51-shown-diagrams-v1-first),
[hla §3.10](hla.md#310-shown-diagrams)). No file access at all, so this
ships before the files service.

Waits for: SDK phase 1 (WIT 0.1, `storage`, `settings`, `tasks`,
`pitwall extension new/dev/check`, the harness, folder install) and
phase 2 (canvas, `tree`, `segmented`, `chip`, `menu`, `code`, `split`,
drag inside a tile, and show-only agent tools: the host-provided open,
declared `present` tools delivered as view content, registration in
Pitwall's MCP server, placement next to the calling agent), plus the v1
gaps below. [hla §4](hla.md#4-v1-what-it-needs-from-the-sdk-and-when)
has the table.

Done when, in Claude Code and in Codex started by Pitwall, "draw me the
containers of this repo as a C4 diagram and show it on the right" opens
the drawing right of that agent's terminal with no setup, and "add a
cache" updates the same tile in place with Cache highlighted.

SDK requests (each is a change to the SDK's design and plan, made there):
- [x] G1 canvas pointer position raised in the SDK design ([hla §7](hla.md#7-sdk-gaps)). Done when the SDK's `ui.md` has the decision and the SDK plan has an item, or the owner decides against it (#123: hit regions with clicks, and pointer events on the canvas)
- [x] G2 text measurement raised. Done as for G1 (#PR: `ui.measure-text` in the SDK, [ui.md 3.11](../tile-sdk/ui.md#311-the-seed-tiles-gaps-as-built); the tile keeps its table, since its core lays out without the SDK, and `tests/text_widths.rs` checks it equals Pitwall's measure, [layout-lld §1](layout-lld.md#1-text-widths))
- [x] G3 canvas text in px, or a scale op, raised. Done as for G1 (#123: text in pixels, and a transform)
- [x] G4 stroke width and dashes raised. Done as for G1 (#123)
- [x] G5 wheel and pinch raised. Done as for G1 (#123)
- [x] Show-only agent tools in SDK phase 2, with `c4__show` as their first user. Done when the SDK's [agent-tools §11.2](../tile-sdk/agent-tools.md#112-four-kinds) and [plan](../tile-sdk/plan.md) say so (this pull request) (#125)

The tile:
- [x] Manifest: id `c4` (the owner's choice, [frd.md §9](frd.md#9-open-questions-for-the-owner)), the tile, its `"input"`, the `show` tool (kind `present`), no permissions. Done when `pitwall extension check` passes (#131)
- [x] Dependency check in CI ([hla §6](hla.md#6-where-it-lives)): `tiles/c4` built for `wasm32-wasip2`, tests, `pitwall extension check`, and a `cargo metadata` check that no Pitwall crate but `pitwall-extension` (and the harness, dev only) is used. Done when the job runs on every pull request and a test branch that adds `pitwall-core` fails it. (Phase A added the `tile-c4` job with all of this but `pitwall extension check`, which doesn't exist yet; `tiles/c4/scripts/check-deps.sh` was checked locally against a fake `pitwall-core` path dependency.) (#131: the job as Notes', with `pitwall extension check` and the harness tests; the check-deps script now Notes' too, failing on a `pitwall-core` path dependency, checked locally)
- [x] `c4__show`: all-or-nothing parse, items per agent and project, update in place, the reply (FR-34 to FR-38). Done when harness tests cover a new item, an update with the same item, a call with problems that changes nothing, a skipped part listed in the reply, and an unsupported type (#131)
- [x] Placement through the SDK: `where` and `of` reach Pitwall, not the tile; a later call reuses the view (FR-35). Done when a harness test calls with `where: "below"` and then without `where`, and the second call lands in the same view (#131: proven headless in the app through Pitwall's MCP server, `where: right` and then no `where`, the same view in the same place; `where` and `of` never reach the tile, so the harness has nothing to test)
- [x] "Changes since last show" (FR-40) and the host's "from <agent>" mark with close (FR-39). Done when harness tests update an item and switch to Changes, and close an item (#131: Now / Changes in the bar; the host's mark is Pitwall's, close is the list's ×)
- [x] Shown diagrams kept across restarts, with the limit per project (FR-41, FR-25). Done when a harness test drops and restores the tile and the shown diagrams come back, and a 51st item evicts the oldest unopened one (#131)
- [x] Painter and Navigator: boxes, shapes, boundaries, arrows, labels, pan, zoom, fit, theme tokens (FR-5 to FR-10). Done when harness snapshot tests of draw ops pass in Dark and Light (#131)
- [x] Click to zoom across shown diagrams, breadcrumb, selection and footer (FR-12 to FR-15). Done when harness tests click boxes (with G1) and the zoom path goes from a shown context diagram to a shown container diagram (#131: a box several diagrams detail asks which in a menu)
- [x] Sequence diagrams drawn. Done when the fixtures' sequence diagrams have snapshot tests (#131)
- [x] Highlights for `new`, `changed`, `external` and the legend (FR-16, FR-10). Done when harness snapshot tests of a classed fixture pass (#131)
- [x] Fallbacks shown for a shown diagram that is too large or partly understood (FR-20, FR-21). Done when harness tests cover both (#131)
- [x] Outline ("Shown" group, by agent) and keys (FR-23, FR-26). Done when harness tests drive the outline and canvas by keyboard only (#131: `+` `-` `F` `0` zoom and fit, arrows, Enter, Backspace, `B` for Changes; ⌘+/⌘−/⌘0 are Pitwall's own keys, so plain keys)
- [x] Speed measured in Wasmtime on an M-series Mac (NFR-2): parse, layout and first draw of 50, 150 and 300 boxes. Done when the numbers are recorded in `layout-lld.md` and the FR-21 limit is set from them (#131: 24, 76 and 164 ms on an M5 Max; the limit stays at 300, [layout-lld §6](layout-lld.md#6-limits-and-checks))
- [ ] Runs on macOS, Linux and Windows. Done when the CI job's tests pass on all three, and the owner has opened it on each
- [ ] Checked with real agents: Claude Code and Codex see `c4__show` (Claude Code as `mcp__pitwall__c4__show`) without user setup and call it without asking. Done when both pass the phase's "done when" above, and the result for each other agent kind with an MCP launch mode is noted here
- [ ] Shared as a preview folder with the SDK preview authors. Done when it installs with `pitwall extension install <folder>`
- [x] Website docs: the user part (ask an agent to show a diagram, where it opens, zoom, Changes since last show) on the Extensions pages, and the `c4__show` declaration as the example of a show tool on the Agent tools page, taken from this tile's source ([authoring §9.6](../tile-sdk/authoring.md#96-website-docs)). Done when published and the link check passes (#131: [C4 diagrams and Notes](../../reference/extensions/seed-tiles.md), and the Agent tools page's example)

## Phase C: v1, second part: diagrams in the docs

Browsing the diagrams agents wrote in the project's Markdown
([frd.md §5.3](frd.md#53-finding-diagrams-in-the-docs-v1-second)).

Waits for: phase B, and `files.read` with `list` and `watch` from the
on-demand services.

Done when the tile, pointed at this repository, lists every Mermaid
block under `docs/`, draws this folder's `hla.md` with click to zoom
from "C4 tile" to its container diagram, and redraws within a second of
an edit.

SDK requests:
- [ ] G6 recursive file listing raised. Done as for G1, in `runtime.md` §3.8
- [ ] G7 `files.watch` design raised, with this tile as its first user. Done as for G1
- [ ] The SDK's [platform services §14.3](../tile-sdk/platform-services.md#143-later-on-demand) names the C4 tile as a trigger for `files.read` and `watch`. Done when merged in the SDK's docs

The tile:
- [ ] Manifest adds `files.read` and `show` with reasons, the path-list setting, and `diagram` in the tile's `"input"` (FR-42). Done when `pitwall extension check` passes and the update review lists the new permission
- [ ] Scanner with project pick, file listing, limits (FR-1, FR-2). Done when harness tests with a fake project pass, including the limits
- [ ] Following changes with `files.watch`, coalescing, and the re-check when shown again (FR-4). Done when a harness test edits, renames and deletes files and the index follows
- [ ] Zoom links between shown and docs diagrams of the same project (FR-12, FR-13). Done when a harness test zooms from a shown context diagram into a container diagram found in a doc
- [ ] Before / After / Changes for pairs in the docs (FR-17). Done when harness tests switch views on the pair fixture
- [ ] Fallbacks and problems in files (FR-18, FR-19, FR-22). Done when harness tests cover an unsupported type, a parse error with lines counted in the Markdown file, and a break after a good drawing
- [ ] Open source and the docs outline (FR-23, FR-24). Done when a harness test opens a block's source at its line
- [ ] Index cache ([hla §3.6](hla.md#36-state-and-the-index-cache)). Done when a harness test drops and restores the tile and the index comes back without re-reading unchanged files
- [ ] Website docs: pick a project, diagrams in the docs, change views. Done when published

## Phase D: v1.1, Save to file

Waits for: phase C, and `files.write` from the on-demand services.

Done when "Save to file" on a shown diagram writes a `mermaid` block
into a chosen Markdown file of the agent's project, and the tile then
lists it among the docs diagrams.

- [ ] Save to file: file and heading picker, new file default, block written (FR-44, FR-45). Done when harness tests cover a new file, an existing file under a heading, and a refused `files.write`
- [ ] Manifest adds `files.write` (off by default). Done when the update review lists it
- [ ] Website docs: Save to file. Done when published

## Phase E: v2, agent tools for the docs and SVG export

Waits for: SDK phase 3 (`read` and `act` tools, approvals, Undo, the
skill through rulesync, commands) and `files.write`.

- [ ] `agent-tools-lld.md` written in this folder: each tool's input and output schema, errors and limits. Done when merged and linked from here
- [ ] `c4__list_diagrams`, `c4__get_diagram` (`read`) answered from the index and its cache ([hla §5](hla.md#5-v2-agent-tools-for-the-docs)). Done when harness tests call each, including on a dropped tile
- [ ] `c4__propose_diagram` (`present`): all-or-nothing parse, change view, origin mark, Undo. Done when harness tests cover a good proposal, one with errors (nothing changes) and Undo
- [ ] `c4__apply_diagram` (`act`) and the Apply button: block swap, hash check, refusal on a changed file. Done when harness tests cover apply, a stale hash and a new block under a heading
- [ ] SVG writer and `c4__export_svg` (`act`), also a palette and CLI command. Done when exported SVGs of the fixtures open in a browser and match the tile's layout
- [ ] Manifest adds the `read` and `act` tools. Done when `pitwall extension check` passes and the update review lists the new tools
- [ ] `agents/SKILL.md`: the convention ([frd.md §3](frd.md#3-the-input-format)) and when to call each tool, `c4__show` included, within the SDK's limits ([agent-tools §11.11](../tile-sdk/agent-tools.md#1111-a-skill-for-agents)). Done when `check` accepts it and an agent with it adds a container to a fixture through the tools
- [ ] Website docs: the agent part (asking an agent for a change in the docs, approvals, export). Done when published

## Phase F: own repository

Waits for: SDK phase 4 (packages) for the `.pwtile`, and phase 5
(marketplace) for the move.

- [ ] Released as a signed `.pwtile` from this repository. Done when it installs from the file with its signature checked
- [ ] Moved to its own repository with its history, the path dependency replaced by a crates.io version of `pitwall-extension`, CI moved with it, `tiles/c4/` removed here. Done when the new repository's CI passes and this file links to it
- [ ] Listed in the index with `pitwall extension publish`. Done when Browse shows it
