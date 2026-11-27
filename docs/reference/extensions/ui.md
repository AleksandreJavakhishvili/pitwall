# Tiles and UI components

**Preview.** A tile describes what to show as a tree of elements, and Pitwall lays it out and draws it in its own theme. This page covers the core elements of WIT 0.1, styles and theme colours, events and keys, the `Tile` trait, and opening a tile with content. Further components (lists, tables, trees, tabs, menus, Markdown, a canvas, images) come in phase 2.

The names follow GPUI, the UI framework Pitwall is built with, so `div().flex().gap_2().child(…)` reads as it does there. Everything below comes from `use pitwall_extension::prelude::*;`.

## The `Tile` trait

A tile is a type that implements `Tile`. One value of it runs per extension; each place in the layout that shows one of the extension's tiles is a view of it, rendered with its own id.

| Method | Called | Default |
|---|---|---|
| `new(cx)` | Once, before any view opens. Make the state, read `storage`, start timers. Deadline 2 s | Required |
| `render(view, cx)` | After a view opens, after a listener or timer calls `cx.notify()`, and when the tile is shown again. Returns the view's whole tree | Required |
| `on_view_opened(view, cx)` | A view opened; its first render follows | Nothing |
| `on_content(view, cx)` | A view was given content, when it opened or later ([below](#opening-a-tile-with-content)) | Nothing |
| `on_view_closed(view, cx)` | A view closed | Nothing |
| `on_event(event, cx)` | Shown or hidden, focus, a view's size, the theme, settings or grants changed | Renders again on theme, settings and grants changes |
| `save_state()` | Before the tile is reloaded or dropped to save memory. At most 1 MiB, 50 ms | Nothing kept |

End the crate with `pitwall_extension::export!(MyTile);`. The [quick start](quick-start.md#the-tile) shows a whole tile.

**Keep `render` quick.** Aim for under 8 ms; past 250 ms the tile is stopped and the user sees why. Compute in listeners and timers, and only read state in `render`. A listener or event has 1 s.

`cx` (the context, `Cx`) is passed to every call. Besides `listener`, `notify` and the timers ([Platform services](platform-services.md#tasks-timers)), it tells the tile about its views (`cx.views()`, `cx.view(id)`, `cx.content(id)`), the theme (`cx.theme()`), its grants (`cx.is_granted("memory")`), whether it is shown (`cx.is_visible()`), and its own id and version.

## Elements

| Element | Made with | What it is |
|---|---|---|
| Box | `div()`, `v_flex()`, `h_flex()` | A flex container. `div()` and `v_flex()` are columns; `h_flex()` is a row with its children centred across it. The usual root |
| Text | `text("…")` | A paragraph. Its children are inline runs, each with its own style: `text("Built in ").child(text("3 s").font_bold())`. Plain `&str` and `String` children become text |
| Button | `button("Save")`, `icon_button(Icon::X, "Remove")` | Pitwall's own buttons. Looks: `.primary()` (one main action per area), the default hairline, `.danger()`, `.small()`, `.link()`. `.icon(Icon::Plus)` puts an icon before the label; `.disabled(true)` dims it and drops clicks. An icon button's label is its accessible name and tooltip |
| Icon | `icon(Icon::Check)` | One of Pitwall's icons, in the text colour and size: `Refresh`, `Check`, `X`, `Plus`, `Search`, `Gear`, `Folder`, `File`, `Branch`, `Terminal`, `Trash`, `Copy`, `External`, or `Icon::Named("…")` |
| Text field | `input("key")` | A one-line or multi-line field ([below](#text-fields)). Its key is required |
| Scroll | `.overflow_y_scroll()`, `.overflow_x_scroll()`, `.overflow_scroll()` on a box | Its children scroll inside it. Pitwall draws the scrollbar and keeps the position |

Every element can have:

- `.id("key")`: its key among its siblings ([below](#keys)).
- `.tooltip("…")`: text shown when the pointer rests on it.
- `.label("…")`: its accessible name, read by screen readers.
- `.focusable()`: it can take keyboard focus. Buttons and fields always can.
- `.when(condition, |el| …)` and `.when_some(option, |el, value| …)`: apply styles or children only sometimes.

Children are added with `.child(…)` and `.children(iter)`. `.into_any()` turns any element into an `AnyElement`, so different builders fit one type (for a helper function, or the arms of a `match`).

A row of the `list` template: a button that toggles the item, its text, and a button to remove it.

```rust
{{#include crates/pitwall-cli/templates/tile/list/lib.rs.tmpl#list-row}}
```

## Styles

Styles use GPUI's names:

| Kind | Methods |
|---|---|
| Flex | `flex`, `flex_row`, `flex_col`, `flex_wrap`, `flex_1`, `flex_grow`, `flex_shrink_0`, `flex_none`, `gap_0` … `gap_8`, `gap(length)` |
| Alignment | `items_start`, `items_center`, `items_end`, `items_baseline`, `justify_start`, `justify_center`, `justify_end`, `justify_between`, `justify_around`, `self_align` |
| Size | `w`, `h`, `size`, `w_full`, `h_full`, `size_full`, `min_w`, `max_w`, `min_h`, `max_h` |
| Spacing | `p_0` … `p_6`, `px_1` … `px_4`, `py_1` … `py_3`, `pt_*`, `pb_*`, `pl_2`, `pr_2`, `m_1`, `m_2`, `mx_2`, `my_2`, `mt_2`, `mb_2`, `p(length)`, `m(length)` |
| Borders and corners | `border_1`, `border_t_1`, `border_b_1`, `border_l_1`, `border_r_1`, `border_color`, `rounded_none`, `rounded_sm`, `rounded_md`, `rounded_lg`, `rounded_full` |
| Colour | `text_color`, `bg`, `hover_bg`, `opacity` |
| Text | `text_xs`, `text_sm`, `text_base`, `text_lg`, `text_xl`, `font_mono`, `font_ui`, `font_normal`, `font_medium`, `font_semibold`, `font_bold`, `italic`, `underline`, `truncate`, `line_clamp(n)`, `selectable` |
| Other | `overflow_hidden`, `cursor_pointer`, `cursor_text` |

Lengths are `px(n)` (logical pixels), `rems(n)` (multiples of Pitwall's text size, so they follow the user's font size), `relative(f)` (a fraction of the parent) or a bare number (pixels). The spacing scale (`gap_2`, `p_3`) is in rems.

### Theme colours

A tile has no raw colours. `text_color`, `bg`, `hover_bg` and `border_color` take a theme token, named by meaning. Each has a light and a dark value, and the user's theme picks one, so a tile follows Pitwall's appearance by itself.

| Token | For |
|---|---|
| `Bg` | The window's background, behind tiles |
| `Surface`, `Surface2`, `Surface3` | A tile's background, then raised areas inside it (a header, a selected row, a field) |
| `Raised` | Floating surfaces |
| `Line`, `LineStrong` | Hairlines and borders; stronger ones that must be seen |
| `Text`, `Text2`, `Text3`, `Text4` | Main text, secondary, muted, faint |
| `Focus` | The focus ring |
| `Good`, `Warn`, `Bad`, `Done` | Success, needs attention, failure, finished |
| `GoodSoft`, `WarnSoft`, `BadSoft`, `DoneSoft` | Soft backgrounds for the same |
| `DiffAddBg`, `DiffDelBg` | Added and removed lines in a diff |

`cx.theme()` gives the rest of the user's appearance: light or dark, density (comfortable, compact, dense) and whether they asked for less motion. When it changes, the tile hears `Event::Theme` and renders again.

## Events

A listener is made with `cx.listener` while rendering and handed to the element at once. Its closure gets the tile, the event and the context; it calls `cx.notify()` when what the tile shows changed.

| Method | On | The closure gets |
|---|---|---|
| `on_click` | Any element; also Space or Enter while it has focus | `Click`: button, modifier keys, click count, position |
| `on_hover` | Any element | `Hover`: entered or left, position |
| `on_pointer` | Any element | `Pointer`: press, release or move, with position and modifiers. After a press, moves and the release go to the same node |
| `on_change` | Text field | The new text (`&str`) or the whole `InputChange` with the selection |
| `on_submit` | Text field | `Submit`: Enter (⌘Enter, Ctrl+Enter off macOS, in a multi-line field) |
| `on_select` | Text field | `Selection`: the cursor or selection moved |
| `on_scroll` | Scroll container | `ScrollPosition`: offset and content size, for loading more at the end |

Events that aren't for one element (shown or hidden, focus, a view's size, the theme, settings, grants) go to `Tile::on_event`.

## Keys

Give each element that can move a key with `.id("key")`: list rows take their item's id. Pitwall keeps scroll positions, focus and typed text by key path (the keys from the root down, joined by `/`), so they survive when the list around them changes. Children without a key get their position as their key. Keys are unique among siblings (a duplicate gets a suffix and a Log line) and at most 128 bytes.

The `list` template's scrolling list, keyed by `items`, with one keyed row per item:

```rust
{{#include crates/pitwall-cli/templates/tile/list/lib.rs.tmpl#list-scroll}}
```

## Text fields

Pitwall owns what the user types: typing, selection, undo and input methods never wait for the tile. The tile hears each change with `on_change`, keeps its own copy, and passes that copy back with `.value(…)`. The SDK sends the value only when it differs from what the field holds, so this never undoes what the user typed meanwhile, and setting your copy to `""` clears the field.

```rust
{{#include crates/pitwall-cli/templates/tile/list/lib.rs.tmpl#list-submit}}
```

`.placeholder(…)` shows faint text while it is empty. `.multiline()` makes it several lines (Enter adds a line, ⌘Enter submits) and `.rows(n)` its height; `.select(range)` places the cursor when `.value` changes the text; `.disabled(true)` makes it read-only. The `form` template's fields:

```rust
{{#include crates/pitwall-cli/templates/tile/form/lib.rs.tmpl#form-inputs}}
```

## Keyboard and focus

- A tile gets keys only while it has focus. Pitwall's own shortcuts always win.
- Tab and Shift-Tab move focus among focusable elements in tree order, with Pitwall's focus ring.
- While rendering, `cx.bind_key("j", handler)` makes a key call a listener (`Handler<KeyPress>`) while the view has focus. Keys are written like `j`, `enter`, `/`, `shift-tab` or `cmd-r`; `cmd` is ⌘ on macOS and Ctrl elsewhere.
- While rendering, `cx.focus("list/row-3")` asks for focus on the element at that key path. Pitwall honours it only while the tile already has focus, so a tile can't take the keyboard from an agent's terminal.

## Opening a tile with content

A tile can be opened with content: a JSON value that matches a schema the tile names in its manifest entry (`"input": "input.schema.json"`). Pitwall checks the content against the schema before the tile sees it. A later call with the same content id goes to the same view and updates it in place.

The tile hears `on_view_opened` and then `on_content`, or only `on_content` when an open view gets new content. `Content::parse` reads the JSON into the tile's own `serde` type. The `list` template's input type and handler:

```rust
{{#include crates/pitwall-cli/templates/tile/list/lib.rs.tmpl#list-content-type}}
```

```rust
{{#include crates/pitwall-cli/templates/tile/list/lib.rs.tmpl#list-on-content}}
```

`Content` also has `id()` (the caller's id for it), `json_text()` and `bytes()` (a binary payload, at most 8 MiB).

Keep the schema small and describe each field: agents read it. In this preview, content reaches a tile in tests (`open_with`, `set_content`, see the [quick start](quick-start.md#test-it)). Agents opening a tile with content come in phase 2 ([Agent tools](agent-tools.md)).

`ui::open_view("main")` opens one of the extension's own tiles, or focuses it when it is open ([Platform services](platform-services.md#ui)).

## Limits

| | Limit |
|---|---|
| Nodes in a view | 20 000 |
| A frame | About 2 MiB |
| A key | 128 bytes |
| A text field's text | 1 MiB |
| `render` | 250 ms (aim for under 8 ms) |
| A listener or event | 1 s |
| `new` | 2 s |

A tile over a frame limit or a deadline is stopped, and the user sees the reason.
