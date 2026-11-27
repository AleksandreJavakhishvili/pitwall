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
| `world.wit` | world `tile` | imports `ui`, `log`, `tasks`, `storage`, `settings`, `agents`, `badges`, `asks`, `net`, `sql`, `notify`, `show`, `toolbar`, `service`, `agent-tools`; exports `guest`; WASI 0.2 for std (stdout/stderr to the Log, no files or sockets) |
| `types.wit` | `types` | the frame (flat node list with parent indices), elements `box`, `text`, `button` (kind and size), `icon`, `scroll`, `input`, `canvas` (ops, transform, hit regions), `code-view`, `diff-view`; drag and drop styles; styles with theme tokens only, hover groups; listeners; events (`focus-changed` for the focused node, `agents-changed`, `ask-answered`, `fetched`, `toolbar-clicked`, `service-changed`); views and content; `host-error` |
| `guest.wit` | `guest` (export) | `init`, `render(view)`, `handle(event) -> bool`, `save-state` |
| `ui.wit` | `ui` | `request-render`, `scroll-to`, `toast`, `open-view`, `put-image`, `drop-image`, `measure-text` (widths in Pitwall's Inter and JetBrains Mono from tables `build.rs` makes of the app's font files, `measure.rs`; ≤ 1 000 texts, 64 KiB a call) |
| `log.wit` | `log` | `debug`, `info`, `warn`, `error` |
| `tasks.wit` | `tasks` | `timer(ms, repeat)`, `cancel` |
| `storage.wit` | `storage` | key-value: `get`, `get-many` (≤ 1024 keys, one call), `set`, `delete`, `list`, `usage` (10 MiB) |
| `settings.wit` | `settings` | `get`, `secret`, `open` |
| `agents.wit` | `agents` | `list` (`agents.read`), `queue-add` (`agents.queue`, ≤ 4 000 characters): the daemon's `agent.list` and `queue.add` through `HostConfig::methods`, called as the extension |
| `badges.wit` | `badges` | `set`: the whole set of sidebar badges (`"badges": true`), checked and cut here, handed to the app as `UiRequest::Badges` |
| `asks.wit` | `asks` | `set` (`"asks": true`) and `reply`: questions for the floating control (`UiRequest::Asks`, `UiRequest::AskResult`); answers come back as the `ask-answered` event |
| `net.wit` | `net` | `fetch(request) -> request-id` (answer later as the `fetched` event), `cancel`; needs `net` and a host from the manifest (https; http only to loopback when listed), or the extension's own server; 8 at once, 8 MiB up, 32 MiB down, 30 s (≤ 120 s), ≤ 5 redirects re-checked; no cookies, proxy or credentials (`net::HttpFetcher`; `TileHost::set_fetcher` for a fake) |
| `sql.wit` | `sql` | the extension's SQLite file (`<data>/extension-data/<id>/db.sqlite`): `execute`, `query` (≤ 10 000 rows, 8 MiB), `migrate(steps)` (`user_version`, a transaction per step), `begin`/`commit`/`rollback` (open past a call: rolled back), `usage`, `quota` (100 MiB, or the `sql` grant's size); no `ATTACH`, extensions or writing pragmas; stopped at the call's deadline |
| `notify.wit` | `notify` | `send(title, body)`: needs `notify`; 3 a minute, the same words once in 10 minutes |
| `show.wit` | `show` | `file(path, line)` (the editor), `view(path, line)` (the tile's file viewer window, `UiRequest::ViewFile`), `web(url)`: need `show` |
| `toolbar.wit` | `toolbar` | `set-badge`, `set-project-badge` (needs `projects.read`), `set-tooltip`, `set-shown` (≤ 1 in 5 s), `close-popover`; the popover is view `toolbar:popover` (`POPOVER_VIEW`) |
| `service.wit` | `service` | `info`, `restart`, `open-card` for the extension's own server; `TileHandle::set_service` gives its state (the first after start isn't a change) |
| `agent-tools.wit` | `agent-tools` | an agent's call answered: `reply(call, tool-output)` (text, JSON, item; or an error with problems), `user-edited(call)` |
| `files.wit` | `files` | the view's project (`project(view)`) and the extension's own files folder (`root.data`): `read`, `stat`, `list` (deep, `.gitignore` honoured), `watch`/`unwatch` (coalesced `files-changed` events), `write`, `create`, `make-folder`, `rename`, `delete` (to the Trash in a project); `files.read` to read, `files.write` to change, refused in gated agent calls; `.git` never reached, `.pitwall.json` and `.pitwall/` never changed, links out refused; changes and refusals as `UiRequest::Activity`. `ViewSpec.project` and `TileHandle::set_project` say the view's project (`project-changed`); `TileHost::set_watcher` and `set_trash` swap in fakes (`files::FakeWatcher`, `files::FolderTrash`) |

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
let host = TileHost::new(HostConfig { secrets: Arc::new(MySecureStore), methods: Arc::new(MyDaemon), ..HostConfig::new(data_dir) })?;

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
visible: bool, state: Option<Vec<u8>>, clock: Clock }`; `StartOptions::view(tile)` opens
one view of that tile, visible, nothing granted, on the wall clock.
`Clock::manual()` is a virtual clock for tests: time (timers, rate limits,
WASI's `Instant` and `SystemTime`) stands still until
`TileHandle::advance(d)`, which fires each timer at its own time; call
deadlines stay on the wall clock.

**Keys.** `keys::reserved(keys)` says whether a binding is one of Pitwall's
own (`keys::reserved_list()` lists them for the docs); the host drops such a
binding from a frame's keymap with a Log line, once per key.

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
`set_grants(Grants)` (send it on every change while it runs: the next call is checked against it; requests on their way when `net`'s reach shrank arrive `denied` with their answers dropped, project watches end without `files.read`, badges on rows it may no longer see go; memory and the database's size apply after a restart), `set_project(view, Option<ui::Project>)`, `settings_changed()`, `save_state()`, `rows(view, list,
start, count)` (rows of the list, table or tree at key path `list` in the
view's newest frame; answered after the batch's renders, so they match its
revision; ignored for no such list), `advance(Duration)`
(a virtual clock only), `stop()`
(interrupts a running call within one epoch tick), `is_running()`, `join()`
(blocking; not for the UI thread), `id()`. Dropping it stops the tile.

`TileOutput`, in order, through the sink:

| Output | Meaning |
|---|---|
| `Preparing` | not in the cache: compiling (show "Preparing…") |
| `Started { cached, took }` | instantiated, `init` ok |
| `Frame(FrameUpdate { view, tile, frame: Arc<ui::Frame>, diff: Diff, took })` | a view's whole tree, already checked against the limits, with its keyed diff (`continues[i]`: the old node index whose state node `i` keeps; `changed`, `added`, `removed`). Unchanged renders aren't sent |
| `Log(LogLine { level, message, at, from_host })` | the tile's Log: its `log` calls, stdout (info) and stderr (warn), and host notes (slow calls, dropped toasts) |
| `Ui(UiRequest)` | `Toast { text, tone }`, `OpenView(tile)`, `ScrollTo { view, key_path }`, `OpenSettings`, `Badges(list)`, `Asks(list)`, `AskResult { id, refused }`, `PutImage { name, format, bytes }` (sniffed, within the limits; before the frames after it), `DropImage(name)`, `Notify { title, body }`, `OpenFile { path, line }`, `OpenWeb { url }`, `Toolbar(ToolbarRequest)`, `RestartService`, `ViewFile { path, line }`, `Activity { method, detail, refused }` (a line for the extension's Activity: a change to a project's files, or a refusal): apply with `cx.defer` |
| `Rows(RowsUpdate { view, list, revision, start, rows, took })` | answer to `rows(..)`: the rows, checked (`components::check_rows`: ≤ 64 nodes of box, text, icon, button, image, context-menu; unique keys); rows that fail stop the tile (`FrameRefused`) |
| `StateSaved(Vec<u8>)` | answer to `save_state()` |
| `Answered { call, answer }` | an agent's call delivered as view content (`content.call`) ended: `Answer::Reply(tool-output)`, `Handled` (no reply: the caller says where it opened) or `NotDelivered(why)` |
| `UserEdited(call)` | the user edited what call `call` showed: `agent::ShowCalls::user_edited` |
| `Stopped(StopReason)` | the last output: `Stopped` (by `stop()`), `Deadline { call, limit }`, `Crashed(_)` (trap, panic, memory limit), `FrameRefused(_)` (tree limits), `InitFailed(_)`, `LoadFailed(_)`. `Display` gives the user-facing words |

`ui` re-exports the WIT types (`Frame`, `Node`, `Element`, `Style`, `Token`,
`Event`, `Click`, `Content`, `ThemeInfo`, …); `ui::default_theme()`.

`Grants::for_extension(manifest, granted).with_own_server(addr)` (or
`Grants::new(["memory"])`): the host enforces what the caller passes.
`memory` raises linear memory to the manifest's `"memory": {"reason",
"mib"}` (64 MiB otherwise, at most 512); `sql` raises the database quota
(100 MiB otherwise, at most 2048; read when the tile starts); `net` allows
the manifest's `hosts`; `notify` and `show` their calls. The own server's
address is reachable over http without `net`.

`ExtensionData` (`host.extension_data(id)` or `ExtensionData::new(data_dir,
id)`): `settings()`, `set_setting(key, Option<&str>)` (values as
`SettingDef::check_value` returns them; never secrets), `storage_bytes()`,
`dir()`. After writing a setting, call `handle.settings_changed()`.
Secrets come from the `Secrets` trait in `HostConfig` (the app's secure
store; called on the worker thread). The agents' calls go through the
`Methods` trait (`call(extension, method, params)`: the daemon's method
table, called as that extension; `NoMethods` answers "unsupported"); the
answer to a question goes back as an event, `handle.send(ui::Event::AskAnswered(..))`.

`frame::check`, `frame::diff` and `frame::frame_bytes` are public for the
test harness; `components` has the components' checks and helpers
(`rows_of`, `list_at`, `key_paths`, `asset_path`, `image_format`);
`widgets` has batch 2's (form controls, feedback, layout, data display,
charts; ui.md §3.10): their frame checks (`check`, `check_tree`), and
the rules the app and the harness share so both behave the same:
`on_click` and `on_key` (what a click or a key does to a component, as an
`Outcome`), `focusable`, `snap`, `parse_number`, `step_number`,
`split_share`, `relative_time` and `relative_time_next`, `initials`,
`avatar_tokens`, `series_token`, `axis` and `chart_summary`; `limits`
has every limit as a constant.

`agent` (agent-tools.md §11.6–11.7): `ShowCalls` gives each agent's call an
id and its `Content` (`call`, `tool`, `caller`, the item as the id),
refuses an item the user edited, more than 10 calls in 10 s per agent and
tile, and input over 1 MiB; `capped` keeps a reply within 4 KiB of text and
256 KiB of JSON. The app keeps one per extension; so does the harness.

## Limits (runtime.md §2.5)

Deadlines are wall clock, checked on every 10 ms epoch tick: `init` 2 s,
`handle` 1 s (warned over 50 ms), `render` 250 ms (warned over 16 ms),
`save-state` 50 ms. Memory 64 MiB (grow failures trap). Frames: 20 000
nodes, about 2 MiB, keys ≤ 128 bytes, a text field ≤ 1 MiB. 200 host calls
a second; 32 timers, ≥ 100 ms; storage 10 MiB; 3 toasts a minute; content
≤ 8 MiB. Components: ≤ 1 000 000 rows a list, rows 12–200 px, ≤ 256 rows a
`rows` call (250 ms), a row ≤ 64 nodes (boxes, text, icons, buttons, images, a context menu, badges, avatars, relative times), ≤ 64 columns, ≤ 1 000 tabs, options
or menu items, Markdown ≤ 1 MiB, an image put ≤ 8 MiB (64 MiB a tile), an
asset ≤ 16 MiB. Over a frame limit or a deadline the tile stops with the reason.

## Threading

The store lives on the worker; the UI thread only posts and receives. The
worker takes every queued message at once (scroll, pointer moves, resize,
typed text and theme changes coalesce to the newest), handles them, fires
due timers (only while visible), renders the views that changed, writes the
key-value store if it changed, then hands over Log lines and UI requests.
Hidden: no renders, no timers; showing renders every open view.

## In the app

`crates/pitwall-app/src/tiles/` runs one `TileHost` for the app, with
Pitwall's data folder, and draws each tile with the kit (`TileView`, in a
view tile of the layout). A component starts when one of its tiles is
shown, is suspended while none shows, stops when its last tile closes or
the extension is turned off. Its Log lines are in Settings → Extensions
(the card's Log). Settings written there go through `ExtensionData`,
then `settings_changed()`.

### Development reload: `extension.devReload`

The daemon method `pitwall extension dev` calls after each build
(capability `extensions`, never an extension's own):

```json
{"method": "extension.devReload", "params": {"path": "/abs/path/to/my-tile"}}
```

- `path`: the extension's folder (where its `extension.json` is),
  absolute. It must contribute tiles and run no programs (no adapter,
  service or setup actions).
- The first call for a folder asks the user in Pitwall, then loads it as
  a development extension (listed where it is, never copied) and turns
  it on. Later calls ask nothing: the running component saves its state
  (`save-state`), a new one starts from the folder with that state and
  the same open tiles, and its Log says "Reloaded".
- Answer (`ExtDevReloaded`): `{proto, id, name, loaded, tiles,
  reloaded, text, message}`. `loaded`: loaded and turned on by this call;
  `reloaded`: the tiles that were running and were replaced; `text`: what
  happened, in plain words (`message`: the same, which the CLI prints).
- Errors: `bad_params` (not a folder of tiles), `conflict` (that id is
  loaded from another folder), `denied` / `approval_timeout` (the user
  declined, or nobody answered), `busy` (a call for that folder is under
  way).

## Tests

`cargo test -p pitwall-tile-host` (macOS, Linux and Windows in CI):
load, render, click round trip, storage across restarts, settings and
`settings-changed`, views with content, hidden start and `save-state`,
deadline stop, user stop during a hang, tree-limit refusal, panic and
memory traps, bad manifests and a non-component file.
