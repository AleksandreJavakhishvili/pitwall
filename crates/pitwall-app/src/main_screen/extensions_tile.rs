//! Extensions in a tile of the layout (docs/spec/layout.md "The Extensions
//! tile"), a peer of the Settings tile: Settings → Extensions at full size
//! (Installed and Browse, their detail pages; wide, the list beside the
//! page; narrow, one column). Opened with "Extensions" in the command
//! palette, the sidebar's Tiles menu, or "Open Extensions" on Settings'
//! Extensions page; placed, moved, maximised and put in a window of its
//! own as an inbox is ([`super::inbox_window`], which takes its key,
//! [`EXTENSIONS_KEY`]); back after a restart (`ui.json`: the pane's
//! `extensions`, or `extensionsWindow`).
//!
//! One source of truth: what it shows (what is installed and on, grants,
//! secrets, the index) is the app's, as the dialog's page reads it; the
//! dialog's and the Settings tile's Extensions page is compact and hands
//! a detail page, a trust screen or an asked-for secret to this tile
//! ([`show`]), so no two pages hold the same setup half done.

use gpui::{div, prelude::*, px, AnyElement, App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton, Pixels, Point, SharedString, Subscription, Window};

use super::files_tile::TileDrag;
use super::tree::{Content, Surface, EXTENSIONS_KEY};
use super::workspace::FilesSpot;
use super::MainScreen;
use crate::explorer::dock::FilesPlace;
use crate::kit;
use crate::settings::view::{SettingsEvent, SettingsView};
use crate::theme::{theme, Theme};

type PlaceMenu = (Point<Pixels>, Vec<(String, String)>);

pub struct ExtensionsTile {
    page: Entity<SettingsView>,
    in_window: bool,
    pinned: bool,
    menu: Option<PlaceMenu>,
    _subs: Vec<Subscription>,
}

impl Focusable for ExtensionsTile {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.page.read(cx).focus_handle(cx)
    }
}

impl ExtensionsTile {
    fn new(cx: &mut Context<Self>) -> Self {
        let page = cx.new(SettingsView::extensions_surface);
        let store = crate::ui_state::store(cx);
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe(&page, |_, _, e: &SettingsEvent, cx| {
                if let SettingsEvent::Close = e {
                    cx.defer(close)
                }
            }),
        ];
        ExtensionsTile { page, in_window: false, pinned: false, menu: None, _subs: subs }
    }

    /// Who draws it now (a window of its own, or a tile).
    pub fn held(&mut self, in_window: bool, cx: &mut Context<Self>) {
        if self.in_window != in_window {
            self.in_window = in_window;
            self.menu = None;
            cx.notify();
        }
    }

    /// The page it draws.
    pub fn page(&self) -> Entity<SettingsView> {
        self.page.clone()
    }

    fn header(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let in_window = self.in_window;
        let what = Content::Extensions;
        let maximized = {
            let ui = crate::ui_state::get(cx);
            ui.tile_of(&what).is_some_and(|(space, pane)| ui.space(&space).is_some_and(|s| s.maximized_pane_id.as_deref() == Some(pane.as_str())))
        };
        let btn = |name: &str, icon: &str, tip: String, on: Option<bool>| {
            let sel = format!("extensions-tile-{name}");
            kit::icon_btn_state(sel.clone(), icon, tip, true, 13., on, t).debug_selector(move || sel.clone()).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let name: SharedString = "Extensions".into();
        div()
            .id("extensions-tile-bar")
            .debug_selector(|| "extensions-tile-bar".into())
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
                    .id("extensions-tile-grip")
                    .debug_selector(|| "extensions-tile-grip".into())
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .tooltip(kit::tooltip(if in_window { "Drag into a Pitwall window to place it in the layout" } else { "Drag to move Extensions" }))
                    .when(in_window, |d| {
                        d.on_mouse_down(MouseButton::Left, cx.listener(|_, _, window, cx| {
                            cx.stop_propagation();
                            window.defer(cx, |window, cx| super::inbox_window::drag_start(EXTENSIONS_KEY, window, cx));
                        }))
                    })
                    .child(kit::icon("grip", 13., t.text_4)),
            )
            .child(kit::icon("extension", 15., t.text_2))
            .child(div().flex_1().min_w_0().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(kit::one_line("Extensions".to_string())))
            .child(
                btn(if in_window { "dock" } else { "move" }, "grid", (if in_window { "Place it in a Pitwall window's layout" } else { "Move it in the layout, or to a window" }).into(), Some(self.menu.is_some())).on_click(cx.listener(|v, e: &gpui::ClickEvent, _, cx| {
                    v.menu = match v.menu {
                        Some(_) => None,
                        None => Some((e.position(), super::inbox_window::replaceable(EXTENSIONS_KEY, cx))),
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
                        let f = v.page.read(cx).focus_handle(cx);
                        window.focus(&f, cx);
                        window.dispatch_action(Box::new(super::ToggleMaximize), cx);
                    })))
                    .child(btn("undock", "window", "Open in a window of its own".into(), None).on_click(cx.listener(|_, _, _, cx| {
                        cx.defer(|cx| {
                            if let Err(e) = super::inbox_window::pop_out(EXTENSIONS_KEY, None, true, cx) {
                                eprintln!("pitwall: could not open Extensions' window: {e}");
                            }
                        });
                    })))
                    .child(btn("close", "x", "Close Extensions".into(), None).on_click(cx.listener(|_, _, _, cx| cx.defer(close))))
                }
            })
            .into_any_element()
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
                    if let Err(e) = super::inbox_window::place(EXTENSIONS_KEY, place, of.as_deref(), cx) {
                        eprintln!("pitwall: could not move Extensions: {e}");
                    }
                });
            }
        };
        crate::explorer::dock::menu(at, !self.in_window, None, replace, "Extensions", t, pick)
    }
}

impl Render for ExtensionsTile {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let menu = self.menu.clone().map(|(at, replace)| self.place_menu(at, replace, &t, cx));
        div()
            .id("extensions-tile")
            .debug_selector(|| "extensions-tile".into())
            .key_context("ExtensionsTile")
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
                let f = v.page.read(cx).focus_handle(cx);
                if !f.contains_focused(window, cx) {
                    window.focus(&f, cx);
                }
                window.prevent_default();
            }))
            .child(self.header(&t, cx))
            .child(div().id("extensions-tile-body").flex_1().min_w_0().min_h_0().flex().child(self.page.clone()))
            .children(menu)
    }
}

// ── wherever it is ───────────────────────────────────────────────────────

/// The one Extensions tile, wherever it is drawn.
#[derive(Default)]
struct Tile(Option<Entity<ExtensionsTile>>);

impl gpui::Global for Tile {}

/// The Extensions tile (made when it isn't yet).
pub fn view(cx: &mut App) -> Entity<ExtensionsTile> {
    if let Some(v) = get(cx) {
        return v;
    }
    let v = cx.new(ExtensionsTile::new);
    cx.default_global::<Tile>().0 = Some(v.clone());
    v
}

/// The Extensions tile, if it is made.
pub fn get(cx: &App) -> Option<Entity<ExtensionsTile>> {
    cx.try_global::<Tile>()?.0.clone()
}

/// Whether Extensions is in a tile of a layout or a window of its own.
pub fn placed(cx: &App) -> bool {
    let ui = crate::ui_state::get(cx);
    ui.tile_of(&Content::Extensions).is_some() || ui.surface_in_window(&Surface::Extensions)
}

/// The Extensions tile closed: out of every layout, its window closed,
/// its view let go.
pub fn close(cx: &mut App) {
    crate::ui_state::update(cx, |ui| *ui = ui.clone().close_surface(&Surface::Extensions));
    super::inbox_window::close_for_move(EXTENSIONS_KEY, cx);
    if cx.has_global::<Tile>() {
        cx.global_mut::<Tile>().0 = None;
    }
}

/// The Extensions tile opened (or shown where it is) from the Pitwall
/// window in front, then `then` run on its page: a detail page opened, a
/// secret asked for. Runs outside every window's update.
pub fn show(then: impl FnOnce(&mut SettingsView, &mut Context<SettingsView>) + 'static, cx: &mut App) {
    if let Some(h) = crate::windows::active_or_main(cx) {
        let _ = h.update(cx, |view, window, cx| {
            if let Some(s) = view.screen() {
                s.update(cx, |s, cx| s.open_extensions_tile(window, cx));
            }
        });
    }
    let page = view(cx).read(cx).page();
    page.update(cx, then);
}

/// [`show`] for what the user didn't press (an agent's `show_settings`
/// naming an extension, a tile's `settings.open`): placed where it is,
/// else in the space the frontmost Pitwall window shows, without taking
/// the keyboard or bringing anything forward.
pub fn show_quietly(then: impl FnOnce(&mut SettingsView, &mut Context<SettingsView>) + 'static, cx: &mut App) {
    view(cx);
    if !placed(cx) {
        match crate::web::place::spot_near(&Content::Extensions, FilesPlace::Own, None, cx) {
            Ok((label, space, spot)) => {
                if let Err(e) = super::inbox_window::dock(EXTENSIONS_KEY, &label, &space, &spot, cx) {
                    eprintln!("pitwall: could not place Extensions: {e}");
                }
            }
            Err(e) => eprintln!("pitwall: could not place Extensions: {e}"),
        }
    }
    let page = view(cx).read(cx).page();
    page.update(cx, then);
}

impl MainScreen {
    /// "Extensions" in the palette or the sidebar's Tiles menu, "Open
    /// Extensions" in Settings: the Extensions tile focused where it is
    /// (its window brought forward when it is in one), else placed as an
    /// inbox is (right of the focused tile, in it when it is empty).
    pub fn open_extensions_tile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let v = view(cx);
        let f = v.read(cx).focus_handle(cx);
        if let Some(h) = crate::windows::inbox_window(EXTENSIONS_KEY, cx) {
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
        match self.ui.tile_of(&Content::Extensions) {
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
                        let (next, at) = u.surface_to_layout(&space.id, &Surface::Extensions, &spot, fit);
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

    /// Tile `pane` showing Extensions: a tile like the inbox's.
    pub(super) fn render_extensions_pane(&mut self, pane: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let v = view(cx);
        v.update(cx, |v, cx| v.held(false, cx));
        let keys = Some(v.read(cx).focus_handle(cx));
        let body = div().flex_1().min_w_0().min_h_0().flex().child(v).into_any_element();
        self.tile_shell(pane, Content::Extensions, "extensions-tile-pane".into(), body, keys, t, cx)
    }
}
