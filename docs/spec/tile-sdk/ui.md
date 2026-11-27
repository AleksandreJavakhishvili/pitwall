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
region click. Limits: 20 000 ops, 5 000 regions. No text-measurement
call: canvas text is Inter or JetBrains Mono at any size, widths scale
linearly with size, and GPUI's text system is not reachable from the
tile's worker or the harness, so a width table in the guest (as the C4
tile has) is the measure; `max-width` guards the rest. The SDK's
`Viewport` turns drag, wheel, ⌘-wheel and pinch into zoom and pan.



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
    ([15.5](extension-model.md#155-headless-components)). So the install screen also shows that section's
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
