//! One web page (docs/spec/web.md): its native view ([`crate::webview`]),
//! its bar (back, forward, reload, the address field, the title, the
//! place's buttons) and what it says when there's no page. An entity of
//! its own, kept by tile id in [`super::pages`], so the same page is drawn
//! by whatever holds it now: a tile of a Pitwall window's layout, or a
//! window of its own ([`super::window`]). Moving between them keeps it:
//! the native view is moved into the new window where the platform can
//! ([`crate::webview::WebView::reparent`], macOS), else made again there
//! (the page loads again).
//!
//! Its holder places the view after every frame ([`WebPage::frame`]): where
//! the page's element was drawn, hidden where it wasn't, and swapped for a
//! snapshot while something drawn later covers it ([`Stage`]).
//!
//! Keyboard: a click into the page gives it the keys and the page's focus
//! handle GPUI's focus, so the window's shortcuts still dispatch from
//! there ([`WebEvent::Shortcut`] for the ones the engine sees first). Esc,
//! a click outside the page, or GPUI's focus moving on gives the keys back
//! to the holder.

use std::cell::Cell;
use std::rc::Rc;

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{
    canvas, div, prelude::*, px, AnyElement, AnyWindowHandle, App, AppContext, Bounds, Context,
    Entity, FocusHandle, HitboxId, MouseButton, MouseDownEvent, Pixels, Point, SharedString, Subscription, Task,
    Window, WindowId,
};

use pitwall_proto::AgentView;

use crate::explorer::dock::FilesPlace;
use crate::kit::HoverText as _;
use crate::kit::{icon, tooltip, InputEvent, TextInput};
use crate::main_screen::files_tile::TileDrag;
use crate::main_screen::tree::Content;
use crate::main_screen::workspace::WebTile;
use crate::preview::{self, Act, Follow, Server, Stage, StageOp, DROP_AFTER, PHONE_WIDTH};
use crate::theme::Theme;
use crate::webview::{self, Options, Policy, Snapshot, WebEvent, WebView};

/// The "Move…" / "Dock…" menu: where it opened, the terminals it may replace.
type PlaceMenu = (Point<Pixels>, Vec<(String, String)>);

/// The page's bar: the drag handle, with the page's controls.
pub const BAR_H: f32 = 32.;

/// What holds a page now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKind {
    /// A tile of a Pitwall window's layout.
    Tile,
    /// A window of its own.
    Window,
}

/// The window drawing the page, and where the keys go back to.
#[derive(Clone)]
pub struct Host {
    pub window: AnyWindowHandle,
    pub focus: FocusHandle,
    pub kind: HostKind,
}

/// What the views send the page: their callbacks run outside any GPUI
/// update, so they go through a channel.
enum Msg {
    Event(WebEvent),
    /// A snapshot (the request it answers).
    Snapshot(u64, Option<Snapshot>),
}

pub struct WebPage {
    /// Its tile id (`ui.json` `webTiles`).
    id: String,
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
    /// A page (not a Preview): its browser profile (`None`: Default;
    /// [`super::profiles`]). Its view is made in that profile's store.
    profile: Option<String>,
    /// The bar's profile menu, open at this point.
    profile_menu: Option<Point<Pixels>>,
    /// Said once under the bar: it opened in Default, its own profile gone.
    profile_note: Option<String>,
    /// `None`: not made yet (no address, not drawn yet), or let go.
    view: Option<Box<dyn WebView>>,
    /// The window the native view is in.
    view_window: Option<WindowId>,
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
    host: Option<Host>,
    /// A window holding it: kept above other apps.
    pinned: bool,
    /// The "Move…" / "Dock…" menu, open at this point.
    menu: Option<PlaceMenu>,
    tx: mpsc::UnboundedSender<Msg>,
    /// Loads started (debug: a page moved to another window that didn't
    /// load again kept its count).
    loads: u32,
    _events: Task<()>,
    /// GPUI's focus leaving the page, in its holder's window.
    focus_sub: Option<Subscription>,
    _subs: Vec<Subscription>,
}

/// The setting that turns web pages in Pitwall on and off.
pub const SETTING: &str = "general.webPages";

/// Web pages may be shown in Pitwall (on unless turned off).
pub fn enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(SETTING)
        .and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Pitwall's data folder (web views' cookies and storage go under it).
pub fn data_root(cx: &mut App) -> std::path::PathBuf {
    crate::ui_state::store(cx)
        .read(cx)
        .root()
        .cloned()
        .unwrap_or_else(pitwall_core::paths::Paths::default_root)
}

impl WebPage {
    /// Page `id` showing `kept` (its address; an agent's Preview follows
    /// `agent`'s terminal). No view yet: the first frame that draws it
    /// makes it.
    pub fn new(id: &str, kept: &WebTile, agent: Option<&AgentView>, cx: &mut Context<Self>) -> WebPage {
        let follow = kept.agent.as_ref().map(|_| {
            let mut f = match agent {
                Some(a) => Follow::new(&kept.url, &a.preview_urls, a.preview_seen),
                None => Follow::new(&kept.url, &[], 0),
            };
            // Not running (or not printing) now: stopped until it does.
            if agent.is_none_or(|a| !a.running || a.preview_urls.is_empty()) {
                f.server = Server::Stopped;
            }
            f
        });
        // Its profile, if it is still one here; else Default, said once.
        let (profile, fell_back) = match kept.agent {
            Some(_) => (None, false),
            None => super::profiles::resolve_now(kept.profile.as_deref(), cx),
        };
        let profile_note = fell_back.then(|| {
            keep_profile(id, None, cx);
            if super::profiles::separate(cx) {
                "Its browser profile was removed, so it opened in Default.".to_string()
            } else {
                format!("It opened in Default. {}", super::profiles::NEEDS_STORES)
            }
        });
        let focus = cx.focus_handle();
        let url_field = cx.new(|cx| {
            let mut f = TextInput::single("Type a web address", cx).mono().bare();
            f.set_text(kept.url.as_str(), cx);
            // Not focused: the start of the address shows (its host).
            f.cursor_to_start(cx);
            f
        });
        let subs = vec![cx.subscribe(&url_field, |this, _, e: &InputEvent, cx| match e {
            InputEvent::Submit | InputEvent::SubmitShift => this.in_host(cx, |p, window, cx| p.commit_address(window, cx)),
            InputEvent::Cancel => this.in_host(cx, |p, window, cx| {
                // The address as it was; the keys back to the holder.
                let u = p.page_url.clone();
                p.note = None;
                p.url_field.update(cx, |f, cx| {
                    f.set_text(u, cx);
                    f.cursor_to_start(cx);
                });
                p.give_back_keys(window, cx);
                cx.notify();
            }),
            _ => {}
        })];
        let (tx, mut rx) = mpsc::unbounded::<Msg>();
        let events = cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let host = match this.read_with(cx, |p, _| p.host.as_ref().map(|h| h.window)) {
                    Ok(h) => h,
                    Err(_) => break,
                };
                // Its holder's window gone (it moved and the new one hasn't
                // drawn it yet): heard without a window.
                let host = host.filter(|h| cx.update(|cx| cx.windows().iter().any(|w| w.window_id() == h.window_id())));
                let this = this.clone();
                match msg {
                    // A shortcut the page saw first: Pitwall's, dispatched
                    // from the page's focus once nothing is being updated
                    // (its actions update the holder).
                    Msg::Event(WebEvent::Shortcut(k)) => {
                        let esc = k.key == "escape" && !k.modifiers.modified();
                        if let Some(h) = host {
                            let _ = cx.update_window(h, |_, window, cx| {
                                if esc {
                                    let _ = this.update(cx, |p, cx| p.blur(window, cx));
                                } else {
                                    window.dispatch_keystroke(k, cx);
                                }
                            });
                        }
                    }
                    Msg::Event(e) => match host {
                        Some(h) => {
                            let _ = cx.update_window(h, |_, window, cx| this.update(cx, |p, cx| p.web_event(e, Some(window), cx)));
                        }
                        None => {
                            let _ = this.update(cx, |p, cx| p.web_event(e, None, cx));
                        }
                    },
                    Msg::Snapshot(n, shot) => {
                        let _ = this.update(cx, |p, cx| p.snapshot(n, shot, cx));
                    }
                }
            }
        });
        WebPage {
            id: id.to_string(),
            project: agent.map(|a| a.project.clone()).unwrap_or_default(),
            profile,
            profile_menu: None,
            profile_note,
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
            view_window: None,
            stage: Stage::default(),
            slot: Default::default(),
            snap: 0,
            drop_timer: None,
            focus,
            keys: false,
            url_field,
            note: None,
            host: None,
            pinned: false,
            menu: None,
            tx,
            loads: 0,
            _events: events,
            focus_sub: None,
            _subs: subs,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// An agent's Preview: whose.
    pub fn agent(&self) -> Option<&str> {
        self.agent.as_deref()
    }

    pub fn page_url(&self) -> &str {
        &self.page_url
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Its browser profile (`None`: Default).
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// What it said about its profile (it opened in Default instead).
    pub fn profile_note(&self) -> Option<&str> {
        self.profile_note.as_deref()
    }

    /// Into browser profile `id` (`None`: Default): a page is bound to one
    /// store, so its view goes and is made again there, on the address it
    /// shows now (the next frame that draws it). Kept in `ui.json`. Not a
    /// Preview's (its project's own store).
    pub fn set_profile(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let id = id.filter(|i| i != pitwall_core::accounts::DEFAULT);
        self.profile_menu = None;
        if self.agent.is_some() || self.profile == id {
            return cx.notify();
        }
        self.profile = id;
        self.profile_note = None;
        self.note = None;
        self.error = None;
        self.view = None;
        self.view_window = None;
        self.stage = Stage::default();
        self.loading = false;
        self.can_back = false;
        self.can_forward = false;
        self.drop_timer = None;
        keep_profile(&self.id, self.profile.as_deref(), cx);
        cx.notify();
    }

    /// Loads started so far.
    pub fn loads(&self) -> u32 {
        self.loads
    }

    pub fn has_view(&self) -> bool {
        self.view.is_some()
    }

    pub fn server(&self) -> Option<Server> {
        self.follow.as_ref().map(|f| f.server.clone())
    }

    /// Why there's no page, or the note under the bar.
    pub fn error(&self) -> Option<String> {
        self.error.clone().or(self.note.clone())
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn field(&self) -> &Entity<TextInput> {
        &self.url_field
    }

    pub fn host(&self) -> Option<&Host> {
        self.host.as_ref()
    }

    pub fn slot(&self) -> Option<HitboxId> {
        self.slot.get()
    }

    pub fn pinned(&self) -> bool {
        self.pinned
    }

    pub fn set_pinned(&mut self, on: bool, cx: &mut Context<Self>) {
        self.pinned = on;
        cx.notify();
    }

    /// The "Move…" / "Dock…" menu is open (tests).
    #[cfg(test)]
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Held by `host` now (drawn in its window): GPUI's focus leaving the
    /// page there takes the keys from it.
    pub fn set_host(&mut self, host: Host, window: &mut Window, cx: &mut Context<Self>) {
        let same = self.host.as_ref().is_some_and(|h| h.window.window_id() == host.window.window_id() && h.kind == host.kind);
        if same {
            return;
        }
        self.menu = None;
        self.focus_sub = Some(cx.on_focus_out(&self.focus, window, |p, _, _, _| {
            // GPUI's focus moved on (a shortcut, a click, Esc): so do the keys.
            p.keys = false;
            if let Some(v) = p.view.as_mut() {
                v.blur();
            }
        }));
        self.host = Some(host);
        cx.notify();
    }

    /// `f` with the holder's window (nothing when no window holds it).
    fn in_host(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut WebPage, &mut Window, &mut Context<WebPage>) + 'static) {
        let Some(h) = self.host.as_ref().map(|h| h.window) else { return };
        let me = cx.entity().downgrade();
        // (After this update: the page is being updated now.)
        cx.defer(move |cx| {
            let _ = cx.update_window(h, |_, window, cx| me.update(cx, |p, cx| f(p, window, cx)));
        });
    }

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

    /// Where its cookies and storage are.
    fn data_dir(&self, cx: &mut App) -> std::path::PathBuf {
        match self.policy() {
            Policy::Local => preview::data_dir(&data_root(cx), &self.project),
            Policy::Web => super::profiles::dir(self.profile.as_deref(), cx),
        }
    }

    /// A web view on its page in `window` (an agent's Preview opens it
    /// first, to fall back to the browser when none can be made).
    pub fn make_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let tx = self.tx.clone();
        // Never a profile's page in a store shared with the others.
        if self.policy() == Policy::Web && !super::profiles::usable(self.profile.as_deref(), super::profiles::separate(cx)) {
            return Err(super::profiles::NEEDS_STORES.into());
        }
        let dir = self.data_dir(cx);
        let v = preview::create(
            window,
            Options {
                url: self.page_url.clone(),
                policy: self.policy(),
                data_dir: dir,
                // A profile besides Default: a store of its own.
                own_store: self.policy() == Policy::Web && self.profile.is_some(),
                events: Box::new(move |e| {
                    let _ = tx.unbounded_send(Msg::Event(e));
                }),
            },
            cx,
        )?;
        self.view = Some(v);
        self.view_window = Some(window.window_handle().window_id());
        self.loading = true;
        self.stage = Stage::default();
        Ok(())
    }

    /// Go to `url` (an agent asked, or the Preview's chip): the address
    /// field says it, the page loads it.
    pub fn navigate(&mut self, url: &str, cx: &mut Context<Self>) {
        self.note = None;
        self.error = None;
        if let Some(f) = self.follow.as_mut() {
            f.url = url.to_string();
            f.retry();
        }
        if self.page_url != url {
            self.page_url = url.to_string();
            self.url_field.update(cx, |f, cx| {
                f.set_text(url, cx);
                f.cursor_to_start(cx);
            });
            if let Some(v) = self.view.as_mut() {
                v.navigate(url);
            }
        }
        cx.notify();
    }

    /// An agent's Preview opened again on `url` with the terminal's
    /// addresses: followed from there.
    pub fn follow_from(&mut self, url: &str, a: &AgentView, cx: &mut Context<Self>) {
        if self.page_url != url {
            self.follow = Some(Follow::new(url, &a.preview_urls, a.preview_seen));
            self.page_url = url.to_string();
            self.url_field.update(cx, |f, cx| {
                f.set_text(url, cx);
                f.cursor_to_start(cx);
            });
            if let Some(v) = self.view.as_mut() {
                v.navigate(url);
            }
            cx.notify();
        }
    }

    /// The keys to its address field (all of it selected).
    pub fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keys = false;
        if let Some(v) = self.view.as_mut() {
            v.blur();
        }
        self.url_field.update(cx, |f, cx| f.focus_all(window, cx));
        cx.notify();
    }

    /// The keys back to the holder (its focused tile's terminal).
    pub fn blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keys = false;
        if let Some(v) = self.view.as_mut() {
            v.blur();
        }
        if self.focus.contains_focused(window, cx) {
            self.give_back_keys(window, cx);
        }
    }

    fn give_back_keys(&self, window: &mut Window, cx: &mut App) {
        if let Some(h) = &self.host {
            window.focus(&h.focus, cx);
        }
    }

    /// The keys are in the page or its bar.
    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    /// Enter in the address field: go there (a page that isn't one is said
    /// under the bar, nothing loads).
    fn commit_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.url_field.read(cx).text().to_string();
        match webview::address(&text) {
            Ok(url) => {
                self.note = None;
                self.profile_note = None;
                self.error = None;
                if let Some(f) = self.follow.as_mut() {
                    // Picked here: the terminal's next new address still wins.
                    f.url = url.clone();
                    f.retry();
                }
                self.page_url = url.clone();
                self.url_field.update(cx, |f, cx| f.set_text(url.as_str(), cx));
                if let Some(v) = self.view.as_mut() {
                    v.navigate(&url);
                    v.focus();
                }
                keep_url(&self.id, &url, cx);
                // The page takes the keys once it is made and shown.
                if self.view.is_none() {
                    window.focus(&self.focus, cx);
                }
            }
            Err(e) => self.note = Some(e),
        }
        cx.notify();
    }

    /// A frame of `window` was drawn: the view follows its element
    /// (`placed`: its bounds and whether something covers it; `None`: not
    /// drawn) and steps aside while covered or `dragging`. Moved in from
    /// another window, the view goes along (or is made again). Whether the
    /// window needs drawing again (the picture changed).
    pub fn frame(&mut self, placed: Option<(Bounds<Pixels>, bool)>, dragging: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let here = window.window_handle().window_id();
        let on = enabled(cx);
        let mut repaint = false;
        // Drawn here now, its view in another window: moved along.
        if placed.is_some() && self.view.is_some() && self.view_window != Some(here) {
            let moved = self.view.as_mut().is_some_and(|v| v.reparent(window));
            if moved {
                self.view_window = Some(here);
                // Placed afresh in its new window.
                self.stage = Stage::default();
            } else {
                self.view = None;
                self.view_window = None;
                self.stage = Stage::default();
            }
        }
        let live = self.live() && on;
        // Not made yet (or let go while hidden), shown now: made, on its page.
        if self.view.is_none() && live && placed.is_some() {
            let focus_page = self.focus.is_focused(window);
            match self.make_view(window, cx) {
                Ok(()) => {
                    if focus_page {
                        if let Some(v) = self.view.as_mut() {
                            v.focus();
                        }
                    }
                }
                Err(e) => {
                    match self.follow.as_mut() {
                        Some(f) => f.failed(&e),
                        None => self.error = Some(format!("Web pages can't be shown in Pitwall here: {e}")),
                    }
                    cx.notify();
                    return true;
                }
            }
        }
        // Its view lives in another window that doesn't draw it: left alone.
        if self.view_window.is_some_and(|w| w != here) {
            return false;
        }
        let tx = self.tx.clone();
        let Some(view) = self.view.as_mut() else { return repaint };
        let had_picture = self.stage.picture().is_some();
        let covered = placed.is_some_and(|(_, c)| preview::covering(c, dragging));
        let ops = self.stage.frame(placed.map(|(b, _)| b), covered, live);
        for op in ops {
            match op {
                StageOp::SetBounds(b) => view.set_bounds(b),
                StageOp::Show => view.set_visible(true),
                StageOp::Hide => view.set_visible(false),
                StageOp::Snapshot => {
                    self.snap += 1;
                    let (n, tx) = (self.snap, tx.clone());
                    view.snapshot(Box::new(move |shot| {
                        let _ = tx.unbounded_send(Msg::Snapshot(n, shot));
                    }));
                }
            }
        }
        repaint |= had_picture != self.stage.picture().is_some();
        // An agent's Preview hidden (not just covered): let go after a
        // while. A page the user opened stays loaded while its tile is.
        if self.agent.is_none() || !self.stage.idle() {
            self.drop_timer = None;
        } else if self.drop_timer.is_none() {
            self.drop_timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DROP_AFTER).await;
                let _ = this.update(cx, |p, _| p.drop_idle());
            }));
        }
        if repaint {
            cx.notify();
        }
        repaint
    }

    /// Hidden for [`DROP_AFTER`]: let the view go, keep its address.
    fn drop_idle(&mut self) {
        self.drop_timer = None;
        if self.stage.idle() {
            self.view = None;
            self.view_window = None;
            self.stage = Stage::default();
        }
    }

    fn snapshot(&mut self, n: u64, shot: Option<Snapshot>, cx: &mut Context<Self>) {
        if self.snap != n {
            return;
        }
        let ops = self.stage.snapshot(shot.and_then(preview::to_image));
        if let Some(v) = self.view.as_mut() {
            for op in ops {
                if op == StageOp::Hide {
                    v.set_visible(false);
                }
            }
        }
        // Draw the picture in its place.
        cx.notify();
    }

    fn web_event(&mut self, e: WebEvent, window: Option<&mut Window>, cx: &mut Context<Self>) {
        match e {
            WebEvent::Url(u) => {
                if webview::stays(self.policy(), &u) && self.page_url != u {
                    // Not while the address is being typed.
                    let typing = window.as_deref().is_some_and(|w| self.url_field.read(cx).is_focused(w));
                    if !typing {
                        self.url_field.update(cx, |f, cx| {
                            f.set_text(u.as_str(), cx);
                            f.cursor_to_start(cx);
                        });
                    }
                    self.page_url = u.clone();
                    keep_url(&self.id, &u, cx);
                }
            }
            WebEvent::Title(t) => {
                self.title = t;
                if self.host.as_ref().is_some_and(|h| h.kind == HostKind::Window) {
                    if let Some(w) = window {
                        w.set_window_title(&super::window::title(&self.title, &self.page_url));
                    }
                }
            }
            WebEvent::Loading(l) => {
                self.loading = l;
                if l {
                    self.note = None;
                    self.loads += 1;
                }
            }
            WebEvent::Failed { error, .. } => match self.follow.as_mut() {
                Some(f) => f.failed(&error),
                None => self.error = Some(error),
            },
            WebEvent::CanGo { back, forward } => {
                self.can_back = back;
                self.can_forward = forward;
            }
            WebEvent::Focused => {
                self.keys = true;
                if let Some(w) = window {
                    w.focus(&self.focus, cx);
                }
            }
            WebEvent::Download(u) => {
                self.note = Some(format!("Downloads aren't saved from Pitwall's web pages: {u} can be opened in your browser."));
            }
            WebEvent::Shortcut(_) => {}
        }
        cx.notify();
    }

    /// The agents changed: a Preview follows its terminal.
    pub fn follow_agents(&mut self, agents: &[AgentView], cx: &mut Context<Self>) {
        let (Some(agent), Some(f)) = (self.agent.as_deref(), self.follow.as_mut()) else { return };
        let Some(a) = agents.iter().find(|a| a.id == agent) else { return };
        match f.update(&a.preview_urls, a.preview_seen, a.running) {
            Some(Act::Navigate(u)) => {
                self.page_url = u.clone();
                self.url_field.update(cx, |f, cx| {
                    f.set_text(u.as_str(), cx);
                    f.cursor_to_start(cx);
                });
                if let Some(v) = self.view.as_mut() {
                    v.navigate(&u);
                }
                keep_url(&self.id, &u, cx);
            }
            Some(Act::Reload) => {
                if let Some(v) = self.view.as_mut() {
                    v.reload();
                }
            }
            Some(Act::Stop) | None => {}
        }
        cx.notify();
    }

    fn with_view(&mut self, f: impl FnOnce(&mut Box<dyn WebView>), cx: &mut Context<Self>) {
        if let Some(v) = self.view.as_mut() {
            f(v);
            cx.notify();
        }
    }

    /// Reload (or Retry): the page again, its error forgotten.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.note = None;
        if let Some(f) = self.follow.as_mut() {
            f.retry();
        }
        if let Some(v) = self.view.as_mut() {
            v.reload();
        }
        cx.notify();
    }

    // ── drawing ──────────────────────────────────────────────────────────

    /// The bar, then the page (or why there's none).
    fn body(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let id = self.id.clone();
        let me = cx.entity().downgrade();
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
                        let _ = me.update(cx, |p, cx| {
                            if p.keys {
                                p.blur(window, cx)
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size_full();
        let on = enabled(cx);
        let page: AnyElement = match (&self.follow, &self.error) {
            _ if !on => calm("Web pages are off", "Turn them on in Settings → General, or open this page in your browser.".into(), t).into_any_element(),
            (Some(f), _) if f.server == Server::Stopped => calm(
                "Server stopped",
                format!("The page comes back when {} prints its address again.", agent_name(self.agent.as_deref(), cx)),
                t,
            )
            .into_any_element(),
            (Some(f), _) if matches!(f.server, Server::Unreachable(_)) => {
                let Server::Unreachable(e) = &f.server else { unreachable!() };
                calm(
                    format!("Can't reach {}", crate::main_screen::model::preview_port(&f.url)),
                    format!("{e} It loads again when the server prints its address, or on Reload."),
                    t,
                )
                .into_any_element()
            }
            (None, Some(e)) => calm(format!("Can't open {}", host(&self.page_url)), e.clone(), t)
                .child(crate::kit::small_btn(SharedString::from(format!("web-retry-{id}")), "Try again", t).on_click(cx.listener(|p, _, _, cx| p.reload(cx))))
                .into_any_element(),
            (None, None) if self.page_url.is_empty() => {
                calm("Open a web page", "Type an address above: example.com, localhost:3000. There's no search.".into(), t).into_any_element()
            }
            _ => {
                let surface = preview::surface(self.slot.clone(), self.stage.picture(), t.surface);
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .justify_center()
                    .when(self.phone, |d| d.bg(t.surface_2))
                    .child(
                        div()
                            .h_full()
                            .when(self.phone, |d| d.w(px(PHONE_WIDTH)).max_w_full().flex_none())
                            .when(!self.phone, |d| d.flex_1().min_w_0())
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
                let id = id.clone();
                move || format!("web-pane-{id}")
            })
            .track_focus(&self.focus)
            .key_context("PwWeb")
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.surface)
            .child(outside)
            .child(self.bar(t, cx))
            .children(self.note.clone().or(self.profile_note.clone()).map(|n| {
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
                        let id = id.clone();
                        move || format!("web-note-{id}")
                    })
                    .child(n)
            }))
            .child(page)
            .children(self.menu.clone().map(|(at, replace)| self.place_menu(at, replace, t, cx)))
            .children(self.profile_menu.map(|at| self.profile_menu(at, t, cx)))
            .into_any_element()
    }

    /// The bar: the drag handle, with back, forward, reload (stop while
    /// loading), the address, the title, and the place's buttons (a tile:
    /// Move…, Open in window, Close; a window: Dock…, Pin).
    fn bar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.id.clone();
        let tid = id.clone();
        let kind = self.host.as_ref().map_or(HostKind::Tile, |h| h.kind);
        let btn = |name: &str, icon_name: &str, tip: &str, on: Option<bool>| {
            crate::kit::icon_btn_state(format!("web-{name}-{tid}"), icon_name, tip.to_string(), true, 13., on, t)
                .debug_selector({
                    let s = format!("web-{name}-{tid}");
                    move || s.clone()
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let name: SharedString = if !self.title.is_empty() {
            self.title.clone().into()
        } else if !self.page_url.is_empty() {
            host(&self.page_url).into()
        } else {
            "Web page".into()
        };
        let label = match &self.agent {
            Some(a) => format!("{}'s preview", agent_name(Some(a), cx)),
            None => self.title.clone(),
        };
        let has_page = !self.page_url.is_empty();
        // Its browser profile: shown once there is more than Default (or
        // it is in another); Default subdued.
        let profile_chip = self.agent.is_none().then(|| super::profiles::all(cx)).and_then(|all| {
            (!all.is_empty() || self.profile.is_some()).then(|| {
                let name = super::profiles::name(self.profile.as_deref(), cx);
                (name, self.profile.is_none())
            })
        });
        let menu_tip = match kind {
            HostKind::Tile => "Move it in the layout, or to a window",
            HostKind::Window => "Place it in a Pitwall window's layout",
        };
        div()
            .id(SharedString::from(format!("web-bar-{id}")))
            .debug_selector({
                let id = id.clone();
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
            // A tile: dragged in its window's layout by the bar (GPUI's
            // drag); a window: its grip drags it into a layout.
            .when(kind == HostKind::Tile, |d| {
                d.on_drag(TileDrag { what: Content::Web(id.clone()), name: name.clone() }, |d, grab, w, cx| d.ghost(grab, w, cx))
            })
            .child(
                div()
                    .id(SharedString::from(format!("web-grip-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("web-grip-{id}")
                    })
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .tooltip(tooltip(match kind {
                        HostKind::Tile => "Drag to move the page",
                        HostKind::Window => "Drag into a Pitwall window to place it in the layout",
                    }))
                    .when(kind == HostKind::Window, |d| {
                        d.on_mouse_down(MouseButton::Left, cx.listener(|p, _, window, cx| {
                            cx.stop_propagation();
                            let id = p.id.clone();
                            window.defer(cx, move |window, cx| super::place::drag_start(&id, window, cx));
                        }))
                    })
                    .child(icon("grip", 13., t.text_4)),
            )
            .child(
                btn("back", "back", "Back", None)
                    .when(!self.can_back, |d| d.opacity(0.4))
                    .on_click(cx.listener(|p, _, _, cx| p.with_view(|v| v.back(), cx))),
            )
            .child(
                btn("fwd", "chevron", "Forward", None)
                    .when(!self.can_forward, |d| d.opacity(0.4))
                    .on_click(cx.listener(|p, _, _, cx| p.with_view(|v| v.forward(), cx))),
            )
            .child(if self.loading {
                btn("stop", "stop", "Stop", None).on_click(cx.listener(|p, _, _, cx| {
                    p.loading = false;
                    p.with_view(|v| v.stop(), cx);
                    cx.notify();
                }))
            } else {
                btn("reload", "refresh", "Reload", None).on_click(cx.listener(|p, _, _, cx| p.reload(cx)))
            })
            .child(
                div()
                    .id(SharedString::from(format!("web-url-{id}")))
                    // As wide as it may be first; the free bar gets the rest.
                    .flex_basis(px(640.))
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
                    .when(self.loading, |d| d.child(div().flex_none().mr(px(6.)).size(px(6.)).rounded_full().bg(t.text_4)))
                    .child(div().flex_1().min_w_0().child(self.url_field.clone())),
            )
            .when_some(profile_chip, |d, (pname, subdued)| {
                let sel = format!("web-profile-{id}");
                d.child(
                    div()
                        .id(SharedString::from(sel.clone()))
                        .debug_selector(move || sel.clone())
                        .flex_none()
                        .h(px(20.))
                        .max_w(px(140.))
                        .px(px(7.))
                        .mr(px(2.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(t.line_strong)
                        .when(self.profile_menu.is_some(), |d| d.bg(t.surface_2))
                        .text_size(px(11.))
                        .text_color(if subdued { t.text_3 } else { t.text_2 })
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .cursor_pointer()
                        .hover_probed(|s| s.bg(t.surface_2))
                        .tooltip(tooltip(format!("Browser profile: {pname}, with its own sign-ins and site data. Click to switch.")))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|p, e: &gpui::ClickEvent, _, cx| {
                            p.menu = None;
                            p.profile_menu = match p.profile_menu {
                                Some(_) => None,
                                None => Some(e.position()),
                            };
                            cx.notify();
                        }))
                        .child(pname),
                )
            })
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
            // Free bar to drag by: what the address field leaves past its
            // widest.
            .child(div().flex_1().min_w(px(4.)).h_full())
            .when(self.agent.is_some(), |d| {
                d.child(btn("phone", "phone", "Phone width", Some(self.phone)).on_click(cx.listener(|p, _, _, cx| {
                    p.phone = !p.phone;
                    cx.notify();
                })))
                .child(btn("inspect", "inspect", "Web Inspector", None).on_click(cx.listener(|p, _, _, cx| p.with_view(|v| v.open_inspector(), cx))))
            })
            .when(has_page, |d| {
                d.child(btn("browser", "external", "Open in default browser", None).on_click(cx.listener(|p, _, _, cx| {
                    if !p.page_url.is_empty() {
                        crate::platform::files::open_url(cx, &p.page_url);
                    }
                })))
            })
            .child(btn(if kind == HostKind::Tile { "move" } else { "dock" }, "grid", menu_tip, Some(self.menu.is_some())).on_click(cx.listener(
                |p, e: &gpui::ClickEvent, _, cx| {
                    p.menu = match p.menu {
                        Some(_) => None,
                        None => Some((e.position(), super::place::replaceable(&p.id, cx))),
                    };
                    cx.notify();
                },
            )))
            .map(|d| match kind {
                HostKind::Tile => d
                    .child(btn("undock", "window", "Open in a window of its own", None).on_click(cx.listener(|p, _, _, cx| {
                        let id = p.id.clone();
                        cx.defer(move |cx| {
                            if let Err(e) = super::place::pop_out(&id, None, true, cx) {
                                eprintln!("pitwall: could not open the web page's window: {e}");
                            }
                        });
                    })))
                    .child(btn("close", "x", "Close", None).on_click(cx.listener(|p, _, _, cx| {
                        let id = p.id.clone();
                        cx.defer(move |cx| super::pages::close(&id, cx));
                    }))),
                HostKind::Window => d.when(crate::platform::can_keep_on_top(), |d| {
                    let pinned = self.pinned;
                    d.child(
                        btn("pin", "pin", if pinned { "Kept on top of other windows: click to let it go behind them" } else { "Keep on top of other windows" }, Some(pinned))
                            .on_click(cx.listener(|p, _, _, cx| {
                                p.pinned = !p.pinned;
                                let (on, host) = (p.pinned, p.host.as_ref().map(|h| h.window));
                                if let Some(h) = host {
                                    cx.defer(move |cx| {
                                        let _ = cx.update_window(h, |_, window, _| crate::platform::keep_on_top(window, on));
                                    });
                                }
                                cx.notify();
                            })),
                    )
                }),
            })
    }

    /// The profile chip's menu: Default and every profile (this page's
    /// ticked); picking another loads the page again there. Profiles that
    /// can't be used here (macOS before 14) are shown, not picked.
    fn profile_menu(&self, at: Point<Pixels>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use gpui::{anchored, deferred};
        let sep = super::profiles::separate(cx);
        let mut items: Vec<(Option<String>, String)> = vec![(None, super::profiles::DEFAULT_NAME.into())];
        items.extend(super::profiles::all(cx).into_iter().map(|a| (Some(a.id), a.name)));
        let me = cx.entity().downgrade();
        let rows = items.into_iter().enumerate().map(|(i, (pid, pname))| {
            let on = pid == self.profile;
            let can = super::profiles::usable(pid.as_deref(), sep);
            let me = me.clone();
            crate::kit::menu::menu_item(("web-profile-item", i), if on { "check" } else { "" }, pname, (!can).then(|| "needs macOS 14".into()), false, t)
                .debug_selector(move || format!("web-profile-item-{i}"))
                .when(!can, |d| d.opacity(0.5).cursor_default())
                .on_click(move |_, _, cx| {
                    if can {
                        let pid = pid.clone();
                        let _ = me.update(cx, |p, cx| p.set_profile(pid, cx));
                    }
                })
        });
        let dismiss = cx.entity().downgrade();
        deferred(
            anchored().position(at).snap_to_window().child(
                crate::kit::menu::menu_panel(t)
                    .id("web-profile-menu")
                    .debug_selector(|| "web-profile-menu".into())
                    .mt(px(6.))
                    .max_w(px(300.))
                    .child(div().px(px(8.)).py(px(4.)).text_size(px(11.)).text_color(t.text_3).child("Browser profile"))
                    .children(rows)
                    .child(
                        div()
                            .px(px(8.))
                            .py(px(4.))
                            .text_size(px(11.))
                            .text_color(t.text_3)
                            .child(if sep { "Each has its own sign-ins. Switching loads this page again there." } else { super::profiles::NEEDS_STORES }),
                    )
                    .on_mouse_down_out(move |_, _, cx| {
                        let _ = dismiss.update(cx, |p, cx| {
                            p.profile_menu = None;
                            cx.notify();
                        });
                    }),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }

    /// The "Move…" / "Dock…" menu's places ([`crate::explorer::dock::menu`]).
    fn place_menu(&self, at: Point<Pixels>, replace: Vec<(String, String)>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        let id = self.id.clone();
        let tile = self.host.as_ref().is_none_or(|h| h.kind == HostKind::Tile);
        let anchor = super::place::anchor_agent(&self.id, cx).map(|a| agent_name(Some(&a), cx));
        let pick = move |p: Option<(FilesPlace, Option<String>)>, cx: &mut App| {
            let _ = me.update(cx, |p, cx| {
                p.menu = None;
                cx.notify();
            });
            if let Some((place, of)) = p {
                let id = id.clone();
                cx.defer(move |cx| {
                    if let Err(e) = super::place::place(&id, place, of.as_deref(), cx) {
                        eprintln!("pitwall: could not move the web page: {e}");
                    }
                });
            }
        };
        crate::explorer::dock::menu(at, tile, anchor, replace, "page", t, pick)
    }
}

impl Render for WebPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).clone();
        self.body(&t, window, cx)
    }
}

/// The address page `id` shows now, kept in `ui.json` for the next start.
fn keep_url(id: &str, url: &str, cx: &mut App) {
    let (id, url) = (id.to_string(), url.to_string());
    // (Not inside the page's update: after it.)
    cx.defer(move |cx| {
        if crate::ui_state::get(cx).web_tiles.get(&id).is_some_and(|t| t.url != url) {
            crate::ui_state::update(cx, |s| *s = s.clone().set_web_url(&id, &url));
        }
    });
}

/// The browser profile page `id` is in now, kept in `ui.json`.
fn keep_profile(id: &str, profile: Option<&str>, cx: &mut App) {
    let (id, profile) = (id.to_string(), profile.map(String::from));
    cx.defer(move |cx| {
        if crate::ui_state::get(cx).web_tiles.get(&id).is_some_and(|t| t.profile != profile) {
            crate::ui_state::update(cx, |s| *s = s.clone().set_web_profile(&id, profile.as_deref()));
        }
    });
}

/// Agent `agent`'s name ("the agent" when unknown).
fn agent_name(agent: Option<&str>, cx: &App) -> String {
    agent.and_then(|a| crate::explorer::show::agent_name(a, cx)).unwrap_or_else(|| "the agent".into())
}

/// An address's host (the address itself when it has none).
pub fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let h = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if h.is_empty() {
        url.to_string()
    } else {
        h.to_string()
    }
}

/// Where a page would be: why there's none, calmly.
pub fn calm(title: impl Into<SharedString>, detail: String, t: &Theme) -> gpui::Div {
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
