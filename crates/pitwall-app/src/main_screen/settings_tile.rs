//! Settings in a tile of the layout (docs/spec/layout.md "The Settings
//! tile"), a peer of the inbox and view tiles: the same Settings as the
//! dialog (one [`SettingsView`], every page, its page list inside the
//! tile), at the tile's full size. Opened with the dialog's "Open as a
//! tile", "Open Settings as a tile" in the command palette, or an agent's
//! `show_settings` ([`show_for_agent`]); placed, moved, maximised and put
//! in a window of its own as an inbox is ([`super::inbox_window`], which
//! takes its key, [`SETTINGS_KEY`]); back after a restart (`ui.json`: the
//! pane's `settings`, or `settingsWindow`; the page is `settingsPage`, as
//! the dialog's).
//!
//! One Settings at a time: the dialog's "Open as a tile" hands its view
//! (unsaved fields and all) to the tile and closes; opening the tile any
//! other way closes a dialog left open; and while the tile is there,
//! Settings' shortcut (and the palette's "Settings: <page>") focuses it
//! instead of opening the dialog ([`reveal`]). So there is never a
//! second copy of Settings' fields to diverge.

use gpui::{div, prelude::*, px, AnyElement, App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton, Pixels, Point, SharedString, Subscription, Window};
use pitwall_proto::settings::Page;
use pitwall_proto::{code, ErrorBody, SettingsShow, SettingsShown};

use super::files_tile::TileDrag;
use super::tree::{Content, Surface, SETTINGS_KEY};
use super::workspace::FilesSpot;
use super::MainScreen;
use crate::explorer::dock::FilesPlace;
use crate::kit;
use crate::settings::view::{SettingsEvent, SettingsView};
use crate::theme::{theme, Theme};

type PlaceMenu = (Point<Pixels>, Vec<(String, String)>);

/// The setting that lets agents show Settings.
pub const SETTING: &str = "agents.showSettings";

/// The setting's value (on unless turned off).
pub fn enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(SETTING)
        .and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

pub struct SettingsTile {
    settings: Entity<SettingsView>,
    in_window: bool,
    pinned: bool,
    menu: Option<PlaceMenu>,
    _subs: Vec<Subscription>,
}

impl Focusable for SettingsTile {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.settings.read(cx).focus_handle(cx)
    }
}

impl SettingsTile {
    /// The tile, drawing `settings` (the dialog's, handed over) or a new
    /// Settings. Not tied to a window: drawn wherever Settings is.
    fn new(settings: Option<Entity<SettingsView>>, cx: &mut Context<Self>) -> Self {
        // A tile is in a window with agents (as the dialog over one).
        let settings = settings.unwrap_or_else(|| cx.new(|cx| SettingsView::new(true, cx)));
        settings.update(cx, |v, cx| v.set_tiled(true, cx));
        let store = crate::ui_state::store(cx);
        let subs = vec![
            // Maximised or not, where it is: its bar follows.
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe(&settings, |_, _, e: &SettingsEvent, cx| match e {
                SettingsEvent::Close => cx.defer(close),
                // "Scan again": the rescan screen over the window in front,
                // in place of Settings.
                SettingsEvent::ScanAgain => cx.defer(|cx| {
                    close(cx);
                    if let Some(h) = crate::windows::active_or_main(cx) {
                        let _ = h.update(cx, |v, _, cx| v.open_rescan(cx));
                    }
                }),
                // An account's login started an agent: shown and focused
                // where agents are, as from the dialog.
                SettingsEvent::Started(a) => {
                    let a = (**a).clone();
                    cx.defer(move |cx| {
                        if let Some(h) = crate::windows::active_or_main(cx) {
                            let _ = h.update(cx, |v, window, cx| v.show_started(a, window, cx));
                        }
                    });
                }
                SettingsEvent::ToTile => {}
            }),
        ];
        SettingsTile { settings, in_window: false, pinned: false, menu: None, _subs: subs }
    }

    /// Who draws it now (a window of its own, or a tile).
    pub fn held(&mut self, in_window: bool, cx: &mut Context<Self>) {
        if self.in_window != in_window {
            self.in_window = in_window;
            self.menu = None;
            cx.notify();
        }
    }

    /// The Settings it draws.
    pub fn settings(&self) -> Entity<SettingsView> {
        self.settings.clone()
    }

    fn header(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let in_window = self.in_window;
        let what = Content::Settings;
        let maximized = {
            let ui = crate::ui_state::get(cx);
            ui.tile_of(&what).is_some_and(|(space, pane)| ui.space(&space).is_some_and(|s| s.maximized_pane_id.as_deref() == Some(pane.as_str())))
        };
        let btn = |name: &str, icon: &str, tip: String, on: Option<bool>| {
            let sel = format!("settings-tile-{name}");
            kit::icon_btn_state(sel.clone(), icon, tip, true, 13., on, t)
                .debug_selector(move || sel.clone())
                // A press on a control is never a drag of the tile.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let name: SharedString = "Settings".into();
        div()
            .id("settings-tile-bar")
            .debug_selector(|| "settings-tile-bar".into())
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(4.))
            .pr(px(8.))
            .border_b_1()
            .border_color(if t.is_glass() { t.g.line } else { t.line })
            // A tile: dragged in its window's layout by the bar; a window:
            // its grip drags it into a layout.
            .when(!in_window, |d| d.on_drag(TileDrag { what: what.clone(), name: name.clone() }, |d, grab, w, cx| d.ghost(grab, w, cx)))
            .child(
                div()
                    .id("settings-tile-grip")
                    .debug_selector(|| "settings-tile-grip".into())
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .tooltip(kit::tooltip(if in_window { "Drag into a Pitwall window to place it in the layout" } else { "Drag to move Settings" }))
                    .when(in_window, |d| {
                        d.on_mouse_down(MouseButton::Left, cx.listener(|_, _, window, cx| {
                            cx.stop_propagation();
                            window.defer(cx, |window, cx| super::inbox_window::drag_start(SETTINGS_KEY, window, cx));
                        }))
                    })
                    .child(kit::icon("grip", 13., t.text_4)),
            )
            .child(kit::icon("gear", 15., t.text_2))
            .child(div().flex_1().min_w_0().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(kit::one_line("Settings".to_string())))
            .child(
                btn(if in_window { "dock" } else { "move" }, "grid", (if in_window { "Place it in a Pitwall window's layout" } else { "Move it in the layout, or to a window" }).into(), Some(self.menu.is_some())).on_click(cx.listener(|v, e: &gpui::ClickEvent, _, cx| {
                    v.menu = match v.menu {
                        Some(_) => None,
                        None => Some((e.position(), super::inbox_window::replaceable(SETTINGS_KEY, cx))),
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
                        // Its tile is the space's focused one (the press made
                        // it so): ⌘⏎'s own action.
                        let f = v.settings.read(cx).focus_handle(cx);
                        window.focus(&f, cx);
                        window.dispatch_action(Box::new(super::ToggleMaximize), cx);
                    })))
                    .child(btn("undock", "window", "Open in a window of its own".into(), None).on_click(cx.listener(|_, _, _, cx| {
                        cx.defer(|cx| {
                            if let Err(e) = super::inbox_window::pop_out(SETTINGS_KEY, None, true, cx) {
                                eprintln!("pitwall: could not open Settings' window: {e}");
                            }
                        });
                    })))
                    .child(btn("close", "x", "Close Settings".into(), None).on_click(cx.listener(|_, _, _, cx| cx.defer(close))))
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
                    if let Err(e) = super::inbox_window::place(SETTINGS_KEY, place, of.as_deref(), cx) {
                        eprintln!("pitwall: could not move Settings: {e}");
                    }
                });
            }
        };
        crate::explorer::dock::menu(at, !self.in_window, None, replace, "Settings", t, pick)
    }
}

impl Render for SettingsTile {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let menu = self.menu.clone().map(|(at, replace)| self.place_menu(at, replace, &t, cx));
        div()
            .id("settings-tile")
            .debug_selector(|| "settings-tile".into())
            .key_context("SettingsTile")
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .text_color(t.text)
            // A press anywhere in it (its bar, a page's margin): Settings
            // has the keys, not the screen around it (which takes a press
            // nothing inside took).
            .on_mouse_down(MouseButton::Left, cx.listener(|v, _, window, cx| {
                if window.default_prevented() {
                    return;
                }
                let f = v.settings.read(cx).focus_handle(cx);
                if !f.contains_focused(window, cx) {
                    window.focus(&f, cx);
                }
                window.prevent_default();
            }))
            .child(self.header(&t, cx))
            .child(div().id("settings-tile-body").flex_1().min_w_0().min_h_0().flex().child(self.settings.clone()))
            .children(menu)
    }
}

// ── wherever it is ───────────────────────────────────────────────────────

/// The one Settings tile, wherever it is drawn (a tile of any Pitwall
/// window's layout, or a window of its own): moving it keeps it.
#[derive(Default)]
struct Tile(Option<Entity<SettingsTile>>);

impl gpui::Global for Tile {}

/// The Settings tile (made when it isn't yet, from `handed`: the dialog's
/// view, when it is handed over).
pub fn view_with(handed: Option<Entity<SettingsView>>, cx: &mut App) -> Entity<SettingsTile> {
    if let Some(v) = get(cx) {
        return v;
    }
    let v = cx.new(|cx| SettingsTile::new(handed, cx));
    cx.default_global::<Tile>().0 = Some(v.clone());
    v
}

/// The Settings tile (made when it isn't yet).
pub fn view(cx: &mut App) -> Entity<SettingsTile> {
    view_with(None, cx)
}

/// The Settings tile, if it is made.
pub fn get(cx: &App) -> Option<Entity<SettingsTile>> {
    cx.try_global::<Tile>()?.0.clone()
}

/// Whether Settings is in a tile of a layout or a window of its own.
pub fn placed(cx: &App) -> bool {
    let ui = crate::ui_state::get(cx);
    ui.tile_of(&Content::Settings).is_some() || ui.surface_in_window(&Surface::Settings)
}

/// The Settings tile closed: out of every layout, its window (if it is in
/// one) closed, its view let go (the page shown is kept for next time).
pub fn close(cx: &mut App) {
    crate::ui_state::update(cx, |ui| *ui = ui.clone().close_surface(&Surface::Settings));
    super::inbox_window::close_for_move(SETTINGS_KEY, cx);
    if cx.has_global::<Tile>() {
        cx.global_mut::<Tile>().0 = None;
    }
}

/// Every Settings dialog left open (in any Pitwall window) closed, and
/// the view of one of them, for the tile (its unsaved fields kept).
pub fn take_dialogs(cx: &mut App) -> Option<Entity<SettingsView>> {
    let mut taken = None;
    for h in crate::windows::main_windows(cx) {
        if let Ok(Some(v)) = h.update(cx, |view, _, cx| view.take_settings_dialog(cx)) {
            taken.get_or_insert(v);
        }
    }
    taken
}

/// Settings' shortcut while the tile is there: the tile focused, with the
/// keys (its window brought forward when it is in one of its own; its
/// space shown in its Pitwall window). `false`: there is no tile, and the
/// dialog opens as before. Runs outside every window's update.
pub fn reveal(cx: &mut App) -> bool {
    if !placed(cx) {
        return false;
    }
    let f = view(cx).read(cx).focus_handle(cx);
    if let Some(h) = crate::windows::inbox_window(SETTINGS_KEY, cx) {
        let _ = h.update(cx, |_, window, cx| {
            // (A bench instance never comes forward.)
            if !crate::bench::enabled() {
                window.activate_window();
            }
            window.focus(&f, cx);
        });
        return true;
    }
    let ui = crate::ui_state::get(cx);
    let Some((space, pane)) = ui.tile_of(&Content::Settings) else { return false };
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

/// An agent's `show_settings` (`settings.open`), on the main thread:
/// Settings shown to the user at `req.page`, in its tile wherever it is
/// (its space shown in its window), else a new tile right of the agent's
/// terminal (else in the frontmost Pitwall window's space). It never takes
/// the keyboard, never brings Pitwall or a window forward, and says
/// nothing about what Settings holds.
pub fn show_for_agent(req: &SettingsShow, cx: &mut App) -> Result<SettingsShown, ErrorBody> {
    if !enabled(cx) {
        return Err(ErrorBody::new(code::OFF, OFF));
    }
    let page = match req.page.as_deref() {
        Some(p) => Some(Page::parse(p).ok_or_else(|| ErrorBody::new(code::BAD_PARAMS, format!("no Settings page \"{p}\" (one of {})", page_ids())))?),
        None => None,
    };
    let page = match (page, &req.extension) {
        (None, Some(_)) => Some(Page::Extensions),
        (p, _) => p,
    };
    let in_front = crate::platform::app_in_front(cx);
    let reused = placed(cx);
    // A dialog left open goes into the tile (one Settings at a time).
    let handed = take_dialogs(cx);
    let err = |e: String| ErrorBody::new(code::OTHER, e);
    let mut note = None;
    if !reused {
        let anchor = Some(req.agent_id.clone()).filter(|a| !a.is_empty());
        let spot = match crate::web::place::spot_near(&Content::Settings, FilesPlace::Next(super::tree::Side::Right), anchor, cx) {
            Ok(s) => s,
            Err(_) => {
                note = Some("The agent's terminal isn't shown in any open Pitwall window, so Settings opened in the space Pitwall shows.".to_string());
                crate::web::place::spot_near(&Content::Settings, FilesPlace::Own, None, cx).map_err(err)?
            }
        };
        view_with(handed, cx);
        let (label, space, spot) = spot;
        super::inbox_window::dock(SETTINGS_KEY, &label, &space, &spot, cx).map_err(err)?;
    } else {
        // Where it is: its space shown in its window (nothing comes
        // forward, the keys stay where they are).
        view_with(handed, cx);
        let ui = crate::ui_state::get(cx);
        if let Some((space, _)) = ui.tile_of(&Content::Settings) {
            if let Some(h) = crate::windows::main_window(ui.window_of_space(&space), cx) {
                let _ = h.update(cx, |view, _, cx| {
                    if let Some(s) = view.screen() {
                        s.update(cx, |s, cx| {
                            if s.current_space().map(|c| c.id) != Some(space.clone()) {
                                s.show_space(&space, cx);
                            }
                            cx.notify();
                        });
                    }
                });
            }
        }
    }
    if let Some(p) = page {
        crate::settings::data::set_page(cx, p);
    }
    let tile = view(cx);
    let settings = tile.read(cx).settings();
    if let Some(ext) = req.extension.as_deref().filter(|e| !e.trim().is_empty()) {
        let ext = ext.trim();
        match req.secret.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(key) => settings.update(cx, |v, cx| v.ask_secret(ext, key, req.connection.as_deref(), cx)),
            None => settings.update(cx, |v, cx| v.open_extension(ext, cx)),
        }
    }
    settings.update(cx, |_, cx| cx.notify());
    let page = crate::settings::data::page(cx);
    let in_window = crate::windows::inbox_window(SETTINGS_KEY, cx).is_some();
    let at = super::inbox_window::described(SETTINGS_KEY, cx);
    let window = if in_window {
        super::inbox_window::title(SETTINGS_KEY, cx)
    } else {
        crate::web::place::holder_of(&Content::Settings, cx).map(|(l, _, _)| l).unwrap_or_default()
    };
    Ok(SettingsShown { window, in_background: !in_front, page: page.id().to_string(), at, note, reused })
}

/// What `show_settings` says while `agents.showSettings` is off.
pub const OFF: &str = "Showing Settings is turned off in Pitwall (Settings → Agents → “Agents can show you Settings”, agents.showSettings). Nothing was opened.";

fn page_ids() -> String {
    Page::ALL.iter().map(|p| p.id()).collect::<Vec<_>>().join(", ")
}

impl MainScreen {
    /// "Open as a tile" in the dialog (`handed`: its view), "Open Settings
    /// as a tile" in the palette: the Settings tile focused where it is
    /// (its window brought forward when it is in one), else placed as an
    /// inbox is (right of the focused tile, in it when it is empty).
    pub fn open_settings_tile(&mut self, handed: Option<Entity<SettingsView>>, window: &mut Window, cx: &mut Context<Self>) {
        let v = view_with(handed, cx);
        // A dialog left open in another window closes (one Settings).
        cx.defer(|cx| {
            take_dialogs(cx);
        });
        let f = v.read(cx).focus_handle(cx);
        if let Some(h) = crate::windows::inbox_window(SETTINGS_KEY, cx) {
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
        match self.ui.tile_of(&Content::Settings) {
            Some((space, pane)) if self.ui.window_of_space(&space) == self.label => self.focus_settings_tile(&space, &pane, cx),
            // Not here (or in another Pitwall window: it moves here).
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
                        let (next, at) = u.surface_to_layout(&space.id, &Surface::Settings, &spot, fit);
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

    /// Extension `ext`'s card in Settings → Extensions, in the Settings
    /// tile (a tile's `settings.open`).
    pub fn show_extension_settings(&mut self, ext: &str, window: &mut Window, cx: &mut Context<Self>) {
        crate::settings::data::set_page(cx, Page::Extensions);
        let settings = view(cx).read(cx).settings();
        settings.update(cx, |v, cx| v.open_extension(ext, cx));
        self.open_settings_tile(None, window, cx);
    }

    /// The Settings tile (`pane` of space `space`, in this window) shown
    /// and made the space's focused tile.
    pub fn focus_settings_tile(&mut self, space: &str, pane: &str, cx: &mut Context<Self>) {
        self.active = space.to_string();
        self.set_route(super::Route::Space, cx);
        let (space, pane) = (space.to_string(), pane.to_string());
        self.update_ui(|s| s.focus_pane(&space, &pane), cx);
        cx.notify();
    }

    /// Tile `pane` showing Settings: a tile like the inbox's.
    pub(super) fn render_settings_pane(&mut self, pane: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let v = view(cx);
        v.update(cx, |v, cx| v.held(false, cx));
        let keys = Some(v.read(cx).focus_handle(cx));
        let body = div().flex_1().min_w_0().min_h_0().flex().child(v).into_any_element();
        self.tile_shell(pane, Content::Settings, "settings-tile-pane".into(), body, keys, t, cx)
    }

    /// Bench and debug builds, `settings-tile …` (nothing clicked or typed
    /// on the screen): `open` (as the palette), `page <id>`, `close`,
    /// `agent <agent id> [page] [extension]` (as `show_settings`), `max`
    /// (⌘⏎ on it), `share <fraction>` (its side of a two-tile space),
    /// `state` (on stderr).
    pub fn debug_settings_tile(&mut self, cmd: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut w = cmd.split_whitespace();
        match w.next().unwrap_or_default() {
            "open" => self.open_settings_tile(None, window, cx),
            "page" => {
                if let Some(p) = w.next().and_then(Page::parse) {
                    crate::settings::data::set_page(cx, p);
                    if let Some(v) = get(cx) {
                        v.read(cx).settings().update(cx, |_, cx| cx.notify());
                    }
                }
            }
            "close" => cx.defer(close),
            "max" => {
                if let Some((space, pane)) = self.ui.tile_of(&Content::Settings) {
                    self.update_ui(|s| s.toggle_maximize(&space, Some(&pane)), cx);
                    cx.notify();
                }
            }
            "share" => {
                let share = w.next().and_then(|f| f.parse::<f64>().ok()).unwrap_or(0.5).clamp(0.1, 0.9);
                if let Some((space, _)) = self.ui.tile_of(&Content::Settings) {
                    self.update_ui(
                        |s| {
                            let layout = match s.space(&space).map(|sp| sp.layout.clone()) {
                                Some(super::tree::Node::Split { id, children, .. }) if children.len() == 2 => {
                                    let last = children[1].leaves().iter().any(|p| p.settings);
                                    let sizes = if last { [1. - share, share] } else { [share, 1. - share] };
                                    s.space(&space).map(|sp| super::tree::set_split_sizes(&sp.layout, &id, &sizes))
                                }
                                _ => None,
                            };
                            match layout {
                                Some(l) => s.set_layout(&space, l),
                                None => s,
                            }
                        },
                        cx,
                    );
                    cx.notify();
                }
            }
            "agent" => {
                let agent = w.next().unwrap_or_default().to_string();
                let page = w.next().map(String::from);
                let extension = w.next().map(String::from);
                cx.defer(move |cx| {
                    let r = show_for_agent(&SettingsShow { agent_id: agent, page, extension, secret: None, connection: None }, cx);
                    eprintln!("pitwall: show_settings: {r:?}");
                });
            }
            "state" => {
                let narrow = get(cx).map(|v| v.read(cx).settings().read(cx).narrow());
                eprintln!("pitwall: settings tile: placed {}, made {}, narrow {narrow:?}, page {}, at {}", placed(cx), get(cx).is_some(), crate::settings::data::page(cx).id(), super::inbox_window::described(SETTINGS_KEY, cx));
            }
            _ => {}
        }
    }
}
