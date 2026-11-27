---
name: pitwall-extension-authoring
description: "How to build, test and check a Pitwall tile (a Rust WebAssembly component made with the pitwall-extension SDK): the project layout, extension.json, the Tile trait and elements, content from agents, storage and settings, the test harness, and the pitwall extension commands. Use it whenever you change this extension."
---

# Building a Pitwall tile

This project is a **Pitwall extension with a tile**: Rust, built to a
WebAssembly component for `wasm32-wasip2`, that Pitwall runs in a sandbox and
draws in its own theme. The tile describes what to show as a tree of
elements; Pitwall lays it out and sends back what the user did. It never
draws pixels and can't reach files, the network or other programs (WIT
`pitwall:tile@0.1.0`, a preview: it may change before 1.0).

## Commands

| Command | Does |
|---|---|
| `rustup target add wasm32-wasip2` | Once per machine: the only toolchain addition needed (no cargo-component) |
| `pitwall extension dev` | Build (`cargo build --release --target wasm32-wasip2`), copy the component to `tile.wasm`, check it, load it into the running Pitwall; again on every save. `--once` does one round |
| `pitwall extension check .` | Offline: the manifest, the component's world and imports, limits, input schema, permission reasons. Exit status 1 when something fails |
| `cargo test` | The tests in `tests/`: they build `tile.wasm` and run it in Pitwall's own tile runtime |
| `pitwall extension new <id> --template blank\|list\|form` | A new project like this one |

Run `pitwall extension check .` and `cargo test` before you say a change is
done.

## Layout

- `extension.json`: the manifest (below).
- `src/lib.rs`: the tile. `crate-type = ["cdylib"]` in `Cargo.toml`.
- `tests/*.rs`: tests with `pitwall-extension-test`.
- `input.schema.json` (if the tile takes content): JSON schema of what the
  tile can be opened with.
- `tile.wasm`: built; not committed.

## The manifest (`extension.json`)

```json
{
  "v": 2,
  "id": "my-tile",
  "name": "My tile",
  "version": "0.1.0",
  "pitwall": ">=0.9.0",
  "icon": "grid",
  "description": "One line",
  "settings": [ { "key": "repo", "type": "text", "label": "Repository", "default": "" } ],
  "permissions": { "memory": { "reason": "To hold large diagrams", "mib": 128 } },
  "contributes": {
    "tiles": [ { "id": "main", "title": "My tile", "icon": "grid", "wasm": "tile.wasm",
                 "world": "pitwall:tile@0.1.0", "input": "input.schema.json" } ]
  }
}
```

- `id`: lowercase letters, digits and hyphens; the folder is named after it.
- Every tile entry names the same `wasm` (one component per extension); each
  tile id is a view the component renders.
- **Permissions**: every permission needs your reason, one line the user
  reads before granting it. WIT 0.1 needs none for its services; `memory`
  (more than 64 MiB, at most 512) is the only grant with an effect so far.
  Ask only for what the tile uses.
- Setting types: `text`, `url`, `number`, `bool`, `choice`, `secret` (kept
  in the system's secure store, read with `settings::secret`).

## The tile

```rust
use pitwall_extension::prelude::*;

#[derive(Default)]
struct App { clicks: u32 }

impl Tile for App {
    fn new(cx: &mut Cx<Self>) -> Self { App::default() }

    fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        v_flex().gap_2().p_3()
            .child(text(format!("Clicked {} times", self.clicks)).text_color(Token::Text2))
            .child(button("Click me").primary().on_click(cx.listener(|this, _, cx| {
                this.clicks += 1;
                cx.notify();                       // render again
            })))
    }
}

pitwall_extension::export!(App);
```

- **Elements** read like GPUI: `div()`, `h_flex()`, `v_flex()`, `text(..)`
  (`.child(text(..))` for inline runs), `button(..)` (`.primary()`,
  `.danger()`, `.small()`, `.link()`, `.icon(Icon::Plus)`,
  `.disabled(b)`), `icon_button(Icon::X, "Remove")`, `icon(..)`,
  `input("key")` (`.placeholder`, `.value`, `.multiline()`, `.rows(n)`,
  `.on_change`, `.on_submit`). Scroll with `.overflow_y_scroll()` on a
  `div`.
- **Styles** by GPUI's names: `flex`, `flex_col`, `flex_1`, `gap_1`…`gap_8`,
  `p_2`, `px_3`, `size_full`, `w(px(120.0))`, `items_center`,
  `justify_between`, `border_b_1`, `rounded_md`, `text_sm`, `font_mono`,
  `font_semibold`, `truncate`. **Colours are theme tokens only**:
  `.text_color(Token::Text2)`, `.bg(Token::Surface2)`,
  `.border_color(Token::Line)`, `Token::Good`/`Warn`/`Bad`. No raw colours.
- **Keys**: `.id("key")` on nodes that move (list rows: the item's id).
  Pitwall keeps scroll, focus and typed text by key path; text fields always
  have a key (`input("key")`).
- **Listeners**: `cx.listener(|this, event, cx| …)`, made while rendering and
  handed to the element at once. `on_click` takes `Click`; `on_change` takes
  `&str` (annotate it: `|this, v: &str, cx|`) or `InputChange`; `on_submit`
  takes `Submit`. Call `cx.notify()` when what shows changed.
- **Text fields**: Pitwall owns what the user types. Keep your copy from
  `on_change` and pass it back with `.value(..)`: the SDK sends it only
  when it differs, so setting your copy to `""` clears the field.
- **Render quickly** (under 8 ms; the deadline is 250 ms): compute in
  listeners and timers, read state in `render`. `handle` calls have 1 s,
  `new` 2 s. A tile over a deadline is stopped and the user sees why.
- **Limits**: 20 000 nodes and about 2 MiB a frame, 64 MiB of memory, 32
  timers (≥ 100 ms), 10 MiB of storage, 3 toasts a minute.

## Content from agents and the user

A tile entry with `"input": "input.schema.json"` can be opened with content
matching that schema (an agent showing something to the user). Pitwall checks
it against the schema, then:

```rust
#[derive(serde::Deserialize)]
struct Shown { title: Option<String>, items: Vec<Item> }

fn on_content(&mut self, view: &ViewInfo, cx: &mut Cx<Self>) {
    if let Some(c) = &view.content {
        match c.parse::<Shown>() {
            Ok(shown) => { /* replace what the view shows */ }
            Err(e) => log::warn(e.to_string()),
        }
    }
}
```

The same content id again updates the same view (`on_content` again). Keep
the schema small and describe each field: agents read it.

## Services (no permission needed)

- `storage`: the tile's own key-value store on disk (`get`, `set`,
  `delete`, `list`, `get_json`, `set_json`). What the user must not lose
  goes here.
- `settings`: `text(key)`, `number`, `flag`, `secret(key)`, `open()`.
  `Tile::on_event` gets `Event::SettingsChanged` (by default it renders
  again).
- `ui`: `toast(text, Tone::Good)`, `open_view(tile)`, `scroll_to(view,
  key_path)`.
- `log`: `debug`, `info`, `warn`, `error` (and `println!`), shown in the
  tile's Log and by `pitwall extension dev`.
- Timers: `cx.after(Duration::from_secs(5), |this, cx| …)`,
  `cx.every(..)`, `cx.cancel(timer)`. They pause while the tile is hidden.
- State across a reload: `fn save_state(&self) -> Option<SavedState>` with
  `SavedState::json(&…)`, read back in `new` with `cx.restore::<T>()`.

There is no network, file or agent access in WIT 0.1; don't try to reach
them (no `std::fs`, no sockets): they fail.

## Tests

```rust
use pitwall_extension_test::Harness;

#[test]
fn counts() {
    let mut tile = Harness::build(env!("CARGO_MANIFEST_DIR"))   // builds tile.wasm
        .setting("repo", "octo/app")
        .open("main");                                          // or .open_with("main", json!({..}))
    tile.click_text("Click me");                                 // also click_key("list/row-1"), click_label("Remove")
    assert!(tile.find_text("Clicked 1 times").is_some(), "{}", tile.text_snapshot());
    tile.type_into("filter", "abc").submit("filter");
    assert_eq!(tile.stored_json::<u32>("clicks"), Some(1));
}
```

Each action waits until the tile handled it and rendered. `wait(duration)`
lets timers fire; `wait_for_text(text, timeout)` waits for one.
`text_snapshot()` prints the tree; `log_text()`, `toasts()`, `stored(key)`,
`reload()` (save state, restart), `set_setting`, `set_content`.

## Docs

The design and reference: `docs/spec/tile-sdk/` in Pitwall's repository
(`authoring.md`, `runtime.md`, `ui.md`) and the WIT files in the SDK's
`wit/` folder, every item documented.
