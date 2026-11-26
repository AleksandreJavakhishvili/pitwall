# Tile SDK: the UI description

Part of the [Tile SDK and marketplace](README.md) design. The element tree a tile sends, how the host draws it with GPUI, events, keyboard, drag and drop, canvas. Sources (S1…) and *unverified* claims are listed in [sources.md](sources.md).

### 3.3 The UI description

```wit
interface types {
  type view-id = string;
  type handler = u32;          // a guest closure, by number

  enum token {                 // Pitwall's theme (theme.rs), by meaning
    bg, surface, surface-2, surface-3, raised, line, line-strong,
    text, text-2, text-3, text-4, focus,
    good, good-soft, warn, warn-soft, bad, bad-soft, done, done-soft,
    diff-add-bg, diff-del-bg,
  }
  variant color { token(token), rgba(u32) }   // rgba: canvas only
  variant length { px(f32), rem(f32), fraction(f32), auto }
  enum text-size { xs, sm, md, lg, xl }       // Pitwall's scale
  enum font { ui, mono }

  variant style {               // a list of these per node
    flex-row, flex-col, wrap, grow(f32), shrink(f32),
    gap(length), padding(edges), margin(edges),
    width(length), height(length), min-width(length), max-width(length),
    min-height(length), max-height(length),
    align-items(align), justify(justify), self-align(align),
    overflow-x(overflow), overflow-y(overflow),
    border(edges), border-color(token), radius(radius),
    background(token), color(token), opacity(f32),
    text-size(text-size), font(font), weight(weight),
    italic, underline, truncate, line-clamp(u8), selectable,
    hover-background(token), cursor(cursor),
  }

  variant element {
    box,                                  // a div
    text(string),                         // inline runs: children of kind text
    icon(icon),                           // Pitwall's icon set or a tile asset (svg, tinted)
    image(image),                         // a tile asset or bytes from a task
    button(button), icon-button(icon-button),
    input(input), checkbox(checked), toggle(checked), radio(checked),
    select(select), segmented(segmented), tabs(tabs),
    chip(chip), status(status), spinner, divider, split(split),
    virtual-list(virtual-list), table(table), tree(tree),
    code(code), markdown(string), canvas(canvas),
    scroll(scroll),
    block(u32),                           // a native building block by handle (16.2)
  }

  record node {
    parent: option<u32>,        // index into frame.nodes; none = root
    key: string,                // stable among siblings (diffing, focus, scroll)
    element: element,
    style: list<style>,
    on: list<listener>,         // events this node wants, with their handler
    focusable: bool,
    tooltip: option<string>,
    label: option<string>,      // accessible name; required for icon-only buttons
    drag: option<drag-source>,
    drop: option<drop-target>,
    origin: option<u64>,        // the tool call that put this content here;
                                // the host draws "from <agent>" and Undo (11.6)
  }

  record frame {
    nodes: list<node>,
    focus: option<string>,      // key path the tile wants focused, honoured only
                                // after a user action in the tile (3.5)
    keymap: list<binding>,      // keys the tile handles while focused (3.5)
  }
}
```

How this maps onto GPUI (vendored at the commit named in
`third_party/zed/README.md`):

| Tile element | Host draws with |
|---|---|
| `box` + styles | `div()` with the matching `Styled` calls (Taffy flex layout) |
| `text` | GPUI text (`StyledText` with runs for inline children), Pitwall's fonts from `kit::fonts` |
| `scroll` | `div().overflow_y_scroll()` with a `ScrollHandle` kept by key; `kit::scrollbar` |
| `virtual-list` | `uniform_list` (fixed row height) or `list` (variable, measured) |
| `table` | a header row + `uniform_list` rows; column widths resizable with `kit::divider` |
| `tree` | `uniform_list` over the flattened open nodes |
| `input` | `kit::input::TextInput` (IME through `EntityInputHandler`) |
| `button`, `checkbox`, `toggle`, `select`, `segmented`, `chip`, `status`, `spinner`, `tabs` | `kit::button`, `kit::controls`, `kit::chip`, `kit::status` |
| `code` | the read-only code view (`code_view`) with Pitwall's highlighting by language name |
| `markdown` | `markdown_view` (inert: no HTML, no remote images) [today] |
| `canvas` | GPUI `canvas()`; draw ops → `paint_quad`, `paint_path`, `paint_svg`, `paint_image`, shaped text |
| `image` | `img()` with an `ImageSource` from decoded bytes |
| tooltips, menus | `kit::tooltip`, `kit::menu` |
| `split` | `kit::divider` |
| glass | none: a tile's body is content, so it stays solid ([layout.md](../layout.md), "Only chrome is glass") |

Prior art: Zed's extension API has no UI at all (S18); Zellij's wasm
plugins draw by printing text for their pane (S20); Figma runs plugin
logic in a minimal sandbox and its UI in a separate iframe, joined only
by messages (S20); Raycast built native views from a serialised React
tree (S13). Pitwall's design is closest to Raycast's, with wasm in
place of Node and Pitwall's kit in place of AppKit.

What the tile can't do: set fonts other than Pitwall's UI and mono
faces, set colours outside a canvas, draw outside its bounds, open
overlays outside its tile (menus, tooltips and popovers are anchored
inside it by the host), or animate the layout. Hover, press, focus and
open/close motion are the host's (and follow Reduce motion).

### 3.4 Events

```wit
variant event {
  click(click),                 // handler, button, modifiers, count
  input-changed(input-change),  // handler, value, revision
  submit(handler),              // Enter / ⌘⏎ in an input
  key(key-event),               // a binding from the keymap fired
  select(selection),            // list/table/tree/tabs/select: key or index
  toggle-row(tree-toggle),
  scroll(scroll-pos),           // only where the node asked (infinite load)
  hover(hover),                 // only where asked; coalesced
  drop(dropped),                // text, a project file path (files.read), an agent id (agents.read)
  context-menu(menu-pick),
  resize(size), visibility(bool), focus(bool),
  theme(theme-info),            // mode, look, density, reduce-motion
  timer(u32), task-done(task-result),
  settings-changed, grants-changed(list<string>),
  agents-changed,               // with agents.read; the snapshot is read via `pitwall`
  undo-present(u64),            // the user pressed Undo on an agent's change (11.6)
  tool-cancelled(u64),          // the call timed out or the agent went away
}
```

Handlers are numbers. The SDK keeps the closures guest-side in a table
rebuilt on each render and sends only the numbers. A handler from an
older frame that no longer exists is dropped by the SDK.

### 3.5 Keyboard and focus

- A tile gets keys only while it has focus, as every tile [today].
  Pitwall's own bindings always win: the palette, ⌘⏎, ⌘W, space
  switching, ⌘1–9, Esc to leave a maximised tile. The reserved set is
  listed in the SDK docs and checked by `pitwall tile check`.
- A tile declares its keys in `keymap` (for example `j`, `k`, `enter`,
  `cmd-r`, `/`). The host turns them into a GPUI key context named
  `tile:<id>` with those bindings; a fired binding becomes a `key`
  event. No global or default bindings outside the tile (decision 8 in
  extension-api.md [today]).
- Focus moves with Tab / Shift-Tab among `focusable` nodes in tree
  order, drawn with Pitwall's focus ring. A tile can ask for focus by
  key in its frame; the host honours it only while the tile already
  has focus (so a tile can't steal the keyboard from an agent's
  terminal).

### 3.6 Drag and drop

Inside a tile: a node with `drag` can be dropped on a node with `drop`
in the same tile; the host draws the drag preview. Between tiles:
Pitwall's own drags [today: files, agents] can be dropped on a tile
that accepts them, giving it a project path (needs `files.read` on
that project) or an agent id (needs `agents.read`). A tile can drag
out only text. Dragging a tile itself is the host's (`TileDrag`
[today]).

### 3.7 Canvas

`draw` returns a list of ops in the canvas's own coordinates: `rect`
(fill, border, radius), `path` (move/line/quad/cubic, fill or stroke),
`text` (string, size token, colour, alignment; shaped by the host),
`icon`, `image` (asset or task bytes), `clip-push`/`clip-pop`. At most
20 000 ops a draw. Colours may be `rgba` here, or tokens (recommended,
so a chart follows the theme). A canvas is redrawn on resize, theme
change, or when the guest asks with `ui.redraw(canvas)`, at most once
per frame. A tile that wants motion (a live graph) calls
`ui.redraw` each frame; that counts against the frame budget and stops
while hidden or with Reduce motion on, where it gets one draw per
second.
