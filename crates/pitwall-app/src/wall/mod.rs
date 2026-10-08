//! The Wall (Tauri: `src/components/Wall.tsx`, `src/styles/wall.css`,
//! docs/spec/wall.md): every agent's live screen at once, grouped by
//! project. View-only: it never resizes a PTY or sends input; a click (or
//! Enter/Space on the selected tile) leaves the Wall and shows that agent.
//!
//! - Tiles are drawn from the engine's `ScreenFrame`s ([`source`]), not a
//!   terminal emulator, and only while within 200 px of the viewport
//!   ([`layout::visible`]); the engine sends at most ~10 frames a second
//!   per tile, only when something changed.
//! - Frames reach the main thread in batches ([`FRAME_BATCH`]) and notify
//!   only their tile; every tile is a cached view, so the others are not
//!   laid out or painted again.
//! - Rows outside the viewport are empty boxes of the right height.

pub mod layout;
pub mod paint;
pub mod screen;
pub mod source;
pub mod style;
pub mod tile;

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use gpui::{
    actions, canvas, div, point, prelude::*, px, AnyElement, AnyView, App, Context, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, Pixels, ScrollHandle, Size,
    StyleRefinement, Subscription, Task, WeakEntity, Window,
};

use pitwall_core::Shared;

use crate::agents::{AgentStore, ProjectGroup};
use crate::theme::{self, Theme, RADIUS};
use layout::{Move, WallRow};
use source::{FrameMsg, ScreenSource, Screens};
use tile::WallTile;

pub use paint::screen_view;

actions!(
    wall,
    [
        /// Show or leave the Wall (⌘E / Ctrl+Shift+E).
        ToggleWall,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectFirst,
        SelectLast,
        SelectNext,
        SelectPrevious,
        /// Show the selected tile's agent.
        OpenSelected,
    ]
);

/// Frames are applied at most this often (the engine already limits each
/// tile to one per 100 ms).
pub const FRAME_BATCH: Duration = Duration::from_millis(40);

/// Glass draws the Wall's bar on translucent chrome; tiles stay solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Look {
    #[default]
    Flat,
    Glass,
}

/// What the Wall asks of the window around it.
#[derive(Debug, Clone, PartialEq)]
pub enum WallEvent {
    /// Leave the Wall and show this agent.
    Open(String),
    /// Leave the Wall (Back, Esc).
    Exit,
    /// The collapsed sections changed (persisted as `ui.wallCollapsed`).
    Collapsed(BTreeSet<String>),
}

/// Bindings: ⌘E toggles everywhere; arrows, Home/End, Tab and Enter/Space
/// inside the Wall.
pub fn bindings() -> Vec<KeyBinding> {
    let toggle = if cfg!(target_os = "macos") {
        "cmd-e"
    } else {
        "ctrl-shift-e"
    };
    let w = Some("Wall");
    vec![
        KeyBinding::new(toggle, ToggleWall, None),
        KeyBinding::new("left", SelectLeft, w),
        KeyBinding::new("right", SelectRight, w),
        KeyBinding::new("up", SelectUp, w),
        KeyBinding::new("down", SelectDown, w),
        KeyBinding::new("home", SelectFirst, w),
        KeyBinding::new("end", SelectLast, w),
        KeyBinding::new("tab", SelectNext, w),
        KeyBinding::new("shift-tab", SelectPrevious, w),
        KeyBinding::new("enter", OpenSelected, w),
        KeyBinding::new("space", OpenSelected, w),
    ]
}

/// Register the bindings and, with a hosted engine, where screens come from.
pub fn register(cx: &mut App, engine: Option<Shared>) {
    cx.bind_keys(bindings());
    if let Some(engine) = engine {
        cx.set_global(Screens(Arc::new(source::EngineScreens(engine))));
    }
}

/// One live watch of an agent's screen.
struct Watch {
    token: u64,
    /// `None`: watching failed (retried, see [`RETRY`]).
    id: Option<u64>,
    /// A new watch sends the whole screen at once; until it has, it may
    /// be on a terminal that is being replaced (the engine re-attaching to
    /// its holders after a start), so it is retried.
    got_frame: bool,
    since: std::time::Instant,
}

/// Failed watches, and watches that sent no first frame, are tried again
/// after this long.
pub const RETRY: Duration = Duration::from_secs(1);

pub struct WallView {
    store: Entity<AgentStore>,
    source: Option<Arc<dyn ScreenSource>>,
    tiles: HashMap<String, Entity<WallTile>>,
    watches: HashMap<String, Watch>,
    next_token: u64,
    frames: UnboundedSender<FrameMsg>,
    collapsed: BTreeSet<String>,
    selected: Option<String>,
    /// Tiles in reading order with their places, and each row's top (for
    /// keys and scrolling the selection into view).
    cells: Vec<(String, layout::Cell)>,
    tops: Vec<f32>,
    tile_h: f32,
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// The scroll area's size as last painted.
    viewport: Rc<Cell<Size<Pixels>>>,
    font_size: f32,
    look: Look,
    /// The app window's width for the breakpoints, when the Wall is not
    /// in it (previews).
    app_width: Option<f32>,
    _drain: Task<()>,
    _retry: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<WallEvent> for WallView {}

impl Focusable for WallView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl WallView {
    pub fn new(store: Entity<AgentStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        style::ensure_fonts(cx);
        let source = Screens::get(cx);
        let (tx, rx) = unbounded();
        let drain = cx.spawn(async move |this, cx| drain(this, rx, cx).await);
        let subs = vec![cx.observe(&store, |w, _, cx| {
            w.sync_agents(cx);
            cx.notify();
        })];
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut w = WallView {
            store,
            source,
            tiles: HashMap::new(),
            watches: HashMap::new(),
            next_token: 1,
            frames: tx,
            collapsed: BTreeSet::new(),
            selected: None,
            cells: Vec::new(),
            tops: Vec::new(),
            tile_h: layout::tile_height(0.),
            focus,
            scroll: ScrollHandle::new(),
            viewport: Rc::new(Cell::new(Size::default())),
            font_size: paint::DEFAULT_FONT,
            look: Look::Flat,
            app_width: None,
            _drain: drain,
            _retry: cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(RETRY).await;
                if this.update(cx, |w, cx| w.retry_stale(cx)).is_err() {
                    return;
                }
            }),
            _subscriptions: subs,
        };
        w.sync_agents(cx);
        w
    }

    /// Sections folded (from `ui.wallCollapsed`).
    pub fn set_collapsed(&mut self, keys: BTreeSet<String>, cx: &mut Context<Self>) {
        self.collapsed = keys;
        cx.notify();
    }

    /// Size tiles for an app window this wide (previews; the app uses its
    /// own window).
    pub fn set_app_width(&mut self, width: Option<f32>, cx: &mut Context<Self>) {
        self.app_width = width;
        cx.notify();
    }

    pub fn set_look(&mut self, look: Look, cx: &mut Context<Self>) {
        self.look = look;
        cx.notify();
    }

    /// The terminals' font size (`ui.fontSize`).
    pub fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.font_size = size;
        for t in self.tiles.values() {
            t.update(cx, |t, cx| {
                t.font_size = size;
                cx.notify();
            });
        }
    }

    /// Agents being watched now (tests, diagnostics).
    pub fn watching(&self) -> BTreeSet<String> {
        self.watches
            .iter()
            .filter(|(_, w)| w.id.is_some())
            .map(|(a, _)| a.clone())
            .collect()
    }

    pub fn tile(&self, agent_id: &str) -> Option<&Entity<WallTile>> {
        self.tiles.get(agent_id)
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub(crate) fn open(&mut self, id: String, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(WallEvent::Open(id));
    }

    pub fn toggle_section(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_string());
        }
        cx.emit(WallEvent::Collapsed(self.collapsed.clone()));
        cx.notify();
    }

    /// Follow the store: new tiles, changed agents, gone ones. A tile is
    /// notified only when its agent changed.
    fn sync_agents(&mut self, cx: &mut Context<Self>) {
        let agents = self.store.read(cx).agents.clone();
        let ids: HashSet<&str> = agents.iter().map(|a| a.id.as_str()).collect();
        let gone: Vec<String> = self
            .tiles
            .keys()
            .filter(|k| !ids.contains(k.as_str()))
            .cloned()
            .collect();
        for id in gone {
            self.tiles.remove(&id);
            self.stop_watch(&id);
        }
        let wall = cx.entity().downgrade();
        for a in agents {
            match self.tiles.get(&a.id).cloned() {
                Some(tile) => {
                    let restarted = tile.read(cx).agent.running != a.running;
                    if restarted {
                        // A new process (or none): watch again from scratch.
                        self.stop_watch(&a.id);
                        self.watches.remove(&a.id);
                    }
                    tile.update(cx, |t, cx| {
                        if t.agent != a {
                            t.agent = a;
                            if restarted {
                                t.failed = false;
                            }
                            cx.notify();
                        }
                    });
                }
                None => {
                    let (font, body_h, wall) = (
                        self.font_size,
                        self.tile_h - layout::TILE_HEAD_H,
                        wall.clone(),
                    );
                    let id = a.id.clone();
                    let tile = cx.new(|_| WallTile::new(a, font, body_h, wall));
                    self.tiles.insert(id, tile);
                }
            }
        }
    }

    fn stop_watch(&mut self, id: &str) {
        if let Some(Watch { id: Some(wid), .. }) = self.watches.remove(id) {
            if let Some(s) = &self.source {
                s.unwatch(id, wid);
            }
        }
    }

    /// Drop watches that failed or never sent their first frame within
    /// [`RETRY`]; the next render watches those agents again.
    fn retry_stale(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let stale: Vec<String> = self
            .watches
            .iter()
            .filter(|(_, w)| (w.id.is_none() || !w.got_frame) && now - w.since >= RETRY)
            .map(|(id, _)| id.clone())
            .collect();
        if stale.is_empty() {
            return;
        }
        for id in stale {
            self.stop_watch(&id);
        }
        cx.notify();
    }

    /// Watch exactly the running agents in `want`.
    fn reconcile(&mut self, want: &BTreeSet<String>, cx: &mut Context<Self>) {
        let drop: Vec<String> = self
            .watches
            .iter()
            .filter(|(id, w)| !want.contains(*id) && w.id.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for id in drop {
            self.stop_watch(&id);
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        for id in want {
            if self.watches.contains_key(id) {
                continue;
            }
            let token = self.next_token;
            self.next_token += 1;
            let sink = source::forward(self.frames.clone(), id.clone(), token);
            let result = source.watch(id, sink);
            let failed = result.is_err();
            self.watches.insert(
                id.clone(),
                Watch {
                    token,
                    id: result.ok(),
                    got_frame: false,
                    since: cx.background_executor().now(),
                },
            );
            if let Some(tile) = self.tiles.get(id) {
                tile.update(cx, |t, cx| {
                    if t.failed != failed {
                        t.failed = failed;
                        cx.notify();
                    }
                });
            }
        }
    }

    /// Apply a batch of frames; each touched tile is notified once.
    fn apply_frames(&mut self, batch: Vec<FrameMsg>, cx: &mut Context<Self>) {
        let mut touched: Vec<String> = Vec::new();
        for m in batch {
            let current = match self.watches.get_mut(&m.agent) {
                Some(w) if w.token == m.token => {
                    w.got_frame = true;
                    true
                }
                _ => false,
            };
            let Some(tile) = self.tiles.get(&m.agent).filter(|_| current) else {
                continue;
            };
            tile.update(cx, |t, _| {
                t.screen.apply(&m.frame);
            });
            if !touched.contains(&m.agent) {
                touched.push(m.agent);
            }
        }
        for id in touched {
            if let Some(tile) = self.tiles.get(&id) {
                tile.update(cx, |_, cx| cx.notify());
            }
        }
    }

    fn select(&mut self, m: Move, window: &mut Window, cx: &mut Context<Self>) {
        let Some(next) = layout::step(&self.cells, self.selected.as_deref(), m) else {
            return;
        };
        self.set_selected(Some(next), cx);
        self.reveal_selected();
        window.focus(&self.focus);
        cx.notify();
    }

    fn set_selected(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if self.selected == id {
            return;
        }
        for (old, new) in [(self.selected.clone(), false), (id.clone(), true)] {
            if let Some(tile) = old.and_then(|o| self.tiles.get(&o)) {
                tile.update(cx, |t, cx| {
                    t.selected = new;
                    cx.notify();
                });
            }
        }
        self.selected = id;
    }

    /// Scroll so the selected tile is in view.
    fn reveal_selected(&mut self) {
        let Some((_, (row, _))) = self
            .cells
            .iter()
            .find(|(a, _)| Some(a.as_str()) == self.selected.as_deref())
        else {
            return;
        };
        let Some(top) = self.tops.get(*row).copied() else {
            return;
        };
        let height = f32::from(self.viewport.get().height);
        let scrolled = -f32::from(self.scroll.offset().y);
        let bottom = top + self.tile_h;
        let to = if top < scrolled {
            top - layout::PAD_TOP
        } else if bottom > scrolled + height {
            bottom - height + layout::GAP
        } else {
            return;
        };
        self.scroll.set_offset(point(px(0.), -px(to.max(0.))));
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            cx.emit(WallEvent::Open(id));
        }
    }

    fn bar(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line = match self.look {
            Look::Flat => t.line,
            Look::Glass => t.line.opacity(0.6),
        };
        div()
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(12.))
            .pr(px(8.))
            .border_b_1()
            .border_color(line)
            .child(style::tracked_label(
                window,
                "Wall",
                style::LABEL,
                12.,
                FontWeight::SEMIBOLD,
                0.12,
                t.text_3,
            ))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .truncate()
                    .child("every agent, live · view only · click a tile to take over"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("wall-back")
                    .h(px(26.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded(RADIUS)
                    .bg(t.surface_3)
                    .border_1()
                    .border_color(t.line_strong)
                    .hover(|s| s.border_color(t.text_4))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .font_weight(FontWeight(550.))
                    .text_color(t.text)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(WallEvent::Exit)))
                    .child("Back")
                    .child(kbd("esc", t)),
            )
    }

    fn section_head(
        &self,
        g: &ProjectGroup,
        t: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.collapsed.contains(&g.key);
        let key = g.key.clone();
        let n = g.agents.len();
        let summary = if collapsed {
            layout::summarize(&g.agents)
        } else {
            format!("{n} agent{}", if n == 1 { "" } else { "s" })
        };
        div()
            .id(gpui::SharedString::from(format!("wall-section-{}", g.key)))
            .h(px(layout::SECTION_HEAD_H))
            .pt(px(10.))
            .pb(px(8.))
            .px(px(2.))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .on_click(cx.listener(move |w, _, _, cx| w.toggle_section(&key, cx)))
            .child(chevron(!collapsed, t.text_4))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .child(style::tracked_label(
                        window,
                        style::project_name(&g.display),
                        style::LABEL,
                        14.,
                        FontWeight::BOLD,
                        0.12,
                        if g.blocked > 0 { t.text } else { t.text_2 },
                    )),
            )
            .when(g.blocked > 0, |d| {
                d.child(
                    div()
                        .flex_none()
                        .font_family(style::MONO)
                        .text_size(px(10.5))
                        .text_color(t.amber)
                        .child(format!("▲ {}", g.blocked)),
                )
            })
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .child(summary),
            )
            .into_any_element()
    }

    fn tiles_row(
        &self,
        g: &ProjectGroup,
        start: usize,
        end: usize,
        cols: usize,
        last: bool,
    ) -> AnyElement {
        let h = px(self.tile_h);
        let mut row = div().flex().gap(px(layout::GAP)).h(h).flex_none();
        for a in &g.agents[start..end] {
            if let Some(tile) = self.tiles.get(&a.id) {
                let mut style = StyleRefinement::default().flex_1().min_w_0().h(h);
                style = style.flex_basis(px(0.));
                row = row.child(AnyView::from(tile.clone()).cached(style));
            }
        }
        // `auto-fill` keeps the column width on a short last row.
        for _ in (end - start)..cols {
            row = row.child(div().flex_1().flex_basis(px(0.)).min_w_0());
        }
        div()
            .when(!last, |d| d.pb(px(layout::GAP)))
            .child(row)
            .into_any_element()
    }

    fn empty(t: &Theme) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(t.text_2)
                    .child("No agents yet"),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .child("Their screens show up here, live, once they start."),
            )
    }
}

/// The `chevron` icon (`src/components/Icon.tsx`, 20-unit box, 1.5 stroke)
/// at 12 px, pointing down when `open`.
fn chevron(open: bool, color: gpui::Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let k = f32::from(b.size.width) / 20.;
            let at = |x: f32, y: f32| {
                // Rotated a quarter turn around the centre when open.
                let (x, y) = if open { (20. - y, x) } else { (x, y) };
                point(b.left() + px(x * k), b.top() + px(y * k))
            };
            let mut p = gpui::PathBuilder::stroke(px(1.5 * k.max(0.8)));
            p.move_to(at(8., 5.));
            p.line_to(at(13., 10.));
            p.line_to(at(8., 15.));
            if let Ok(path) = p.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(12.))
    .flex_none()
}

/// A small key cap (`.kbd`).
fn kbd(text: &'static str, t: &Theme) -> impl IntoElement {
    div()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(t.line_strong)
        .bg(t.surface_2)
        .font_family(style::MONO)
        .text_size(px(10.5))
        .text_color(t.text_3)
        .child(text)
}

/// Move frames from the engine's threads to the tiles, in batches.
async fn drain(
    this: WeakEntity<WallView>,
    mut rx: UnboundedReceiver<FrameMsg>,
    cx: &mut gpui::AsyncApp,
) {
    while let Some(first) = rx.next().await {
        let mut batch = vec![first];
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }
        if this.update(cx, |w, cx| w.apply_frames(batch, cx)).is_err() {
            return;
        }
        cx.background_executor().timer(FRAME_BATCH).await;
    }
}

impl Drop for WallView {
    fn drop(&mut self) {
        if let Some(s) = &self.source {
            for (id, w) in self.watches.drain() {
                if let Some(wid) = w.id {
                    s.unwatch(&id, wid);
                }
            }
        }
    }
}

impl Render for WallView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let groups = self.store.read(cx).groups();
        let win = window.viewport_size();
        let tile_h = layout::tile_height(self.app_width.unwrap_or(f32::from(win.width)));
        if tile_h != self.tile_h {
            self.tile_h = tile_h;
            let body_h = tile_h - layout::TILE_HEAD_H;
            for tile in self.tiles.values() {
                tile.update(cx, |t, cx| {
                    t.body_h = body_h;
                    cx.notify();
                });
            }
        }
        let vp = self.viewport.get();
        let (width, height) = if vp.width > px(0.) {
            (f32::from(vp.width), f32::from(vp.height))
        } else {
            (
                f32::from(win.width - theme::SIDEBAR_W),
                f32::from(win.height),
            )
        };
        let cols = layout::columns(width);
        let rows = layout::rows(&groups, &self.collapsed, cols);
        let scrolled = -f32::from(self.scroll.offset().y);
        let shown = layout::visible(&rows, tile_h, scrolled, height);
        self.cells = layout::tile_cells(&rows, &groups);
        self.tops = layout::row_tops(&rows, tile_h);
        if let Some(sel) = &self.selected {
            if !self.cells.iter().any(|(a, _)| a == sel) {
                self.set_selected(None, cx);
            }
        }

        let mut want = BTreeSet::new();
        let mut children: Vec<AnyElement> = Vec::with_capacity(rows.len());
        for (row, on) in rows.iter().zip(&shown) {
            if !on {
                children.push(
                    div()
                        .h(px(layout::row_height(row, tile_h)))
                        .into_any_element(),
                );
                continue;
            }
            children.push(match row {
                WallRow::Machine(label) => div()
                    .h(px(layout::MACHINE_HEAD_H))
                    .pt(px(6.))
                    .pb(px(4.))
                    .px(px(2.))
                    .flex()
                    .child(style::tracked_label(
                        window,
                        label,
                        style::UI,
                        10.5,
                        FontWeight::SEMIBOLD,
                        0.06,
                        t.text_4,
                    ))
                    .into_any_element(),
                WallRow::Head(gi) => self.section_head(&groups[*gi], &t, window, cx),
                WallRow::Tiles {
                    group,
                    start,
                    end,
                    last,
                } => {
                    let g = &groups[*group];
                    want.extend(
                        g.agents[*start..*end]
                            .iter()
                            .filter(|a| a.running)
                            .map(|a| a.id.clone()),
                    );
                    self.tiles_row(g, *start, *end, cols, *last)
                }
            });
        }
        self.reconcile(&want, cx);

        // The scroll area's size, as painted: a change re-renders the Wall
        // (columns, which tiles are visible).
        let viewport = self.viewport.clone();
        let me = cx.entity().downgrade();
        let measure = canvas(
            move |bounds, _, cx| {
                if viewport.get() != bounds.size {
                    viewport.set(bounds.size);
                    cx.defer(move |cx| {
                        let _ = me.update(cx, |_, cx| cx.notify());
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let empty = groups.is_empty();
        div()
            .id("wall")
            .key_context("Wall")
            .track_focus(&self.focus)
            .on_action(cx.listener(|w, _: &SelectLeft, win, cx| w.select(Move::Left, win, cx)))
            .on_action(cx.listener(|w, _: &SelectRight, win, cx| w.select(Move::Right, win, cx)))
            .on_action(cx.listener(|w, _: &SelectUp, win, cx| w.select(Move::Up, win, cx)))
            .on_action(cx.listener(|w, _: &SelectDown, win, cx| w.select(Move::Down, win, cx)))
            .on_action(cx.listener(|w, _: &SelectFirst, win, cx| w.select(Move::First, win, cx)))
            .on_action(cx.listener(|w, _: &SelectLast, win, cx| w.select(Move::Last, win, cx)))
            .on_action(cx.listener(|w, _: &SelectNext, win, cx| w.select(Move::Right, win, cx)))
            .on_action(cx.listener(|w, _: &SelectPrevious, win, cx| w.select(Move::Left, win, cx)))
            .on_action(cx.listener(|w, _: &OpenSelected, _, cx| w.open_selected(cx)))
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .font_family(style::UI)
            .text_size(px(13.))
            .bg(t.bg)
            .text_color(t.text)
            .child(self.bar(&t, window, cx))
            .when(empty, |d| d.child(Self::empty(&t)))
            .when(!empty, |d| {
                d.child(
                    div().relative().flex_1().min_h_0().child(measure).child(
                        div()
                            .id("wall-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .pt(px(layout::PAD_TOP))
                            .px(px(layout::PAD_X))
                            .pb(px(layout::PAD_BOTTOM))
                            .children(children),
                    ),
                )
            })
    }
}

#[cfg(test)]
mod tests;
