Status: design, not built. Supersedes the "proposed" parts of
[extension-api.md](../extension-api.md) on signed packages, a curated list
and dev reload (see "What this replaces" at the end).

# Tile SDK and marketplace

Third-party tiles for Pitwall, written in Rust, compiled to a
WebAssembly component, run in a sandbox inside Pitwall, and drawn by
Pitwall's own GPUI code. Plus a way to find and install them: one
package file, a signed index on GitHub, and a Browse page in Settings.

This document builds on the extension system that exists today
([extensions.md](../extensions.md), [extension-api.md](../extension-api.md),
[docs/extensions/](../../extensions/README.md)). A tile is one more thing
an extension can contribute. It reuses the manifest, the permission
screen, grants, the activity log, sources, install, Go back and the
one-step update.

Terms ([15.2](extension-model.md#152-terms-used-in-this-document)): the **extension** is the package; a **tile** is a UI
contribution; the **component** is the extension's wasm code, with or
without tiles; a **service** is a long-running server; the **adapter**
is today's process runtime. [Section 15](extension-model.md#15-one-extension-model) decides that the component is
the one way an extension runs its own code.

Marks: **[today]** is built; **[new]** is proposed here.
Claims about third-party software carry a source number (S1…, listed
in [sources.md](sources.md)) or the word *unverified*.

## Contents

- [Summary](#summary-one-page), [goals](#1-goals-non-goals-user-stories) and the [plan](#10-plan-risks-and-decisions) (this file)
- [runtime.md](runtime.md): Architecture, the WIT interface and versioning: how a tile's component runs, its limits, and how it reaches the host.
- [ui.md](ui.md): The element tree a tile sends, how the host draws it with GPUI, events, keyboard, drag and drop, canvas, toolbar items.
- [native-blocks.md](native-blocks.md): No native plugin API; native building blocks; which built-in tiles move to the SDK.
- [permissions.md](permissions.md): Permissions and the security model.
- [platform-services.md](platform-services.md): What Pitwall gives every tile: the minimum, then on demand.
- [agent-tools.md](agent-tools.md): Tools a tile offers to agents, the agent skill and commands.
- [services.md](services.md): A tile plus a managed service, and headless wasm services.
- [packaging.md](packaging.md): The `.pwtile` file, signing, install, update and uninstall.
- [marketplace.md](marketplace.md): The index repository, submitting, Verified, Browse, reports and takedown.
- [authoring.md](authoring.md): The SDK crate, its examples, the CLI, dev reload, the test harness and docs for authors and their agents.
- [extension-model.md](extension-model.md): Components as the one way an extension runs its own code; adapters removed.
- [alternatives.md](alternatives.md): Alternatives considered.
- [sources.md](sources.md): Sources (S1…) cited across the Tile SDK spec, and the claims not verified.

## Decisions

Every open question was decided by the owner on 2026-11-26, each as
recommended. Unverified claims stay unverified: they are facts to
check, not decisions ([sources.md](sources.md#not-verified)).

- [10.3](#103-decisions): Runtime, packaging and marketplace
- [11.13](agent-tools.md#1113-decisions): Tools for agents
- [13.6](services.md#136-decisions): Tile apps with a server
- [15.8](extension-model.md#158-decisions): One extension model
- [16.4](native-blocks.md#164-decisions): Native building blocks and ports

## Summary (one page)

**What.** An author writes a tile in Rust against a crate,
`pitwall-tile`, whose UI code reads like GPUI
(`div().flex().gap_2().child(…)`). It compiles to one `tile.wasm`
(target `wasm32-wasip2`, a WebAssembly component). Pitwall runs it with
Wasmtime, with no ambient authority: no files, no network, no
programs, nothing but what the user granted. The tile returns a
description of its UI (an element tree of boxes, text and Pitwall's own
components). Pitwall draws that tree with GPUI, so the tile has
Pitwall's fonts, theme, Flat and Glass looks, focus ring, keyboard
model and IME. The tile's code never touches a window.

**Key decisions.**
1. **WebAssembly component, in process, one worker thread per tile.**
   Not a native library (crashes and full rights), not a separate
   window (can't look native), not a scripting language. The host crate
   is shaped so the runtime can move into a helper process later
   without changing the WIT.
2. **The guest describes, the host draws.** A retained element tree
   with stable keys, sent whole after each state change; the host
   diffs it against the last one to keep GPUI state (scroll, focus,
   caret, hover). Patches from the guest come later only if
   measurements ask for them.
3. **Colours and type come from Pitwall.** Styles name theme tokens
   (`text-2`, `surface`, `bad`), not colours, except inside a canvas.
   A tile follows Dark, Light, Flat, Glass and Reduce motion by itself.
4. **Rich components are Pitwall's.** Text input (with IME), lists,
   virtual lists, tables, trees, tabs, a read-only code view,
   Markdown, menus, tooltips, splits: drawn by the host's kit, driven
   by the tile. A canvas covers charts and custom drawing.
5. **No re-entrancy.** GPUI handlers never call the guest. They post an
   event to the tile's worker; the worker calls the guest and posts the
   new tree back; the app applies it on the main thread. Anything the
   guest asks of the UI (open a web tile, focus an agent) is applied
   with `cx.defer`.
6. **Host APIs start at a minimum** ([section 14](platform-services.md#14-platform-services-what-pitwall-gives-every-tile)): key-value storage,
   SQLite run by Pitwall, HTTPS to granted hosts, settings and secrets,
   notifications, opening a file, the editor or a web tile, timers. The
   rest (files, git, agents' status, Next up, clipboard, …) is designed
   here but built only when a seed tile needs it. Each is a named
   permission with the author's reason; Pitwall data comes through the
   existing method table.
7. **One file to install.** A `.pwtile` (zip): `extension.json`,
   `tile.wasm`, `assets/`, `README.md`, `screenshots/`, `LICENSE`. The
   index names its URL (usually a GitHub release asset), its SHA-256
   and the author's Ed25519 key; Pitwall checks both, shows the
   permission screen, unpacks into `<data>/extensions/<id>/`, compiles
   the wasm to native code once and caches it. Before packages
   (phase 1), an unsigned folder installs as a preview, for SDK-preview
   authors.
8. **Marketplace v1 is a GitHub repository.** A pull request lists a
   tile. CI checks the package. A maintainer's review earns
   "Verified". Pitwall reads a signed `index.json` from it; no server
   of ours. Takedown is a signed block list Pitwall checks daily.
9. **Updates show what changes** (new permissions first) and keep the
   previous version for Go back, through today's `safe_update`.
10. **Seed tiles on the public SDK only**: git client, database
    browser, API client, CI and pull requests, notes, diagrams.
11. **Tiles offer tools to agents** ([section 11](agent-tools.md#11-tools-for-agents)), through Pitwall's
    existing MCP server and CLI. Four kinds: `show` (open it),
    `present` (put content in it: a diagram, a table, a draft),
    `read` (get back what it shows, with the user's edits), `act`
    (effects outside the tile). `show` and `present` are allowed by
    default, `read` once per tile, `act` asks every time in the approval
    dialog. Built-in tiles follow the same model; the web tile stays
    show-only on purpose. A package may also bring an agent skill (a
    Markdown file, through the existing rules path, per project) and
    expose tools as commands (`pitwall tile run`, the palette); never
    native scripts.
12. **A tile app can have a server** ([section 13](services.md#13-tile-apps-with-a-server)). First, a tile plus
    the managed `service` that exists today (any language, runs as the
    user, disclosed as "runs a program on your computer"), with a
    loopback grant to it generated for the tile. Later, once tiles are
    stable, a headless `service.wasm` written with the same SDK: Pitwall
    runs an HTTP server and calls the component per request
    (`wasi:http/incoming-handler`), in a small host process of its own
    that keeps running when the window closes.
13. **Platform services: the minimum, then on demand** ([section 14](platform-services.md#14-platform-services-what-pitwall-gives-every-tile)).
    A service is added only when a first-party seed tile, built solely
    on the public SDK, or an author on the SDK preview, can't reasonably
    do without it. The WIT stays 0.x
    (breaking changes with notice) until the seed tiles ship, and 1.0
    freezes only then. Never terminal contents, credentials or other
    tiles' data; no hosted database, sync or accounts. The minimum
    includes what today's extensions already have for agents' questions
    (badges and row notes, questions in the floating control, Next up),
    so inbox-style tiles are a v0.x target.
14. **One extension model** ([section 15](extension-model.md#15-one-extension-model), decided). The extension is
    the one concept; a tile is one thing it contributes. The component
    becomes the only way an extension runs its own code, with or
    without a tile. The extensions bundled today are ported by Pitwall
    or handed to their authors, who build them on the public SDK and
    docs; adapters are removed once each has a published component
    version. Native code only through a managed service. Authors'
    coding agents get a skill and a one-file reference ([9.5](authoring.md#95-docs-for-authors-agents)).
15. **No native plugin API; native building blocks instead**
    ([section 16](native-blocks.md#16-native-building-blocks-and-native-capabilities)). Pitwall never loads an extension's dynamic library. What needs
    native speed or OS access is built by Pitwall and offered through
    the SDK: code view and diff view in the minimum; web view, device
    view and a terminal view on demand (terminal view not in v0.x).
    Review and source control, the files tree and the inbox tile are
    rebuilt on the SDK; review is the first-party proof tile.
    Terminals, the editor, web, devices and settings stay native.
16. **Website docs ship with each phase** ([9.6](authoring.md#96-website-docs)). A phase
    is done only when the public website docs match what shipped. The
    pages live in a new "Extensions" group of the site's docs, are
    written in Markdown like the reference pages, take their examples
    from the seed tiles' source and their WIT reference and `llms.txt`
    from the WIT, all built in CI.
17. **Toolbar items** ([3.9](ui.md#39-toolbar-items)), asked for by the owner (2026-11-26). An
    extension may opt in to one button in the top bar, drawn by Pitwall
    (none unless the manifest declares it; it may start hidden). It
    has an icon, a tooltip and an optional badge. A click focuses or
    opens its tile, runs a command, opens a menu of its commands, or
    opens a small popover view. Items sit in their own group, which the
    user can hide and reorder, and overflow into "…". No permission is
    needed beyond the install screen line. Part of the v0.x minimum
    (phase 2).

18. **Gates before going further** ([10.2](#phase-0-gates), [5.5](permissions.md#55-threat-model-and-outside-review)). Phase 0 has
    pass/fail gates (binary size, frame cost, start time, stopping a
    runaway tile) measured on macOS, Linux and Windows; a miss changes
    course, and several misses mean the owner decides whether to go
    on. Browse opens to the public only after a threat model and an
    outside security review of the host and permission code.
19. **Share early, unsigned** ([10.2](#102-order-of-work), [6.3](packaging.md#63-install)). From phase 1, SDK-preview
    authors can pass tiles around as a folder: install from a folder
    or URL, unsigned, marked preview, through the same permission
    screen. Phase 4 adds the `.pwtile`, signatures and updates on top.

**Phases**, in order; each needs the one before it unless it says
otherwise. Durations are left out on purpose: agents do the work, so
only the order and the dependencies mean something.

0. **Spike.** Nothing else starts until its gates pass, or until the
   owner decides how to go on after a miss ([phase 0 gates](#phase-0-gates)).
1. **Runtime**: WIT 0.1, core elements, dev mode and unsigned install
   from a folder (preview), on macOS, Linux and Windows. The authoring
   skill, the `llms.txt` reference and the website docs build come
   with it and are kept current after.
2. **Components and the minimum platform services**, with badges,
   floating-control questions and Next up; toolbar items; the code
   view and diff view blocks. Tile plus managed service is documented
   and wired here.
3. **Agent tools, skill and commands.**
4. **Package, signing, install, update**, on top of phase 1's folder
   install.
5. **Index and Browse.** Browse opens to the public only after the
   threat model and the outside security review
   ([5.5](permissions.md#55-threat-model-and-outside-review)).
6. **Seed tiles**, in parallel from phase 2 (Notes and CI need 2,
   Diagrams needs 3), with on-demand services as they trigger them.
   The review port as a proof tile also starts after phase 2,
   alongside them, pulling `git` and `files` into the plan; the files
   tree and inbox ports come after it.
7. **WIT 1.0**, once the seed tiles ship.
8. **Port or hand over the bundled extensions** (one extension model,
   decided), after 7.
9. **Remove adapters**, once each bundled extension has a component
   version.

Headless wasm services come after phase 7. Each phase also publishes
or updates its website docs pages ([9.6](authoring.md#96-website-docs))
before it counts as done.

**Owner decisions** (2026-11-26, every recommendation accepted) are
recorded in [10.3](#103-decisions), [11.13](agent-tools.md#1113-decisions), [13.6](services.md#136-decisions), [15.8](extension-model.md#158-decisions) and [16.4](native-blocks.md#164-decisions).


## 1. Goals, non-goals, user stories

### Goals
- A tile by someone else looks and behaves like Pitwall's own: same
  fonts, colours, density, glass, focus ring, keys (⌘⏎ maximise, Esc,
  ↑↓), same tile chrome (move, window of its own, close, restore).
- A crash, a hang or a hostile tile can't take Pitwall down, read what
  it wasn't given, or keep running after the user stops it.
- Installing is one click from Browse, one file, or one command, and
  never runs code before the user turns it on.
- Updates come from the author without a Pitwall release, and always
  show what they change. Go back is one click.
- Authors write ordinary Rust with a small, typed API, see changes on
  save, and test tiles headless in CI.
- Old tiles keep working across Pitwall releases for a stated time.

### Non-goals
- Native dynamic libraries loaded into Pitwall.
- Compatibility with VS Code extensions, or running their JavaScript.
- Arbitrary HTML or a script bridge (the web tile [today] stays the
  escape hatch, with no bridge).
- Changing how agents behave: no rules, skills, system prompts, hooks
  or modes from tiles ([extension-api.md](../extension-api.md), "Fixed
  constraints" [today]).
- Default keybindings outside the tile, status items in the chrome,
  reading terminal output or screens.
- A hosted store, payments, ratings or install counts (later, out of
  scope).
- A tile that is itself a server. A tile's package can bring one
  instead: a managed `service` [today] or, later, a headless wasm
  service ([section 13](services.md#13-tile-apps-with-a-server)).

### User stories
**User installs.**
1. In Settings → Extensions → Browse I type "postgres", see "Database
   browser" with a Verified badge, its screenshots, "Reads and writes
   files in projects you choose; connects to hosts you add", and
   Install. The permission screen lists each permission with the
   author's reason. I press Turn on, then "Open Database browser" in
   the palette, and a tile appears beside my agents.
2. A colleague sends me `ci-status-1.2.0.pwtile`. I drop it on
   Settings → Extensions (or run `pitwall extension install
   ci-status-1.2.0.pwtile`). It says "Not listed: signed by key
   `RWQ…k3`, which you haven't seen before" and the same permission
   screen.
3. An update adds `agents.queue`. The card says "Update to 1.3.0 —
   asks for a new permission: Send text to agents". I update; the new
   permission stays off until I press Allow. The tile misbehaves, so I
   press Go back to 1.2.0.
4. A tile hangs. Its tile says "CI status stopped: it didn't answer for
   1 s" with Restart and Turn off. The rest of Pitwall never paused.
5. I ask my agent "draw the request flow of the auth service". A
   Diagrams tile opens right of its terminal with a flowchart marked
   "from api". I drag two boxes and rename one. I say "add the cache
   between them"; the agent reads my edited diagram, sends an update to
   the same diagram, and I can Undo its change. When it asks to export
   the diagram as SVG into the repository, Pitwall asks me first.

**Author builds.**
6. `pitwall tile new ci-status` writes a Cargo project. `pitwall tile
   dev` builds it on every save and reloads it in my running Pitwall,
   keeping its state. `cargo test` runs it headless against a fake
   Pitwall. `pitwall tile pack` makes the `.pwtile`, `pitwall tile
   sign` signs it, I attach both to a GitHub release, and `pitwall tile
   publish` opens a pull request on the index.

**Maintainer reviews.**
7. The index's CI downloads the package, checks hash, signature,
   manifest, imports and size, rebuilds the wasm from the tagged source
   and compares hashes. For Verified, a maintainer reads the source at
   that commit, checks each permission's reason, merges, and the
   signed index is regenerated. A malware report leads to a block
   entry; within a day every Pitwall turns that tile off and says why.


## 10. Plan, risks and decisions

### 10.1 Seed tiles (first party, public SDK only)
Built in their own repositories (or `tiles/` in this repo, outside the
crates), released and listed like anyone's, so every gap they hit is a
gap in the SDK.

| Tile | Exercises | Needs |
|---|---|---|
| Notes | input, Markdown, storage, files | `files.read/write` (a `notes/` folder) |
| CI and pull requests | net, secrets, virtual list, agents, Next up, notifications | `net`, `agents.read`, `agents.queue`, `notify` |
| API client | inputs, tabs, code view, `net.ask`, storage | `net.ask` |
| Git client | tree, table, diff in code view, keyboard, drag | `git.read/write/remote`, `files.read` |
| Database browser | table with 100k rows (virtual), code view for SQL, canvas for a schema | `files.read` (SQLite in the guest); Postgres needs [10.3](#103-decisions), item 3 |
| Diagrams | canvas, drag, agent tools of all four kinds ([4.2](authoring.md#42-a-diagram-tile-as-an-author-writes-it)) | `files.write` for export only |
| Review and source control (proof tile, a port of the built-in one; [16.3](native-blocks.md#163-which-built-in-tiles-move-to-the-sdk)) | code view, diff view, lists, comments, Next up, git operations | `agents.read`, `worktrees.read`, `git.read/write`, `files.read`, `agents.queue` |

### 10.2 Order of work
A phase is done only when the public website docs match what it
shipped: the pages in its "Website docs" cell are published or
updated in the same pull requests as the features ([9.6](authoring.md#96-website-docs)).
Phase 1 also builds the docs pipeline (pages from Markdown, examples
from source, a link check).
Steps 8 and 9 ([15.7](extension-model.md#157-effect-on-the-plan))
publish Migrating from adapters and replace the adapter pages.

The table gives the order; it has no durations, since agents do the
work. What must come before what is in the summary's phase list
([above](#summary-one-page)) and in each row.

| Phase | Content | Website docs ([9.6](authoring.md#96-website-docs)) |
|---|---|---|
| 0. Spike | Wasmtime in `pitwall-tile-host`, measure release-binary growth per platform and cold/warm compile times, a hello tile rendering a box tree, canonical-ABI cost of a 2 000-node frame, epoch kill, confirm the threading model; on macOS, Linux and Windows (CI runners are fine for Linux and Windows). Done when the [gates](#phase-0-gates) pass, or the owner decides how to go on | none: nothing public yet |
| 1. Runtime | WIT 0.1 (box, text, button, icon, scroll, input, events, ui, log, and the first platform services: `tasks`, `storage`, `settings`), `TileView` in the layout, worker threads, limits, cache, Log, `tile new/dev/check`, the harness's core; the authoring skill and the generated `llms.txt` reference ([9.5](authoring.md#95-docs-for-authors-agents)), first shared with authors as the SDK preview; unsigned install from a folder for that preview ([6.3](packaging.md#63-install)): Settings → Extensions and `pitwall tile install <dir or url>`, marked developer preview, with the permission screen and an "Unsigned, preview" warning, no updates. Tiles target macOS, Linux and Windows from here, and the host's tests run on all three in CI | new "Extensions" group in the docs sidebar; Extensions overview, Quick start (`new`, `dev`, share a preview tile), Tiles and UI components (core elements), Permissions (first grants), Platform services (`tasks`, `storage`, `settings`), WIT reference 0.1, `llms.txt` and the skill download; all marked Preview |
| 2. Components and minimum services | virtual list, table, tree, tabs, select, code, Markdown, canvas, images, menus, drag inside a tile; the rest of the minimum ([14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12)): `sql` with quotas and migrations, `net`, notifications and toasts, opening a file, the editor or a web tile; and the agents' questions group ([14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12)): `agents.read`, `badges`, `asks` into the floating control, `queue.add` with its origin mark (mostly wiring existing host code); permission screen changes; toolbar items ([3.9](ui.md#39-toolbar-items)) | Tiles and UI components (all components), Native building blocks (code view, diff view), Platform services (the minimum and the agents' questions group), Permissions (`net`, `notify`, `agents.*`), Servers in extensions (tile plus managed service); toolbar items on Tiles and UI components |
| 3. Agent tools | `agent-tools` WIT, the `#[tools]` macro, tile tools in `mcp.tools`/`mcp.call` and `pitwall tile tools` and `call`, agent access on the permission screen, approvals for `act`, origin marks and Undo, built-in tiles described in the same kinds; the agent skill through rulesync ([11.11](agent-tools.md#1111-a-skill-for-agents)); commands, `pitwall tile run` and `commands` and palette entries ([11.12](agent-tools.md#1112-commands-for-the-shell-and-the-palette)) | Agent tools, skill and commands; Permissions (agents' access to a tile's tools); the CLI reference gains `pitwall tile tools`, `call`, `run` and `commands` |
| 4. Packages | On top of phase 1's folder install: `.pwtile`, minisign verification, install from file/URL/CLI with a signature (unsigned stays possible behind the same warning, [6.3](packaging.md#63-install)), `Release` source updates, `safe_update` step for tiles, Go back, remove | Packaging and signing; Quick start gains `pack`; Permissions' user part (install, grants, Go back); the Settings reference's Extensions section; a short Extensions part in How it works for users |
| 5. Marketplace | index repo and its CI, signed `index.json` and `blocked.json`, Browse tab, Verified, report flow, `tile pack/sign/publish`. Exit condition: the threat model is written and an outside security review of the host and permission code is done, with its serious findings fixed, before Browse opens to the public ([5.5](permissions.md#55-threat-model-and-outside-review)); until then Browse stays limited to preview authors | Publishing and the Verified badge (with its user part: what Verified means, reporting); Quick start gains `publish`; the Settings reference gains Browse |
| 6. Seed tiles | Notes and CI from phase 2, Diagrams from phase 3; API client, git client, database browser after; in parallel with the other phases from phase 2 | examples on every page switch to the seed tiles' source as each one ships |
| 6a. Native blocks and ports ([16](native-blocks.md#16-native-building-blocks-and-native-capabilities)) | code view and diff view blocks in phase 2; the review port as proof tile from phase 2 (read-only first, then comments, then commit and merge), which brings `git.read/write` and `files.read` into the plan; then the files tree (with file watching and `files.search`) and the inbox tile; native tiles dropped one release after each port matches | Native building blocks (each block as it ships); How it works says which built-in tiles are now extensions, if users can see it |
| On demand | the services of [14.3](platform-services.md#143-later-on-demand), each when a seed tile triggers it (files for Notes, git for the git client, agents' status and Next up for CI, …), alongside phase 6 | Platform services and Permissions gain a row for each service as it ships |
| 7. Stabilise | WIT 1.0 once the seed tiles ship, docs site, deprecation policy in force | Preview label removed; WIT reference 1.0 and one page per published WIT version; the deprecation policy on the overview |

#### Phase 0 gates
Phase 0 measures, and each measure has a line that says go on or
change course. The numbers are proposed and the owner may adjust
them (decided, [10.3](#103-decisions) item 15). Every gate is measured
on macOS, Linux and Windows: release builds, default Wasmtime features
off. The time gates are judged on an M-series Mac; Linux and Windows
run on the CI runners Pitwall already uses (`.github/workflows/ci.yml`),
their numbers are recorded, and one far off the Mac's is looked into
before phase 1. The size gate holds on every platform.

| Gate | Pass | On a miss |
|---|---|---|
| Release-binary growth | ≤ 10 MB per platform with Wasmtime's default features off ([10.3](#103-decisions) item 4); on macOS, for each architecture in the universal build | Turn off more features and measure again; then the Pulley interpreter (no Cranelift in the binary, slower tiles) or Winch, as decision 4 lists (S17). Not a different runtime |
| Frame cost | A 2 000-node frame lifted through the canonical ABI plus the host's decode, diff and apply in ≤ 4 ms ([2.7](runtime.md#27-performance-budget)) | Guest-side patches move from WIT 0.2 into WIT 0.1 ([10.3](#103-decisions) item 7, [8.4](runtime.md#84-planned-evolutions-each-as-a-new-minor-or-major-world)); the tree limits ([2.5](runtime.md#25-sandbox-and-limits-summary-details-in-5)) may shrink |
| Cold start | A hello tile compiled and instantiated with an empty cache in ≤ 300 ms ([2.6](runtime.md#26-compile-and-cache)) | Compile stays off the main thread at turn on and after updates, and the tile shows that it is starting; Winch, which is meant to compile faster than Cranelift (*unverified*), is measured as the compiler |
| Warm start | The same tile loaded from the compile cache and instantiated in ≤ 50 ms | Loading is a memory map (S5), so a miss is looked into as a bug before anything changes in the design |
| Stopping a runaway tile | A tile in an endless loop is stopped by epoch interruption within 100 ms of its deadline or of the user's Stop ([2.5](runtime.md#25-sandbox-and-limits-summary-details-in-5)) | A shorter epoch tick; then fuel for the check that misses (S4, slower); if neither, the helper process ([10.3](#103-decisions) item 1) moves forward, so a stuck tile can be ended as a process |

A miss on one gate takes its fallback and is measured again. A miss on
several, or a fallback that also misses, means the owner decides
whether to go on, change course or stop before phase 1 starts.

### 10.3 Decisions
Each was an open question with a recommendation; the owner accepted
every recommendation on 2026-11-26.

1. **In process or a helper process?** In process is simpler and
   faster; a Wasmtime escape or crash then hits the app.
   Decided (owner, 2026-11-26): in process for v1, behind a message interface so
   moving the worker loop into a `pitwall-tiled` helper later changes
   no WIT and no SDK.
2. **This reverses "no third-party code in Pitwall's process"**
   (extension-api.md, Principles [today]). Decided (owner, 2026-11-26): the principle
   now reads "no third-party code with access to Pitwall's process":
   wasm code has none of the process's rights, unlike an adapter that
   has all of the user's. extension-api.md is updated to match.
3. **TCP for databases.** WASI sockets exist (S7) but raw TCP widens
   what a tile can reach. Decided (owner, 2026-11-26): not in v1; the database browser
   ships SQLite first; then `net.tcp` with `host:port` pairs the user
   enters, through the same allow-list code.
4. **Binary size.** Wasmtime's only published numbers are for its C
   library on Linux x86-64: 19 MB with default features, 2.1 MB with
   none, 1.2 MB with LTO and `panic=abort` (S17). Pitwall needs
   Cranelift, the component model and part of WASI, so the real cost
   is somewhere between, and no size work has been done on Cranelift
   yet (S17). Decided (owner, 2026-11-26): measure in phase 0 with default features
   off, and accept up to ~10 MB per platform (a gate, [Phase 0 gates](#phase-0-gates)); Winch (still in
   development) or the Pulley interpreter (S17) are fallbacks, not a
   different runtime.
5. **Wasmtime releases.** A new major version ships every month; most
   are supported for 2 months, every 12th (36, 48, …) for 24 months,
   and security fixes are backported to all supported releases (S1).
   Decided (owner, 2026-11-26): follow the LTS line (48 now, 60 next), check
   advisories in the release checklist, and ship a Pitwall patch
   within a week of a sandbox-escape fix.
6. **Signatures: minisign now, Sigstore later?** Decided (owner, 2026-11-26): minisign
   (small, offline, keys in the index); add Sigstore attestations as a
   second signal for reproducible Verified builds once the index is
   busy.
7. **Full tree vs. patches.** Decided (owner, 2026-11-26): full tree + host diff in
   WIT 0.1; decide on patches from phase 0–2 measurements.
8. **Colours outside canvases.** Authors will ask for brand colours.
   Decided (owner, 2026-11-26): no in 0.x (tokens keep tiles native in both themes);
   revisit with a small "accent" token per tile if real tiles need it.
9. **Background work while hidden.** Decided (owner, 2026-11-26): none (suspend, drop
   after 10 min); a tile that needs to watch something all day pairs
   with an adapter or service, which already exist.
10. **Verified review load.** Decided (owner, 2026-11-26): Verified only for tiles that
    ask for a High permission or for the first version of an author;
    automated checks for the rest; the badge's tooltip says so.
11. **Index name and owner.** Decided (owner, 2026-11-26): a separate public repo under
    the same owner as Pitwall. CI signs `index.json` with an index key
    kept as a repository secret; Pitwall ships that key and the next
    one ([6.2](packaging.md#62-signing)), and the next key is held offline, so a leaked CI key is
    replaced by a Pitwall release that drops it. Minisign has no
    subkeys or certificate chains (S9), which is why two keys ship.
12. **Zed themes and grammars.** What is known: Zed's extensions are
    Rust built for `wasm32-wasip2` against versioned WIT folders, and
    can add languages (tree-sitter grammars and language servers),
    debug adapters, themes, icon themes, snippets and MCP servers, but
    no custom UI (S18). Zed themes are JSON files with a published
    schema (`https://zed.dev/schema/themes/v0.2.0.json`); grammars are
    compiled to wasm with wasi-sdk and loaded through the `tree-sitter`
    crate's `wasm` feature, which runs them on Wasmtime (S18). Zed's
    index requires each extension to carry a licence from a fixed list
    (MIT, Apache-2.0, BSD, GPLv3, LGPLv3, CC-BY-4.0, zlib, Unlicense)
    (S18). Pitwall's theme is a fixed token set and its editor links
    grammars natively (`tree-sitter` 0.25 [today]). Decided (owner, 2026-11-26): not in
    this plan. Later, as separate designs: a "theme import" mapping a
    Zed theme's colours onto Pitwall's tokens (lossy: Zed has many more
    keys; whether the mapping looks right is *unverified*), and wasm
    grammars through tree-sitter's `wasm` feature, which would share the
    Wasmtime engine this plan adds. Each theme's or grammar's own
    licence must allow redistribution, and GPL-licensed ones need a
    licence review before Pitwall bundles any (installing them by the
    user's choice is simpler).
13. **Agent tools.** Their own decisions are in [11.13](agent-tools.md#1113-decisions).
14. **Caller kind for tiles' `METHODS` calls** ([10.4](#104-what-gets-in-the-way-in-todays-code)): a new
    `tile(<id>)` or reuse `extension(<id>)`? Decided (owner, 2026-11-26): reuse
    `extension(<id>)`: one identity per extension.
15. **Pass/fail gates for phase 0.** The spike measured things, but no
    result said stop. Decided (owner, 2026-11-26): phase 0 has the
    gates in [Phase 0 gates](#phase-0-gates): binary growth ≤ 10 MB per
    platform, a 2 000-node frame in ≤ 4 ms, a hello tile's cold start
    ≤ 300 ms and warm start ≤ 50 ms, a runaway tile stopped within
    100 ms. Each miss takes the fallback this design already lists; a
    miss on several means the owner decides whether to continue. The
    numbers are proposed, and the owner may adjust them.
16. **Sharing before packages.** Installing from a file used to arrive
    only in phase 4, so SDK-preview authors had no way to pass a tile
    to anyone. Decided (owner, 2026-11-26): phase 1 adds an unsigned
    install from a folder, in Settings → Extensions and as `pitwall
    tile install <dir or url>` ([6.3](packaging.md#63-install)). It is marked developer
    preview, shows the permission screen with an "Unsigned, preview"
    warning, and has no updates. Phase 4 adds the `.pwtile`, signatures
    and updates on top of the same path.
17. **Security review before the marketplace.** Decided (owner,
    2026-11-26): a threat model is written ([5.5](permissions.md#55-threat-model-and-outside-review)) and an outside
    security review of the host and permission code is done, with its
    serious findings fixed, as an exit condition of phase 5. Browse
    doesn't open to the public before that.
18. **Platforms.** Pitwall runs on macOS, Linux and Windows (preview).
    Decided (owner, 2026-11-26): phase 0 measures on all three (CI
    runners are fine for Linux and Windows), and tiles target all three
    from phase 1; one `tile.wasm` runs everywhere ([8.1](runtime.md#81-three-versions)), and the
    host's tests run on all three in CI. What this design already knows
    about Windows: Wasmtime's most recent sandbox escape was in WASI's
    filesystem code on Windows (S1, [5.3](permissions.md#53-what-a-hostile-tile-can-still-do)); tiles get no folders through
    WASI ([5.2](permissions.md#52-enforcement)), but Pitwall's own path checks for `files.*` must be
    tested on Windows too. The compile cache's "mode 600 in a 700
    folder" ([2.6](runtime.md#26-compile-and-cache)) has no direct Windows meaning and needs a
    per-user equivalent there; and Wasmtime's per-OS trap handling is
    *unverified in detail* ([2.2](runtime.md#22-crates-new)), so phase 0 checks it on each.

### 10.4 What gets in the way in today's code
- `contributes` keys are checked against a fixed list; `tiles` must be
  added in `pitwall-core/src/extensions/mod.rs` with its limits.
- Contributions "each need an adapter" (views.md [today]); tiles
  don't, so the manifest check changes.
- `Content` and `Surface` (`main_screen/tree.rs`) get a `Tile` case
  beside `View`; layouts store `tile` keys (`<extension>/<tile>`) so
  older Pitwalls show an empty pane, as for views [today].
- `install::Origin` and `Source` gain package and index variants;
  `install::digest` must skip nothing new (the cache lives elsewhere).
- The permission screen and `permissions.rs` must accept object
  values (hosts, memory) and per-project grants.
- `pitwall_proto::mcp` builds a fixed tool list (`tools_for(files,
  web, settings, device)`); tile tools make it dynamic, merged by the
  daemon's `mcp.tools` with the tiles that are on. A new daemon method
  `tile.call` (Access: agents only, checked per kind) and the approval
  dialog's requester gain the tile and tool.
- `METHODS` calls from tiles use the caller kind `extension(<id>)`
  (decided, [10.3](#103-decisions) item 14: one identity per extension).


## What this replaces in extension-api.md

- "Distribution and updates" → phase 6 feed and `ed25519` signatures:
  replaced by [6.2](packaging.md#62-signing)–[6.4](packaging.md#64-updates-and-go-back) here (packages signed with minisign, keys in the
  index or trusted on first install).
- "A curated list, later" and decision 11 (registry): replaced by [7](marketplace.md#7-marketplace).
- Phase 6 "Feeds, signatures, sandbox, list": signatures and the list
  are here; the opt-in OS sandbox for adapters stays as written there.
- "Principles: A separate process, not code in the app": still true for
  services; adapters are removed ([section 15](extension-model.md#15-one-extension-model)) and components are the
  exception decided in [10.3](#103-decisions) (2): the principle now
  reads "no third-party code with access to Pitwall's process".
- Author experience "Dev mode": for tiles, reload on save ([9.2](authoring.md#92-dev-reload));
  adapters' manual Reload is unchanged.
- "Later: agents as callers" (extension commands as MCP tools): for
  tiles, [section 11](agent-tools.md#11-tools-for-agents) is that design. Adapters' commands could later be
  listed the same way, as `act` tools.
