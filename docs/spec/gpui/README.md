# GPUI port: Pitwall's UI on GPUI

Pitwall's UI moves from Tauri 2 + React (a web view) to
[GPUI](https://www.gpui.rs), Zed's GPU-accelerated Rust UI framework, for
speed, memory use and a native feel on macOS, Windows and Linux.

**Switched in v0.2.0**: the GPUI app (`crates/pitwall-app`, binary
`pitwall`) is the released Pitwall. The Tauri shell (`src-tauri/`) is gone;
the React UI (`src/`) stays only as the website's live demo
([src/README.md](../../../src/README.md)). The [inventory](inventory.md) is
the parity checklist the port was built against; the sections below keep
the port's design notes.

Files in this folder:

| File | Contents |
|---|---|
| README.md (this) | Goals, architecture, engine hosting, state, theming, windows, phases, risks |
| [inventory.md](inventory.md) | Feature-parity checklist of the current app, with file references |
| [in-house.md](in-house.md) | Components Pitwall builds itself (Zed's other crates are GPL-3.0) |
| [platform.md](platform.md) | Per-OS plan: menus, badge, tray, notifications, single instance, glass, … |
| [packaging.md](packaging.md) | Bundles without Tauri, sidecars, signing, CI, tests |
| [licensing.md](licensing.md) | GPUI version choice and the license of every new dependency |

## Goals

- **Same product.** Every item in [inventory.md](inventory.md) works in the
  GPUI app before the switch. Nothing is dropped for licensing reasons:
  what Zed's GPL crates would provide is built in-house ([in-house.md](in-house.md)).
- **Faster and lighter.** No web view process, no IPC per terminal chunk: the
  engine and the UI share one process and one memory space. Budgets in
  [../perf.md](../perf.md) carry over and get stricter (the WebKit GPU
  process, ~220 MB with visible terminals, goes away).
- **Native feel.** Real menus, keyboard handling and window behaviour per OS.
- **Same engine.** `pitwall-core`, the providers, the CLI socket server, the
  holders and hooks are untouched; the app hosts them exactly as `src-tauri`
  does.
- **Safe coexistence** while both apps exist: the GPUI app never disturbs a
  running Tauri Pitwall or its agents.

## Non-goals

- No change to the engine, the providers, the holder, the CLI or the wire
  protocol for the sake of the port (bugs found on the way are fixed
  separately).
- No redesign. Layout, words, shortcuts and behaviour follow the current app;
  the inventory is the spec. Improvements come after the switch.
- No GPUI web target. The website demo keeps the existing React mock
  (see "Website demo").
- No code from Zed's GPL-3.0 crates, copied or closely paraphrased.

## Architecture

```
crates/pitwall-app            (bin "pitwall")
├─ lib.rs        run(): choose the data folder, start the host, open windows
├─ home.rs       data folder choice and the coexistence guard
├─ host.rs       Engine + providers + CLI socket server + approvals (= src-tauri setup)
├─ bridge.rs     EventSink → channel → main thread (AppEvent)
├─ agents.rs     AgentStore entity (agents, approvals, blocked count) + grouping
├─ theme.rs      tokens (tokens.css, glass.css, motion.css) and the Appearance global
├─ ui_state.rs   the one typed ui.json (layout, settings, explorer prefs)
├─ kit/          the shared widgets, fonts, icons and the one asset source
├─ terminal.rs   pitwall-term-view in panes, fed through the engine
├─ window_state  main window bounds (app-windows.json)
├─ menu.rs       actions, key bindings, native menu
├─ main_screen/  sidebar, spaces and panes, right panel, dialogs, Elsewhere
├─ explorer/ settings/ approvals/ remote/ platform/ packaging/
└─ ui/           MainView: the main screen, explorer, settings layer, approvals
crates/pitwall-term-view      GPUI terminal element over alacritty_terminal (separate branch)
```

Later phases add `platform/` (badge, tray, notifications, glass, single
instance), `kit/` (the in-house component set, [in-house.md](in-house.md))
and one module per screen (`ui/wall`, `ui/review`, `ui/explorer`, …).

Dependencies: `gpui` (Apache-2.0, pinned `=0.2.2`), `futures` (the bridge
channel), `serde`/`serde_json`, and Pitwall's own crates. `gpui-component`
is Apache-2.0 and allowed; its releases up to 0.5.1 build on gpui 0.2.2,
later ones on a different GPUI (`gpui-pre`). It is not used in phase 0;
phase 1 decides per component ([in-house.md](in-house.md), [licensing.md](licensing.md)).

### Hosting the engine

`host::Host::start` mirrors `src-tauri/src/lib.rs` `setup` step by step:

| Tauri (`src-tauri`) | GPUI (`crates/pitwall-app`) |
|---|---|
| `platform::prepare_env` → `shell::adopt_login_path` | same call in `run()`, with the chosen folder's `login-path` cache |
| `Paths::new(Paths::default_root())` | `home::choose` (below), then `Paths::new(root)` |
| `LocalProvider` (hold dir, `holder::holder_bin()`, hook + CLI sockets) | same; `host::holder_bin()` searches the same places |
| `AgwProvider::new(AgwConfig::new(hold_dir, holder))` | same |
| `Engine::open(Deps { FileStore(state.json), events, SystemClock, providers })` | same, with `events = Bridge` |
| `engine.start()` (kinds, hook relay script + hook socket, ticker, saver) | same |
| `server::start`: `Approvals` (120 s), `ProcessIdentity`, `pitwall_daemon::serve` on the CLI socket | same, approvals reported through the bridge |
| `RunEvent::Exit`: `core.save()`, `server.stop()` | `cx.on_app_quit` → `Host::shutdown` |
| "Quit and Stop Agents": `lifecycle::stop_all` then exit | same, on the background executor |
| Elsewhere (`onboarding::elsewhere`) managed state | phase 2, owned by the host |

Agents keep running in their holders when the app quits, exactly as now.

### Data folder

Everything Pitwall owns sits under one data folder (`~/.pitwall` on macOS,
`$XDG_DATA_HOME/pitwall` on Linux, `%APPDATA%\Pitwall` on Windows;
`$PITWALL_HOME` overrides): the same folder the Tauri app used up to v0.1.x,
so an upgrade keeps the agents, spaces, windows and settings (`home.rs`):

1. Without `PITWALL_HOME` the app runs on the platform default. With it,
   on that folder: its own state, sockets and holders (development, tests,
   a second copy next to the installed one).
2. Two apps on one folder would fight: binding the hook socket replaces
   the other app's (`LocalListener::bind` removes the old file), both would
   attach to the same holders, and both would write `state.json`. So the
   app **refuses** when another Pitwall answers on that folder's CLI socket
   (a connect to `run/pitwalld.sock` or its named pipe): a second copy of
   this app first asks the running one to come forward
   (`platform::single_instance`) and exits; an older Pitwall (the Tauri
   app) still running gets a window that says to quit it. No engine starts
   and nothing is written.
3. What carries over from the Tauri app: `state.json`, `projects.json`,
   `rules.json` (engine-owned, unchanged); `ui.json` (the same JSON shape,
   typed in `ui_state.rs`); `windows.json` (the same format, physical
   pixels per window label; `windows/file.rs`); `review.json` (moved into
   `ui.json` once, `review/ops.rs`). The main window's own bounds are in
   `app-windows.json` (logical pixels); until that exists the main entry of
   `windows.json` is used. Running agents: the holders (`run/hold/`) are
   re-attached, as after any restart (`host.rs` test
   `a_second_app_reattaches_the_first_apps_agents`).

The `pitwall` CLI connects to `~/.pitwall/run/pitwalld.sock` by default: when
testing the GPUI app, always pass `--socket <PITWALL_HOME>/run/pitwalld.sock`
(agents started inside it get `PITWALL_CLI_SOCKET` right by themselves).

### Commands and events

The Tauri commands (`src-tauri/src/commands/*.rs`, 92 in `generate_handler!`,
inventory §30) are thin wrappers over `pitwall-core` service modules. In the
GPUI app there is no IPC layer:

- **Calls**: views call the same core functions directly. Anything that may
  block (git, disk, holders, agw over ssh, login shells) runs on
  `cx.background_executor()`; the result is applied to an entity on the main
  thread. One small `ops` module per area keeps the call sites in one place
  (`ops::agents::send_prompt(engine, id, text)`), so screens never touch
  engine internals.
- **Events**: the engine's `EventSink` is a [`Bridge`](../../../crates/pitwall-app/src/bridge.rs):
  an unbounded channel of `AppEvent` (engine `Event`s, approvals). One task
  on the main thread drains it into the `AgentStore` entity; views
  `cx.observe` the store and re-render. `Attention` becomes a `StoreEvent`
  that notifications and toasts subscribe to.
- **Streams**: `attach_output` / `watch_screen` (Tauri channels) become
  direct subscriptions of `pitwall-term-view` to the agent's `TermHost`
  output, without JSON or base64.
- **Webview-only events disappear**: `ui-state-changed` (all windows read one
  entity), `open-settings` (an action), `bench` (a module calling the same
  actions).

### State model

| Today | GPUI app |
|---|---|
| `agents-changed` + `useAgents` | `AgentStore` entity (done in phase 0) |
| `projects-changed`, onboarding scan | `Projects`, `Onboarding` entities |
| `ui.json` (opaque blob, every window) | typed `UiState` entity, same file and JSON shape so the switch keeps the user's spaces and layout |
| `windows.json` (backend) | `Windows` entity; phase 9 reads the Tauri file once |
| localStorage (10 keys, inventory §28) | fields of `UiState` or a small `prefs.json` |
| `state.json`, `projects.json`, `rules.json` | unchanged (engine-owned) |

Writes are debounced and done on the background executor, atomically
(temp file + rename), as now.

### Theming

- Tokens from `src/styles/tokens.css` live in `theme.rs` as a `Theme`
  global (done: colours, radii, bar and panel sizes; a test fails if the
  CSS values disappear). Light/dark follows the system now; Settings →
  Appearance (System/Dark/Light) comes in phase 6.
- **Density**: a `Density` global scales row heights, paddings and terminal
  font size, as the CSS density classes do.
- **Glass/Flat**: Flat paints opaque. Glass sets
  `WindowBackgroundAppearance::Blurred` and translucent surface tokens;
  per-OS details and the "Reduce transparency" fallback in
  [platform.md](platform.md).
- **Reduce motion**: one flag (setting OR the OS preference) that every
  animation checks (pulse dots, blocked glow, drawer slides).
- **Fonts**: Inter, Barlow Condensed and JetBrains Mono (OFL-1.1, already in
  `LICENSES/`) are embedded and registered with `cx.text_system().add_fonts`.
  Phase 0 uses the system UI font.

### Windows

- Main window: title, 1400×900 default, 640×480 minimum, bounds saved
  0.5 s after the last move or resize to `app-windows.json` and restored if
  still on a connected display (done). gpui 0.2.2 on macOS reports the frame
  but opens with that size as content, so the content size is saved.
- Closing main: macOS keeps the app (Dock reopens it); Linux quits;
  Windows hides to the tray once the tray exists (phase 7). Same as
  `HostInfo::hides_on_close`.
- Extra windows (one per moved-out space) in phase 2, sharing every entity
  of the one process: no cross-window sync events.

## Accessibility gap

The web view gives the current UI an accessibility tree for free (ARIA
labels, roles, focus order). **GPUI 0.2.2 exposes none**: VoiceOver,
Narrator and Orca see an empty window. Plan:

1. Keyboard-complete from phase 1: every action reachable without a mouse,
   visible focus rings, logical tab order (`tab_index`, focus handles).
2. Contrast, text scaling (respect the OS text size), Reduce motion and
   Reduce transparency in phase 6.
3. A screen-reader bridge in phase 8: either Zed upstream's work, if it
   lands in a gpui release, or an in-house AccessKit (MIT/Apache-2.0)
   adapter fed from Pitwall's own view tree (roles and labels declared next
   to each component in `kit/`). Prototype it on macOS first.
4. The switch (phase 9) needs a decision: ship with step 3 done, or accept a
   documented, time-boxed regression. Open question for the maintainer.

## Website demo

The website (`website/`) embeds the React UI built against the browser mock
(`src/mock*.ts`, `src/lib/demoBridge.ts`, `?demo=1`). GPUI has no web
target, so the demo keeps that React build. Since the switch the React code
is demo-only (no Tauri imports; [src/README.md](../../../src/README.md)),
frozen except to follow visible changes; screenshots come from the GPUI app.

## Phases and parity milestones

Each phase ends with its inventory sections ticked, tests green on all three
OSes and the app usable for that slice in a separate `PITWALL_HOME`.

| Phase | Scope | Inventory | Done when |
|---|---|---|---|
| 0 | **Shell**: crate, window + bounds, app menu, theme tokens, engine hosted, live sidebar, terminal placeholder, CI on 3 OSes | — | this commit |
| 1 | **Foundation**: in-house kit (button, text input, list, scroll, popover/menu, modal, tooltip, tabs, toast), title bar + in-window menu (Windows/Linux), fonts, `UiState` on `ui.json`, keyboard/focus model | §1, §23 (base) | kit has tests; shortcuts table per OS works |
| 2 | **Terminal + main screen**: `pitwall-term-view` wired to holders, spaces, tiling and panes, full sidebar, right panel (Next up, queue, prompt box), top bar, status strip, command palette, new-agent and terminal dialogs, drag and drop, multi-window | §2–§8, §10, §16–§18, §25 | daily use of agents and terminals works without the Tauri app |
| 3 | **Wall** | §9 | Wall matches perf budget with 20 agents |
| 4 | **Review**: task list, diff viewer (in-house), commit and merge | §11–§13 | review and merge of a worktree agent end to end |
| 5 | **Explorer + worktrees**: file tree, viewer, quick open, search | §14, §15 | |
| 6 | **Settings, onboarding, rules, appearance** (Glass/Flat, density, theme, Reduce motion) | §19–§21, §24 | first launch on a clean folder works |
| 7 | **agw, CLI approvals, OS integration**: approvals dialog, badge, tray, notifications, permissions onboarding, CLI install, Elsewhere | §22, §26, §27 | parity on all three OSes |
| 8 | **Packaging**: bundles, sidecars, signing, release workflow, accessibility bridge, perf pass | §33, packaging.md | signed builds from CI |
| 9 | **Switch** (done, v0.2.0): GPUI app becomes `pitwall` (`dev.pitwall.app`), runs on the default data folder, reads `windows.json`; Tauri app and `src-tauri/` removed; React kept for the website only | all | one release ships the GPUI app |

## Risks

| Risk | Mitigation |
|---|---|
| **GPUI API churn**: crates.io `gpui` is 0.2.2 (Oct 2025); Zed's main moves weekly | Pin `=0.2.2` exactly; one upgrade per phase at most, in its own commit; `pitwall-term-view` and `pitwall-app` move together (one gpui in the workspace) |
| GPUI releases on crates.io stall | Fall back to a pinned Zed git rev (gpui crate only), or the `gpui-pre` snapshots; [licensing.md](licensing.md) |
| **Licensing**: accidental GPL code or dependency | Only `gpui` from Zed; licence of every new dependency recorded in licensing.md and reviewed on lockfile changes; clean-room rule for in-house components |
| Accessibility regression | Plan above; decision before phase 9 |
| Platform gaps in GPUI (badge, tray, notifications, vibrancy) | Small in-house platform modules ([platform.md](platform.md)) |
| Build cost: GPUI adds ~400 crates; Linux needs X11/Wayland/Vulkan dev packages | CI caches (WebKitGTK and Tauri's ~180 crates went with the switch) |
| Two apps on one data folder | `home.rs` guard: refuses while another Pitwall answers on the folder's CLI socket |
| Text input, IME, selection quality | Built and tested in phase 1 before any screen depends on it |

## Post-migration ideas

Not built during the migration (parity only); candidates once the GPUI app
has replaced the Tauri app.

- Multi-window: a space tab's right-click menu to move the space back to
  the main window or to another open window (today a space returns to main
  only when its window closes).
- Multi-window: close a secondary window by itself when its last space
  moves out or closes, and don't reopen saved windows that have no space
  left (today, as in the Tauri app, they stay open showing "No spaces in
  this window").
- Multi-window: title secondary windows after their space (all windows are
  titled "Pitwall", so the Window menu lists the same name for each).
