# Extensions quick start

**Preview.** This page takes you from an empty folder to a tile that builds, passes its checks and runs its tests in Pitwall's own tile runtime. Two steps are still being built in this preview and are marked below: showing the tile in Pitwall's layout (which `pitwall extension dev` loads it into), and installing it from a folder.

## Before you start

You need Rust, installed with [rustup](https://rustup.rs), and its WebAssembly target. The target is the only addition the SDK needs: no `cargo-component`, no other tools.

```sh
rustup target add wasm32-wasip2
```

You also need Pitwall's CLI, `pitwall`, which comes with the app (see [the CLI reference](../cli.md#install)). The `pitwall extension new`, `check` and `test` commands work on a folder without the app running.

## Make a tile

`pitwall extension new` writes a Cargo project into a new folder. The folder's name is the extension's id: lowercase letters, digits and hyphens.

```sh
pitwall extension new my-tile --template blank
cd my-tile
```

There are three templates:

| Template | What it is | Shows |
|---|---|---|
| `blank` | A counter kept in storage | Elements, listeners, `storage` |
| `list` | A checklist you add to, tick and filter; it can be opened with a list to show | Keys on rows, scrolling, text fields, content, `save_state` |
| `form` | A form that saves notes, signed with a setting; it can be opened with a draft | Multi-line fields, `settings`, toasts |

`--name "My tile"` sets the name Pitwall shows (by default it comes from the folder's name). Until the SDK is on crates.io, the project takes it from Pitwall's repository on GitHub; `--sdk <checkout>` takes it from a local checkout instead.

The new project holds:

| File | What |
|---|---|
| `extension.json` | The manifest: id, name, version, the tile (`contributes.tiles`), settings, permissions with your reasons |
| `Cargo.toml` | A library with `crate-type = ["cdylib"]`, the `pitwall-extension` SDK, and `pitwall-extension-test` for tests |
| `src/lib.rs` | The tile: its state, `render`, listeners |
| `tests/tile.rs` | Tests that build the tile and run it in Pitwall's tile runtime |
| `input.schema.json` | `list` and `form` only: what the tile can be opened with, as a JSON schema |
| `README.md` | How to develop it |
| `AGENTS.md`, `.claude/skills/`, `.agents/skills/` | The authoring skill, for a coding agent working in the project ([For coding agents](for-agents.md)) |

## The tile

This is the `blank` template's tile. `NAME` is a constant above it that `new` fills in with the tile's name.

```rust
{{#include crates/pitwall-cli/templates/tile/blank/lib.rs.tmpl#blank-tile}}
```

- `new` makes the tile's state. Here it reads the count kept in `storage`.
- `render` returns the tree of one view. Pitwall calls it after the view opens and after a listener calls `cx.notify()`.
- `cx.listener` makes a click handler. It gets the tile, the event and the context.
- `export!` makes `App` the component's tile, once per crate.

[Tiles and UI components](ui.md) describes the elements, styles and events.

## The manifest

`extension.json` describes the extension. The fields a tile project uses:

| Field | Means |
|---|---|
| `v` | The manifest format: `2` for extensions with tiles |
| `id` | The extension's id, the same as the folder's name |
| `name`, `description`, `icon` | What Pitwall shows for it |
| `version` | Its version, like `0.1.0` |
| `pitwall` | The oldest Pitwall it runs on, like `>=0.9.0` |
| `settings` | Settings Pitwall draws a form for ([Platform services](platform-services.md#settings)) |
| `permissions` | Each permission it asks for, with your reason ([Permissions](permissions.md)) |
| `contributes.tiles` | Its tiles. Each has an `id`, a `title`, an `icon`, the component (`"wasm": "tile.wasm"`), the world (`"world": "pitwall:tile@0.1.0"`) and, when it can be opened with content, its schema (`"input": "input.schema.json"`) |

An extension has one component. Every tile entry names the same `tile.wasm`, and each tile id is a view that component renders.

## Develop with `pitwall extension dev`

```sh
pitwall extension dev
```

Each round, `dev` builds the tile (`cargo build --release --target wasm32-wasip2`), copies the component to `tile.wasm` beside `extension.json`, and checks it as `pitwall extension check` does. It does this once at the start and again after every save in the folder (changes in `target/` and to `tile.wasm` are ignored), and prints one JSON line per round. `--once` does one round and exits, with status 1 when the build or the check failed.

**Coming in this preview: loading into Pitwall.** After a round that passes, `dev` asks the running Pitwall to load the folder as a development extension, or to reload it there, keeping the tile's state across the reload. The app's side of this comes with tiles in Pitwall's layout. Until then the running Pitwall answers that it can't load development tiles yet, and `dev` says so and goes on building and checking.

Keep what the user is typing or looking at across a reload with `save_state`. The `list` template keeps its filter and the item being typed:

```rust
{{#include crates/pitwall-cli/templates/tile/list/lib.rs.tmpl#list-save-state}}
```

and reads them back in `new`:

```rust
{{#include crates/pitwall-cli/templates/tile/list/lib.rs.tmpl#list-restore}}
```

## Check it with `pitwall extension check`

```sh
pitwall extension check .
```

`check` reads the folder without running anything: the manifest, the component's world and imports, its size (at most 20 MiB), each tile's input schema, and each permission with its reason. It exits with status 1 when something fails. You can also give it `extension.json` or a `.wasm` file alone.

## Test it

The project's tests run the built tile in the same runtime Pitwall uses, with a test host: a temporary data folder, the settings and grants you give it, and the toasts and Log lines it produces. Each action waits until the tile has handled it and rendered.

```rust
{{#include crates/pitwall-cli/templates/tile/blank/test.rs.tmpl#blank-test}}
```

Run them with `cargo test`, or with:

```sh
pitwall extension test .
```

which builds the tile, checks it, and then runs `cargo test`.

A test can open a view with content, as an agent would, and give it new content later:

```rust
{{#include crates/pitwall-cli/templates/tile/list/test.rs.tmpl#list-test-content}}
```

The harness, `pitwall-extension-test`, can also type into fields (`type_into`, `submit`), press keys (`press`), let time pass for timers (`wait`, `wait_for_text`), change a setting (`set_setting`), reload the tile (`reload`), and read what it stored (`stored`, `stored_json`), logged (`log_text`) and toasted (`toasts`). `text_snapshot()` prints the tree, which helps when an assertion fails.

## Share a preview tile

**Coming in this preview.** Before packages exist, an author can pass a tile around as a folder: the one holding `extension.json`, `tile.wasm` and the files the manifest names (such as `input.schema.json`).

- **In Pitwall:** Settings → Extensions → Install from folder…
- **From a terminal:** `pitwall extension install <folder>`, or a URL of such a folder served over HTTPS.

Pitwall copies the folder into its own data folder, checks the component's imports, and shows the permission screen with an "Unsigned, preview: only install tiles from people you trust" warning ([Permissions](permissions.md#unsigned-preview-tiles)). The tile is marked as a developer preview wherever it appears. It is never updated: installing a newer folder replaces it. Unlike `pitwall extension dev`, the folder is copied, not loaded in place.

Today `pitwall extension install` takes only the id of an extension Pitwall ships. Signed packages, updates and a marketplace come in later releases.

## Next

- [Tiles and UI components](ui.md): elements, styles, events and keys.
- [Platform services](platform-services.md): storage, settings, timers, toasts and the Log.
- [WIT reference 0.1](wit.md): the contract between a tile and Pitwall.
- [The CLI reference](../cli.md#extension-new) for each `pitwall extension` command's options.
