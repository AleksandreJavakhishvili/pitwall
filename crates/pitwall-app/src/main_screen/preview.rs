//! An agent pane's Preview (docs/spec/terminals.md §4): "Preview" in its
//! header splits the pane, the terminal on one side and the dev server it
//! printed on the other, in a native web view ([`crate::webview`]).
//!
//! The preview is the agent's: it follows what its terminal prints
//! ([`Follow`]), shows "Server stopped" while the agent isn't running, and
//! closes with the agent. The view is made on the first click, hidden while
//! its pane isn't drawn and dropped after [`DROP_AFTER`] hidden (its
//! address is kept: shown again, it comes back). After every frame the
//! window draws, [`MainScreen::preview_frame`] places each view where its
//! element was drawn and swaps it for a snapshot while something drawn later
//! (a menu, a dialog, the palette, a tooltip) covers it ([`Stage`]).
//!
//! Keyboard: a click into the page gives it the keys and the preview's
//! focus handle GPUI's focus, so the window's shortcuts still dispatch from
//! there ([`WebEvent::Shortcut`] for the ones the engine sees first). Esc,
//! a click outside the page, or GPUI's focus moving on gives the keys back.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{
    canvas, div, prelude::*, px, AnyElement, Bounds, Context, Entity, FocusHandle, HitboxId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString, Subscription,
    Task, Window,
};

use pitwall_proto::AgentView;

use super::strip::Toast;
use super::MainScreen;
use crate::kit::{icon, tooltip, TextInput};
use crate::preview::{self, Act, Follow, Server, Stage, StageOp, DROP_AFTER, PHONE_WIDTH};
use crate::theme::Theme;
use crate::webview::{self, Options, Snapshot, WebEvent, WebView};

/// The terminal's share of the pane at first.
const SPLIT: f32 = 0.5;
const MIN_SHARE: f32 = 0.15;
const DIVIDER: f32 = 6.;
const TOOLBAR_H: f32 = 30.;

/// What the views send the screen: their callbacks run outside any GPUI
/// update, so they go through a channel.
enum Msg {
    Event(String, WebEvent),
    /// A snapshot (agent, the request it answers).
    Snapshot(String, u64, Option<Snapshot>),
}

/// One agent's preview.
pub(super) struct Pane {
    follow: Follow,
    /// What the page shows now (links followed inside it).
    page_url: String,
    title: String,
    loading: bool,
    can_back: bool,
    can_forward: bool,
    phone: bool,
    /// The terminal's share of the split.
    frac: f32,
    project: String,
    /// `None`: not made yet, or dropped while hidden.
    view: Option<Box<dyn WebView>>,
    stage: Stage,
    /// Where the page's element was drawn ([`preview::surface`]).
    slot: Rc<Cell<Option<HitboxId>>>,
    /// The split's own box (the divider's drag).
    split: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The snapshot asked for last.
    snap: u64,
    drop_timer: Option<Task<()>>,
    focus: FocusHandle,
    /// The page has the keys (it said [`WebEvent::Focused`]); some engines
    /// leave GPUI's window looking unfocused meanwhile.
    keys: bool,
    url_field: Entity<TextInput>,
    _subs: Vec<Subscription>,
}

/// The window's previews (one per agent).
#[derive(Default)]
pub(super) struct Previews {
    panes: HashMap<String, Pane>,
    tx: Option<mpsc::UnboundedSender<Msg>>,
    _events: Option<Task<()>>,
    /// The frame listener is on.
    listening: bool,
    drag: Option<Drag>,
    /// A view couldn't be made and the user was told why (once).
    told: bool,
}

/// The divider between terminal and page being dragged.
struct Drag {
    agent: String,
    row: bool,
    start: Pixels,
    total: f32,
    from: f32,
}

impl Previews {
    #[cfg(test)]
    pub fn is_open(&self, agent: &str) -> bool {
        self.panes.contains_key(agent)
    }

    #[cfg(test)]
    pub fn has_view(&self, agent: &str) -> bool {
        self.panes.get(agent).is_some_and(|p| p.view.is_some())
    }

    #[cfg(test)]
    pub fn server(&self, agent: &str) -> Option<Server> {
        self.panes.get(agent).map(|p| p.follow.server.clone())
    }

    #[cfg(test)]
    pub fn page_url(&self, agent: &str) -> Option<String> {
        self.panes.get(agent).map(|p| p.page_url.clone())
    }

    #[cfg(test)]
    pub fn focus_of(&self, agent: &str) -> Option<FocusHandle> {
        self.panes.get(agent).map(|p| p.focus.clone())
    }
}

impl MainScreen {
    /// "Preview" in a pane header: open `url` beside `a`'s terminal (or go
    /// there when it is open). Where no web view can be made, the browser
    /// opens it instead and the reason shows once.
    pub(super) fn open_preview(&mut self, a: &AgentView, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.previews.panes.get_mut(&a.id) {
            if p.page_url != url {
                p.follow = Follow::new(url, &a.preview_urls, a.preview_seen);
                p.page_url = url.to_string();
                if let Some(v) = p.view.as_mut() {
                    v.navigate(url);
                }
            }
            cx.notify();
            return;
        }
        let view = match self.make_view(&a.id, &a.project, url, window, cx) {
            Ok(v) => v,
            Err(e) => {
                crate::platform::files::open_url(cx, url);
                if !self.previews.told {
                    self.previews.told = true;
                    self.toast(
                        Toast::info("Opened in the browser", Some(format!("The in-app preview isn't available: {e}"))),
                        cx,
                    );
                }
                return;
            }
        };
        let focus = cx.focus_handle();
        let url_field = cx.new(|cx| {
            let mut f = TextInput::single("", cx).mono().bare();
            f.set_text(url, cx);
            f.set_disabled(true, cx);
            f
        });
        let id = a.id.clone();
        let subs = vec![cx.on_focus_out(&focus, window, move |this, _, _, _| {
            // GPUI's focus moved on (a shortcut, a click, Esc): so do the keys.
            if let Some(p) = this.previews.panes.get_mut(&id) {
                p.keys = false;
                if let Some(v) = p.view.as_mut() {
                    v.blur();
                }
            }
        })];
        self.previews.panes.insert(
            a.id.clone(),
            Pane {
                follow: Follow::new(url, &a.preview_urls, a.preview_seen),
                page_url: url.to_string(),
                title: String::new(),
                loading: true,
                can_back: false,
                can_forward: false,
                phone: false,
                frac: SPLIT,
                project: a.project.clone(),
                view: Some(view),
                stage: Stage::default(),
                slot: Default::default(),
                split: Default::default(),
                snap: 0,
                drop_timer: None,
                focus,
                keys: false,
                url_field,
                _subs: subs,
            },
        );
        self.listen_frames(window, cx);
        cx.notify();
    }

    pub(super) fn close_preview(&mut self, agent: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.previews.panes.remove(agent) {
            if p.focus.contains_focused(window, cx) {
                window.focus(&self.focus);
            }
            cx.notify();
        }
    }

    pub(super) fn preview_open(&self, agent: &str) -> bool {
        self.previews.panes.contains_key(agent)
    }

    /// A web view for `agent` on `url`, its events into the screen.
    fn make_view(
        &mut self,
        agent: &str,
        project: &str,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Box<dyn WebView>, String> {
        let tx = self.preview_channel(window, cx);
        let id = agent.to_string();
        let root = crate::ui_state::store(cx)
            .read(cx)
            .root()
            .cloned()
            .unwrap_or_else(pitwall_core::paths::Paths::default_root);
        preview::create(
            window,
            Options {
                url: url.to_string(),
                data_dir: preview::data_dir(&root, project),
                events: Box::new(move |e| {
                    let _ = tx.unbounded_send(Msg::Event(id.clone(), e));
                }),
            },
            cx,
        )
    }

    /// The views' channel, and the task that hands what comes through it
    /// to the screen.
    fn preview_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> mpsc::UnboundedSender<Msg> {
        if let Some(tx) = &self.previews.tx {
            return tx.clone();
        }
        let (tx, mut rx) = mpsc::unbounded::<Msg>();
        self.previews.tx = Some(tx.clone());
        self.previews._events = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let done = match msg {
                    // A shortcut the page saw first: Pitwall's, dispatched
                    // from the preview's focus (outside the screen's update:
                    // its actions update the screen).
                    Msg::Event(id, WebEvent::Shortcut(k)) => {
                        let esc = k.key == "escape" && !k.modifiers.modified();
                        if esc {
                            this.update_in(cx, |s, window, cx| s.preview_blur(&id, window, cx))
                        } else {
                            cx.update(|window, cx| {
                                window.dispatch_keystroke(k, cx);
                            })
                        }
                    }
                    Msg::Event(id, e) => this.update_in(cx, |s, window, cx| s.preview_event(&id, e, window, cx)),
                    Msg::Snapshot(id, n, shot) => this.update(cx, |s, cx| s.preview_snapshot(&id, n, shot, cx)),
                };
                if done.is_err() {
                    break;
                }
            }
        }));
        tx
    }

    /// After every frame while there are previews: place the views.
    fn listen_frames(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.previews.listening {
            return;
        }
        self.previews.listening = true;
        let screen = cx.entity().downgrade();
        window.on_frame_drawn(move |window, cx| {
            screen
                .update(cx, |s, cx| s.preview_frame(window, cx))
                .unwrap_or(false)
        });
        // Draw a frame for the listener to see the new pane in.
        window.refresh();
    }

    /// A frame was drawn: each view follows its element (or hides when it
    /// wasn't drawn) and steps aside for what covers it. `false`: no
    /// previews left, the listener goes.
    pub(super) fn preview_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.previews.panes.is_empty() {
            self.previews.listening = false;
            return false;
        }
        let places: Vec<(String, Option<Placed>)> = {
            let hitboxes = window.rendered_hitboxes();
            let tip = window.rendered_tooltip_bounds();
            self.previews
                .panes
                .iter()
                .map(|(id, p)| (id.clone(), p.slot.get().and_then(|h| preview::placed(hitboxes, tip, h))))
                .collect()
        };
        let mut repaint = false;
        for (id, place) in places {
            let Some(p) = self.previews.panes.get(&id) else { continue };
            let live = p.follow.server == Server::Live;
            // Dropped while hidden, shown again: back on its page.
            if p.view.is_none() && live && place.is_some() {
                let (project, url) = (p.project.clone(), p.page_url.clone());
                match self.make_view(&id, &project, &url, window, cx) {
                    Ok(v) => {
                        let p = self.previews.panes.get_mut(&id).expect("pane");
                        p.view = Some(v);
                        p.stage = Stage::default();
                    }
                    Err(e) => {
                        if let Some(p) = self.previews.panes.get_mut(&id) {
                            p.follow.failed(&e);
                        }
                        repaint = true;
                        continue;
                    }
                }
            }
            let tx = self.previews.tx.clone();
            let Some(p) = self.previews.panes.get_mut(&id) else { continue };
            let Some(view) = p.view.as_mut() else { continue };
            let had_picture = p.stage.picture().is_some();
            let ops = p.stage.frame(place.map(|(b, _)| b), place.is_some_and(|(_, c)| c), live);
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
            // Hidden (not just covered): dropped after a while.
            if !p.stage.idle() {
                p.drop_timer = None;
            } else if p.drop_timer.is_none() {
                let id = id.clone();
                p.drop_timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(DROP_AFTER).await;
                    let _ = this.update(cx, |s, _| s.preview_drop_idle(&id));
                }));
            }
        }
        if repaint {
            cx.notify();
        }
        true
    }

    /// Hidden for [`DROP_AFTER`]: let the view go, keep its address.
    fn preview_drop_idle(&mut self, agent: &str) {
        if let Some(p) = self.previews.panes.get_mut(agent) {
            p.drop_timer = None;
            if p.stage.idle() {
                p.view = None;
                p.stage = Stage::default();
            }
        }
    }

    fn preview_snapshot(&mut self, agent: &str, n: u64, shot: Option<Snapshot>, cx: &mut Context<Self>) {
        let Some(p) = self.previews.panes.get_mut(agent).filter(|p| p.snap == n) else {
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

    fn preview_event(&mut self, agent: &str, e: WebEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.previews.panes.get_mut(agent) else { return };
        match e {
            WebEvent::Url(u) => {
                if webview::allowed(&u) && p.page_url != u {
                    p.url_field.update(cx, |f, cx| f.set_text(u.as_str(), cx));
                    p.page_url = u;
                }
            }
            WebEvent::Title(t) => p.title = t,
            WebEvent::Loading(l) => p.loading = l,
            WebEvent::Failed { error, .. } => p.follow.failed(&error),
            WebEvent::CanGo { back, forward } => {
                p.can_back = back;
                p.can_forward = forward;
            }
            WebEvent::Focused => {
                p.keys = true;
                let f = p.focus.clone();
                window.focus(&f);
            }
            WebEvent::Shortcut(_) => {}
        }
        cx.notify();
    }

    /// The keys back to the screen (its focused pane's terminal).
    fn preview_blur(&mut self, agent: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.previews.panes.get_mut(agent) {
            p.keys = false;
            if let Some(v) = p.view.as_mut() {
                v.blur();
            }
            if p.focus.contains_focused(window, cx) {
                window.focus(&self.focus);
            }
        }
    }

    /// The agents changed: previews follow their terminals, and go with
    /// their agents.
    pub(super) fn previews_follow(&mut self, cx: &mut Context<Self>) {
        if self.previews.panes.is_empty() {
            return;
        }
        let agents = self.agents(cx).to_vec();
        self.previews.panes.retain(|id, _| agents.iter().any(|a| a.id == *id));
        for a in &agents {
            let Some(p) = self.previews.panes.get_mut(&a.id) else { continue };
            match p.follow.update(&a.preview_urls, a.preview_seen, a.running) {
                Some(Act::Navigate(u)) => {
                    p.page_url = u.clone();
                    p.url_field.update(cx, |f, cx| f.set_text(u.as_str(), cx));
                    if let Some(v) = p.view.as_mut() {
                        v.navigate(&u);
                    }
                }
                Some(Act::Reload) => {
                    if let Some(v) = p.view.as_mut() {
                        v.reload();
                    }
                }
                Some(Act::Stop) | None => {}
            }
        }
    }

    fn with_pane(&mut self, agent: &str, f: impl FnOnce(&mut Pane), cx: &mut Context<Self>) {
        if let Some(p) = self.previews.panes.get_mut(agent) {
            f(p);
            cx.notify();
        }
    }

    pub(super) fn preview_drag_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = &self.previews.drag else { return };
        let pos = if d.row { e.position.x } else { e.position.y };
        let frac = (d.from + f32::from(pos - d.start) / d.total).clamp(MIN_SHARE, 1. - MIN_SHARE);
        let agent = d.agent.clone();
        self.with_pane(&agent, |p| p.frac = frac, cx);
    }

    pub(super) fn preview_drag_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.previews.drag.take().is_some() {
            cx.notify();
        }
    }

    // ── drawing ──────────────────────────────────────────────────────────

    /// `term` (a pane's terminal) split with its agent's preview, if open.
    pub(super) fn with_preview(&mut self, a: &AgentView, term: AnyElement, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(p) = self.previews.panes.get(&a.id) else {
            return term;
        };
        let row = !self.ui.preview_below;
        let frac = p.frac;
        let split = p.split.clone();
        let measure = canvas(move |b, _, _| split.set(Some(b)), |_, _, _, _| {})
            .absolute()
            .size_full();
        let cell = |share: f32| {
            let mut c = div().relative().flex().min_w_0().min_h_0().flex_basis(px(0.));
            c.style().flex_grow = Some(share);
            c.style().flex_shrink = Some(1.);
            c
        };
        let id = a.id.clone();
        let divider = div()
            .id(SharedString::from(format!("preview-divider-{}", a.id)))
            .flex_none()
            .when(row, |d| d.w(px(DIVIDER)).h_full().cursor_col_resize().border_l_1())
            .when(!row, |d| d.h(px(DIVIDER)).w_full().cursor_row_resize().border_t_1())
            .border_color(t.line)
            .bg(t.surface)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let Some(p) = this.previews.panes.get(&id) else { return };
                    let Some(b) = p.split.get() else { return };
                    let total = f32::from(if row { b.size.width } else { b.size.height }).max(1.);
                    this.previews.drag = Some(Drag {
                        agent: id.clone(),
                        row,
                        start: if row { e.position.x } else { e.position.y },
                        total,
                        from: p.frac,
                    });
                    cx.notify();
                }),
            );
        let page = self.preview_cell(a, t, cx);
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .when(!row, |d| d.flex_col())
            .child(measure)
            .child(cell(frac).child(term))
            .child(divider)
            .child(cell(1. - frac).child(page))
            .into_any_element()
    }

    /// The page side: toolbar, then the page (or why there's none).
    fn preview_cell(&self, a: &AgentView, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.previews.panes[&a.id];
        let focus = p.focus.clone();
        let screen = cx.entity().downgrade();
        let id = a.id.clone();
        let cell_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Default::default();
        let at = cell_bounds.clone();
        // A click anywhere outside the page takes the keys back from it
        // (before what was clicked takes them itself).
        let outside = canvas(
            move |b, _, _| at.set(Some(b)),
            move |_, _, window, _| {
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Capture
                        && cell_bounds.get().is_some_and(|b| !b.contains(&e.position))
                    {
                        let _ = screen.update(cx, |s, cx| {
                            let has = s.previews.panes.get(&id).is_some_and(|p| p.keys);
                            if has || focus.contains_focused(window, cx) {
                                s.preview_blur(&id, window, cx)
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size_full();
        let body: AnyElement = match &p.follow.server {
            Server::Live => {
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
            Server::Stopped => calm(
                "Server stopped",
                format!("The page comes back when {} prints its address again.", a.name),
                t,
            ),
            Server::Unreachable(e) => calm(
                format!("Can't reach {}", super::model::preview_port(&p.follow.url)),
                format!("{e} It loads again when the server prints its address."),
                t,
            ),
        };
        div()
            .id(SharedString::from(format!("preview-pane-{}", a.id)))
            .debug_selector({
                let id = a.id.clone();
                move || format!("preview-pane-{id}")
            })
            .track_focus(&p.focus)
            .key_context("PwPreview")
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.surface)
            .child(outside)
            .child(self.preview_toolbar(a, t, cx))
            .child(body)
            .into_any_element()
    }

    fn preview_toolbar(&self, a: &AgentView, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.previews.panes[&a.id];
        let id = a.id.clone();
        let btn = |name: &str, icon_name: &str, tip: &str, on: Option<bool>| {
            crate::kit::icon_btn_state(format!("pv-{name}-{id}"), icon_name, tip.to_string(), true, 13., on, t)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let act = |cx: &mut Context<Self>, f: PaneAct| on_click(cx, &a.id, f);
        let below = self.ui.preview_below;
        let title = if p.title.is_empty() { p.page_url.clone() } else { p.title.clone() };
        div()
            .h(px(TOOLBAR_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(4.))
            .border_b_1()
            .border_color(t.line)
            .bg(t.surface)
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
            .child(btn("reload", "refresh", "Reload", None).on_click(act(cx, |s, id, _, cx| {
                if let Some(p) = s.previews.panes.get_mut(id) {
                    p.follow.retry();
                }
                s.with_view(id, |v| v.reload(), cx)
            })))
            .child(
                div()
                    .id(SharedString::from(format!("pv-url-{}", a.id)))
                    .flex_1()
                    .min_w_0()
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
                    .tooltip(tooltip(title))
                    .when(p.loading, |d| d.child(div().flex_none().mr(px(6.)).size(px(6.)).rounded_full().bg(t.text_4)))
                    .child(div().flex_1().min_w_0().child(p.url_field.clone())),
            )
            .child(
                btn("phone", "phone", "Phone width", Some(p.phone))
                    .on_click(act(cx, |s, id, _, cx| s.with_pane(id, |p| p.phone = !p.phone, cx))),
            )
            .child(
                btn("layout", if below { "split-right" } else { "split-down" }, if below { "Beside the terminal" } else { "Below the terminal" }, None)
                    .on_click(cx.listener(|this, _, _, cx| this.update_ui(|mut s| {
                        s.preview_below = !s.preview_below;
                        s
                    }, cx))),
            )
            .child(
                btn("inspect", "inspect", "Web Inspector", None)
                    .on_click(act(cx, |s, id, _, cx| s.with_view(id, |v| v.open_inspector(), cx))),
            )
            .child(btn("browser", "external", "Open in browser", None).on_click(act(cx, |s, id, _, cx| {
                if let Some(u) = s.previews.panes.get(id).map(|p| p.page_url.clone()) {
                    crate::platform::files::open_url(cx, &u);
                }
            })))
            .child(
                btn("close", "x", "Close preview", None)
                    .on_click(act(cx, |s, id, window, cx| s.close_preview(id, window, cx))),
            )
    }

    fn with_view(&mut self, agent: &str, f: impl FnOnce(&mut Box<dyn WebView>), cx: &mut Context<Self>) {
        if let Some(v) = self.previews.panes.get_mut(agent).and_then(|p| p.view.as_mut()) {
            f(v);
            cx.notify();
        }
    }
}

/// Where a page was drawn, and whether something covers it.
type Placed = (Bounds<Pixels>, bool);
type ClickFn = Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App)>;
type PaneAct = fn(&mut MainScreen, &str, &mut Window, &mut Context<MainScreen>);

/// A toolbar button's click: `f` on the preview of `agent`.
fn on_click(
    cx: &mut Context<MainScreen>,
    agent: &str,
    f: PaneAct,
) -> ClickFn {
    let id = agent.to_string();
    Box::new(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| f(this, &id, window, cx)))
}

/// Where the page would be: why there's none, calmly.
fn calm(title: impl Into<SharedString>, detail: String, t: &Theme) -> AnyElement {
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
        .child(div().max_w(px(320.)).text_size(px(12.)).text_color(t.text_3).text_center().child(detail))
        .into_any_element()
}
