# pitwall-tile-host

Runs extensions' tile components: Wasmtime 48 (LTS, lean build), the WIT
world `pitwall:tile@0.1.0`, one worker thread per running component, the
limits of [runtime.md §2.5](../../docs/spec/tile-sdk/runtime.md), Pitwall's
own compile cache, and the host side of `ui`, `log`, `tasks`, `storage` and
`settings`. No GPUI: the app draws the frames this crate hands it. Design:
[docs/spec/tile-sdk/](../../docs/spec/tile-sdk/README.md); progress:
[plan.md](../../docs/spec/tile-sdk/plan.md).

## The WIT

`wit/` is the 0.1 package, one file per interface, every item documented
(the doc comments are the WIT reference and feed `llms.txt`):

| File | Interface | What |
|---|---|---|
| `world.wit` | world `tile` | imports `ui`, `log`, `tasks`, `storage`, `settings`; exports `guest`; WASI 0.2 for std (stdout/stderr to the Log, no files or sockets) |
| `types.wit` | `types` | the frame (flat node list with parent indices), elements `box`, `text`, `button`, `icon`, `scroll`, `input`; styles with theme tokens only; listeners; events; views and content; `host-error` |
| `guest.wit` | `guest` (export) | `init`, `render(view)`, `handle(event) -> bool`, `save-state` |
| `ui.wit` | `ui` | `request-render`, `scroll-to`, `toast`, `open-view` |
| `log.wit` | `log` | `debug`, `info`, `warn`, `error` |
| `tasks.wit` | `tasks` | `timer(ms, repeat)`, `cancel` |
| `storage.wit` | `storage` | key-value: `get`, `set`, `delete`, `list`, `usage` (10 MiB) |
| `settings.wit` | `settings` | `get`, `secret`, `open` |

A guest is written with the SDK, `sdk/pitwall-extension` (its `wit/` is a
copy of this one; `tests/sdk_wit.rs` keeps them equal), and built with
`cargo build --target wasm32-wasip2`; see `tests/fixtures/hello-tile/` (an
SDK tile with its own workspace and toolchain file) and
`tests/fixtures/build-hello-tile.sh`, which rebuilds the committed
`tests/fixtures/hello/tile.wasm`. **Changing `wit/` means rebuilding and
committing that file** (and copying it to the SDK). Only CI's Tile SDK job
builds wasm. Tests of a tile from outside: `crates/pitwall-extension-test`.

## The API

```rust
use pitwall_tile_host::{HostConfig, StartOptions, TileHost, TileOutput, ViewSpec, ui};

// Once per app. The data folder is the caller's (never guessed here):
// compiled files go in <data>/cache/tiles/<id>/, each extension's storage
// and settings in <data>/extension-data/<id>/.
let host = TileHost::new(HostConfig { data_dir, secrets: Arc::new(MySecureStore) })?;

// Check an extension folder (manifest with contributes.tiles, its
// component file, its world). Cheap: nothing is compiled.
let ext = host.load(&dir)?;                        // -> Result<Extension, LoadError>

// Start it. Returns at once; the worker thread `tile:<id>` compiles or
// loads the cached code, instantiates, calls init, renders the open views.
let handle = host.start(&ext, StartOptions::view("main"), move |out| {
    let _ = tx.send(out);                          // runs on the worker: only forward
})?;                                               // -> Result<TileHandle, LoadError>
```

`TileHost`: `new(HostConfig)`, `load(&Path) -> Result<Extension, LoadError>`,
`start(&Extension, StartOptions, impl FnMut(TileOutput) + Send) ->
Result<TileHandle, LoadError>`, `compile(&Extension) -> Result<Compiled,
LoadError>` (blocking; for `pitwall extension check`; fills the cache),
`is_compiled(&Extension) -> bool`, `extension_data(id) -> ExtensionData`,
`data_dir()`. Cheap to clone.

`StartOptions { views: Vec<ViewSpec>, grants: Grants, theme: ui::ThemeInfo,
visible: bool, state: Option<Vec<u8>> }`; `StartOptions::view(tile)` opens
one view of that tile, visible, nothing granted.

**Views.** One component runs per extension; every place in the layout
showing one of its tiles is a view, addressed by (extension id, tile, view
id). `ViewSpec { id, tile, content: Option<ui::Content> }`;
`ViewSpec::tile("main")` has view id = tile id. The guest gets
`view-opened` for each view (with its content), `view-content` for later
content, `view-closed`. This is the hook for agents opening a tile with
content (show-only, phase 2): the app keeps a content id → view id map and
calls `set_content` for a repeat.

`TileHandle` (every method only posts; false once the worker is gone):
`send(ui::Event)`, `open_view(ViewSpec)`, `set_content(view, ui::Content)`,
`close_view(view)`, `set_visible(bool)`, `set_theme(ui::ThemeInfo)`,
`set_grants(Grants)`, `settings_changed()`, `save_state()`, `stop()`
(interrupts a running call within one epoch tick), `is_running()`, `join()`
(blocking; not for the UI thread), `id()`. Dropping it stops the tile.

`TileOutput`, in order, through the sink:

| Output | Meaning |
|---|---|
| `Preparing` | not in the cache: compiling (show "Preparing…") |
| `Started { cached, took }` | instantiated, `init` ok |
| `Frame(FrameUpdate { view, tile, frame: Arc<ui::Frame>, diff: Diff, took })` | a view's whole tree, already checked against the limits, with its keyed diff (`continues[i]`: the old node index whose state node `i` keeps; `changed`, `added`, `removed`). Unchanged renders aren't sent |
| `Log(LogLine { level, message, at, from_host })` | the tile's Log: its `log` calls, stdout (info) and stderr (warn), and host notes (slow calls, dropped toasts) |
| `Ui(UiRequest)` | `Toast { text, tone }`, `OpenView(tile)`, `ScrollTo { view, key_path }`, `OpenSettings`: apply with `cx.defer` |
| `StateSaved(Vec<u8>)` | answer to `save_state()` |
| `Stopped(StopReason)` | the last output: `Stopped` (by `stop()`), `Deadline { call, limit }`, `Crashed(_)` (trap, panic, memory limit), `FrameRefused(_)` (tree limits), `InitFailed(_)`, `LoadFailed(_)`. `Display` gives the user-facing words |

`ui` re-exports the WIT types (`Frame`, `Node`, `Element`, `Style`, `Token`,
`Event`, `Click`, `Content`, `ThemeInfo`, …); `ui::default_theme()`.

`Grants::new(["memory"])`: the host enforces what the caller passes.
Nothing in WIT 0.1 needs a permission; `memory` raises linear memory to the
manifest's `"memory": {"reason", "mib"}` (64 MiB otherwise, at most 512).

`ExtensionData` (`host.extension_data(id)` or `ExtensionData::new(data_dir,
id)`): `settings()`, `set_setting(key, Option<&str>)` (values as
`SettingDef::check_value` returns them; never secrets), `storage_bytes()`,
`dir()`. After writing a setting, call `handle.settings_changed()`.
Secrets come from the `Secrets` trait in `HostConfig` (the app's secure
store; called on the worker thread).

`frame::check` and `frame::diff` are public for the test harness; `limits`
has every limit as a constant.

## Limits (runtime.md §2.5)

Deadlines are wall clock, checked on every 10 ms epoch tick: `init` 2 s,
`handle` 1 s (warned over 50 ms), `render` 250 ms (warned over 16 ms),
`save-state` 50 ms. Memory 64 MiB (grow failures trap). Frames: 20 000
nodes, about 2 MiB, keys ≤ 128 bytes, a text field ≤ 1 MiB. 200 host calls
a second; 32 timers, ≥ 100 ms; storage 10 MiB; 3 toasts a minute; content
≤ 8 MiB. Over a frame limit or a deadline the tile stops with the reason.

## Threading

The store lives on the worker; the UI thread only posts and receives. The
worker takes every queued message at once (scroll, pointer moves, resize,
typed text and theme changes coalesce to the newest), handles them, fires
due timers (only while visible), renders the views that changed, writes the
key-value store if it changed, then hands over Log lines and UI requests.
Hidden: no renders, no timers; showing renders every open view.

## Tests

`cargo test -p pitwall-tile-host` (macOS, Linux and Windows in CI):
load, render, click round trip, storage across restarts, settings and
`settings-changed`, views with content, hidden start and `save-state`,
deadline stop, user stop during a hang, tree-limit refusal, panic and
memory traps, bad manifests and a non-component file.
