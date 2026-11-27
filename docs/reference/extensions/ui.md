# Tiles and UI components

**Preview.** A tile describes what to show as a tree of elements, and Pitwall lays it out and draws it in its own theme. This page covers the elements and components of WIT 0.1 (long lists, tables, trees, tabs, selects, menus, Markdown and images, all drawn by Pitwall), styles and theme colours, events and keys, the `Tile` trait, and opening a tile with content. A canvas with drag and drop is on [Canvas and drag](canvas.md), and Pitwall's code and diff views on [Native building blocks](blocks.md).

The names follow GPUI, the UI framework Pitwall is built with, so `div().flex().gap_2().child(…)` reads as it does there. Everything below comes from `use pitwall_extension::prelude::*;`.

## A tile in Pitwall

Each tile an extension that is on contributes is in the command palette as "Open <tile title> (<extension name>)". It opens as a tile in the layout, beside your terminals, files and web pages. The extension's component starts when one of its tiles is shown, and stops when its last tile closes or the extension is turned off. While none of its tiles is shown (another space, another tile maximised over it), it is suspended: no renders, no timers.

While it starts, the tile says so (the first time, Pitwall prepares each version for this computer, which takes a moment). When it stops, over a deadline, after a crash, or with a frame over the limits, it says why in its place, with Restart.

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
| Button | `button("Save")`, `icon_button(Icon::X, "Remove")` | Pitwall's own buttons. Looks: `.primary()` (one main action per area), the default hairline, `.danger()`, `.link()`. Size: regular, or `.small()`, with any look (`.small().danger()`). `.icon(Icon::Plus)` puts an icon before the label; `.disabled(true)` dims it and drops clicks. An icon button is square; its label is its accessible name and tooltip |
| Icon | `icon(Icon::Check)` | One of Pitwall's icons ([the list](#icons)), in the text colour and size |
| Text field | `input("key")` | A one-line or multi-line field ([below](#text-fields)). Its key is required |
| Scroll | `.overflow_y_scroll()`, `.overflow_x_scroll()`, `.overflow_scroll()` on a box | Its children scroll inside it. Pitwall draws the scrollbar and keeps the position |
| Components | `virtual_list`, `table`, `tree`, `tabs`, `select`, `menu_button`, `.context_menu(…)`, `markdown`, `image_asset`, `image_data` | Pitwall's own lists, menus, Markdown and images ([below](#components)) |

Every element can have:

- `.id("key")`: its key among its siblings ([below](#keys)).
- `.tooltip("…")`: text shown when the pointer rests on it.
- `.label("…")`: its accessible name, read by screen readers.
- `.focusable()`: it can take keyboard focus. Buttons and fields always can.
- `.when(condition, |el| …)` and `.when_some(option, |el, value| …)`: apply styles or children only sometimes.

`Div` (what `div()`, `v_flex()` and `h_flex()` return), `Button`, `Input`, `Text` and `IconElement` are in the prelude, for helpers that build part of a tree.

Children are added with `.child(…)` and `.children(iter)`. `.into_any()` turns any element into an `AnyElement`, so different builders fit one type (for a helper function, or the arms of a `match`).

A row of the `list` template: a button that toggles the item, its text, and a button to remove it.

```rust
{{#include crates/pitwall-cli/templates/tile/list/lib.rs.tmpl#list-row}}
```

### Icons

`Icon` has one variant per icon Pitwall draws, and `Icon::from_name` and `.name()` go between them and their names, which are also what `extension.json` names an icon by. An unknown `Icon::Named("…")` draws a dot (`pitwall-extension-test` warns of it), and `pitwall extension check` warns of an unknown icon in the manifest. The names:

`sidebar`, `search`, `gear`, `plus`, `x`, `send`, `stop`, `restart`, `refresh`, `trash`, `branch`, `folder`, `chevron`, `grid`, `wall`, `maximize`, `restore`, `window`, `more`, `review`, `terminal`, `file`, `expand`, `copy`, `chevron-down`, `check`, `headset`, `user`, `laptop`, `server`, `eye`, `external`, `image`, `browser`, `back`, `phone`, `inspect`, `page-right`, `page-left`, `page-below`, `pin`, `grip`, `minus`, `chevron-up`, `lock`, `alert`, `extension`, `inbox`, `bell`, `chat`, `list`.

## Components

Components are drawn by Pitwall as its own lists, menus and Preview are, so they look and behave like the rest of the app (theme, hover, keyboard, scrollbars), and work without a round trip to the tile while the user scrolls or hovers. The examples below come from the gallery tile Pitwall's tests run (`crates/pitwall-tile-host/tests/fixtures/hello-tile/src/gallery.rs`); `pitwall-extension-test` drives each one ([quick start](quick-start.md#test-it)).

### Virtual list

`virtual_list(key, count, rows)` is a list of `count` rows that scrolls smoothly at 100 000 rows. The frame holds only the count; `rows`, made with `cx.rows(|tile, i, cx| …)`, renders row `i` when it comes into sight. Pitwall asks for rows a page (64) at a time and keeps what it has drawn until the list's data changes.

```rust
{{#include crates/pitwall-tile-host/tests/fixtures/hello-tile/src/gallery.rs#gallery-virtual-list}}
```

- **Rows.** A row is a small tree (at most 64 nodes) of boxes, text, icons, buttons, images and a context menu; no text fields or nested lists. Every row is `.row_height(px)` tall (default 28, Pitwall's own rows'; 12 to 200). Give each row its item's id with `.id(…)`, so the selection follows the item; a key has no `/`. Listeners in a row (a button's `on_click`, a context menu) work like any other.
- **Selection.** `.on_select` hears a click, Up, Down, Home, End, Page Up and Page Down (with the row's key, or the whole `Picked` with its index and modifiers), and a double click or Enter with `activate`. `.selected(keys)` draws the selection; the tile keeps it.
- **Changes.** By default every render counts as new data, and Pitwall asks for the rows in sight again. `.revision(n)` with a number that changes only when the rows do saves those calls.
- **Size.** It fills its parent: give the parent a height, or let it grow (`flex_1` in a column that has one).
- **Speed.** Keep the row callback quick: a page of 64 rows in a few milliseconds. In Pitwall's measurement a page of 64 gallery rows took about 0.5 ms, and a jump to row 50 000 was drawn in about 20 ms ([runtime.md §2.7](../../spec/tile-sdk/runtime.md)).

### Table

`table(key, columns, count, rows)` is a virtual list with a header. Each row's root holds its cells, one child per column in order. `column(key, title)` takes `.width(px)` (else it shares the rest), `.align_end()` or `.align_center()`, and `.sortable()`.

```rust
{{#include crates/pitwall-tile-host/tests/fixtures/hello-tile/src/gallery.rs#gallery-table}}
```

A click on a sortable title calls `.on_sort` with the column and the next direction (ascending first); the tile sorts and passes `.sort(column, direction)`, which draws the arrow. The user drags a title's right edge to resize the column; `.on_column_resize` hears the width at the end of the drag, to keep it. Selection works as in a list.

### Tree

`tree(key, count, rows)` is a virtual list of the tree's visible rows, in order: the tile flattens its open branches. A row is a `tree_row(element)` with `.depth(n)` (its level, indented by Pitwall) and, for a branch, `.branch(open)` (drawn with a chevron).

```rust
{{#include crates/pitwall-tile-host/tests/fixtures/hello-tile/src/gallery.rs#gallery-tree}}
```

The chevron, a double click on a branch, and → and ← on the selected row call `.on_toggle` with the row's key and whether it opens. ← on a leaf or a closed branch moves to its parent. The tile opens or closes the branch and renders again.

### Tabs

`tabs(key, [tab(key, label), …])` is Pitwall's segmented control: a row of tabs, each with an optional `.icon(…)`. `.selected(key)` marks the current one; `.on_select` hears a click with the tab's key. Draw the tab's content yourself.

```rust
{{#include crates/pitwall-tile-host/tests/fixtures/hello-tile/src/gallery.rs#gallery-tabs}}
```

### Select

`select(key, [choice(key, label), …])` is a dropdown. `.selected(key)`, `.placeholder("…")` (shown while none is selected), `.disabled(true)`; `.on_select` hears the chosen option's key.

```rust
{{#include crates/pitwall-tile-host/tests/fixtures/hello-tile/src/gallery.rs#gallery-select}}
```

### Menus

A menu is a list of `menu_item(key, label)` (with `.icon(…)`, `.hint("⌘C")`, `.disabled(true)`, `.danger()`) and `separator()`. Pitwall draws it as its own menus, above the tile (a glass pop-up under Liquid Glass), closes it on a click outside or Esc, and calls the listener with the chosen item's key.

- `menu_button(label, entries)` is a button that opens it; `menu_icon_button(icon, label, entries)` an icon-only one, `label` being its accessible name.
- `.context_menu(entries, listener)` on any element opens it on a right-click there, rows of lists included (a right-click selects the row first).

```rust
{{#include crates/pitwall-tile-host/tests/fixtures/hello-tile/src/gallery.rs#gallery-menu-button}}
```

### Markdown

`markdown(text)` draws Markdown with Pitwall's Preview: headings, lists, tasks, code, tables, quotes. It is inert: no HTML, no remote images, nothing runs. Without listeners, `http`, `https` and `mailto` links open in the browser and task boxes can't be clicked. `.on_link` gets every link click instead (`LinkClick` with the target as written, so a relative link can open another of the tile's items); `.on_toggle` makes task boxes clickable and gets `Toggled` with the task's line (0-based) and whether it is ticked now, for the tile to change its text.

```rust
{{#include crates/pitwall-tile-host/tests/fixtures/hello-tile/src/gallery.rs#gallery-markdown}}
```

Given a height (or room to grow), it fills it and scrolls; otherwise it is as tall as its text. At most 1 MiB of text.

### Images

`image_asset("assets/logo.svg")` shows a file of the extension's folder (a path inside it, no `..`); `image_data(name)` shows bytes the tile handed over with `ui::put_image(name, &bytes)` ([Platform services](platform-services.md#ui)). PNG, JPEG, GIF, WebP and SVG; never fetched from the network. `.contain()` (the default), `.cover()` or `.fill()` say how it fits its box; give it a size with `w` and `h`.

```rust
{{#include crates/pitwall-tile-host/tests/fixtures/hello-tile/src/gallery.rs#gallery-image}}
```

An asset is at most 16 MiB; a put image at most 8 MiB, and 64 MiB for all of a tile's. A missing or unreadable image draws an image icon in its place.

## Styles

Styles use GPUI's names:

| Kind | Methods |
|---|---|
| Flex | `flex`, `flex_row`, `flex_col`, `flex_wrap`, `flex_1`, `flex_grow`, `flex_shrink_0`, `flex_none`, `gap_0` … `gap_8`, `gap(length)` |
| Alignment | `items_start`, `items_center`, `items_end`, `items_baseline`, `justify_start`, `justify_center`, `justify_end`, `justify_between`, `justify_around`, `self_align` |
| Size | `w`, `h`, `size`, `w_full`, `h_full`, `size_full`, `min_w`, `max_w`, `min_h`, `max_h` |
| Spacing | `p_0` … `p_6`, `px_1` … `px_4`, `py_1` … `py_3`, `pt_*`, `pb_*`, `pl_2`, `pl_3`, `pr_1`, `pr_2`, `m_1`, `m_2`, `mx_2`, `my_2`, `mt_2`, `mb_2`, `p(length)`, `m(length)` |
| Borders and corners | `border_1`, `border_t_1`, `border_b_1`, `border_l_1`, `border_r_1`, `border_color`, `rounded_none`, `rounded_sm`, `rounded_md`, `rounded_lg`, `rounded_full` |
| Colour | `text_color`, `bg`, `hover_bg`, `opacity` |
| Text | `text_xs`, `text_sm`, `text_base`, `text_lg`, `text_xl`, `font_mono`, `font_ui`, `font_normal`, `font_medium`, `font_semibold`, `font_bold`, `italic`, `underline`, `truncate`, `line_clamp(n)`, `selectable` |
| Hover groups | `group("row")` names a hover group; `visible_on_group_hover("row")` on an element inside it keeps the element hidden (its space kept) until the pointer is over the group, drawn by Pitwall without a round trip: a row's Delete |
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
| `on_wheel` | Any element | `Pointer` with `PointerKind::Wheel` or `PointerKind::Pinch`: for zooming and panning a [canvas](canvas.md#zoom-and-pan) |
| `on_drop` | Any element | `Dropped`: something dragged with `.draggable` was dropped on it ([Drag and drop](canvas.md#drag-and-drop)) |
| `on_scroll` | Scroll container | `ScrollPosition`: offset and content size, for loading more at the end |
| `on_focus`, `on_blur` | Text field or focusable element | `Focused` / `Blur`: it took the keyboard, or lost it (to another element, or out of the tile). A field's blur is the moment to save it |
| `on_select` | List, table, tree, tabs, select, menu button, `context_menu` | The chosen row's, tab's, option's or item's key (`&str`), or the whole `Picked` (key, index, modifiers, `activate`) |
| `on_toggle` | Tree, Markdown | `Toggled`: a branch opened or closed, or a task's box clicked (its line) |
| `on_link` | Markdown | `LinkClick`: a link's target |
| `on_sort`, `on_column_resize` | Table | `SortChange` (column, direction), `ColumnWidth` (column, width) |

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

`.placeholder(…)` shows faint text while it is empty. `.multiline()` makes it several lines (Enter adds a line, ⌘Enter submits) and `.rows(n)` its height, or, given a height or room to grow (`.size_full()`, `.flex_1()`), it fills that, at least its rows; `.select(range)` places the cursor when `.value` changes the text; `.disabled(true)` makes it read-only. The `form` template's fields:

```rust
{{#include crates/pitwall-cli/templates/tile/form/lib.rs.tmpl#form-inputs}}
```

## Keyboard and focus

- A tile gets keys only while it has focus. Pitwall's own shortcuts always win.
- Tab and Shift-Tab move focus among focusable elements in tree order, with Pitwall's focus ring.
- While rendering, `cx.bind_key("j", handler)` makes a key call a listener (`Handler<KeyPress>`) while the view has focus. Keys are written like `j`, `enter`, `/`, `shift-tab` or `cmd-s`; `cmd` is ⌘ on macOS and Ctrl elsewhere.
- A binding without ⌘, Ctrl or Alt (`j`, `/`, `shift-n`) doesn't fire while a text field has focus: the key types into the field.
- Pitwall's own keys are never a tile's: a binding of one is dropped, with a line in the tile's Log. They are Esc, Tab, Shift-Tab, Ctrl-Enter (and Ctrl-Shift-Enter), `cmd-` and `cmd-shift-` with any of `enter`, `0`–`9`, `=`, `+`, `-`, `,`, `a`, `b`, `c`, `e`, `h`, `i`, `j`, `k`, `l`, `m`, `n`, `p`, `q`, `r`, `t`, `v`, `w`, `x`, `z`, and `cmd-shift-f`, `cmd-alt-h`, `cmd-ctrl-f`, `cmd-shift-alt-e`, `-n`, `-r` and `-t`. (Off macOS Pitwall's keys are Ctrl+Shift ones; the list is the same everywhere, so a tile's keys work the same on every system.)
- While rendering, `cx.focus("list/row-3")` asks for focus on the element at that key path: the keys from the root down, joined by `/`, without the root's (the same path `ui::scroll_to` takes). Pitwall honours it only while the tile already has focus, so a tile can't take the keyboard from an agent's terminal. `cx.focused(view)` is the element that has it.

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

Keep the schema small and describe each field: agents read it. In this preview, opening a tile from the palette gives it no content, so content reaches a tile in tests (`open_with`, `set_content`, see the [quick start](quick-start.md#test-it)). Agents opening a tile with content come in phase 2 ([Agent tools](agent-tools.md)).

`ui::open_view("main")` opens one of the extension's own tiles, or focuses it when it is open ([Platform services](platform-services.md#ui)).

## Limits

| | Limit |
|---|---|
| Nodes in a view | 20 000 |
| A frame | About 2 MiB |
| A key | 128 bytes |
| A text field's text | 1 MiB |
| A list's, table's or tree's rows | 1 000 000; a row 64 nodes, 12–200 px tall |
| A table's columns; tabs, options or menu items | 64; 1 000 |
| Markdown | 1 MiB |
| An image | 16 MiB from the folder, 8 MiB put (64 MiB for all) |
| `render` | 250 ms (aim for under 8 ms) |
| A listener or event | 1 s |
| `new` | 2 s |

A tile over a frame limit or a deadline is stopped, and the user sees the reason.
