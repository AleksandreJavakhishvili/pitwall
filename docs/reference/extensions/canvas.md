# Canvas and drag

**Preview.** A canvas is an element the tile draws on: rectangles, paths and text in theme colours, with zoom and pan, and clickable regions laid over what it drew. It is for diagrams, charts and maps, where boxes and lines don't follow a flex layout. This page also covers dragging things from one element of a tile to another.

Everything below comes from `use pitwall_extension::prelude::*;`.

## Drawing

`canvas()` is an element like `div()`: give it a key, a size and styles (`.id("diagram").flex_1()`). Then add what it draws, in order, each over the ones before:

```rust
canvas()
    .id("diagram")
    .size_full()
    .draw(Shape::rect(0., 0., 168., 72.).fill(Token::Surface2).stroke(Token::Line, 1.).radius(8.))
    .draw(Label::new("API", 84., 36.).size(14.).semibold().center().middle())
    .draw(Shape::line(168., 36., 248., 36.).stroke(Token::Text3, 1.5).dash([5., 4.]))
```

| Made with | What it draws |
|---|---|
| `Shape::rect(x, y, w, h)` | A rectangle; `.radius(r)` rounds its corners |
| `Shape::line(x1, y1, x2, y2)` | A straight line |
| `Shape::path()` then `.move_to`, `.line_to`, `.quad_to`, `.cubic_to`, `.close()` | Any outline: arrowheads, curved links, chart lines |
| `Label::new(text, x, y)` | One line of text ([below](#text)) |

A shape is filled with `.fill(colour)`, outlined with `.stroke(colour, width)`, or both; `.dash([on, off, …])` dashes the outline (at most 16 numbers). Colours are theme colours (`Token::Text3`, `Token::Focus`, [the list](ui.md#theme-colours)), so the drawing follows the user's light or dark theme like the rest of the tile. For data colours a theme doesn't have, `Paint::rgb(0x3b82f6)` gives a fixed colour; prefer tokens.

`.clip_push(x, y, w, h)` and `.clip_pop()` limit what is drawn between them to a rectangle. Whatever is drawn, nothing leaves the canvas's own bounds.

Positions are in canvas units: pixels at zoom 1, from the canvas's top-left corner.

### Text

A label is drawn at its point with 13 px Inter in the text colour unless told otherwise:

- `.size(px)`: any size, not only the kit's text sizes, so text zooms with the drawing.
- `.color(token)`, `.medium()`, `.semibold()`, `.bold()`, `.mono()` (JetBrains Mono).
- `.center()` / `.end()` place the point at the middle or the end of the text; `.middle()` / `.bottom()` place it at the text's middle or bottom (the default is its top).
- `.max_width(w)` cuts longer text with "…".

Text smaller than 2 px on screen isn't drawn.

**Sizing boxes to their text.** `ui::measure_text(&texts, TextFont::Ui, 14.0, TextWeight::Semibold)` asks Pitwall how wide each text is in its fonts (Inter, or JetBrains Mono for `TextFont::Mono`) at that size and weight, kerned as Pitwall draws it, in canvas units at zoom 1; `label.width()` measures one label as it is set up. Measure all a drawing's labels in one call (at most 1 000 texts and 64 KiB): each call counts toward a tile's host calls. A width scales with the size, and a character Pitwall's fonts lack counts as an average letter, so still give a label a `.max_width` as a safety margin: if the measure is short by a few pixels, the text ends in "…" instead of running over the box. The test harness measures the same way, so tests see the same widths (`pitwall_extension_test::text_width`). The C4 tile keeps its own table of Inter's widths instead, because its layout runs without the SDK; a test checks that the table and Pitwall's measure agree.

**A box's menu.** A popover can hang from a point of the canvas: `canvas.popover(popover().at(x, y).child(…))`, with (x, y) in the canvas node's pixels, from its top-left corner (map a box's corner with `viewport.to_view(x, y)`). The C4 tile opens its "which diagram?" menu this way, under the box that was clicked ([Popover](components-layout.md#popover)).

## Zoom and pan

A canvas has one transform, set with `.transform(viewport)`: everything drawn and every region is scaled and moved by it. `Viewport` keeps the zoom and position in the tile's state, and turns the user's pointer, wheel and pinch into zoom and pan:

```rust
// In the tile's state: viewport: Viewport,
canvas()
    .transform(self.viewport)
    .on_pointer(cx.listener(|t: &mut MyTile, p: &Pointer, cx| {
        if t.viewport.on_pointer(p) {
            cx.notify();
        }
    }))
    .on_wheel(cx.listener(|t: &mut MyTile, p: &Pointer, cx| {
        if t.viewport.on_pointer(p) {
            cx.notify();
        }
    }))
```

`on_pointer` pans while the user drags with the main button. The wheel or trackpad pans, ⌘-wheel (Ctrl elsewhere) zooms at the pointer, and a pinch zooms. It returns whether anything changed.

| `Viewport` | |
|---|---|
| `fit(x, y, w, h, view_w, view_h, margin)` | Show the canvas rectangle (x, y, w, h) whole in a view of that size, centred |
| `zoom_at(factor, x, y)` | Zoom by a factor, keeping the view point (x, y) still: for zoom buttons |
| `pan(dx, dy)` | Move by view pixels |
| `to_view(x, y)`, `to_canvas(x, y)` | Convert between canvas units and view pixels |
| `min_scale`, `max_scale` | Bounds for the zoom (0.1 and 8 unless set) |

A tile hears its size in `Tile::on_event` (`Event::Resize`), which is what `fit` needs. `on_wheel` is for any element, not only canvases: its `Pointer` is `PointerKind::Wheel { dx, dy }` (pixels, positive when scrolling down) or `PointerKind::Pinch(delta)` (0.1 is 10 % larger).

## Clickable regions

What a canvas draws doesn't take clicks by itself. Lay regions over it, one per thing the user can click or point at:

```rust
.region(
    Region::new("api", 248., 0., 168., 72., "API")
        .tooltip("API: Rust, axum")
        .on_click(cx.listener(|t: &mut MyTile, _: &Click, cx| { /* … */ }))
        .on_hover(cx.listener(|t: &mut MyTile, h: &Hover, cx| { /* … */ })),
)
```

A region has a key (unique in its canvas), a rectangle in canvas units (it follows the zoom and pan) and a label. The label is its accessible name: screen readers read it, and tests find regions by it. Later regions are over earlier ones. A press that moves more than a few pixels is a pan, not a click.

Give the canvas itself a `.label("…")` saying what the drawing shows.

## Drag and drop

Inside a tile, an element can be dragged onto another: a card onto a column, a name onto a list.

```rust
text(item.name.clone()).id(item.id.clone()).draggable("item", item.id.clone(), item.name.clone())

v_flex().id("done").on_drop(["item"], cx.listener(|t: &mut MyTile, d: &Dropped, cx| {
    t.move_to_done(&d.data);
    cx.notify();
}))
```

`.draggable(kind, data, label)` makes an element a drag source: `kind` says what it is, `data` is the tile's own string for it (an id, at most 64 KiB) and `label` is what the chip under the pointer says. `.on_drop(kinds, handler)` makes an element take drops of those kinds; while something it takes is over it, Pitwall highlights it. The handler gets `Dropped`: the kind, the data and where in the element it was dropped.

Drags stay inside one view of one tile: a drop target never takes something dragged from another tile or from outside Pitwall.

## Testing

The harness ([quick start](quick-start.md#test-it)) shows canvases in the text snapshot with their regions, drag sources (`drag=item`) and drop targets (`drop=item`), and acts on them as the user would:

| Call | Does |
|---|---|
| `t.canvas("diagram")` | What the canvas draws, its transform and regions, to assert on |
| `t.click_region("API")`, `t.hover_region("API", true)` | Click or point at the region with that label |
| `t.right_click_region("API")` | Right-click it (a box's menu) |
| `t.wheel(key, dx, dy, x, y)`, `t.pinch(key, delta, x, y)` | Scroll or pinch over an element |
| `t.drag_pointer(key, from, to)` | Press, move and let go: a pan |
| `t.drag(from_key, to_key)`, `t.try_drag(…)` | Drag one element onto another; `try_drag` says why nothing was dropped |

## Limits

| | Limit |
|---|---|
| Drawing operations in a canvas | 20 000 |
| Regions in a canvas | 5 000 |
| A dash pattern | 16 numbers |
| A drag's data | 64 KiB |

They count toward the frame's 2 MiB ([Tiles and UI components](ui.md#limits)).
