# pitwall-extension

Write [Pitwall](https://github.com/pitwalls/pitwall) tiles in Rust. **Preview**:
the API follows WIT `pitwall:tile@0.1.0` and may change until 1.0.

A tile is a WebAssembly component that Pitwall runs in a sandbox and draws
with its own UI code, in the user's theme. You describe what to show with
elements that read like GPUI's; Pitwall sends back what the user did.

```rust
use pitwall_extension::prelude::*;

#[derive(Default)]
struct Counter { clicks: u32 }

impl Tile for Counter {
    fn new(_cx: &mut Cx<Self>) -> Self {
        Counter { clicks: storage::get_json("clicks").ok().flatten().unwrap_or(0) }
    }

    fn render(&mut self, _view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        v_flex().gap_2().p_3()
            .child(text(format!("Clicked {} times", self.clicks)).text_color(Token::Text2))
            .child(button("Click me").primary().on_click(cx.listener(|this, _, cx| {
                this.clicks += 1;
                let _ = storage::set_json("clicks", &this.clicks);
                cx.notify();
            })))
    }
}

pitwall_extension::export!(Counter);
```

## Start

```sh
rustup target add wasm32-wasip2
pitwall extension new my-tile --template blank   # or list, form
cd my-tile
pitwall extension dev                            # build, check, load into Pitwall, rebuild on save
cargo test                                       # runs the built tile in Pitwall's runtime
```

`pitwall extension new` writes a Cargo project (`crate-type = ["cdylib"]`),
its `extension.json`, tests that use `pitwall-extension-test`, and an
authoring skill for your coding agent.

## What's in it

- **Elements**: `div`, `h_flex`, `v_flex`, `text` (with inline runs),
  `button`, `icon_button`, `icon`, `input`; scroll containers with
  `.overflow_y_scroll()`. Styles by GPUI's names (`flex`, `gap_2`, `p_3`,
  `size_full`, `items_center`, `border_b_1`, `text_sm`, `font_mono`), colours
  only as Pitwall's theme tokens (`Token::Text2`, `Token::Surface`, …).
- **The `Tile` trait**: `new`, `render(view)`, and optional `on_view_opened`,
  `on_content`, `on_view_closed`, `on_event`, `save_state`.
- **Listeners**: `cx.listener(|this, event, cx| …)`, typed by the element
  method (`on_click` → `Click`, `on_change` → `&str`, `on_submit` → `Submit`).
- **Content**: a tile's manifest entry names a JSON schema (`"input"`); a view
  opened with content calls `on_content`, and `Content::parse::<MyInput>()`
  reads it with serde.
- **Services**: `log`, `storage` (10 MiB key-value, with JSON helpers),
  `settings` (and secrets), `ui` (toast, open a view, scroll to a node), and
  timers (`cx.after`, `cx.every`).

The design is in `docs/spec/tile-sdk/` in Pitwall's repository
(authoring.md §4). Licensed Apache-2.0.
