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
   window", ⌘⇧N, or drag a tab out) — for a second monitor. Every window has the
   same sidebar, its own tabs, and the global "needs you" strip.

Rules that keep it simple:
- **An agent is shown in at most one pane at a time.** Clicking an agent that is
  visible elsewhere focuses that window/space/pane instead of duplicating it.
  Dragging it to another space moves it. (PTY size = its single pane's size.)
- **Tiling**: drag an agent onto a pane's left/right/top/bottom drop zone to
  split; drag dividers to resize; presets 1 / 2 / 3 (1 big + 2) / 2×2 / 3×2.
  Double-click a pane header (or ⌘⏎) to maximise it within the space and back.
- **Minimum pane size** ~80 cols × 20 rows at the current font; when a space has
  more agents than fit, extra panes collapse into a compact strip of agent chips
  at the bottom of the space (click to swap in) instead of becoming unreadable.
- **Right panel** (Next up / Changes / Last sent) always belongs to the focused pane.

Responsive breakpoints (per window width):
| Width | Sidebar | Right panel | Max panes per row |
|---|---|---|---|
| ≥ 2200 (ultrawide/big) | full | docked | 4 |
| 1500–2200 | full | docked | 3 |
| 1100–1500 | full, collapsible | drawer (overlay), toggle ⌘. | 2 |
| 700–1100 | icon rail (status dots + initials) | drawer | 1 (others as chips) |
| < 700 | overlay | drawer | 1 |
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
