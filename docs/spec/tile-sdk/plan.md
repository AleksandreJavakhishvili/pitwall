# Tile SDK: plan and progress

This is where the Tile SDK's progress is tracked (see the `sdd` skill in
`.claude/skills/sdd/`). The design is the rest of this folder; the order
comes from [README §10.2](README.md#102-order-of-work). Tick an item in the
pull request that finishes it, once its definition of done holds on `main`,
and put the pull request number after it. When a pull request changes the
plan, it updates this file too.

Phases 0–2 are broken down in detail; later phases are listed coarsely and
are split up when the phase before them is nearly done.

**Status:** phase 0 done (go with changes, [10.3](README.md#103-decisions) item 19); phase 1 in progress.

## Phase 0: Spike

Done when every [gate](README.md#phase-0-gates) passes, or the owner has
decided how to go on. Nothing ships to users; the code stays on a draft
pull request until then.

- [x] `crates/pitwall-tile-host` runs a component in Wasmtime (LTS line, default features off), with a minimal tile world in WIT (#107: Wasmtime 48.0.10)
- [x] A hello tile in Rust (`wasm32-wasip2`) renders a box tree that Pitwall draws with GPUI, outside the real app's layout (#107)
- [x] Results recorded in `spike-results.md`, with pass or miss for each gate (#107):
  - [x] Release-binary growth ≤ 10 MB on macOS (each architecture), Linux and Windows (#107; miss as first built: +13.1 MB Linux, +11.7 MB Windows, +11.8 MB macOS x86-64, +8.2 MB arm64; after the gate's fallback, no Wasmtime cache and opt-level "s" on its crates: pass on all four: +5.8 MB macOS arm64, +7.3 MB macOS x86-64, +8.4 MB Linux, +7.3 MB Windows)
  - [x] 2 000-node frame (canonical ABI plus host diff) ≤ 4 ms on an M-series Mac (#107; pass: 0.28 ms median, 0.30 ms p95)
  - [x] Cold start ≤ 300 ms, warm start ≤ 50 ms (#107; pass: hello 7.5 ms / 0.16 ms, a 1.0 MB tile 56–108 ms / 1.3–1.9 ms)
  - [x] Runaway tile stopped by epoch interruption ≤ 100 ms (#107; pass with a wall-clock check on each tick: ≈ 7 ms, ≤ 13 ms; miss as runtime.md specifies, by tick count: 200–430 ms late at 1 s)
  - [x] Threading model confirmed: where the store lives, how calls leave the UI thread (#107; store on the `tile:<id>` worker, UI thread only sends and receives)
- [x] The unverified fallbacks checked: Winch compile speed, fuel as the stop for a runaway tile (#107; Winch compiles 3–5× faster and runs 1.9× slower; fuel costs 25–30 % a frame and isn't time)
- [x] Owner's go, change or stop decision recorded in [README §10.3](README.md#103-decisions) (#111: go with changes, item 19; size gate per architecture)

## Phase 1: Runtime and SDK preview

Done when an author can run `pitwall extension new`, `dev` and `install`
on macOS, Linux and Windows, and the Preview docs describe exactly that.

- [x] WIT 0.1: box, text, button, icon, scroll, input, events, ui, log (#111: `crates/pitwall-tile-host/wit/`, documented; host-owned `input` with selection, pointer positions, views with content for agents)
- [x] Platform services `tasks`, `storage`, `settings` ([14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12)) (#111: host side in `pitwall-tile-host`; storage and settings under `<data>/extension-data/<id>/`; the Settings form is the app's)
- [x] Host runtime: worker thread per tile, limits with wall-clock deadlines, stop, compile cache, Log lines, frames with diff, grants ([2](runtime.md)) (#111: `pitwall-tile-host`, API in its README)
- [ ] `TileView` in the layout, drawing the host's frames and routing events ([2](runtime.md))
- [x] SDK crate `pitwall-extension` (guest side) and `pitwall-extension-test` harness core ([4](authoring.md)) (#116: `sdk/pitwall-extension`, outside the workspace, with a copy of the WIT kept equal by a test; `crates/pitwall-extension-test`; the hello fixture is written with the SDK)
- [x] `pitwall extension new`, `dev`, `check` for components ([9.1](authoring.md#91-commands-pitwall-extension-)) (#116: `new --template blank|list|form`, `dev` asks the app with `extension.devReload {path}` (the app's side is `TileView`'s), `check` reads the component, `test` runs `cargo test`; templates built and tested in CI's Tile SDK job)
- [ ] Unsigned install from a folder or URL: Settings → Extensions and `pitwall extension install`, permission screen, "Unsigned, preview" warning ([6.3](packaging.md#63-install))
- [x] Host tests run on macOS, Linux and Windows in CI (#111)
- [x] Lean Wasmtime in the release build (#111): no `cache` feature, Wasmtime's and Cranelift's crates at `opt-level = "s"` ([10.3](README.md#103-decisions) item 19)
- [ ] A 2–3 MB tile compiles in under 1 s on a 4-core CI runner ([10.3](README.md#103-decisions) item 19)
- [x] Authoring skill and generated `llms.txt` ([9.5](authoring.md#95-docs-for-authors-agents)) (skill #116: `sdk/pitwall-extension/skills/`, written by `new` with AGENTS.md; `llms.txt` #109)
- [x] Docs pipeline: pages from Markdown, examples from source, a link check ([9.6](authoring.md#96-website-docs)) (#109)
- [ ] Website docs, marked Preview: Extensions sidebar group, overview, Quick start (`new`, `dev`, share a preview tile), Tiles and UI components (core elements), Permissions (first grants), Platform services (`tasks`, `storage`, `settings`), WIT reference 0.1, `llms.txt` and skill download
- [ ] First outside author has built a tile with the preview

## Phase 2: Components and minimum services

Done when the seed tiles Notes and CI, and the C4 tile's shown
diagrams, can be built on the public SDK alone, agents can open any
tile and show content in it, and the pages below match.

- [ ] Components: virtual list, table, tree, tabs, select, Markdown, canvas, images, menus, drag inside a tile ([3](ui.md))
- [ ] Agents open tiles: the host-provided `<extension-id>__open` for every extension with tiles, taking the tile's declared `"input"` as content, placed next to the calling agent with `where` and `of` as `show_web` does, a later call reusing the same view ([11.3](agent-tools.md#113-declaring-tools), [11.8](agent-tools.md#118-where-the-tile-opens)). Done when an agent in Claude Code opens the hello tile on the right with a payload, and a second call with the same item updates it in place
- [ ] Show-only tools: `show` and `present` declared in the manifest and with `#[tools]`, delivered as `view-opened` and `view-content` with the new `content` fields, `reply`, item ids per agent, the host's origin mark, the refusal on an item the user edited, the limits for inline input ([11.2](agent-tools.md#112-four-kinds), [11.6](agent-tools.md#116-presented-content-ids-marks-undo), [11.7](agent-tools.md#117-size-time-streaming-errors)). Done when harness tests cover a good call, a call with problems that changes nothing, an update in place and the refusal
- [ ] Registered with every agent: tile show-only tools in `mcp.tools`/`mcp.call` and `notifications/tools/list_changed`, named `<extension-id>__<tool>`, with `readOnlyHint` and added to the launch-time allow lists, plus one line per tile in the server's instructions ([11.4](agent-tools.md#114-how-agents-reach-them)). Done when an extension's declared show tool appears in Claude Code's and Codex's tool list without user setup, and each agent kind with an MCP launch mode is checked for the names
- [ ] Agent access, show-only part: the per-tile "Agents can use <name>" switch, Settings → Agents "Tile tools", the tools on the permission screen, calls in the tile's Activity ([11.5](agent-tools.md#115-permissions), [11.10](agent-tools.md#1110-records)). Done when turning either switch off removes the tools from a running agent's list
- [ ] Native building blocks: code view and diff view ([16](native-blocks.md#16-native-building-blocks-and-native-capabilities))
- [ ] `sql` with quotas and migrations; `net`; notifications and toasts; open a file, the editor or a web tile ([14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12))
- [ ] Agents' questions group: `agents.read`, `badges`, `asks` in the floating control, `queue.add` with its origin mark
- [ ] Toolbar items, opt-in and hideable ([3.9](ui.md#39-toolbar-items))
- [ ] Permission screen changes for the new grants ([5](permissions.md))
- [ ] Tile plus managed service documented and wired ([13.2](services.md#132-a-a-tile-plus-a-managed-service))
- [ ] Website docs: all components, Native building blocks, Platform services (minimum and agents' questions), Permissions (`net`, `notify`, `agents.*`, agents opening a tile), Servers in extensions, toolbar items, Agent tools (show-only part); the Extensions overview's first lines say that extensions bring their own agent tools and Pitwall registers them with every agent; the authoring skill and `llms.txt` cover declaring show tools

## Later phases (to be broken down)

- [ ] **3. Agent tools, the rest:** `read` and `act` through the `tool` export, approvals, the ask-once grant, Undo and `undo-present`, `from_file` and `append`, `pitwall extension tools`/`call`/`run`/`commands`, the agent skill, palette entries ([11](agent-tools.md))
- [ ] **4. Packages:** `.pwtile`, minisign, signed install, updates, Go back, remove ([6](packaging.md))
- [ ] **5. Marketplace:** index repo, Browse, Verified, report flow, `pitwall extension pack`/`sign`/`publish`; exit gate: threat model written and outside security review done ([7](marketplace.md), [5.5](permissions.md#55-threat-model-and-outside-review))
- [ ] **6. Seed tiles:** C4 diagrams (flagship, [spec](../2026-11-27-c4-tile/plan.md)) and Notes ([spec](../2026-11-27-notes-tile/)) first, then CI, API client, git client, database browser ([10.1](README.md#101-seed-tiles-first-party-public-sdk-only))
- [ ] **6a. Native blocks and ports:** the review port as proof tile, then the files tree and inbox ([16.3](native-blocks.md#163-which-built-in-tiles-move-to-the-sdk))
- [ ] **On demand:** services from [14.3](platform-services.md#143-later-on-demand), each when a seed tile needs it
- [ ] **7. Stabilise:** WIT 1.0, Preview label removed, deprecation policy
- [ ] **8. Port or hand over the bundled extensions** ([15.7](extension-model.md#157-effect-on-the-plan))
- [ ] **9. Remove adapters** once each bundled extension has a component version
