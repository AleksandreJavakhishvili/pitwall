# Layout, screens and windows


Goal: works on a 13" laptop half-screen up to an ultrawide or two monitors, and
stays intuitive. Concepts, in the order a user meets them:

1. **Projects** group agents. The sidebar lists agents grouped under collapsible
   project headers (blocked agents still float to the top of their group, and a
   project header shows ▲ when any of its agents is blocked).
2. **Spaces** are tabs at the top of a window. Each space is a tiled layout of
   1–N terminal panes. Defaults: one "All" space; a project header button
   "Open as space" creates a space tiling that project's agents. Users can make
   custom spaces mixing agents from 2–3 projects (drag agents in from the sidebar).
3. **Windows**: any space can be moved to its own OS window ("Move to new
   window", ⌘⇧N, or drag a tab out) — for a second monitor; a tab dropped on
   another window moves its space there. Every window has the same sidebar, its
   own tabs, and the global "needs you" strip.

Rules that keep it simple:
- **An agent is shown in at most one pane at a time.** Clicking an agent that is
  visible elsewhere focuses that window/space/pane instead of duplicating it.
  Dragging it to another space moves it. (PTY size = its single pane's size.)
- **Tiling**: drag an agent onto a pane's left/right/top/bottom drop zone to
  split; drag dividers to resize; presets 1 / 2 / 3 (1 big + 2) / 2×2 / 3×2.
  Double-click a pane header (or ⌘⏎) to maximise it within the space and back.
  Every tile takes part alike, terminals, files, web pages and the inbox: a
  press anywhere in one makes it the space's focused tile, shown by its
  border, and ⌘⏎ (Ctrl+Shift+Enter on Windows and Linux) maximises and
  restores that tile, which keeps what it shows (a viewer's folder, tabs,
  scroll and unsaved edits; a page as it was). A click into a web page
  counts too, though the page's native view takes it: the page says it
  has the keys. A text field's own ⌘↵ (send) wins over Maximise.
- **Minimum pane size** ~80 cols × 20 rows at the current font; when a space has
  more agents than fit, extra panes collapse into a compact strip of agent chips
  at the bottom of the space (click to swap in) instead of becoming unreadable.
- **No right panel.** The right Details panel (Next up / Changes / Last
  sent) and its ⌘. toggle were removed on 2026-11-26. Next up lives on
  the agent's tile: "2 queued" in its header while something waits,
  opening a popover with the queue, Auto-send and the last prompt sent.

Responsive breakpoints (per window width):
| Width | Sidebar | Max panes per row |
|---|---|---|
| ≥ 2200 (ultrawide/big) | full | 4 |
| 1500–2200 | full | 3 |
| 1100–1500 | full, collapsible | 2 |
| 700–1100 | icon rail (status dots + initials) | 1 (others as chips) |
| < 700 | overlay | 1 |
Window min size 640×480. ⌘+ / ⌘− / ⌘0 change terminal font size globally.

Persistence: spaces, layouts, which window holds which space, and window
bounds are stored by the backend as an opaque UI-owned JSON blob so all windows
share it and it survives restarts.

Backend additions:
| Command | Args | Returns |
|---|---|---|
| `get_ui_state` | – | `unknown` (JSON blob or null) |
| `set_ui_state` | `state: unknown` | `void` (persist to `~/.pitwall/ui.json`, emit `ui-state-changed` to all windows with `{ state, sourceWindow }`) |
| `open_window` | `spaceId: string` | `string` (new window label `pitwall-<n>`; loads the app with `?space=<spaceId>`) |
| `focus_window` | `label: string` | `void` |
| `list_windows` | – | `string[]` labels |
- Capabilities must cover all windows (`"windows": ["main", "pitwall-*"]`).
- Restore extra windows (with saved bounds) on launch; close of a secondary
  window moves its spaces back to "main" (UI updates the blob); closing "main"
  hides it as before.
- Events (`agents-changed`, `attention`) go to all windows. `attach_output`
  already supports multiple subscribers; when a pane moves windows the new
  window attaches and the old one drops its channel (add `detach_output(agentId, subscriptionId)`; `attach_output` returns the subscription id as a number).


## Revision: more and smaller tiles

Need: more than two rows of tiles, with room for small ones.
- **Density** setting replaces the fixed 80×20 minimum: Comfortable 80×20,
  Compact 60×12 (new default), Dense 40×8. Global default in Settings + ⌘K,
  overridable per space (persisted in the UI state blob).
- **Per-tile font size**: a tile may auto-shrink its terminal font (floor ~10px)
  before folding into chips; ⌘+ / ⌘− / ⌘0 act on the focused tile (global
  font stays the base). Resizing still sends the real cols/rows to the PTY.
- **Presets**: add 3×3, 4×3, 4×4 (offered when the window can fit them at the
  current density) and "Auto grid" (fit all of the space's agents as evenly as
  possible, then chips).
- **Free splitting**: any number of rows/columns, limited only by density.
- Breakpoint table's "max panes per row" becomes a default for Auto grid, not a cap.

## Sidebar order and moves by drag

- The sidebar keeps the order the user drags things into, nothing else:
  accounts (Default stays first; the order is `accounts.json`'s, so
  Settings shows it too), projects (within this Mac's, or one machine's),
  and agents within their project (`sidebarOrder` in `ui.json`). What was
  never dragged follows: projects by name, agents oldest first. Status
  never reorders it; a blocked agent is amber where it is. ⌘1–9 and ⌘J
  follow this order.
- A line shows where a drop lands (upper or lower half of a row or header);
  dropping on a project's header puts an agent first in it.
- Grouped by account, dropping onto another account (its header, or the
  same project's header or rows under it) moves there:
  - an agent: it restarts on that login at once, its conversation with it
    (the agent menu's "Run on account");
  - a project: it becomes the project's account for that tool (its own
    pick, over its profile's), and its running agents of that tool
    restart there at once (as Accounts…).
  A project is grouped under the account it resolves to (its own pick,
  else its profile's, else Default: docs/spec/profiles.md).
  An account that isn't logged in yet starts its login terminal first.
  Not allowed (nothing highlights): another tool's account, agents on other
  machines, terminals, the Race Engineer, an agent into another project.
- **Once a profile exists** (docs/spec/profiles.md), "Group by account"
  reads "Group by profile" and This Mac's sections are its profiles, in
  their order (Settings → Profiles shows the same), each even when empty,
  then "No profile" for projects without one (hidden when empty). A
  project sits under its profile even when it picks something itself; a
  small hollow dot after its name says so ("Uses its own git identity
  instead of Work's"). Dropping a project on a profile's header (or a
  project under it) gives it that profile, exactly as picking it in the
  project's Profile… dialog (agents move or restart under the rules
  above); on "No profile", it has none. Profile headers drag to reorder
  ("No profile" stays last); a header's menu has "Edit profile…" (Settings
  → Profiles, that profile unfolded). Agents are not dropped across
  profiles: a project's agents follow its profile. Other machines keep
  their accounts. With no profile the sidebar is as above.
- With 2+ machines the sidebar is machines → accounts → projects → agents
  (tailnet-peers.md, "Sidebar"). Drags stay within one machine's section:
  only This Mac's accounts move or take agents and projects, and another
  machine's Pitwall (a peer) is read-only: nothing is dragged into, out of
  or within it.
- Extensions that are on (extensions.md, draft) have a section each after
  the projects (and machines), before Elsewhere: foldable like a project,
  never dragged, never a drop target.

## Revision: drag & drop must rearrange properly

Need: dragging an agent rearranges the layout and adds it properly.
Expected behaviour (VS Code editor-group feel), with a clear drop preview:
- Drag an agent (sidebar row, pane header, Wall tile, chip in the overflow
  strip) over a pane → highlight zones: left/right/top/bottom half = split
  that side and insert; centre = swap the two agents (or replace if the source
  isn't shown anywhere).
- Drop on empty space area / an empty pane → add there.
- Drop on a space tab → move the agent into that space (auto-placed), keep the
  current space's layout tidy (closing the gap, no empty holes).
- An agent is shown in at most one pane: dragging one that is visible
  elsewhere MOVES it (source pane closes and the tree collapses), never
  duplicates; dropping on itself is a no-op.
- Respect density limits: if the target would get too small, place it as a
  chip in the overflow strip instead and say so in a toast.
- Escape cancels a drag. Works across windows via "Move to window…" menu
  (cross-window native drag is out of scope).
- Never restart or resize-thrash a process: moving only re-parents the xterm
  and sends one final resize.

## Revision: the "needs you" strip never changes the layout

Need: the amber "needs you" strip must not change the layout; it lives at
the bottom.
- The bottom of every window is a **permanent status strip** of fixed height
  (`--statusbar-h`, 28px; a fixed grid row). Only its look changes, never its
  size, so terminals are never resized by it (no PTY resize churn or TUI redraws):
  - calm (neutral): "◐ 3 working · Nothing needs you" and a ⌘K hint;
  - amber: "▲ NAME needs you: detail · +N more" and **Jump ⌘J**;
  - mint: "⚑ NAME finished · +N more" and **Show**, when only finished agents wait.
- Narrow windows (sm/xs) drop hints, never the height. Both themes use the
  status tokens; colour changes fade (none with reduced motion).
- Other transient UI never takes layout space either: toasts float above the
  strip (`position: fixed`), stopped/drop overlays sit on top of their pane.

## Revision: Appearance — Look (Flat / Glass) and motion

Settings → Appearance has **Theme** (System / Dark / Light), **Look** (Flat /
Glass; default Flat) and **Reduce motion**. All three live in the shared UI
state (every window follows them, they survive restarts) and are mirrored to
localStorage for index.html's pre-paint script (`src/lib/theme.ts`,
`src/lib/look.ts`).

**Glass**: atmospheric colour behind frosted, translucent chrome with light
hairline edges and pill buttons (the primary one white in Dark, ink in
Light). Pitwall's own palette stays: amber "needs you", mint "done", the
chequered flag; status colours are identical in both looks.
- **Only chrome is glass**: top bar and tabs, sidebar, status
  strip, palette, dialogs, menus, toasts, chips. Terminal panes, Wall tiles,
  Review / Files code views and the diff dialog keep solid backgrounds
  (legibility; nothing blends behind text that repaints all the time).
- **Tiers**, decided in one place: the desktop offers a window material
  (`HostInfo.glass`, `pitwall_core::host`: `vibrancy` on macOS, `mica` on
  Windows 11 ≥ build 22000, `none` on Linux and Windows 10), and each
  window asks for it itself (`crates/pitwall-app/src/platform/glass.rs`;
  the Tauri shell had `set_window_glass`), so spaces moved to their own windows get it too. Settings
  names the tier:
  - **Glass · native** (macOS): an `NSVisualEffectView`
    (under-window-background material, kept active so unfocused windows
    stay glass)
    behind a transparent webview. The compositor does the blur; the chrome
    only tints it.
  - **Glass · Mica** (Windows 11): the Mica backdrop, same tints. Not
    Acrylic (it lags while windows are dragged or resized).
  - **Glass lite** (Linux, Windows 10, browser): no window material, no
    transparent window and no blur; Pitwall paints the atmospheric gradient
    once and the same tinted panels over it — the same look without the
    desktop showing through. `PITWALL_GLASS=lite|mica|vibrancy` forces a
    tier (development; e.g. to see Glass lite on a Mac).
- `backdrop-filter` only where the window material can't reach: surfaces
  floating over terminals (palette, dialogs, menus, toasts, overlay sidebar /
  drawer), on native and Mica tiers only, never per row or tile. On Glass
  lite those surfaces are nearly opaque instead.
- Glass starts on its lite tier before first paint and switches to native
  once the window confirms its material (no white flash).
- **Accessibility**: macOS "Reduce transparency" (read natively;
  `prefers-reduced-transparency` too) shows Flat and removes the window
  material; "Increase contrast" (`prefers-contrast: more` too) makes glass
  nearly solid with stronger hairlines. Small text on glass keeps ≥ 4.5:1
  (`--text-3` is lifted on glass).
- **Window**: the standard title bar stays (traffic lights, drag and double
  click unchanged; the content below it is the glass). The webview only stops
  painting its own background while Glass is on (`set_background_color`
  alpha 0; on macOS WebKit's private `drawsBackground` key, which Tauri
  ≥ 2.12.1 uses for any transparent webview — no `macOSPrivateApi` flag is
  needed any more). Private API use rules out the Mac App Store, not
  Developer ID signing or notarization, which is how Pitwall ships. macOS
  needs no transparent window (the effect view sits behind the webview);
  on Windows DWM draws Mica only behind a transparent window, so windows
  are created transparent there (`tauri.windows.conf.json`, `windows.rs`)
  and the page paints opaque unless Glass is on. Linux windows stay opaque.

**Motion** (`src/styles/motion.css`, both looks): transform and opacity only.
Palette, dialogs and menus open with a fade + rise/scale on a spring-like
curve (150–220 ms; closing stays instant), the overlay sidebar / drawer
slide, toasts slide + fade in, switching space / Wall / Review / Files
cross-fades, buttons pop slightly when pressed. "Needs you" plays a halo
twice and the glyph bounces when the status starts (it used to breathe
forever); "done" pops its chequered flag once. Glass adds a ring on the
working dot and a sheen across primary buttons on hover. Only the working
dot loops. Nothing animates on the Wall, while the page is hidden (minimised
or occluded: `visibilitychange`), with Reduce motion or the system's reduced
motion. No blur-in: animating `filter` would cost GPU work on every frame.

## The Settings tile

Settings can be a tile of the layout, beside terminals, files, web pages,
the inbox and extensions' views: the same Settings as the dialog (one
`SettingsView`: every page, its page list, its unsaved fields), drawn at
the tile's full size. The page list is a column beside the page; in a tile
narrower than 560 px it wraps in rows above the page. There is no fixed
width or height as in the dialog, and no head (the tile's own bar has its
name, Move…, Maximise, Open in a window of its own and Close).

- **Opening it.** "Open as a tile" (a button in the dialog's head),
  "Open Settings as a tile" in the command palette, or an agent's
  `show_settings` / `pitwall open --settings [<page>]` (below). From the
  dialog or the palette it goes where the inbox goes: into the focused
  tile when that is empty, else right of it; focused, with the keys.
- **A tile like the others.** A press anywhere in it makes it the space's
  focused tile and gives it the keys; ⌘⏎ (Ctrl+Shift+Enter on Windows and
  Linux) maximises and restores it; its bar drags it in the layout or out
  of every window (a window of its own, as an inbox); Move… places it or
  opens it in a window; Dock… brings it back, the same Settings. Close
  closes it.
- **Kept.** The pane is saved with `"settings": true` in `ui.json`; in a
  window of its own, `settingsWindow: true`. The page is `settingsPage`,
  shared with the dialog. After a restart the tile (or its window) is back
  at that page.
- **One Settings at a time.** The dialog's "Open as a tile" hands its view
  to the tile and closes, so what you were typing moves with it. Opening
  the tile any other way (the palette, an agent) closes a dialog left open
  in any window, its view going into the tile when there is none yet.
  While the tile is placed (in a layout or its own window), Settings'
  shortcut, the menu's Settings… and the palette's "Settings: <page>"
  focus the tile (showing its space, bringing its window forward) instead
  of opening the dialog. So two copies of Settings' fields never exist,
  and none can diverge: there is one view, and its fields are in one
  place.
- **Agents** (`show_settings`, `settings.open`, `agents.showSettings`):
  the tile placed right of the agent's terminal, or (its terminal isn't in
  a window) in the space the frontmost Pitwall window shows, with a note
  saying so. A tile that is already somewhere stays there: its space is
  shown in its window and it switches page. `extension` opens that
  extension's page in the Extensions tile (placed, when it isn't, in the
  space the frontmost Pitwall window shows, the same quiet way); the
  Settings tile shows its compact Extensions page. It never takes the keyboard,
  never brings Pitwall or a window forward, and its answer is only where
  it is and which page (never a value). The agent changes nothing.
  Extensions are refused (`Ext::Never`): their `show` permission is for
  files and pages.
- **The dialog's size.** The dialog scales to its window: about 72% of
  its width (860–1080 px) and 78% of its height (600–820 px), never under
  720 × 540 and always 24 px inside the window. Its bottom-right corner
  resizes it (both ways from its middle); the size let go is kept in
  `ui.json` as `settingsSize: [width, height]` and fitted to the window
  each time it opens.
- **Extensions there are compact.** In the dialog and in the Settings
  tile, the Extensions page is the installed list only: rows with their
  switches, the search, and **Open Extensions**. A row, an agent's
  `extension` or a secret asked for opens that extension's page in the
  Extensions tile (below); the dialog closes as it does.

## The Extensions tile

Settings → Extensions at full size, one at most, beside the other tiles:
Installed and Browse, an extension's detail page, the trust screen and
Report a problem. It is a `SettingsView` that shows only the Extensions
page and no page list (`SettingsView::extensions_surface`).

- **Opening it.** "Extensions" in the command palette, the last entry of
  the sidebar's Tiles menu (a project's ⋯), or Open Extensions on
  Settings' Extensions page. It goes where the Settings tile goes: into
  the focused tile when that is empty, else right of it; focused, with the
  keys. Already placed, it is focused where it is (its window brought
  forward when it is in one).
- **Two columns when wide.** At least 900 px wide, the list is a 360 px
  column on the left (the tabs, the search, filters and order, the rows or
  the index's cards) and the page picked in it is on the right (nothing
  picked: what extensions are and those Pitwall ships, or "Pick an
  extension"). Narrower, one column as in Settings: the list, and a detail
  page in its place with a way back.
- **A tile like the others.** Its bar drags it; Move…, Maximise (⌘⏎), Open
  in a window of its own and Close, as the Settings tile.
- **Kept.** The pane is saved with `"extensions": true` in `ui.json`; in a
  window of its own, `extensionsWindow: true`; its key is `@extensions`.
  After a restart it is back where it was.
- **One source of truth.** What it shows is the app's: what is installed
  and on (`ui.json`), grants, secrets, the index. The compact pages hand
  their detail pages, trust screens and asked-for secrets to it rather
  than holding a second copy of a setup half done.

## The editor tile

A text editor in a tile of its own, one at most, beside the other tiles:
the pane's `editor: true`, or `editorWindow` in a window of its own; its
open files are `ui.json`'s `editor` (`tabs`, `active`). It moves,
maximises and goes to a window of its own as the Settings tile does.
See [editor.md](editor.md).
