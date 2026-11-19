//! Web tiles (docs/spec/web.md): a web page in a tile of the space's
//! layout, a peer of the terminal and files tiles, in a native web view
//! ([`crate::webview`]). Two kinds, one tile: a page the user opened
//! ("Open web page…", ⌘L), and an agent's Preview of the dev server its
//! terminal printed (docs/spec/terminals.md §4), which follows the terminal
//! ([`Follow`]) and closes with the agent.
//!
//! Where a tile sits is the layout's (`ui.json`, `workspace/web.rs`); this
//! module keeps each window's views, by tile id. A view is made in the
//! window that draws its tile, after a frame drew it; a tile that moves to
//! another window's space is made again there (the page loads again).
//!
//! After every frame the window draws, [`MainScreen::web_frame`] places each
//! view where its element was drawn, hides it where it wasn't (another
//! space, the Wall, Review, a folded tile), and swaps it for a snapshot
//! while something drawn later (a menu, a dialog, the palette, a toast, a
//! tooltip) covers it or anything is dragged over the window ([`Stage`],
//! [`preview::covering`]).
//!
//! Keyboard: a click into the page gives it the keys and the tile's focus
//! handle GPUI's focus, so the window's shortcuts still dispatch from there
//! ([`WebEvent::Shortcut`] for the ones the engine sees first). Esc, a
//! click outside the page, or GPUI's focus moving on gives the keys back.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{
    canvas, div, prelude::*, px, AnyElement, App, Bounds, Context, DragMoveEvent, Entity, FocusHandle,
    HitboxId, MouseButton, MouseDownEvent, Pixels, SharedString, Subscription, Task, Window,
};

use pitwall_proto::AgentView;

use super::strip::Toast;
use super::topbar::AgentGhost;
use super::tree::Content;
use super::workspace::{FilesSpot, WebTile};
use super::MainScreen;
use crate::kit::{icon, tooltip, InputEvent, TextInput};
use crate::preview::{self, Act, Follow, Server, Stage, StageOp, DROP_AFTER, PHONE_WIDTH};
use crate::theme::Theme;
use crate::webview::{self, Options, Policy, Snapshot, WebEvent, WebView};

/// The tile's bar: the drag handle, with the page's controls.
const BAR_H: f32 = 32.;

/// The setting that turns web pages in Pitwall on and off.
pub const SETTING: &str = "general.webPages";

/// Web pages may be shown in Pitwall (on unless turned off).
pub fn enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(SETTING)
        .and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// What the views send the screen: their callbacks run outside any GPUI
/// update, so they go through a channel.
enum Msg {
    Event(String, WebEvent),
    /// A snapshot (tile, the request it answers).
    Snapshot(String, u64, Option<Snapshot>),
}

/// One web tile in this window.
pub(super) struct Tile {
    /// An agent's Preview: whose.
    agent: Option<String>,
    /// An agent's Preview: what its terminal prints.
    follow: Option<Follow>,
    /// A page the user opened: why it isn't shown (a failed load, no web
    /// view here). Reload or another address tries again.
    error: Option<String>,
    /// What the page shows now (links followed inside it).
    page_url: String,
    title: String,
    loading: bool,
    can_back: bool,
    can_forward: bool,
    phone: bool,
    /// An agent's Preview: its project (its own cookies and storage).
    project: String,
    /// `None`: not made yet (no address, not drawn yet), or let go.
    view: Option<Box<dyn WebView>>,
    stage: Stage,
    /// Where the page's element was drawn ([`preview::surface`]).
    slot: Rc<Cell<Option<HitboxId>>>,
    /// The snapshot asked for last.
    snap: u64,
    drop_timer: Option<Task<()>>,
    focus: FocusHandle,
    /// The page has the keys (it said [`WebEvent::Focused`]).
    keys: bool,
    url_field: Entity<TextInput>,
    /// A word under the bar: an address that isn't one, a declined download.
    note: Option<String>,
    _subs: Vec<Subscription>,
}

impl Tile {
    /// The page is to be shown (else the tile says why not).
    fn live(&self) -> bool {
        match &self.follow {
            Some(f) => f.server == Server::Live,
            None => self.error.is_none() && !self.page_url.is_empty(),
        }
    }

    fn policy(&self) -> Policy {
        if self.agent.is_some() {
            Policy::Local
        } else {
            Policy::Web
        }
    }
}

/// The window's web tiles.
#[derive(Default)]
pub(super) struct Webs {
    tiles: HashMap<String, Tile>,
    tx: Option<mpsc::UnboundedSender<Msg>>,
    _events: Option<Task<()>>,
    /// The frame listener is on.
    listening: bool,
    /// A Preview couldn't be made and the user was told why (once).
    told: bool,
}

impl Webs {
    #[cfg(test)]
    pub fn is_open(&self, id: &str) -> bool {
        self.tiles.contains_key(id)
    }

    #[cfg(test)]
    pub fn has_view(&self, id: &str) -> bool {
        self.tiles.get(id).is_some_and(|p| p.view.is_some())
    }

    #[cfg(test)]
    pub fn server(&self, id: &str) -> Option<Server> {
        self.tiles.get(id).and_then(|p| p.follow.as_ref().map(|f| f.server.clone()))
    }

    #[cfg(test)]
    pub fn page_url(&self, id: &str) -> Option<String> {
        self.tiles.get(id).map(|p| p.page_url.clone())
    }

    #[cfg(test)]
    pub fn error(&self, id: &str) -> Option<String> {
        self.tiles.get(id).and_then(|p| p.error.clone().or(p.note.clone()))
    }

    #[cfg(test)]
    pub fn focus_of(&self, id: &str) -> Option<FocusHandle> {
        self.tiles.get(id).map(|p| p.focus.clone())
    }

    #[cfg(test)]
    pub fn field_of(&self, id: &str) -> Option<Entity<TextInput>> {
        self.tiles.get(id).map(|p| p.url_field.clone())
    }
}

/// A web tile dragged by its bar.
#[derive(Clone, Debug)]
pub struct WebDrag {
    pub id: String,
    pub name: SharedString,
}

impl WebDrag {
    fn ghost(&self, grab: gpui::Point<Pixels>, _: &mut Window, cx: &mut App) -> Entity<AgentGhost> {
        let name = self.name.clone();
        cx.new(|_| AgentGhost::named(name, grab))
    }
}

impl MainScreen {
    /// ⌘L / "Open web page…": in a web tile already (its page or its
    /// field), its address; else a new web tile right of the focused tile
    /// (in it, when it is empty), its address field taking the keys.
    pub(super) fn open_web_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.webs.tiles.iter().find(|(_, p)| p.focus.contains_focused(window, cx)).map(|(id, _)| id.clone());
        if let Some(id) = here {
            self.web_address(&id, window, cx);
            return;
        }
        if !enabled(cx) {
            self.toast(Toast::info("Web pages are off", Some("Turn them on in Settings → General.".into())), cx);
            return;
        }
        let id = super::workspace::new_web_id();
        if !self.place_web(&id, WebTile::default(), None, cx) {
            return;
        }
        self.web_tile_state(&id, window, cx);
        self.web_address(&id, window, cx);
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
        let Some(p) = self.webs.tiles.get_mut(id) else { return };
        p.keys = false;
        if let Some(v) = p.view.as_mut() {
            v.blur();
        }
        p.url_field.update(cx, |f, cx| f.focus_all(window, cx));
        cx.notify();
    }

    /// "Preview" in a pane header: `url` in agent `a`'s Preview tile, made
    /// right of its terminal when it has none. Where no web view can be
    /// made (or web pages are off), the browser opens it instead and the
    /// reason shows once.
    pub(super) fn open_preview(&mut self, a: &AgentView, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.ui.preview_of(&a.id) {
            self.web_tile_state(&id, window, cx);
            if let Some(p) = self.webs.tiles.get_mut(&id) {
                if p.page_url != url {
                    p.follow = Some(Follow::new(url, &a.preview_urls, a.preview_seen));
                    p.page_url = url.to_string();
                    p.url_field.update(cx, |f, cx| f.set_text(url, cx));
                    if let Some(v) = p.view.as_mut() {
                        v.navigate(url);
                    }
                }
            }
            let u = url.to_string();
            self.update_ui(|s| s.set_web_url(&id, &u), cx);
            cx.notify();
            return;
        }
        let made = if enabled(cx) {
            let id = super::workspace::new_web_id();
            self.make_view(&id, Policy::Local, &preview::data_dir(&self.data_root(cx), &a.project), url, window, cx)
                .map(|v| (id, v))
        } else {
            Err("Web pages are off in Settings → General.".into())
        };
        let (id, view) = match made {
            Ok(v) => v,
            Err(e) => {
                crate::platform::files::open_url(cx, url);
                if !self.webs.told {
                    self.webs.told = true;
                    self.toast(Toast::info("Opened in the browser", Some(format!("The in-app preview isn't available: {e}"))), cx);
                }
                return;
            }
        };
        let beside = self.active_space().and_then(|s| s.layout.find_by_agent(&a.id)).map(|p| p.id);
        let tile = WebTile { url: url.to_string(), agent: Some(a.id.clone()) };
        if !self.place_web(&id, tile, beside, cx) {
            return;
        }
        self.web_tile_state(&id, window, cx);
        if let Some(p) = self.webs.tiles.get_mut(&id) {
            p.view = Some(view);
            p.loading = true;
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
    pub(super) fn close_web_tile(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.webs.tiles.remove(id) {
            if p.focus.contains_focused(window, cx) {
                window.focus(&self.focus, cx);
            }
        }
        self.update_ui(|s| s.close_web(id), cx);
        cx.notify();
    }

    /// Pitwall's data folder (web views' cookies and storage go under it).
    fn data_root(&self, cx: &mut App) -> std::path::PathBuf {
        crate::ui_state::store(cx)
            .read(cx)
            .root()
            .cloned()
            .unwrap_or_else(pitwall_core::paths::Paths::default_root)
    }

    /// Web tile `id`'s state in this window, made from `ui.json` when it
    /// isn't yet (a restart, a tile moved in from another window). No view
    /// yet: the frame after it is drawn makes it.
    fn web_tile_state(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.webs.tiles.contains_key(id) {
            return;
        }
        let Some(kept) = self.ui.web_tiles.get(id).cloned() else { return };
        let agent = kept.agent.as_ref().and_then(|a| self.agents(cx).iter().find(|x| x.id == *a).cloned());
        let follow = agent.as_ref().map(|a| {
            let mut f = Follow::new(&kept.url, &a.preview_urls, a.preview_seen);
            // Not running (or not printing) now: stopped until it does.
            if !a.running || a.preview_urls.is_empty() {
                f.server = Server::Stopped;
            }
            f
        });
        let focus = cx.focus_handle();
        let url_field = cx.new(|cx| {
            let mut f = TextInput::single("Type a web address", cx).mono().bare();
            f.set_text(kept.url.as_str(), cx);
            f
        });
        let fid = id.to_string();
        let fid2 = id.to_string();
        let subs = vec![
            cx.on_focus_out(&focus, window, move |this, _, _, _| {
                // GPUI's focus moved on (a shortcut, a click, Esc): so do the keys.
                if let Some(p) = this.webs.tiles.get_mut(&fid) {
                    p.keys = false;
                    if let Some(v) = p.view.as_mut() {
                        v.blur();
                    }
                }
            }),
            cx.subscribe_in(&url_field, window, move |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::Submit | InputEvent::SubmitShift => this.commit_address(&fid2, window, cx),
                InputEvent::Cancel => {
                    // The address as it was; the keys back to the page's
                    // window (its terminal).
                    if let Some(p) = this.webs.tiles.get_mut(&fid2) {
                        let u = p.page_url.clone();
                        p.note = None;
                        p.url_field.update(cx, |f, cx| f.set_text(u, cx));
                    }
                    window.focus(&this.focus, cx);
                    cx.notify();
                }
                _ => {}
            }),
        ];
        self.webs.tiles.insert(
            id.to_string(),
            Tile {
                project: agent.as_ref().map(|a| a.project.clone()).unwrap_or_default(),
                agent: kept.agent.clone(),
                follow,
                error: None,
                page_url: kept.url.clone(),
                title: String::new(),
                loading: false,
                can_back: false,
                can_forward: false,
                phone: false,
                view: None,
                stage: Stage::default(),
                slot: Default::default(),
                snap: 0,
                drop_timer: None,
                focus,
                keys: false,
                url_field,
                note: None,
                _subs: subs,
            },
        );
        self.listen_frames(window, cx);
    }

    /// Enter in the address field: go there (a page that isn't one is said
    /// under the bar, nothing loads).
    fn commit_address(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.webs.tiles.get_mut(id) else { return };
        let text = p.url_field.read(cx).text().to_string();
        match webview::address(&text) {
            Ok(url) => {
                p.note = None;
                p.error = None;
                if let Some(f) = p.follow.as_mut() {
                    // Picked here: the terminal's next new address still wins.
                    f.url = url.clone();
                    f.retry();
                }
                p.page_url = url.clone();
                p.url_field.update(cx, |f, cx| f.set_text(url.as_str(), cx));
                if let Some(v) = p.view.as_mut() {
                    v.navigate(&url);
                    v.focus();
                }
                let u = url.clone();
                let id2 = id.to_string();
                self.update_ui(|s| s.set_web_url(&id2, &u), cx);
                // The page takes the keys once it is made and shown.
                if let Some(p) = self.webs.tiles.get(id) {
                    if p.view.is_none() {
                        let f = p.focus.clone();
                        window.focus(&f, cx);
                    }
                }
            }
            Err(e) => p.note = Some(e),
        }
        cx.notify();
    }

    /// A web view for tile `id` on `url`, its events into the screen.
    fn make_view(
        &mut self,
        id: &str,
        policy: Policy,
        data_dir: &std::path::Path,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Box<dyn WebView>, String> {
        let tx = self.web_channel(window, cx);
        let tid = id.to_string();
        preview::create(
            window,
            Options {
                url: url.to_string(),
                policy,
                data_dir: data_dir.to_path_buf(),
                events: Box::new(move |e| {
                    let _ = tx.unbounded_send(Msg::Event(tid.clone(), e));
                }),
            },
            cx,
        )
    }

    /// The views' channel, and the task that hands what comes through it
    /// to the screen.
    fn web_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> mpsc::UnboundedSender<Msg> {
        if let Some(tx) = &self.webs.tx {
            return tx.clone();
        }
        let (tx, mut rx) = mpsc::unbounded::<Msg>();
        self.webs.tx = Some(tx.clone());
        self.webs._events = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let done = match msg {
                    // A shortcut the page saw first: Pitwall's, dispatched
                    // from the tile's focus (outside the screen's update:
                    // its actions update the screen).
                    Msg::Event(id, WebEvent::Shortcut(k)) => {
                        let esc = k.key == "escape" && !k.modifiers.modified();
                        if esc {
                            this.update_in(cx, |s, window, cx| s.web_blur(&id, window, cx))
                        } else {
                            cx.update(|window, cx| {
                                window.dispatch_keystroke(k, cx);
                            })
                        }
                    }
                    Msg::Event(id, e) => this.update_in(cx, |s, window, cx| s.web_event(&id, e, window, cx)),
                    Msg::Snapshot(id, n, shot) => this.update(cx, |s, cx| s.web_snapshot(&id, n, shot, cx)),
                };
                if done.is_err() {
                    break;
                }
            }
        }));
        tx
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

    /// A frame was drawn: each view follows its element (or hides when it
    /// wasn't drawn) and steps aside for what covers it. `false`: no web
    /// tiles left, the listener goes.
    pub(super) fn web_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // Tiles closed, or moved to another window's space: their views go.
        let mine: Vec<String> = self.ui.spaces_of(&self.label).iter().flat_map(|s| s.layout.web_ids()).collect();
        self.webs.tiles.retain(|id, _| mine.contains(id));
        if self.webs.tiles.is_empty() {
            self.webs.listening = false;
            return false;
        }
        // A drag over the window: tiles, agents (GPUI's), files (followed
        // on the screen).
        let dragging = cx.has_active_drag() || self.files_hint.is_some();
        let places: Vec<(String, Option<Placed>)> = {
            let hitboxes = window.rendered_hitboxes();
            let tip = window.rendered_tooltip_bounds();
            self.webs
                .tiles
                .iter()
                .map(|(id, p)| (id.clone(), p.slot.get().and_then(|h| preview::placed(hitboxes, tip, h))))
                .collect()
        };
        let on = enabled(cx);
        let mut repaint = false;
        for (id, place) in places {
            let Some(p) = self.webs.tiles.get(&id) else { continue };
            let live = p.live() && on;
            // Not made yet (or let go while hidden), shown now: made, on its page.
            if p.view.is_none() && live && place.is_some() {
                let (policy, url) = (p.policy(), p.page_url.clone());
                let dir = match policy {
                    Policy::Local => preview::data_dir(&self.data_root(cx), &p.project),
                    Policy::Web => preview::profile_dir(&self.data_root(cx), preview::DEFAULT_PROFILE),
                };
                let focus_page = p.focus.is_focused(window);
                match self.make_view(&id, policy, &dir, &url, window, cx) {
                    Ok(mut v) => {
                        if focus_page {
                            v.focus();
                        }
                        let p = self.webs.tiles.get_mut(&id).expect("tile");
                        p.view = Some(v);
                        p.loading = true;
                        p.stage = Stage::default();
                    }
                    Err(e) => {
                        if let Some(p) = self.webs.tiles.get_mut(&id) {
                            match p.follow.as_mut() {
                                Some(f) => f.failed(&e),
                                None => p.error = Some(format!("Web pages can't be shown in Pitwall here: {e}")),
                            }
                        }
                        repaint = true;
                        continue;
                    }
                }
            }
            let tx = self.webs.tx.clone();
            let Some(p) = self.webs.tiles.get_mut(&id) else { continue };
            let live = p.live() && on;
            let Some(view) = p.view.as_mut() else { continue };
            let had_picture = p.stage.picture().is_some();
            let covered = place.is_some_and(|(_, c)| preview::covering(c, dragging));
            let ops = p.stage.frame(place.map(|(b, _)| b), covered, live);
            for op in ops {
                match op {
                    StageOp::SetBounds(b) => view.set_bounds(b),
                    StageOp::Show => view.set_visible(true),
                    StageOp::Hide => view.set_visible(false),
                    StageOp::Snapshot => {
                        p.snap += 1;
                        let (n, id, tx) = (p.snap, id.clone(), tx.clone());
                        view.snapshot(Box::new(move |shot| {
                            if let Some(tx) = tx {
                                let _ = tx.unbounded_send(Msg::Snapshot(id, n, shot));
                            }
                        }));
                    }
                }
            }
            repaint |= had_picture != p.stage.picture().is_some();
            // An agent's Preview hidden (not just covered): let go after a
            // while. A page the user opened stays loaded while its tile is.
            if p.agent.is_none() || !p.stage.idle() {
                p.drop_timer = None;
            } else if p.drop_timer.is_none() {
                let id = id.clone();
                p.drop_timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(DROP_AFTER).await;
                    let _ = this.update(cx, |s, _| s.web_drop_idle(&id));
                }));
            }
        }
        if repaint {
            cx.notify();
        }
        true
    }

    /// Hidden for [`DROP_AFTER`]: let the view go, keep its address.
    fn web_drop_idle(&mut self, id: &str) {
        if let Some(p) = self.webs.tiles.get_mut(id) {
            p.drop_timer = None;
            if p.stage.idle() {
                p.view = None;
                p.stage = Stage::default();
            }
        }
    }

    fn web_snapshot(&mut self, id: &str, n: u64, shot: Option<Snapshot>, cx: &mut Context<Self>) {
        let Some(p) = self.webs.tiles.get_mut(id).filter(|p| p.snap == n) else {
            return;
        };
        let ops = p.stage.snapshot(shot.and_then(preview::to_image));
        if let Some(v) = p.view.as_mut() {
            for op in ops {
                if op == StageOp::Hide {
                    v.set_visible(false);
                }
            }
        }
        // Draw the picture in its place.
        cx.notify();
    }

    fn web_event(&mut self, id: &str, e: WebEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.webs.tiles.get_mut(id) else { return };
        match e {
            WebEvent::Url(u) => {
                if webview::stays(p.policy(), &u) && p.page_url != u {
                    // Not while the address is being typed.
                    if !p.url_field.read(cx).is_focused(window) {
                        p.url_field.update(cx, |f, cx| f.set_text(u.as_str(), cx));
                    }
                    p.page_url = u.clone();
                    let id2 = id.to_string();
                    self.update_ui(|s| s.set_web_url(&id2, &u), cx);
                }
            }
            WebEvent::Title(t) => p.title = t,
            WebEvent::Loading(l) => {
                p.loading = l;
                if l {
                    p.note = None;
                }
            }
            WebEvent::Failed { error, .. } => match p.follow.as_mut() {
                Some(f) => f.failed(&error),
                None => p.error = Some(error),
            },
            WebEvent::CanGo { back, forward } => {
                p.can_back = back;
                p.can_forward = forward;
            }
            WebEvent::Focused => {
                p.keys = true;
                let f = p.focus.clone();
                window.focus(&f, cx);
            }
            WebEvent::Download(u) => {
                p.note = Some(format!("Downloads aren't saved from Pitwall's web pages: {u} can be opened in your browser."));
            }
            WebEvent::Shortcut(_) => {}
        }
        cx.notify();
    }

    /// The keys back to the screen (its focused tile's terminal).
    fn web_blur(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.webs.tiles.get_mut(id) {
            p.keys = false;
            if let Some(v) = p.view.as_mut() {
                v.blur();
            }
            if p.focus.contains_focused(window, cx) {
                window.focus(&self.focus, cx);
            }
        }
    }

    /// The agents changed: Previews follow their terminals (and close with
    /// their agents: `ui.json`'s prune).
    pub(super) fn webs_follow(&mut self, cx: &mut Context<Self>) {
        if self.webs.tiles.is_empty() {
            return;
        }
        let agents = self.agents(cx).to_vec();
        let ui = self.ui.clone();
        self.webs.tiles.retain(|id, _| ui.web_tiles.contains_key(id));
        let mut keep: Vec<(String, String)> = Vec::new();
        for (id, p) in self.webs.tiles.iter_mut() {
            let (Some(agent), Some(f)) = (p.agent.as_deref(), p.follow.as_mut()) else { continue };
            let Some(a) = agents.iter().find(|a| a.id == agent) else { continue };
            match f.update(&a.preview_urls, a.preview_seen, a.running) {
                Some(Act::Navigate(u)) => {
                    p.page_url = u.clone();
                    p.url_field.update(cx, |f, cx| f.set_text(u.as_str(), cx));
                    if let Some(v) = p.view.as_mut() {
                        v.navigate(&u);
                    }
                    keep.push((id.clone(), u));
                }
                Some(Act::Reload) => {
                    if let Some(v) = p.view.as_mut() {
                        v.reload();
                    }
                }
                Some(Act::Stop) | None => {}
            }
        }
        for (id, u) in keep {
            self.update_ui(|s| s.set_web_url(&id, &u), cx);
        }
    }

    fn with_tile(&mut self, id: &str, f: impl FnOnce(&mut Tile), cx: &mut Context<Self>) {
        if let Some(p) = self.webs.tiles.get_mut(id) {
            f(p);
            cx.notify();
        }
    }

    fn with_view(&mut self, id: &str, f: impl FnOnce(&mut Box<dyn WebView>), cx: &mut Context<Self>) {
        if let Some(v) = self.webs.tiles.get_mut(id).and_then(|p| p.view.as_mut()) {
            f(v);
            cx.notify();
        }
    }

    /// Reload (or Retry): the page again, its error forgotten.
    pub(super) fn web_reload(&mut self, id: &str, cx: &mut Context<Self>) {
        self.with_tile(
            id,
            |p| {
                p.error = None;
                p.note = None;
                if let Some(f) = p.follow.as_mut() {
                    f.retry();
                }
                if let Some(v) = p.view.as_mut() {
                    v.reload();
                }
            },
            cx,
        );
    }

    // ── dragging a tile ──────────────────────────────────────────────────

    /// The main area as where a dragged web tile lands: its drop zones
    /// shown as it moves ([`MainScreen::tile_hover`]), the layout changed
    /// when it is let go.
    pub(super) fn web_drop_target<E: InteractiveElement + 'static>(&self, el: E, cx: &mut Context<Self>) -> E {
        el.on_drag_move::<WebDrag>(cx.listener(|this, e: &DragMoveEvent<WebDrag>, _, cx| {
            let what = Content::Web(e.drag(cx).id.clone());
            if this.tile_hover(&what, e.event.position, cx).is_none() {
                this.clear_files_hover(cx);
            }
        }))
        .on_drop(cx.listener(|this, d: &WebDrag, _, cx| {
            let what = Content::Web(d.id.clone());
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
        self.web_tile_state(id, window, cx);
        let body: AnyElement = match self.webs.tiles.get(id) {
            Some(_) => self.web_body(id, t, window, cx),
            None => {
                let i = id.to_string();
                calm("This page is gone", "Its tile can be closed.".into(), t)
                    .child(crate::kit::small_btn("web-gone-close", "Close", t).on_click(cx.listener(move |this, _, window, cx| this.close_web_tile(&i, window, cx))))
                    .into_any_element()
            }
        };
        self.tile_shell(pane, Content::Web(id.to_string()), format!("web-tile-{id}"), body, t, cx)
    }

    /// The bar, then the page (or why there's none).
    fn web_body(&self, id: &str, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.webs.tiles[id];
        let focus = p.focus.clone();
        let screen = cx.entity().downgrade();
        let tid = id.to_string();
        let page_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Default::default();
        let at = page_bounds.clone();
        // A click anywhere outside the page (the bar, its address, another
        // tile) takes the keys back from it, before what was clicked takes
        // them itself.
        let outside = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Capture && page_bounds.get().is_some_and(|b| !b.contains(&e.position)) {
                        let _ = screen.update(cx, |s, cx| {
                            let has = s.webs.tiles.get(&tid).is_some_and(|p| p.keys);
                            if has {
                                s.web_blur(&tid, window, cx)
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size_full();
        let on = enabled(cx);
        let page: AnyElement = match (&p.follow, &p.error) {
            _ if !on => calm("Web pages are off", "Turn them on in Settings → General, or open this page in your browser.".into(), t).into_any_element(),
            (Some(f), _) if f.server == Server::Stopped => calm(
                "Server stopped",
                format!("The page comes back when {} prints its address again.", self.agent_name(p.agent.as_deref(), cx)),
                t,
            )
            .into_any_element(),
            (Some(f), _) if matches!(f.server, Server::Unreachable(_)) => {
                let Server::Unreachable(e) = &f.server else { unreachable!() };
                calm(
                    format!("Can't reach {}", super::model::preview_port(&f.url)),
                    format!("{e} It loads again when the server prints its address, or on Reload."),
                    t,
                )
                .into_any_element()
            }
            (None, Some(e)) => {
                let i = id.to_string();
                calm(format!("Can't open {}", host(&p.page_url)), e.clone(), t)
                    .child(crate::kit::small_btn(SharedString::from(format!("web-retry-{id}")), "Try again", t).on_click(cx.listener(move |this, _, _, cx| this.web_reload(&i, cx))))
                    .into_any_element()
            }
            (None, None) if p.page_url.is_empty() => {
                calm("Open a web page", "Type an address above: example.com, localhost:3000. There's no search.".into(), t).into_any_element()
            }
            _ => {
                let surface = preview::surface(p.slot.clone(), p.stage.picture(), t.surface);
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .justify_center()
                    .when(p.phone, |d| d.bg(t.surface_2))
                    .child(
                        div()
                            .h_full()
                            .when(p.phone, |d| d.w(px(PHONE_WIDTH)).max_w_full().flex_none())
                            .when(!p.phone, |d| d.flex_1().min_w_0())
                            .child(surface),
                    )
                    .into_any_element()
            }
        };
        // Where the page is (what "outside" is measured from).
        let page = div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(canvas(move |b, _, _| at.set(Some(b)), |_, _, _, _| {}).absolute().size_full())
            .child(page);
        let _ = window;
        div()
            .id(SharedString::from(format!("web-body-{id}")))
            .debug_selector({
                let id = id.to_string();
                move || format!("web-pane-{id}")
            })
            .track_focus(&focus)
            .key_context("PwWeb")
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.surface)
            .child(outside)
            .child(self.web_bar(id, t, cx))
            .children(p.note.clone().map(|n| {
                div()
                    .flex_none()
                    .px(px(10.))
                    .py(px(5.))
                    .border_b_1()
                    .border_color(t.line)
                    .bg(t.surface_2)
                    .text_size(px(11.5))
                    .text_color(t.text_2)
                    .debug_selector({
                        let id = id.to_string();
                        move || format!("web-note-{id}")
                    })
                    .child(n)
            }))
            .child(page)
            .into_any_element()
    }

    /// Agent `agent`'s name ("the agent" when unknown).
    fn agent_name(&self, agent: Option<&str>, cx: &App) -> String {
        agent
            .and_then(|a| self.agents(cx).iter().find(|x| x.id == a).map(|x| x.name.clone()))
            .unwrap_or_else(|| "the agent".into())
    }

    /// The bar: the tile's drag handle, with back, forward, reload (stop
    /// while loading), the address, the title, and the tile's buttons.
    fn web_bar(&self, id: &str, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.webs.tiles[id];
        let tid = id.to_string();
        let btn = |name: &str, icon_name: &str, tip: &str, on: Option<bool>| {
            crate::kit::icon_btn_state(format!("web-{name}-{tid}"), icon_name, tip.to_string(), true, 13., on, t)
                .debug_selector({
                    let s = format!("web-{name}-{tid}");
                    move || s.clone()
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let act = |cx: &mut Context<Self>, f: TileAct| on_click(cx, id, f);
        let name: SharedString = if !p.title.is_empty() {
            p.title.clone().into()
        } else if !p.page_url.is_empty() {
            host(&p.page_url).into()
        } else {
            "Web page".into()
        };
        let label = match &p.agent {
            Some(a) => format!("{}'s preview", self.agent_name(Some(a), cx)),
            None => p.title.clone(),
        };
        let has_page = !p.page_url.is_empty();
        div()
            .id(SharedString::from(format!("web-bar-{id}")))
            .debug_selector({
                let id = id.to_string();
                move || format!("web-bar-{id}")
            })
            .h(px(BAR_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(4.))
            .border_b_1()
            .border_color(t.line)
            .bg(t.surface)
            .on_drag(WebDrag { id: id.to_string(), name: name.clone() }, |d, grab, w, cx| d.ghost(grab, w, cx))
            // The grip: always there to drag by (the bar's free space
            // drags too).
            .child(
                div()
                    .id(SharedString::from(format!("web-grip-{id}")))
                    .debug_selector({
                        let id = id.to_string();
                        move || format!("web-grip-{id}")
                    })
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .tooltip(tooltip("Drag to move the page"))
                    .child(icon("grip", 13., t.text_4)),
            )
            .child(
                btn("back", "back", "Back", None)
                    .when(!p.can_back, |d| d.opacity(0.4))
                    .on_click(act(cx, |s, id, _, cx| s.with_view(id, |v| v.back(), cx))),
            )
            .child(
                btn("fwd", "chevron", "Forward", None)
                    .when(!p.can_forward, |d| d.opacity(0.4))
                    .on_click(act(cx, |s, id, _, cx| s.with_view(id, |v| v.forward(), cx))),
            )
            .child(if p.loading {
                btn("stop", "stop", "Stop", None).on_click(act(cx, |s, id, _, cx| {
                    s.with_tile(id, |p| p.loading = false, cx);
                    s.with_view(id, |v| v.stop(), cx)
                }))
            } else {
                btn("reload", "refresh", "Reload", None).on_click(act(cx, |s, id, _, cx| s.web_reload(id, cx)))
            })
            .child(
                div()
                    .id(SharedString::from(format!("web-url-{id}")))
                    .flex_1()
                    .min_w(px(60.))
                    .max_w(px(640.))
                    .mx(px(4.))
                    .px(px(6.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .bg(t.surface_2)
                    .text_size(px(11.5))
                    .text_color(t.text_2)
                    .overflow_hidden()
                    .cursor_text()
                    // The field's own clicks: no drag of the tile.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(p.loading, |d| d.child(div().flex_none().mr(px(6.)).size(px(6.)).rounded_full().bg(t.text_4)))
                    .child(div().flex_1().min_w_0().child(p.url_field.clone())),
            )
            .when(!label.is_empty(), |d| {
                d.child(
                    div()
                        .min_w_0()
                        .max_w(px(180.))
                        .px(px(4.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.5))
                        .text_color(t.text_3)
                        .child(label),
                )
            })
            // Free bar to drag by.
            .child(div().flex_1().min_w(px(4.)).h_full())
            .when(p.agent.is_some(), |d| {
                d.child(btn("phone", "phone", "Phone width", Some(p.phone)).on_click(act(cx, |s, id, _, cx| s.with_tile(id, |p| p.phone = !p.phone, cx))))
                    .child(btn("inspect", "inspect", "Web Inspector", None).on_click(act(cx, |s, id, _, cx| s.with_view(id, |v| v.open_inspector(), cx))))
            })
            .when(has_page, |d| {
                d.child(btn("browser", "external", "Open in default browser", None).on_click(act(cx, |s, id, _, cx| {
                    if let Some(u) = s.webs.tiles.get(id).map(|p| p.page_url.clone()).filter(|u| !u.is_empty()) {
                        crate::platform::files::open_url(cx, &u);
                    }
                })))
            })
            .child(btn("close", "x", "Close", None).on_click(act(cx, |s, id, window, cx| s.close_web_tile(id, window, cx))))
    }
}

/// Where a page was drawn, and whether something covers it.
type Placed = (Bounds<Pixels>, bool);
type ClickFn = Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App)>;
type TileAct = fn(&mut MainScreen, &str, &mut Window, &mut Context<MainScreen>);

/// A bar button's click: `f` on web tile `id`.
fn on_click(cx: &mut Context<MainScreen>, id: &str, f: TileAct) -> ClickFn {
    let id = id.to_string();
    Box::new(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| f(this, &id, window, cx)))
}

/// An address's host (the address itself when it has none).
fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let h = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if h.is_empty() {
        url.to_string()
    } else {
        h.to_string()
    }
}

/// Where a page would be: why there's none, calmly.
fn calm(title: impl Into<SharedString>, detail: String, t: &Theme) -> gpui::Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .p(px(16.))
        .child(icon("browser", 22., t.text_4))
        .child(div().text_size(px(13.)).text_color(t.text_2).child(title.into()))
        .child(div().max_w(px(360.)).text_size(px(12.)).text_color(t.text_3).text_center().child(detail))
}
