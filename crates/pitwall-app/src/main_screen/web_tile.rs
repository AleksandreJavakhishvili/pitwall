//! Web tiles (docs/spec/web.md): a web page in a tile of the space's
//! layout, a peer of the terminal and files tiles, in a native web view
//! ([`crate::webview`]). Two kinds, one tile: a page the user opened
//! ("Open web page…", ⌘L, or an agent's `show_web`), and an agent's
//! Preview of the dev server its terminal printed (docs/spec/terminals.md
//! §4), which follows the terminal ([`crate::preview::Follow`]) and closes
//! with the agent.
//!
//! Where a tile sits is the layout's (`ui.json`, `workspace/web.rs`); the
//! page itself is a [`WebPage`] kept across the app by tile id
//! ([`crate::web::pages`]), so a tile that moves to another window, or into
//! a window of its own, keeps its page. This module draws the pages of this
//! window's layout and, after every frame, places each one's view where its
//! element was drawn ([`WebPage::frame`]): hidden where it wasn't (another
//! space, the Wall, Review, a folded tile), swapped for a snapshot while
//! something drawn later (a menu, a dialog, the palette, a toast, a
//! tooltip) covers it or anything is dragged over the window.

use gpui::{div, prelude::*, AnyElement, App, AppContext, Context, DragMoveEvent, Entity, InteractiveElement, Window};

use pitwall_proto::AgentView;

use super::strip::Toast;
use super::tree::Content;
use super::workspace::{FilesSpot, WebTile};
use super::MainScreen;
use crate::theme::Theme;
use super::files_tile::TileDrag;
use crate::web::page::{calm, Host, HostKind, WebPage};
use crate::web::pages;

pub use crate::web::page::{enabled, SETTING};

/// This window's part in the web pages.
#[derive(Default)]
pub(super) struct Webs {
    /// The frame listener is on.
    listening: bool,
}

#[cfg(test)]
impl Webs {
    pub fn is_open(&self, id: &str, cx: &App) -> bool {
        pages::get(id, cx).is_some()
    }

    pub fn has_view(&self, id: &str, cx: &App) -> bool {
        pages::get(id, cx).is_some_and(|p| p.read(cx).has_view())
    }

    pub fn server(&self, id: &str, cx: &App) -> Option<crate::preview::Server> {
        pages::get(id, cx).and_then(|p| p.read(cx).server())
    }

    pub fn page_url(&self, id: &str, cx: &App) -> Option<String> {
        pages::get(id, cx).map(|p| p.read(cx).page_url().to_string())
    }

    pub fn error(&self, id: &str, cx: &App) -> Option<String> {
        pages::get(id, cx).and_then(|p| p.read(cx).error())
    }

    pub fn focus_of(&self, id: &str, cx: &App) -> Option<gpui::FocusHandle> {
        pages::get(id, cx).map(|p| p.read(cx).focus_handle().clone())
    }

    pub fn field_of(&self, id: &str, cx: &App) -> Option<Entity<crate::kit::TextInput>> {
        pages::get(id, cx).map(|p| p.read(cx).field().clone())
    }
}

impl MainScreen {
    /// The web pages of this window's layout.
    fn my_web_ids(&self) -> Vec<String> {
        self.ui.spaces_of(&self.label).iter().flat_map(|s| s.layout.web_ids()).collect()
    }

    /// Page `id` (made when it isn't yet).
    fn web_page(&self, id: &str, cx: &mut App) -> Option<Entity<WebPage>> {
        let agents = self.agents(cx).to_vec();
        pages::ensure(id, &agents, cx)
    }

    /// ⌘L / "Open web page…": in a web tile already (its page or its
    /// field), its address; else a new web tile right of the focused tile
    /// (in it, when it is empty), its address field taking the keys.
    pub(super) fn open_web_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let here = self
            .my_web_ids()
            .into_iter()
            .find(|id| pages::get(id, cx).is_some_and(|p| p.read(cx).has_focus(window, cx)));
        if let Some(id) = here {
            self.web_address(&id, window, cx);
            return;
        }
        if !enabled(cx) {
            self.toast(Toast::info("Web pages are off", Some("Turn them on in Settings → General.".into())), cx);
            return;
        }
        let id = super::workspace::new_web_id();
        let tile = WebTile { profile: self.new_page_profile(cx), ..Default::default() };
        if !self.place_web(&id, tile, None, cx) {
            return;
        }
        self.web_address(&id, window, cx);
    }

    /// The browser profile of a page the user opens here: that of the
    /// project of the agent whose tile has the focus, else Default.
    pub(super) fn new_page_profile(&self, cx: &mut Context<Self>) -> Option<String> {
        let project = self.selected(cx).map(|a| a.project)?;
        crate::web::profiles::for_project(&project, cx)
    }

    /// Put web tile `id` in the active space: right of `beside` (a pane),
    /// else of the focused tile (in it when it is empty), else where there
    /// is most room. Whether it was placed.
    fn place_web(&mut self, id: &str, tile: WebTile, beside: Option<String>, cx: &mut Context<Self>) -> bool {
        let Some(space) = self.active_space().cloned() else { return false };
        let anchor = beside.or(space.focused_pane_id.clone()).and_then(|p| space.layout.find_pane(&p));
        let spot = match anchor {
            Some(p) if p.is_empty() => FilesSpot::Take(p.id),
            Some(p) => FilesSpot::Tile { pane: p.id, side: super::tree::Side::Right },
            None => FilesSpot::Auto,
        };
        let fit = self.fit_for(&space.id);
        let mut placed = false;
        self.update_ui(
            |s| {
                let (next, at) = s.open_web(&space.id, id, tile, &spot, fit);
                placed = at.is_some();
                next
            },
            cx,
        );
        placed
    }

    /// The keys to web tile `id`'s address field (all of it selected).
    fn web_address(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.web_page(id, cx) else { return };
        let host = Host { window: window.window_handle(), focus: self.focus.clone(), kind: HostKind::Tile };
        page.update(cx, |p, cx| {
            p.set_host(host, window, cx);
            p.focus_address(window, cx);
        });
        cx.notify();
    }

    /// "Preview" in a pane header: `url` in agent `a`'s Preview tile, made
    /// right of its terminal when it has none. Where no web view can be
    /// made (or web pages are off), the browser opens it instead and the
    /// reason shows once.
    pub(super) fn open_preview(&mut self, a: &AgentView, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.ui.preview_of(&a.id) {
            if let Some(page) = self.web_page(&id, cx) {
                page.update(cx, |p, cx| p.follow_from(url, a, cx));
            }
            let u = url.to_string();
            self.update_ui(|s| s.set_web_url(&id, &u), cx);
            cx.notify();
            return;
        }
        let id = super::workspace::new_web_id();
        let tile = WebTile { url: url.to_string(), agent: Some(a.id.clone()), ..Default::default() };
        // Its view first: none here, the browser instead.
        let made = if enabled(cx) {
            let page = cx.new(|cx| WebPage::new(&id, &tile, Some(a), cx));
            let host = Host { window: window.window_handle(), focus: self.focus.clone(), kind: HostKind::Tile };
            page.update(cx, |p, cx| {
                p.set_host(host, window, cx);
                p.make_view(window, cx)
            })
            .map(|()| page)
        } else {
            Err("Web pages are off in Settings → General.".into())
        };
        let page = match made {
            Ok(p) => p,
            Err(e) => {
                crate::platform::files::open_url(cx, url);
                if !pages::told_once(cx) {
                    self.toast(Toast::info("Opened in the browser", Some(format!("The in-app preview isn't available: {e}"))), cx);
                }
                return;
            }
        };
        pages::insert(&id, page, cx);
        let beside = self.active_space().and_then(|s| s.layout.find_by_agent(&a.id)).map(|p| p.id);
        if !self.place_web(&id, tile, beside, cx) {
            pages::prune(cx);
            return;
        }
        cx.notify();
    }

    /// Agent `agent`'s Preview tile closed.
    pub(super) fn close_preview(&mut self, agent: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.ui.preview_of(agent) {
            self.close_web_tile(&id, window, cx);
        }
    }

    pub(super) fn preview_open(&self, agent: &str) -> bool {
        self.ui.preview_of(agent).is_some()
    }

    /// Web tile `id` closed: out of the layout, its view freed.
    pub(super) fn close_web_tile(&mut self, id: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.update_ui(|s| s.close_web(id), cx);
        pages::close(id, cx);
        cx.notify();
    }

    /// After every frame while there are web tiles: place the views.
    fn listen_frames(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.webs.listening {
            return;
        }
        self.webs.listening = true;
        let screen = cx.entity().downgrade();
        window.on_frame_drawn(move |window, cx| screen.update(cx, |s, cx| s.web_frame(window, cx)).unwrap_or(false));
        // Draw a frame for the listener to see the new tile in.
        window.refresh();
    }

    /// A frame was drawn: each page of this window's layout that this
    /// window holds follows its element (or hides when it wasn't drawn)
    /// and steps aside for what covers it. `false`: no web tiles left, the
    /// listener goes.
    pub(super) fn web_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let here = window.window_handle().window_id();
        let mine: Vec<Entity<WebPage>> = self
            .my_web_ids()
            .iter()
            .filter_map(|id| pages::get(id, cx))
            .filter(|p| p.read(cx).host().is_some_and(|h| h.window.window_id() == here && h.kind == HostKind::Tile))
            .collect();
        if mine.is_empty() && self.my_web_ids().is_empty() {
            self.webs.listening = false;
            return false;
        }
        // A drag over the window: tiles, agents (GPUI's), files (followed
        // on the screen).
        let dragging = cx.has_active_drag() || self.files_hint.is_some();
        let mut repaint = false;
        for page in mine {
            let placed = page.read(cx).slot().and_then(|h| crate::preview::placed(window.rendered_hitboxes(), window.rendered_tooltip_bounds(), h));
            repaint |= page.update(cx, |p, cx| p.frame(placed, dragging, window, cx));
        }
        if repaint {
            cx.notify();
        }
        true
    }

    /// The agents changed: Previews follow their terminals (and close with
    /// their agents: `ui.json`'s prune).
    pub(super) fn webs_follow(&mut self, cx: &mut Context<Self>) {
        let agents = self.agents(cx).to_vec();
        pages::follow_all(&agents, cx);
    }

    /// Reload (or Retry): the page again, its error forgotten.
    #[cfg(test)]
    pub(super) fn web_reload(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(p) = pages::get(id, cx) {
            p.update(cx, |p, cx| p.reload(cx));
        }
    }

    // ── dragging a tile ──────────────────────────────────────────────────

    /// The main area as where a dragged web or inbox tile lands: its drop
    /// zones shown as it moves ([`MainScreen::tile_hover`]), the layout
    /// changed when it is let go.
    pub(super) fn tile_drop_target<E: InteractiveElement + 'static>(&self, el: E, cx: &mut Context<Self>) -> E {
        el.on_drag_move::<TileDrag>(cx.listener(|this, e: &DragMoveEvent<TileDrag>, _, cx| {
            let what = e.drag(cx).what.clone();
            if this.tile_hover(&what, e.event.position, cx).is_none() {
                this.clear_files_hover(cx);
            }
        }))
        .on_drop(cx.listener(|this, d: &TileDrag, _, cx| {
            let what = d.what.clone();
            let hint = this.files_hint.take().filter(|h| h.what == what);
            cx.notify();
            let (Some(h), Some(space)) = (hint, this.active_space().map(|s| s.id.clone())) else { return };
            let fit = this.fit_for(&space);
            this.update_ui(|s| s.dock(&space, &what, &h.spot, super::workspace::WEB_SHARE, fit).0, cx);
        }))
    }

    // ── drawing ──────────────────────────────────────────────────────────

    /// Tile `pane` showing web tile `id`.
    pub(super) fn render_web_pane(&mut self, pane: &str, id: &str, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let host = Host { window: window.window_handle(), focus: self.focus.clone(), kind: HostKind::Tile };
        let agents = self.agents(cx).to_vec();
        let body: AnyElement = match pages::hold(id, host, &agents, window, cx) {
            Some(page) => {
                self.listen_frames(window, cx);
                // A click into the page (the native view's: GPUI never
                // sees it) gives it the keys, and makes it the focused tile.
                let keys = page.read(cx).focus_handle().clone();
                self.follow_tile_keys(&Content::Web(id.to_string()), &keys, window, cx);
                div().flex_1().min_w_0().min_h_0().flex().child(page).into_any_element()
            }
            None => {
                let i = id.to_string();
                calm("This page is gone", "Its tile can be closed.".into(), t)
                    .child(crate::kit::small_btn("web-gone-close", "Close", t).on_click(cx.listener(move |this, _, window, cx| this.close_web_tile(&i, window, cx))))
                    .into_any_element()
            }
        };
        self.tile_shell(pane, Content::Web(id.to_string()), format!("web-tile-{id}"), body, None, t, cx)
    }
}
