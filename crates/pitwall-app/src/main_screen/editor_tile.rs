//! The text editor in a tile of the layout (docs/spec/editor.md), a peer of
//! the Settings and device tiles: one [`Editor`] (its files in tabs),
//! placed, moved, maximised and put in a window of its own as an inbox is
//! ([`super::inbox_window`], which takes its key, [`EDITOR_KEY`]); back
//! after a restart with its files (`ui.json`: the pane's `editor`, or
//! `editorWindow`; the files are `editor`). Opened from the palette ("New
//! file", "Open file in editor…") and the files viewer ("Open in editor").
//! Closing it asks about unsaved edits first, as closing a file does.

use gpui::{div, prelude::*, px, AnyElement, App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton, Pixels, Point, SharedString, Subscription, Window};

use super::files_tile::TileDrag;
use super::tree::{Content, Surface, EDITOR_KEY};
use super::workspace::FilesSpot;
use super::MainScreen;
use crate::editor::{Editor, EditorEvent};
use crate::explorer::dock::FilesPlace;
use crate::kit;
use crate::theme::{theme, Theme};

type PlaceMenu = (Point<Pixels>, Vec<(String, String)>);

pub struct EditorTile {
    editor: Entity<Editor>,
    in_window: bool,
    pinned: bool,
    menu: Option<PlaceMenu>,
    _subs: Vec<Subscription>,
}

impl Focusable for EditorTile {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.read(cx).focus_handle(cx)
    }
}

impl EditorTile {
    fn new(cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| Editor::new(true, cx));
        let store = crate::ui_state::store(cx);
        let subs = vec![cx.observe(&store, |_, _, cx| cx.notify()), cx.subscribe(&editor, |_, _, _: &EditorEvent, cx| cx.notify()), cx.observe(&editor, |_, _, cx| cx.notify())];
        EditorTile { editor, in_window: false, pinned: false, menu: None, _subs: subs }
    }

    /// Who draws it now (a window of its own, or a tile).
    pub fn held(&mut self, in_window: bool, cx: &mut Context<Self>) {
        if self.in_window != in_window {
            self.in_window = in_window;
            self.menu = None;
            cx.notify();
        }
    }

    pub fn editor(&self) -> Entity<Editor> {
        self.editor.clone()
    }

    fn header(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let in_window = self.in_window;
        let what = Content::Editor;
        let maximized = {
            let ui = crate::ui_state::get(cx);
            ui.tile_of(&what).is_some_and(|(space, pane)| ui.space(&space).is_some_and(|s| s.maximized_pane_id.as_deref() == Some(pane.as_str())))
        };
        let btn = |name: &str, icon: &str, tip: String, on: Option<bool>| {
            let sel = format!("editor-tile-{name}");
            kit::icon_btn_state(sel.clone(), icon, tip, true, 13., on, t).debug_selector(move || sel.clone()).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let ed = self.editor.read(cx);
        let title = ed.active_path().map(pitwall_core::paths::tildify).unwrap_or_else(|| "Editor".to_string());
        let name: SharedString = "Editor".into();
        let files = self.editor.update(cx, |e, cx| e.file_buttons(t, cx));
        div()
            .id("editor-tile-bar")
            .debug_selector(|| "editor-tile-bar".into())
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(4.))
            .pr(px(8.))
            .border_b_1()
            .border_color(if t.is_glass() { t.g.line } else { t.line })
            .when(!in_window, |d| d.on_drag(TileDrag { what: what.clone(), name: name.clone() }, |d, grab, w, cx| d.ghost(grab, w, cx)))
            .child(
                div()
                    .id("editor-tile-grip")
                    .debug_selector(|| "editor-tile-grip".into())
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .tooltip(kit::tooltip(if in_window { "Drag into a Pitwall window to place it in the layout" } else { "Drag to move the editor" }))
                    .when(in_window, |d| {
                        d.on_mouse_down(MouseButton::Left, cx.listener(|_, _, window, cx| {
                            cx.stop_propagation();
                            window.defer(cx, |window, cx| super::inbox_window::drag_start(EDITOR_KEY, window, cx));
                        }))
                    })
                    .child(kit::icon("grip", 13., t.text_4)),
            )
            .child(kit::icon("file", 15., t.text_2))
            .child(div().flex_1().min_w_0().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(kit::one_line(title)))
            .child(files)
            .child(
                btn(if in_window { "dock" } else { "move" }, "grid", (if in_window { "Place it in a Pitwall window's layout" } else { "Move it in the layout, or to a window" }).into(), Some(self.menu.is_some())).on_click(cx.listener(|v, e: &gpui::ClickEvent, _, cx| {
                    v.menu = match v.menu {
                        Some(_) => None,
                        None => Some((e.position(), super::inbox_window::replaceable(EDITOR_KEY, cx))),
                    };
                    cx.notify();
                })),
            )
            .map(|d| {
                if in_window {
                    d.when(crate::platform::can_keep_on_top(), |d| {
                        let pinned = self.pinned;
                        d.child(btn("pin", "pin", (if pinned { "Kept on top of other windows: click to let it go behind them" } else { "Keep on top of other windows" }).into(), Some(pinned)).on_click(cx.listener(|v, _, window, cx| {
                            v.pinned = !v.pinned;
                            crate::platform::keep_on_top(window, v.pinned);
                            cx.notify();
                        })))
                    })
                } else {
                    d.child(btn("maximize", if maximized { "restore" } else { "maximize" }, kit::keys(if maximized { "Restore (⌘⏎)" } else { "Maximise (⌘⏎)" }, false), None).on_click(cx.listener(|v, _, window, cx| {
                        let f = v.editor.read(cx).focus_handle(cx);
                        window.focus(&f, cx);
                        window.dispatch_action(Box::new(super::ToggleMaximize), cx);
                    })))
                    .child(btn("undock", "window", "Open in a window of its own".into(), None).on_click(cx.listener(|_, _, _, cx| {
                        cx.defer(|cx| {
                            if let Err(e) = super::inbox_window::pop_out(EDITOR_KEY, None, true, cx) {
                                eprintln!("pitwall: could not open the editor's window: {e}");
                            }
                        });
                    })))
                    .child(btn("close", "x", "Close the editor".into(), None).on_click(cx.listener(|v, _, window, cx| v.request_close(window, cx))))
                }
            })
            .into_any_element()
    }

    /// Close the editor: asks about unsaved edits first.
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.guard(None, window, cx, |_, _, cx| cx.defer(close)));
    }

    fn place_menu(&self, at: Point<Pixels>, replace: Vec<(String, String)>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        let pick = move |p: Option<(FilesPlace, Option<String>)>, cx: &mut App| {
            let _ = me.update(cx, |v, cx| {
                v.menu = None;
                cx.notify();
            });
            if let Some((place, of)) = p {
                cx.defer(move |cx| {
                    if let Err(e) = super::inbox_window::place(EDITOR_KEY, place, of.as_deref(), cx) {
                        eprintln!("pitwall: could not move the editor: {e}");
                    }
                });
            }
        };
        crate::explorer::dock::menu(at, !self.in_window, None, replace, "the editor", t, pick)
    }
}

impl Render for EditorTile {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let menu = self.menu.clone().map(|(at, replace)| self.place_menu(at, replace, &t, cx));
        div()
            .id("editor-tile")
            .debug_selector(|| "editor-tile".into())
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .text_color(t.text)
            .on_mouse_down(MouseButton::Left, cx.listener(|v, _, window, cx| {
                if window.default_prevented() {
                    return;
                }
                let f = v.editor.read(cx).focus_handle(cx);
                if !f.contains_focused(window, cx) {
                    window.focus(&f, cx);
                }
                window.prevent_default();
            }))
            .child(self.header(&t, cx))
            .child(div().id("editor-tile-body").flex_1().min_w_0().min_h_0().flex().child(self.editor.clone()))
            .children(menu)
    }
}

// ── wherever it is ───────────────────────────────────────────────────────

/// The one editor tile, wherever it is drawn.
#[derive(Default)]
pub(super) struct Tile(pub(super) Option<Entity<EditorTile>>);

impl gpui::Global for Tile {}

/// The editor tile (made when it isn't yet: the files `ui.json` keeps open
/// again).
pub fn view(cx: &mut App) -> Entity<EditorTile> {
    if let Some(v) = get(cx) {
        return v;
    }
    let v = cx.new(EditorTile::new);
    cx.default_global::<Tile>().0 = Some(v.clone());
    v
}

/// The editor tile, if it is made.
pub fn get(cx: &App) -> Option<Entity<EditorTile>> {
    cx.try_global::<Tile>()?.0.clone()
}

/// The editor, if it is made.
pub fn editor(cx: &App) -> Option<Entity<Editor>> {
    get(cx).map(|t| t.read(cx).editor())
}

/// Whether the editor is in a tile of a layout or a window of its own.
pub fn placed(cx: &App) -> bool {
    let ui = crate::ui_state::get(cx);
    ui.tile_of(&Content::Editor).is_some() || ui.surface_in_window(&Surface::Editor)
}

/// The editor closed (its edits dropped: the caller asked): out of every
/// layout, its window closed, its files forgotten.
pub fn close(cx: &mut App) {
    crate::ui_state::update(cx, |ui| {
        *ui = ui.clone().close_surface(&Surface::Editor);
        ui.editor = Default::default();
    });
    super::inbox_window::close_for_move(EDITOR_KEY, cx);
    if cx.has_global::<Tile>() {
        cx.global_mut::<Tile>().0 = None;
    }
}

/// Its window's close button: asks about unsaved edits first; `true` when
/// the window can close now.
pub fn window_may_close(window: &mut Window, cx: &mut App) -> bool {
    let Some(ed) = editor(cx) else { return true };
    if ed.read(cx).unsaved(cx).is_empty() {
        return true;
    }
    ed.update(cx, |e, cx| e.guard(None, window, cx, |_, _, cx| cx.defer(close)));
    false
}

/// Open `path` (full) in the editor, at `line`, the editor placed in the
/// frontmost Pitwall window when it isn't anywhere yet. Runs outside every
/// window's update.
pub fn open_path(path: &str, line: Option<u32>, cx: &mut App) {
    let path = path.to_string();
    if !placed(cx) {
        if let Some(h) = crate::windows::active_or_main(cx) {
            let _ = h.update(cx, |v, window, cx| {
                if let Some(s) = v.screen() {
                    s.update(cx, |s, cx| s.open_editor_tile(window, cx));
                }
            });
        }
    } else {
        reveal(cx);
    }
    let Some(ed) = editor(cx) else { return };
    ed.update(cx, |e, cx| e.open_at(&path, line, cx));
}

/// The editor shown and focused where it is.
pub fn reveal(cx: &mut App) -> bool {
    if !placed(cx) {
        return false;
    }
    let f = view(cx).read(cx).focus_handle(cx);
    if let Some(h) = crate::windows::inbox_window(EDITOR_KEY, cx) {
        let _ = h.update(cx, |_, window, cx| {
            if !crate::bench::enabled() {
                window.activate_window();
            }
            window.focus(&f, cx);
        });
        return true;
    }
    let ui = crate::ui_state::get(cx);
    let Some((space, pane)) = ui.tile_of(&Content::Editor) else { return false };
    let Some(h) = crate::windows::main_window(ui.window_of_space(&space), cx) else { return false };
    let _ = h.update(cx, |view, window, cx| {
        if !crate::bench::enabled() {
            window.activate_window();
        }
        if let Some(s) = view.screen() {
            s.update(cx, |s, cx| s.focus_settings_tile(&space, &pane, cx));
        }
        window.focus(&f, cx);
    });
    true
}

impl MainScreen {
    /// The editor tile focused where it is, else placed as Settings' is
    /// (right of the focused tile, in it when it is empty).
    pub fn open_editor_tile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let v = view(cx);
        let f = v.read(cx).focus_handle(cx);
        if let Some(h) = crate::windows::inbox_window(EDITOR_KEY, cx) {
            cx.defer(move |cx| {
                let _ = h.update(cx, |_, window, cx| {
                    if !crate::bench::enabled() {
                        window.activate_window();
                    }
                    window.focus(&f, cx);
                });
            });
            return;
        }
        match self.ui.tile_of(&Content::Editor) {
            Some((space, pane)) if self.ui.window_of_space(&space) == self.label => self.focus_settings_tile(&space, &pane, cx),
            _ => {
                let Some(space) = self.active_space().cloned() else { return };
                let anchor = space.focused_pane_id.clone().and_then(|p| space.layout.find_pane(&p));
                let spot = match anchor {
                    Some(p) if p.is_empty() => FilesSpot::Take(p.id),
                    Some(p) => FilesSpot::Tile { pane: p.id, side: super::tree::Side::Right },
                    None => FilesSpot::Auto,
                };
                let fit = self.fit_for(&space.id);
                let mut placed = false;
                self.update_ui(
                    |u| {
                        let (next, at) = u.surface_to_layout(&space.id, &Surface::Editor, &spot, fit);
                        placed = at.is_some();
                        match at {
                            Some(pane) => next.focus_pane(&space.id, &pane),
                            None => next,
                        }
                    },
                    cx,
                );
                if !placed {
                    return;
                }
            }
        }
        window.focus(&f, cx);
        cx.notify();
    }

    /// "New file" in the palette: the editor with a new file.
    pub fn new_file_in_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_editor_tile(window, cx);
        if let Some(ed) = editor(cx) {
            ed.update(cx, |e, cx| {
                e.new_file(cx);
                e.focus_active(window, cx);
            });
        }
    }

    /// "Open file in editor…" in the palette: the system's open panel.
    pub fn pick_files_for_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_editor_tile(window, cx);
        if let Some(ed) = editor(cx) {
            ed.update(cx, |e, cx| e.pick_files(cx));
        }
    }

    /// Tile `pane` showing the editor.
    pub(super) fn render_editor_pane(&mut self, pane: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let v = view(cx);
        v.update(cx, |v, cx| v.held(false, cx));
        let keys = Some(v.read(cx).focus_handle(cx));
        let body = div().flex_1().min_w_0().min_h_0().flex().child(v).into_any_element();
        self.tile_shell(pane, Content::Editor, "editor-tile-pane".into(), body, keys, t, cx)
    }

    /// Bench and debug builds, `editor …` (nothing clicked or typed on the
    /// screen): `open <full path> [line]`, `new`, `type <text>` (at every
    /// cursor, as typed), `key <keystroke>` (an editor key: save, save-all,
    /// undo, redo, comment, next-match, all-matches, cursor-below, close),
    /// `goto <line[:col]>`, `save-to <full path>`, `keep-mine`,
    /// `take-theirs`, `difference`, `close` (the tile), `max`, `state` (on
    /// stderr).
    pub fn debug_editor(&mut self, cmd: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (verb, rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
        let rest = rest.trim().to_string();
        let ed = || editor(cx);
        match verb {
            "open" => {
                self.open_editor_tile(window, cx);
                let (p, line) = match rest.rsplit_once(' ') {
                    Some((p, l)) if l.parse::<u32>().is_ok() => (p.to_string(), l.parse().ok()),
                    _ => (rest.clone(), None),
                };
                let p = crate::editor::full_path(&p).to_string_lossy().into_owned();
                if let Some(e) = editor(cx) {
                    e.update(cx, |e, cx| e.open_at(&p, line, cx));
                }
            }
            "new" => self.new_file_in_editor(window, cx),
            "type" => {
                if let Some(code) = ed().and_then(|e| e.read(cx).active_code()) {
                    let text = rest.replace("\\n", "\n");
                    code.update(cx, |c, cx| c.debug_type(&text, cx));
                }
            }
            "key" => {
                if let Some(e) = ed() {
                    e.update(cx, |e, cx| e.debug_key(&rest, window, cx));
                }
            }
            "goto" => {
                if let Some(e) = ed() {
                    e.update(cx, |e, cx| e.go_to(&rest, cx));
                }
            }
            "save-to" => {
                if let Some(e) = ed() {
                    let to = crate::editor::full_path(&rest);
                    e.update(cx, |e, cx| {
                        if let Some(id) = e.active_id() {
                            e.save_to(id, &to, cx).detach();
                        }
                    });
                }
            }
            "keep-mine" => {
                if let Some(e) = ed() {
                    e.update(cx, |e, cx| e.keep_mine(cx));
                }
            }
            "take-theirs" => {
                if let Some(e) = ed() {
                    e.update(cx, |e, cx| e.take_theirs(cx));
                }
            }
            "difference" => {
                if let Some(e) = ed() {
                    e.update(cx, |e, cx| e.toggle_difference(cx));
                }
            }
            "close" => cx.defer(close),
            "max" => {
                if let Some((space, pane)) = self.ui.tile_of(&Content::Editor) {
                    self.update_ui(|s| s.toggle_maximize(&space, Some(&pane)), cx);
                    cx.notify();
                }
            }
            "state" => {
                let s = ed().map(|e| e.read(cx).debug_state(cx)).unwrap_or_else(|| "none".into());
                eprintln!("pitwall: editor: placed {}, at {}, {s}", placed(cx), super::inbox_window::described(EDITOR_KEY, cx));
            }
            _ => {}
        }
        cx.notify();
    }
}
