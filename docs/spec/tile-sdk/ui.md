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

WIT 0.1 (`crates/pitwall-tile-host/wit/`) is the first published cut of
this: elements `box`, `text`, `button`, `icon`, `scroll` and `input`; the
style list above; listeners click, change, select, submit, scroll,
hover and pointer; no drag, drop or `origin` yet (records gain fields in
a later 0.x world). Pointer events (`click`, `hover`, `pointer`) carry the
position in the node, in logical pixels, and the modifiers;
`pointer-kind` is a variant (`down`, `up`, `move`) so wheel and pinch
can be added as cases later without breaking tiles. Views get
`view-opened`, `view-content` and `view-closed` events with optional
content (JSON text or bytes and a content id). Its `input` is host-owned: a frame names the text only to
replace it, and `input-changed` carries the selection
([runtime.md 2.4](runtime.md#24-threading-with-gpui)).

Phase 2 adds the components to WIT 0.1 in place (new cases at the end
of `element`, `listen` and `event`, new records), and how they came out
differs from the sketch above in these ways:

- **Lists.** `virtual-list`, `table` and `tree` all have a fixed row
  height (`uniform_list`); a variable-height `list` isn't in 0.1. A
  frame names each list's count, row height, revision and selected keys;
  the host asks `guest.rows(view, key-path, start, count)` (≤ 256 a call,
  64 a page, a page either side of what is in sight) and keeps up to
  2 048 rows per list by index, re-asking when the frame's revision moves
  (the SDK's default revision is its render count, so data changes need
  no extra call; `.revision(n)` saves calls). Rows are small trees (≤ 64
  nodes) of `box`, `text`, `icon`, `button`, `image` and `context-menu`
  only; a table row's root holds one child per column. A tree's rows
  carry `depth` and `expanded`; the tile flattens its open branches.
  Selection is by row key and kept by the tile (`selection` event with
  `activate` for a double click or Enter).
- **Tabs** are drawn as `kit::seg`, the segmented control Pitwall uses
  for its own view switches. Batch 2's `segmented` element is the same
  control as a form value ([3.10](#310-components-batch-2-as-built)).
- **Menus.** A `context-menu(list<menu-entry>)` node applies to its
  parent (it isn't drawn where it is) and uses its own `select`
  listener; a `menu-button` is a button with entries. Both are drawn
  above the tile as Pitwall's menus (a glass pop-up under Liquid Glass),
  not anchored inside it: a menu clipped by a small tile would be unusable.
- **Markdown** is a record (`markdown { text }`), drawn by
  `markdown_view`; with `link` and `toggle` listeners every link click and
  a task's box go to the tile (Notes' gap G6), without them `http(s)` and
  `mailto` links open in the browser.
- **Images** come from an asset path in the extension's folder (checked
  to stay inside it, ≤ 16 MiB) or from bytes put with
  `ui.put-image(name, bytes)` (≤ 8 MiB, 64 MiB a tile), sniffed as PNG,
  JPEG, GIF, WebP or SVG. Never from the network.
- Not in this cut: form controls, feedback, layout, data display and
  charts came in batch 2 ([3.10](#310-components-batch-2-as-built));
  `icon-button` is a button the SDK builds. `code`, `canvas` and drag
  are the native blocks ([native-blocks.md](native-blocks.md)).
- **Keyboard.** Lists, tables and trees take Up, Down, Home, End, Page
  Up, Page Down and Enter, and a tree ← and →; tabs, selects and menus
  are pointer-only for now (Esc closes a menu).

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
  submit(handler),              // Enter (one line) / ⌘⏎ (multi-line) in an input
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
  undo-present(u64),            // the user pressed Undo on an agent's change (11.6); phase 3
  tool-cancelled(u64),          // the call timed out or the agent went away
}
```

**Opened by an agent.** Every tile can be opened by an agent, with no
code from the author ([11.1](agent-tools.md#111-why-and-what)): Pitwall
places the view next to the calling agent's terminal, or where `where`
and `of` say, as it places a web tile ([11.8](agent-tools.md#118-where-the-tile-opens)),
then sends `view-opened` (or `view-content` for a view already open)
with the agent's input as the content, if the tile declares an
`"input"`. A tile that ignores the content still opens; one that reads
it shows the payload (a diagram, a draft note) and may `reply` with
problems in it. A later call with the same item (the content id)
updates that view in place.

Handlers are numbers. The SDK keeps the closures guest-side in a table
rebuilt on each render and sends only the numbers. A handler from an
older frame that no longer exists is dropped by the SDK.

### 3.5 Keyboard and focus

- A tile gets keys only while it has focus, as every tile [today].
  Pitwall's own bindings always win: the palette, ⌘⏎, ⌘W, space
  switching, ⌘1–9, Esc to leave a maximised tile. The reserved set is
  listed in the SDK docs and checked by `pitwall extension check`.
  One exception, as in Pitwall's own text fields [today: the kit's
  multi-line `TextInput`]: while a multi-line `input` has focus, ⌘⏎
  (Ctrl+Enter off macOS) submits it instead of maximising the tile.
  Enter submits a one-line field; a tile can't bind ⌘⏎ in its keymap.
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

**As built (phase 2, inside a tile).** `drag` and `drop` are two
`style` cases added at the end of the variant (`drag(drag-source)`:
kind, data at most 64 KiB, label; `drop-target(drop-target)`: the kinds
it accepts and a handler), so the node record is unchanged. A drop sends
`event.dropped` with the kind, the data and the position in the target.
The host draws the drag as Pitwall's own chip, highlights a target that
accepts what is over it, and refuses drops from another tile or view.
Drags between tiles and from Pitwall's own lists are not built.

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

**As built (phase 2).** The canvas is a frame element, not a `draw`
callback: `element.canvas` carries its ops, a transform and hit regions,
and is redrawn with the frame (no `ui.redraw`; motion is a timer and a
render). Ops: `rect` and `path` with an optional fill and an optional
`stroke` (colour, width, dash list of at most 16), `text` with a size in
pixels (not a token, so it zooms; Inter or JetBrains Mono, weight,
start/centre/end, top/middle/bottom, a max width cut with an ellipsis),
`clip-push`/`clip-pop`. Colours are `paint`: a token or `rgba`. Not built:
`icon` and `image` ops. The transform (scale, dx, dy) applies to ops
and regions, so a zoom is one number. Hit regions (key, rect, label,
tooltip, click and hover handlers) are laid over the drawing; their
label is the accessible name. Input: `pointer` events had positions
already; `listen.wheel` adds `pointer-kind.wheel(delta)` and
`pointer-kind.pinch(f32)`. A press that moves more than 4 px is not a
region click. Limits: 20 000 ops, 5 000 regions. Text measurement:
`ui.measure-text` ([3.11](#311-the-seed-tiles-gaps-as-built)); `max-width`
guards the rest. The SDK's `Viewport` turns drag, wheel, ⌘-wheel and
pinch into zoom and pan.



### 3.9 Toolbar items
The owner asked for this (2026-11-26): an extension can put a button in
Pitwall's top bar, next to icons such as Settings, so its user can
reach it from anywhere. Below, **Decided (owner)** marks the owner's
requests. *Recommendation* marks what this document adds; those points
are open to change.

#### What the top bar is today
These facts come from the code [today: `main_screen/topbar.rs`,
`platform/decorations.rs`, `theme.rs`].
- **One bar, drawn by Pitwall in GPUI** at the top of the main window's
  content. It is 44 px tall (`TOPBAR_H`), on the surface colour, with a
  hairline under it. The whole bar is the window's drag area. Its
  controls are wrapped in `no_drag`, so a press on them doesn't move
  the window.
- **Left to right:**
  1. Sidebar toggle (⌘B).
  2. The wordmark; only the P mark when compact.
  3. Space tabs, with "New space".
  4. A flexible gap.
  5. The right group: the Search field (⌘K), the live counts (working,
     done, needs you), then icon buttons for the Race Engineer,
     Review (⌘R), Wall (⌘E) and Settings (gear). (A Details button
     (⌘.) followed; it goes with the right Details panel, removed by
     the owner on 2026-11-26.)
  6. The icon buttons are `kit::icon_btn`: 28 px, with a 16 px icon,
     muted, and a tooltip.
- **Width classes** (`Breakpoint`): "compact" means narrower than
  1100 px. In compact, the wordmark shrinks to the P, Search loses its
  word, and the count pills shorten. The window's minimum size is
  640 × 480 (`window_state::MIN_SIZE`).
- **macOS:** the window keeps the native title bar, with the traffic
  lights in it (`decorations.rs`: "macOS: unchanged (the native title
  bar)"; [layout.md](../layout.md), "Window"). Pitwall's top bar sits
  *below* it. The two don't share space, so no inset for the traffic
  lights is needed.
- **Windows:** the native caption, with its controls, snap layouts and
  system menu. Under it is Pitwall's in-window File menu bar
  (`app_menu`), and the top bar comes under that. Dragging the top bar
  is passed to the caption.
- **Linux:** two cases.
  - Server-side decorations (KDE, sway, every X11 window manager): a
    native frame above, as on Windows.
  - Client-side, where the compositor offers no decorations (GNOME's
    mutter, Weston): the top bar *is* the title bar. Pitwall adds
    minimize, maximize and close at its right end, after Settings. A
    double-click maximizes and a right-click opens the window menu.
- **So the only difference between platforms inside the bar** is the
  window controls at the right end on client-side Linux, about 100 px
  (*unverified*: measured from the code's sizes, not on screen). The
  bar is otherwise the same Pitwall-drawn row everywhere.
- **Other windows:** a tile moved into a window of its own doesn't get
  this bar.

#### The item
- **Decided (owner):** Pitwall draws the item; it is not arbitrary UI.
  It has:
  - an icon, from the SDK's icon set or an SVG in the package. The SVG
    is drawn single-colour and tinted, so it follows the theme, as tile
    icons are ([3.3](#33-the-ui-description));
  - a tooltip;
  - an optional badge, a count or a dot, which the component can
    update.
- *Recommendation:* the item is drawn as `kit::icon_btn`, the same
  28 px button as Pitwall's own icons.
  - A count shows up to 99, then "99+".
  - The badge has two tones: normal (the accent colour), or attention
    (amber, like "needs you").
  - Updates are limited to 4 a second, like `contrib.badges` ([14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12)).

#### On click: one of four actions
**Decided (owner):** each item does one of these four things.

| Action | What happens |
|---|---|
| `tile` | Focus the extension's tile if it is open in the current space; otherwise open it where tiles open ([11.8](agent-tools.md#118-where-the-tile-opens)) |
| `command` | Run one of its commands ([11.12](agent-tools.md#1112-commands-for-the-shell-and-the-palette)), as the palette does: the user is the caller. A command that takes input opens the palette's small form |
| `menu` | Open a small menu, drawn by Pitwall, of up to 10 of its commands (*recommendation*), each run as for `command` |
| `popover` | Open a popover holding a small tile view, anchored under the item |

**The popover:**
- **Decided (owner):** its size stays within limits, and Esc or a
  click outside closes it.
- *Recommendation:*
  - Size: from 200 × 100 to 420 × 560 px, as the manifest asks, always
    inside the window.
  - It also closes when its item is clicked again.
  - Keyboard focus moves into it on open, because the user clicked,
    and goes back where it was on close.
  - Opening it creates a view: the guest gets a `view-opened` event
    with a `view-id` of kind `popover` and renders it through the same
    `render(view-id)` as its tiles ([3.2](runtime.md#32-the-guests-exports)). Closing it sends
    `view-closed` and drops the view.
  - Every element and building block is allowed, except the web view
    and the device view ([16.2](native-blocks.md#162-native-building-blocks)), which are too heavy for a popover.

#### Opt-in for authors, hideable by users
- **Decided (owner, 2026-11-26): authors opt in.**
  - The item exists only if the manifest declares `toolbar`. With no
    declaration there is no button, and that is the default.
  - An extension can offer the item but start it hidden
    (`"visibleByDefault": false`); the user then turns it on, in
    Settings or on the extension's card.
- **Decided (owner, 2026-11-26): users can hide it at any time.**
  - Right-click the item, then "Hide from toolbar". The same choice is
    on the toolbar page in Settings and on the extension's card.
  - Hiding never turns the extension off: its tile, tools, commands
    and badges elsewhere keep working.
  - The choice is remembered across updates. An update can't show an
    item the user hid, even if it changes `visibleByDefault`.
- *Recommendation: yes, the component may show or hide its own item at
  runtime*, with `toolbar.set-shown(bool)`. Example: a CI extension
  shows its item only while a run is failing.
  - It is cheap, because the host already lays the group out every
    frame. It also keeps the bar quiet, which is the point of the
    one-item limit.
  - The user's choice is always stronger. A hidden item stays hidden
    whatever the component asks, and `set-shown(true)` never brings
    back an item the user hid.
  - The component can only hide an item the user shows. Then it
    doesn't take room or appear in "…".
  - At most one change in 5 seconds, so the bar doesn't flicker.
  - Shown and hidden by the component are not remembered: each start
    begins from the manifest's state.

#### Limits
- **One item per extension.** This document decided it, so it is a
  *recommendation*, not the owner's. The reasons:
  - The bar is already full: the tabs fill the middle, and the right
    group holds Search, the counts and four icons.
  - The window can be as narrow as 640 px.
  - The `menu` and `popover` actions already give one item room for
    many functions.

  Two per extension would double the room taken by each extension for
  little gain; revisit if real extensions need it.
- **Decided (owner):** extension items sit in a group of their own,
  apart from Pitwall's icons, and never replace or hide them.
  - *Recommendation:* place the group between the counts and the Race
    Engineer button, after a hairline divider. Pitwall's own icons then
    keep their places at the right end, and on client-side Linux the
    window controls stay last.
- **Decided (owner):** the user can hide, show and reorder items.
  - Ways to do it: right-click an item for Hide and Move left / Move
    right; dragging within the group (*recommendation*); and a "Show in
    toolbar" switch on the extension's card in Settings → Extensions.
  - *Recommendation:* the order and the hidden set are kept in
    `ui.json` (`toolbar.order`, `toolbar.hidden`).
  - A hidden item's commands stay in the palette.
- **Decided (owner):** an item appears only once its extension is
  installed and turned on. Turning the extension off removes the item.
- **Decided (owner):** when there is no room, items collapse into a
  "…" menu at the end of the group. The menu shows each item's icon,
  title and badge.
  - *Recommendation:* the width is measured each layout, so whatever
    doesn't fit moves into "…". In compact, all extension items go
    there, and "…" shows a dot when any hidden-away item has a badge.

#### Permission
- **Decided (owner):**
  - No permission is needed beyond the install screen, which lists
    "Adds a button to the toolbar".
  - The badge needs no permission either.
- *Recommendation:* one point the code's model adds. Keeping a badge
  live while no tile is shown needs the component to keep running.
  - A badge-free item needs nothing running: a click starts the
    component.
  - An item with `"badge": true` is a contribution that keeps the
    instance running, like `badges` and `asks`
    ([15.5](extension-model.md#156-headless-components)). So the install screen also shows that section's
    "Runs while you aren't looking at it".

  The badge adds no new grant. It is the existing rule for a component
  that works while hidden.

#### Scope: windows, spaces, projects
- **Decided (owner):** per window.
  - Every main window shows the extension's item.
  - *Recommendation:* the component is one instance ([2.3](runtime.md#23-lifecycle)), so the badge
    is the same in every window. A popover opens in the window that was
    clicked.
- **Decided (owner):** per project, when the extension's tile is per
  project.
  - *Recommendation:* with `"scope": "project"`, the item shows only
    while the window's active space is a project space. The badge is
    the one the component set for that project
    (`set-project-badge(project, badge)`). Naming projects needs
    `projects.read`, the same rule as row badges today ([14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12)).
  - In a custom space or the All space, a per-project item shows the
    component's window-wide badge (`set-badge`), or nothing if it set
    none.
  - `tile` and `popover` then act for the active project: the event
    carries its id.

#### Manifest and WIT (sketch)
One object, not a list, because the limit is one item:

```json
"contributes": {
  "tiles": [{ "id": "inbox", "title": "Support inbox", "icon": "inbox", "wasm": "tile.wasm" }],
  "toolbar": {
    "icon": "inbox",                       // SDK icon name, or "assets/toolbar.svg"
    "tooltip": "Support inbox",
    "badge": true,                         // the component updates a badge (keeps it running, 15.5)
    "scope": "window",                     // or "project"
    "visibleByDefault": true,              // false: offered, but the user turns it on
    "action": { "kind": "popover", "tile": "inbox", "size": { "w": 360, "h": 420 } }
    // or { "kind": "tile", "tile": "inbox" }
    // or { "kind": "command", "command": "sync_now" }
    // or { "kind": "menu", "commands": ["sync_now", "compose", "mark_all_read"] }
  }
}
```

The commands named here must be ones the extension exposes to the
palette ([11.12](agent-tools.md#1112-commands-for-the-shell-and-the-palette)). `check` refuses an unknown icon, an SVG that is not
single-colour, a missing tile or command, or a popover size outside the
limits.

```wit
interface toolbar {
  variant badge { none, dot, count(u32) }
  enum tone { normal, attention }

  /// The window-wide badge (all spaces, or custom and All spaces for a
  /// per-project item). At most 4 updates a second.
  set-badge: func(badge: badge, tone: tone);
  /// A project's badge for a per-project item; needs projects.read.
  set-project-badge: func(project: string, badge: badge, tone: tone);
  /// Change the tooltip, e.g. "3 unread". ≤ 80 characters.
  set-tooltip: func(text: string);
  /// Show or hide its own item. Never overrides the user's Hide;
  /// at most one change in 5 s.
  set-shown: func(shown: bool);
  /// Close this extension's open popover (after a pick, a send).
  close-popover: func();
}
// events (into 3.4's variant):
//   view-opened(view-id, view-kind)   // popover views; kind = popover
//   view-closed(view-id)
//   toolbar-clicked(option<string>)   // for kind tile or popover: the active project id, if any
```

#### Pitwall's own icons, the same way?
*Recommendation:* yes inside the code, and only partly for users.
- Pitwall draws its own buttons from the same item model: icon,
  tooltip, badge and action. Then one code path draws, measures and
  overflows both groups.
- Pitwall's own icons (sidebar, Engineer, Review, Wall,
  Settings) stay fixed in v0.x: users can't hide or move them. Settings
  holds the grants and kill switches and must always be reachable.
- When Review becomes an extension ([16.3](native-blocks.md#163-which-built-in-tiles-move-to-the-sdk)), its button becomes that bundled
  extension's toolbar item, kept in Pitwall's group, with ⌘R kept.
  This tests the contribution with Pitwall's own proof tile. Letting
  users hide built-in icons is a later choice.

#### Phase
**Decided (owner):** toolbar items are part of the v0.x minimum and are
built in phase 2. They are cheap, and useful for inbox-style
extensions that show unread counts. The website's Tiles and UI components page
documents them in the same phase ([9.6](authoring.md#96-website-docs)).


### 3.10 Components, batch 2 (as built)

**Kit parity** (decided by the owner, 2026-11-27: "we need to provide
as rich an SDK as possible, proper components"). Anything Pitwall's own
UI draws, a tile can draw, with the same look. Components are drawn
from the app's kit (`crates/pitwall-app/src/kit/`: `checkbox_box`,
`toggle`, `radio`, `seg`, `spinner`, `chip_tone`, `kbd`, `divider`,
`tooltip`, `menu_panel`, the dialog look) with the same theme tokens;
a control the kit lacks (slider, progress, skeleton, banner, avatar,
charts) is added to the kit in its style, so Pitwall's own screens can
use it too. Tiles never get a second look of their own.

Batch 2 adds, to WIT 0.1 in place (cases at the end of `element`,
`listen` and `event`; records at the end of `types`):

- **Form controls**: `checkbox(check)`, `switch(check)`,
  `radio-group(choice-group)`, `segmented(choice-group)`,
  `slider(slider)`, `number-field(number-field)`,
  `search-field(search-field)`, `form-row(form-row)`.
- **Feedback**: `progress(progress)`, `spinner`, `skeleton(skeleton)`,
  `empty-state(empty-state)`, `banner(banner)`, `badge(badge)`,
  `chip(chip)`, `kbd(string)`. Tooltips were already on every node.
- **Layout**: `split(split)`, `disclosure(disclosure)`, `card(card)`,
  `divider(orientation)`, `breadcrumbs(list<crumb>)`, `toolbar`,
  `popover(popover)`, `confirm-sheet(confirm-sheet)`.
- **Data display**: `key-values(list<key-value>)`, `avatar(avatar)`,
  `relative-time(relative-time)`, `copyable(copyable)`, `link(link)`.
- **Charts**: `sparkline(sparkline)`, `bar-chart(bar-chart)`,
  `line-chart(line-chart)`, `status-timeline(status-timeline)`.
- **Listeners and events**: `listen.value` with
  `event.value-changed(value-change { handler, value: control-value })`,
  where `control-value` is `on(bool)`, `number(f64)`, `choice(string)`
  or `empty`; `listen.dismiss` with `event.dismissed(handler)`.

How it works:

- **Controlled.** A control shows the value in the frame; the user's
  change arrives as `value-changed` and the next frame shows it. A
  search field is a text field (host-owned text with a revision, as
  `input`). A slider is drawn live by the host while dragged and sends
  at most one value every 50 ms, and the last on release; a number
  field sends on Enter or blur, clamped and rounded, or nothing when
  the text isn't a number.
- **One set of rules.** What a click or a key does to a component
  (`on_click`, `on_key`, `focusable`, `snap`, `parse_number`,
  `split_share`, `relative_time`, `initials`, `avatar_tokens`, `axis`,
  `chart_summary`) lives in `pitwall_tile_host::widgets`; the app and
  the harness both call it, so tests see what users get.
- **Keyboard.** Every enabled control is a Tab stop with the focus
  ring; disabled ones are not. Space toggles a checkbox or switch;
  arrows, Home and End move a radio group's or segmented control's
  choice (skipping disabled options, wrapping) and a slider (Page Up
  and Page Down by a tenth); Up and Down step a number field; Enter or
  Space opens a disclosure and clicks a card, link, copyable or chip;
  Backspace or Delete removes a chip; Enter confirms a confirm sheet;
  Esc cancels it and closes a popover.
- **Split.** Exactly two children. The user's share is kept per view
  by key path, across restarts, and `value-changed` (`number`) goes
  to the tile once a drag ends.
- **Popover** is open while it is in the frame, anchored to its parent
  node, drawn by the host inside the tile; Esc or a click outside sends
  `dismissed`. One a frame; never the root.
- **Confirm sheet** replaces the OS dialogs tiles can't open: a sheet
  over the tile, the rest dimmed and inert. `click` confirms,
  `dismissed` cancels (Cancel, Esc, a click outside). One a frame.
- **Relative time** is updated by the host (`relative_time_next` says
  when its words change), so a tile needs no timer.
- **Copyable** writes the clipboard in the host, only on the user's
  click; a tile has no clipboard API.
- **Link** opens through the existing open-web path (as `show_web`)
  only with the `show` grant and a valid `http(s)` address; otherwise
  the click is sent to the tile.
- **Avatar** colours come from the token set, chosen by a stable hash
  of the name; no pictures.
- **Charts** are data-only: labels, up to 8 series and options (unit,
  stacked, min, max, colours as tokens), never drawing operations. The
  host draws axes, legends and tooltips in theme colours, and names a
  chart by its `label` and a summary of its data.
- **Limits** (checked with the frame): 64 choices, 32 crumbs, 500
  key-value lines, 8 series, 200 bar labels, 1 000 line points and
  sparkline values, 500 timeline segments, a badge's text 32 bytes, a
  copyable's 64 KiB, a link's address 2 048 bytes, a skeleton 20 lines;
  a slider's range and a number field's bounds and step finite and in
  order.
- **Gallery.** The hello fixture's `gallery` tile shows every component
  of both batches on its pages (tabs at the top); a view opened with
  `{"component": "<name>"}` shows one alone. The website's component
  pages include its `docs:start gallery-<name>` regions, and their
  screenshots are that tile drawn headless, dark and light.

#### Component catalogue

Every component a tile can use, its WIT shape (element case and
record) and its status. Batch 1 is #126; batch 2 is #128.

| Component | WIT | SDK | Status |
|---|---|---|---|
| Box | `box` | `div`, `h_flex`, `v_flex` | Phase 1 |
| Text | `text(string)` | `text` | Phase 1 |
| Button | `button(button)` | `button`, `icon_button` | Phase 1 |
| Icon | `icon(icon)` | `icon` | Phase 1 |
| Scroll | `scroll(scroll)` | `.overflow_y_scroll()` | Phase 1 |
| Text field | `input(input)` | `input` | Phase 1 |
| Tooltip | `node.tooltip` | `.tooltip` | Phase 1 |
| Canvas | `canvas(canvas)` | `canvas` | Phase 2, native blocks |
| Drag and drop | `style.drag`, `style.drop-target` | `.draggable`, `.on_drop` | Phase 2, native blocks |
| Code view | `code-view(code-view)` | `code_view` | Phase 2, native blocks |
| Diff view | `diff-view(diff-view)` | `diff_view` | Phase 2, native blocks |
| Virtual list | `virtual-list(virtual-list)` | `virtual_list` | Phase 2, batch 1 |
| Table | `table(table)` | `table`, `column` | Phase 2, batch 1 |
| Tree | `tree(tree)` | `tree`, `tree_row` | Phase 2, batch 1 |
| Tabs | `tabs(tabs)` | `tabs`, `tab` | Phase 2, batch 1 |
| Select | `select(select)` | `select`, `choice` | Phase 2, batch 1 |
| Menu button | `menu-button(menu-button)` | `menu_button`, `menu_item` | Phase 2, batch 1 |
| Context menu | `context-menu(list<menu-entry>)` | `.context_menu` | Phase 2, batch 1 |
| Markdown | `markdown(markdown)` | `markdown` | Phase 2, batch 1 |
| Image | `image(image)` | `image_asset`, `image_data` | Phase 2, batch 1 |
| Checkbox | `checkbox(check)` | `checkbox` | Phase 2, batch 2 |
| Switch | `switch(check)` | `switch` | Phase 2, batch 2 |
| Radio group | `radio-group(choice-group)` | `radio_group` | Phase 2, batch 2 |
| Segmented control | `segmented(choice-group)` | `segmented` | Phase 2, batch 2 |
| Slider | `slider(slider)` | `slider` | Phase 2, batch 2 |
| Number field | `number-field(number-field)` | `number_field` | Phase 2, batch 2 |
| Search field | `search-field(search-field)` | `search_field` | Phase 2, batch 2 |
| Form row | `form-row(form-row)` | `form_row` | Phase 2, batch 2 |
| Progress bar | `progress(progress)` | `progress` | Phase 2, batch 2 |
| Spinner | `spinner` | `spinner` | Phase 2, batch 2 |
| Skeleton | `skeleton(skeleton)` | `skeleton` | Phase 2, batch 2 |
| Empty state | `empty-state(empty-state)` | `empty_state` | Phase 2, batch 2 |
| Banner | `banner(banner)` | `banner` | Phase 2, batch 2 |
| Badge | `badge(badge)` | `badge` | Phase 2, batch 2 |
| Chip | `chip(chip)` | `chip` | Phase 2, batch 2 |
| Key hint | `kbd(string)` | `kbd` | Phase 2, batch 2 |
| Split pane | `split(split)` | `h_split`, `v_split` | Phase 2, batch 2 |
| Disclosure | `disclosure(disclosure)` | `disclosure` | Phase 2, batch 2 |
| Card | `card(card)` | `card` | Phase 2, batch 2 |
| Divider | `divider(orientation)` | `divider`, `vertical_divider` | Phase 2, batch 2 |
| Breadcrumbs | `breadcrumbs(list<crumb>)` | `breadcrumbs` | Phase 2, batch 2 |
| Toolbar row | `toolbar` | `toolbar` | Phase 2, batch 2 |
| Popover | `popover(popover)` | `popover` | Phase 2, batch 2 |
| Confirm sheet | `confirm-sheet(confirm-sheet)` | `confirm_sheet` | Phase 2, batch 2 |
| Key-value list | `key-values(list<key-value>)` | `key_values` | Phase 2, batch 2 |
| Avatar | `avatar(avatar)` | `avatar` | Phase 2, batch 2 |
| Relative time | `relative-time(relative-time)` | `relative_time`, `relative_time_ms` | Phase 2, batch 2 |
| Copyable text | `copyable(copyable)` | `copyable` | Phase 2, batch 2 |
| Link | `link(link)` | `link` | Phase 2, batch 2 |
| Sparkline | `sparkline(sparkline)` | `sparkline` | Phase 2, batch 2 |
| Bar chart | `bar-chart(bar-chart)` | `bar_chart` | Phase 2, batch 2 |
| Line chart | `line-chart(line-chart)` | `line_chart` | Phase 2, batch 2 |
| Status timeline | `status-timeline(status-timeline)` | `status_timeline` | Phase 2, batch 2 |
| Secure field | `secret-field(secret-field)` | `secret_field` | Planned ([3.12](#312-the-setup-view-and-host-drawn-fields), plan 8a) |
| Setting field | `setting-field(setting-field)` | `setting_field` | Planned ([3.12](#312-the-setup-view-and-host-drawn-fields), plan 8a) |

### 3.11 The seed tiles' gaps (as built)

What the C4 and Notes seed tiles found missing ([plan](plan.md) item 6),
added to WIT 0.1 in place, the fixtures and `tiles/c4/tile.wasm` rebuilt:

- **Text measurement** (C4 G2). `ui.measure-text(texts, font, size,
  weight) -> list<f32>`: widths on one line in Pitwall's Inter or
  JetBrains Mono, kerned. GPUI's text system isn't reachable from a
  tile's worker or the harness, so the host reads tables `build.rs`
  makes from the app's own font files with rustybuzz (the method the C4
  tile used); the harness gives the same numbers. Batched (≤ 1 000 texts,
  64 KiB a call), since each call counts toward the 200 host calls a
  second. SDK: `ui::measure_text`, `Label::width`. The C4 tile keeps its
  own table: its layout core builds and is tested without the SDK, and a
  layout measures thousands of strings while wrapping; its test now
  checks the table against the host's measure (equal for 100 labels,
  both weights), so a font Pitwall changes fails there.
- **A popover at a point** (C4's box menu). `popover.at: option<point>`
  anchors it to a point of its parent, in the parent's pixels;
  `placement` says which side. A canvas draws its popover children, so
  `canvas.popover(popover().at(..))` opens a menu under a box.
- **Problem lines in the code view.** `line-mark-kind` gains `error` and
  `warning`, `line-mark` a `message`: a red or yellow bar, the lines
  tinted, the message after the first line's text. The C4 tile's
  problems view uses it.
- **The selection of a text input** (Notes G2). No WIT change: WIT 0.1
  already sends the selection with each `input-changed` and in
  `input-selected`; the tile keeps the latest with its copy of the text.
  The harness gains `select_text` and `select_in`; Notes sends the
  selection first when there is one.
- **`viewer.open`** (Notes FR-10). `show.file` is the editor;
  `show.view(path, line)` shows the file in Pitwall's file viewer, as an
  agent's `show_files` does: the extension's own viewer window
  (`tile:<id>`), rooted at the file's git repository (else its folder),
  opened behind the front window without the keyboard. Needs `show`.
- **Rows** may hold badges, avatars and relative times (batch 1's
  follow-up).
- **The app's headless tests** choose a segmented control's or radio
  group's option (`TileView::choose`) and press a bound key
  (`TileView::press_key`), as the harness does; the C4 tile's Changes
  view has its screenshot.

### 3.12 The Setup view and host-drawn fields
Decided by the owner on 2026-11-28 ([15.9](extension-model.md#159-decisions) item 8). Values in
[14.6](platform-services.md#146-settings-secrets-and-the-setup-view).

**The extension's page** (in the Extensions tile, and Settings →
Extensions' compact page) is Pitwall's, the same for every extension,
top to bottom:
1. Header: icon, name, version, source, the on/off switch, the status
   pill (`status.set`), Remove. Updates under it, as today.
2. **The Setup view**, when the manifest names one: the component's own
   view, full width, its height its content's (at most the page's
   height, scrolling inside), drawn inside a frame that carries the
   extension's icon and name, like a tile's header. Before it renders
   (or while the extension is off), a skeleton; after a trap, "Setup
   stopped" with Restart.
3. Permissions with their switches, programs (allowed lines, running
   ones with Stop, Restart, Log), agent environment, data size with
   Clear, Activity, Log. These stay host-drawn, so a Setup view can't
   hide or restyle them.
An extension without a Setup view shows 1 and 3 only.

**The secure field** (`secret-field`):
```wit
record secret-field { key: string, placeholder: option<string> }
```
The component places it; the host draws and owns it, like the tile's
header. Unset: a masked field with a lock mark and "Paste"; on Enter or
blur the value goes to the secure store, never through the component.
Set: "Saved in your keychain" (Credential Manager, the keyring) with
Replace and Remove; the value is never shown or sent. The component
gets `secret-changed(key, set)`. The field refuses a `key` that isn't a
`secret` in the schema (drawn as an error box in dev mode). Its look
is reserved: tiles can't draw the lock mark or that line, and the
harness checks that a tile's own input never carries them.

**The setting field** (`setting-field`): the same idea for
grant-bearing settings (a `url` named by `{setting:KEY}`): a host-drawn
text field showing the value, validated on blur, saved by the host,
then `settings-changed(key)`. A new value that changes a `net` host or a
program line is the user's own entry, so it is granted as typed, and a
program line it changes is asked again. Other settings the Setup view
draws with ordinary components and saves with `settings.set`.

**Agents and the user's attention.** "Setup by an agent"'s
`extension.secret` opens the page at the Setup view next to the agent's
terminal with the secure field for that key marked ("An agent asked you
to set this"); the value never goes to the agent.

