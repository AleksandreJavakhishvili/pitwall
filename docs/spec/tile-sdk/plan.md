# Tile SDK: plan and progress

This is where the Tile SDK's progress is tracked (see the `sdd` skill in
`.claude/skills/sdd/`). The design is the rest of this folder; the order
comes from [README §10.2](README.md#102-order-of-work). Tick an item in the
pull request that finishes it, once its definition of done holds on `main`,
and put the pull request number after it. When a pull request changes the
plan, it updates this file too.

Phases 0–2 are broken down in detail; later phases are listed coarsely and
are split up when the phase before them is nearly done.

**Status:** phase 0 in progress.

## Phase 0: Spike

Done when every [gate](README.md#phase-0-gates) passes, or the owner has
decided how to go on. Nothing ships to users; the code stays on a draft
pull request until then.

- [ ] `crates/pitwall-tile-host` runs a component in Wasmtime (LTS line, default features off), with a minimal tile world in WIT
- [ ] A hello tile in Rust (`wasm32-wasip2`) renders a box tree that Pitwall draws with GPUI, outside the real app's layout
- [ ] Results recorded in `spike-results.md`, with pass or miss for each gate:
  - [ ] Release-binary growth ≤ 10 MB on macOS (each architecture), Linux and Windows
  - [ ] 2 000-node frame (canonical ABI plus host diff) ≤ 4 ms on an M-series Mac
  - [ ] Cold start ≤ 300 ms, warm start ≤ 50 ms
  - [ ] Runaway tile stopped by epoch interruption ≤ 100 ms
  - [ ] Threading model confirmed: where the store lives, how calls leave the UI thread
- [ ] The unverified fallbacks checked: Winch compile speed, fuel as the stop for a runaway tile
- [ ] Owner's go, change or stop decision recorded in [README §10.3](README.md#103-decisions)

## Phase 1: Runtime and SDK preview

Done when an author can run `pitwall extension new`, `dev` and `install`
on macOS, Linux and Windows, and the Preview docs describe exactly that.

- [ ] WIT 0.1: box, text, button, icon, scroll, input, events, ui, log
- [ ] Platform services `tasks`, `storage`, `settings` ([14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12))
- [ ] `TileView` in the layout, on worker threads, with limits, compile cache and Log lines ([2](runtime.md))
- [ ] SDK crate `pitwall-extension` (guest side) and `pitwall-extension-test` harness core ([4](authoring.md))
- [ ] `pitwall extension new`, `dev`, `check` for components ([9.1](authoring.md#91-commands-pitwall-extension-))
- [ ] Unsigned install from a folder or URL: Settings → Extensions and `pitwall extension install`, permission screen, "Unsigned, preview" warning ([6.3](packaging.md#63-install))
- [ ] Host tests run on macOS, Linux and Windows in CI
- [ ] Authoring skill and generated `llms.txt` ([9.5](authoring.md#95-docs-for-authors-agents))
- [ ] Docs pipeline: pages from Markdown, examples from source, a link check ([9.6](authoring.md#96-website-docs))
- [ ] Website docs, marked Preview: Extensions sidebar group, overview, Quick start (`new`, `dev`, share a preview tile), Tiles and UI components (core elements), Permissions (first grants), Platform services (`tasks`, `storage`, `settings`), WIT reference 0.1, `llms.txt` and skill download
- [ ] First outside author has built a tile with the preview

## Phase 2: Components and minimum services

Done when the seed tiles Notes and CI can be built on the public SDK
alone, and the pages below match.

- [ ] Components: virtual list, table, tree, tabs, select, Markdown, canvas, images, menus, drag inside a tile ([3](ui.md))
- [ ] Native building blocks: code view and diff view ([16](native-blocks.md#16-native-building-blocks-and-native-capabilities))
- [ ] `sql` with quotas and migrations; `net`; notifications and toasts; open a file, the editor or a web tile ([14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12))
- [ ] Agents' questions group: `agents.read`, `badges`, `asks` in the floating control, `queue.add` with its origin mark
- [ ] Toolbar items, opt-in and hideable ([3.9](ui.md#39-toolbar-items))
- [ ] Permission screen changes for the new grants ([5](permissions.md))
- [ ] Tile plus managed service documented and wired ([13.2](services.md#132-a-a-tile-plus-a-managed-service))
- [ ] Website docs: all components, Native building blocks, Platform services (minimum and agents' questions), Permissions (`net`, `notify`, `agents.*`), Servers in extensions, toolbar items

## Later phases (to be broken down)

- [ ] **3. Agent tools:** `agent-tools` WIT, `#[tools]`, `pitwall extension tools`/`call`/`run`/`commands`, approvals, the agent skill, palette entries ([11](agent-tools.md))
- [ ] **4. Packages:** `.pwtile`, minisign, signed install, updates, Go back, remove ([6](packaging.md))
- [ ] **5. Marketplace:** index repo, Browse, Verified, report flow, `pitwall extension pack`/`sign`/`publish`; exit gate: threat model written and outside security review done ([7](marketplace.md), [5.5](permissions.md#55-threat-model-and-outside-review))
- [ ] **6. Seed tiles:** Notes, CI, Diagrams, then API client, git client, database browser ([10.1](README.md#101-seed-tiles-first-party-public-sdk-only))
- [ ] **6a. Native blocks and ports:** the review port as proof tile, then the files tree and inbox ([16.3](native-blocks.md#163-which-built-in-tiles-move-to-the-sdk))
- [ ] **On demand:** services from [14.3](platform-services.md#143-later-on-demand), each when a seed tile needs it
- [ ] **7. Stabilise:** WIT 1.0, Preview label removed, deprecation policy
- [ ] **8. Port or hand over the bundled extensions** ([15.7](extension-model.md#157-effect-on-the-plan))
- [ ] **9. Remove adapters** once each bundled extension has a component version
