# Built in-house

Zed's crates other than `gpui` (`ui`, `theme`, `editor`, `terminal`,
`terminal_view`, `workspace`, `picker`, `menu`, `file_icons`, …) are
**GPL-3.0**. Pitwall is Apache-2.0, so it never depends on them and never
copies or closely paraphrases their code. Whatever they would provide is
written here, **clean-room**: from GPUI's public API, its docs and examples
(Apache-2.0), and Pitwall's own React UI as the behavioural spec. **No
feature is dropped for licensing reasons.**

`gpui-component` (Apache-2.0) is allowed. Releases up to 0.5.1 build on
the crates.io `gpui` 0.2.2 Pitwall pins; 0.6 and later build on `gpui-pre`
([licensing.md](licensing.md)). Phase 1 decides per component whether to
take it from `gpui-component` 0.5.1 (after a licence check of that release's
whole dependency tree) or write it here; the table below is the plan when
written here, and the behaviour spec either way.

Components live in `crates/pitwall-app/src/kit/` (one file each), styled
only through `theme.rs` tokens, each with `TestAppContext` /
`VisualTestContext` tests and, from phase 8, accessibility roles and labels
next to it (README "Accessibility gap").

| Component | Design note | Needed by (inventory) | Phase |
|---|---|---|---|
| Basic primitives (button, icon button, label, chip, kbd) | `RenderOnce` elements over `div()`; variants as enums; hover/active/disabled from tokens; icon button has a required label for tooltips and accessibility | everywhere | 1 |
| Checkbox / toggle | `RenderOnce` with `checked` + `on_change`; space/enter toggle when focused | §10, §12, §14, §18–§21, §26 | 1 |
| Text input (single and multi-line) | Own entity implementing GPUI's `EntityInputHandler` (IME marked text, selection ranges), `FocusHandle`, clipboard via `cx.write_to_clipboard`, undo stack, word motion; line layout with `window.text_system().shape_line`; multi-line wraps with `WrappedLine`. GPUI's `examples/input.rs` (Apache-2.0) is the reference | §3, §10, §13, §14, §17, §18, §20, §21, §23 | 1 |
| Scroll view | `overflow_y_scroll` + `ScrollHandle`; overlay scrollbar drawn from the handle's offset, fades unless hovered; keyboard paging | §4, §9–§14, §19, §20 | 1 |
| List / virtual list | `uniform_list` for equal rows (agents, files), `list` with `ListState` for variable rows; keyboard selection model shared by sidebar, palette and pickers | §4, §9, §10, §12, §14, §15, §17, §18, §20, §21, §23 | 1 |
| Popover / context menu | `anchored()` + `deferred()` overlay positioned at the trigger or pointer, flips at window edges; menu items are actions so the keymap shows their shortcuts; Esc and outside click dismiss | §4, §13, §15, §23 | 1 |
| Tooltip | `.tooltip()` with a small view; delay and placement from tokens | most sections | 1 |
| Modal / dialog | One modal layer per window (a stack in `MainView`); backdrop token, focus trapped inside, Esc = `Dismiss` action, confirm variant | §11, §15, §18–§22, §26 | 1 |
| Select / dropdown, radio group, segmented control | Built on popover + list; segmented control is a row of toggle buttons | §5, §12, §14, §18–§21 | 1 |
| Tabs | Tab strip element: overflow scroll, close buttons, drag to reorder (with drag and drop) | §2, §3, §10, §14, §23 | 1 |
| Toast | `Toasts` entity (global per window): stack, auto-dismiss timers on the foreground executor, Reduce motion aware | §3, §7, §14, §16, §18, §20 | 1 |
| Keybinding label | Renders a `KeyBinding` per desktop (⌘K vs Ctrl+Shift+K), from the same bindings the keymap uses | §2, §23 | 1 |
| Title bar | macOS: transparent title bar with traffic lights at a fixed position; Windows/Linux: drawn title bar with window controls, drag region (`WindowControlArea`) and the in-window app menu (Settings, Quit, Quit and Stop Agents) | §2 | 1 |
| Theme | `Theme` global from tokens.css (phase 0 done), density and Glass tiers as globals | §13, §19, §24 | 0–6 |
| Icons, file icons | SVG assets (`AssetSource`), `svg()` element tinted from tokens; file-icon lookup from the existing MIT Material Icon Theme subset map (`src/lib/fileIcons.map.json`) | §14, §24 | 1, 5 |
| Pane / split layout | Pitwall's own split tree (the model in `src/lib/tiling.ts`) rendered with flex and draggable dividers; maximise and fold-to-chips are model operations; no Zed `workspace` concepts | §5, §23 | 2 |
| Drag and drop | GPUI `on_drag` / `on_drop` with typed payloads (`AgentDrag`, `TabDrag`), a drag ghost view, drop-zone highlighting computed from the split tree | §3–§5, §7, §9 | 2 |
| Drawer / overlay | Absolute-positioned panel with scrim, slide animation (off with Reduce motion), closes on breakpoint change | §1, §4 | 2 |
| Command palette | Modal + text input + virtual list; fuzzy scoring written in Pitwall (or `nucleo-matcher`, MPL-2.0: avoided; a small subsequence scorer is enough) | §14, §17 | 2 |
| File tree | Lazy tree model (expand loads children through `ops::explorer`), virtual list of visible rows, status letters and icons | §10, §12, §14, §15, §23 | 4–5 |
| Code / diff viewer | Read-only viewer: own line model over a rope-free `Vec<Line>` (files are read-only), `uniform_list` rows with `StyledText` runs, gutter (line numbers, change markers, comment gutter), side-by-side and inline diff from the core's hunks, folding of unchanged regions, find (Cmd/Ctrl+F) with match highlights, overview ruler. Syntax highlighting with `tree-sitter` (MIT) grammars where licences are permissive, or `syntect` (MIT) | §11, §13–§15, §23, §24 | 4 |
| Terminal view | `pitwall-term-view` (separate crate, alacritty_terminal is Apache-2.0); the Wall's view-only tiles use the same element in read-only mode | §6, §8, §9, §23 | 2–3 |

Rules for contributors:

- Read GPUI's source and examples freely (Apache-2.0). Do not open Zed's GPL
  crates while writing a component; use the React component and the
  inventory as the spec.
- New third-party crates only after the check in [licensing.md](licensing.md).
