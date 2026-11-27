//! A files tile in the space's layout (a pane with `files`: an agent's file
//! viewer, `crate::explorer::dock`), and where files dragged over this
//! window would land: a band along each side of a tile (a new tile on that
//! side, splitting it), along an edge of the main area (a column or row of
//! its own), an empty tile (taken), and the rest of a terminal's tile, most
//! of it: the files take that tile's place, as big and where it is, and the
//! terminal leaves the layout (the agent keeps running). In the middle of
//! another agent's files tile in the same space the two swap places. Drawn
//! as an agent being dragged is (`drop_indicator`'s look): the part a new
//! tile would take, or the whole tile, more strongly, for a replace. Found
//! from the boxes the last frame drew, since the drag comes from another
//! window (`crate::explorer::drag`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{canvas, div, prelude::*, px, AnyElement, App, AppContext, Bounds, Context, DragMoveEvent, FocusHandle, Focusable, FontWeight, Pixels, Point, SharedString, Subscription, Window};

use super::topbar::AgentDrag;
use super::tree::{self, Content, Side};
use super::workspace::FilesSpot;
use super::MainScreen;
use crate::theme::Theme;

/// A web or inbox tile dragged by its bar within a Pitwall window (GPUI's
/// drag; the space's area is where it lands: `MainScreen::tile_drop_target`).
#[derive(Clone, Debug)]
pub struct TileDrag {
    pub what: Content,
    pub name: SharedString,
}

impl TileDrag {
    pub fn ghost(&self, grab: Point<Pixels>, _: &mut Window, cx: &mut App) -> gpui::Entity<super::topbar::AgentGhost> {
        let name = self.name.clone();
        cx.new(|_| super::topbar::AgentGhost::named(name, grab))
    }
}

/// The tiles whose content can take the keys by itself (a click in a web
/// page, which GPUI doesn't see; a field): what each is followed by.
#[derive(Default)]
pub struct TileKeys(HashMap<Content, (FocusHandle, Subscription)>);

/// What a tile shows (where files may go on it).
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Agent,
    Empty,
    /// A files or web tile, and what it shows.
    Tile(Content),
}

/// A tile's box and what it shows.
pub type PaneBox = (String, Bounds<Pixels>, Kind);

#[derive(Default)]
struct Drawn {
    area: Option<Bounds<Pixels>>,
    panes: HashMap<String, (Bounds<Pixels>, Kind)>,
}

/// The boxes the last frame drew: the main area's and each tile's.
#[derive(Clone, Default)]
pub struct Boxes(Rc<RefCell<Drawn>>);

impl Boxes {
    pub fn clear(&self) {
        self.0.borrow_mut().panes.clear();
    }
    pub fn set_area(&self, b: Bounds<Pixels>) {
        self.0.borrow_mut().area = Some(b);
    }
    fn set_pane(&self, id: &str, b: Bounds<Pixels>, kind: Kind) {
        self.0.borrow_mut().panes.insert(id.to_string(), (b, kind));
    }
    /// Tile `id`'s box as the last frame drew it.
    pub fn pane(&self, id: &str) -> Option<Bounds<Pixels>> {
        self.0.borrow().panes.get(id).map(|(b, _)| *b)
    }
    fn get(&self) -> (Option<Bounds<Pixels>>, Vec<PaneBox>) {
        let g = self.0.borrow();
        (g.area, g.panes.iter().map(|(id, (b, k))| (id.clone(), *b, k.clone())).collect())
    }
}

/// Where files (or a web tile) dragged over this window land now.
#[derive(Debug, Clone, PartialEq)]
pub struct FilesHint {
    /// What is dragged.
    pub what: Content,
    pub spot: FilesSpot,
    /// Where they'd be, in the words the tile's place is said in once
    /// there ("Files · above api's terminal").
    pub label: String,
}

/// Along the main area's edge, this deep: its own column or row.
const EDGE_BAND: f32 = 24.;
/// Along a tile's side, this share of its width (or height) means "a new
/// tile on that side", kept between these depths; the rest of the tile,
/// its middle, means "in its place".
const SIDE_BAND: f32 = 0.2;
const SIDE_BAND_MIN: f32 = 28.;
const SIDE_BAND_MAX: f32 = 64.;

/// The side of tile `b` whose band `at` is in (`None`: its middle).
pub fn side_band(at: Point<Pixels>, b: Bounds<Pixels>) -> Option<Side> {
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    let band = |len: f32| (len * SIDE_BAND).clamp(SIDE_BAND_MIN, SIDE_BAND_MAX).min(len / 2.);
    let sides = [
        (Side::Left, f32::from(at.x - b.left()), band(w)),
        (Side::Right, f32::from(b.right() - at.x), band(w)),
        (Side::Top, f32::from(at.y - b.top()), band(h)),
        (Side::Bottom, f32::from(b.bottom() - at.y), band(h)),
    ];
    sides
        .into_iter()
        .filter(|(_, d, band)| d < band)
        .min_by(|a, b| (a.1 / a.2).total_cmp(&(b.1 / b.2)))
        .map(|(side, _, _)| side)
}

/// Where `what` (an agent's files, a web tile) dropped at `at` lands: see
/// the module.
pub fn zone_at(at: Point<Pixels>, area: Bounds<Pixels>, panes: &[PaneBox], what: &Content) -> Option<FilesSpot> {
    if !area.contains(&at) {
        return None;
    }
    let edges = [
        (Side::Left, at.x - area.left()),
        (Side::Right, area.right() - at.x),
        (Side::Top, at.y - area.top()),
        (Side::Bottom, area.bottom() - at.y),
    ];
    let (side, d) = edges.into_iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    if d < px(EDGE_BAND) {
        return Some(FilesSpot::Edge(side));
    }
    let (id, b, kind) = panes.iter().find(|(_, b, _)| b.contains(&at))?;
    // (Its own tile here: another files or web tile can swap with it.)
    let own_here = panes.iter().any(|(_, _, k)| matches!(k, Kind::Tile(c) if c == what));
    match kind {
        Kind::Tile(c) if c == what => None,
        Kind::Empty => Some(FilesSpot::Take(id.clone())),
        _ => match side_band(at, *b) {
            Some(side) => Some(FilesSpot::Tile { pane: id.clone(), side }),
            None if *kind == Kind::Agent || own_here => Some(FilesSpot::Replace(id.clone())),
            None => None,
        },
    }
}

impl MainScreen {
    /// Files of agent `owner` dragged at `at` (this window's coordinates):
    /// shows where they'd land and says so (with the space). `None`:
    /// nowhere here.
    pub fn files_hover(&mut self, owner: &str, at: Point<Pixels>, cx: &mut Context<Self>) -> Option<(String, FilesSpot)> {
        self.tile_hover(&Content::Files(owner.to_string()), at, cx)
    }

    /// [`MainScreen::files_hover`] for any files or web tile.
    pub fn tile_hover(&mut self, what: &Content, at: Point<Pixels>, cx: &mut Context<Self>) -> Option<(String, FilesSpot)> {
        let space = self.active_space().map(|s| s.id.clone()).filter(|_| self.route == super::Route::Space);
        let spot = space.as_ref().and_then(|_| {
            let (area, panes) = self.files_boxes.get();
            zone_at(at, area?, &panes, what)
        });
        let same = self.files_hint.as_ref().is_some_and(|h| h.what == *what && Some(&h.spot) == spot.as_ref());
        if same {
            return Some((space?, spot?));
        }
        let hint = match (&space, &spot) {
            (Some(sp), Some(spot)) => Some(FilesHint { what: what.clone(), spot: spot.clone(), label: self.landing_words(sp, what, spot, cx) }),
            _ => None,
        };
        if self.files_hint != hint {
            self.files_hint = hint;
            cx.notify();
        }
        Some((space?, spot?))
    }

    /// Where agent `owner`'s files dropped at `spot` of space `space` would
    /// be, said as the tile's place is said once there (the same layout
    /// change, then [`whereabouts`](super::workspace::whereabouts)).
    fn landing_words(&self, space: &str, what: &Content, spot: &FilesSpot, cx: &App) -> String {
        let names: Vec<(String, String)> = self.agents(cx).iter().map(|a| (a.id.clone(), a.name.clone())).collect();
        let name = |id: &str| names.iter().find(|(i, _)| i == id).map_or_else(|| id.to_string(), |(_, n)| n.clone());
        let (noun, verb) = match what {
            Content::Files(_) => ("Files", ""),
            Content::Web(_) => ("Web page", "s"),
            Content::Inbox(_) => ("Inbox", "es"),
            Content::View(_) => ("The view", "s"),
            Content::Settings => ("Settings", "s"),
            Content::Device(_) => ("The device", "s"),
            Content::Editor => ("The editor", "s"),
            Content::Extensions => ("Extensions", ""),
        };
        // In a tile's place: what leaves it, said plainly.
        if let FilesSpot::Replace(pane) = spot {
            match self.ui.space(space).and_then(|sp| sp.layout.find_pane(pane)) {
                Some(p) if p.files.is_some() => return format!("{noun} swap{verb} places with {}'s files", name(p.files.as_deref().unwrap_or_default())),
                Some(p) if p.web.is_some() => return format!("{noun} swap{verb} places with the web page"),
                Some(p) if p.inbox.is_some() => return format!("{noun} swap{verb} places with the inbox"),
                Some(p) if p.view.is_some() => return format!("{noun} swap{verb} places with the view"),
                Some(p) if p.settings => return format!("{noun} swap{verb} places with Settings"),
                Some(p) if p.device.is_some() => return format!("{noun} swap{verb} places with the device"),
                Some(p) if p.editor => return format!("{noun} swap{verb} places with the editor"),
                Some(p) if p.agent_id.is_some() => return format!("{noun} replace{verb} {}'s terminal (it keeps running)", name(p.agent_id.as_deref().unwrap_or_default())),
                _ => {}
            }
        }
        let (next, _) = self.ui.clone().dock(space, what, spot, super::workspace::FILES_SHARE, self.fit_for(space));
        let Some((sp, pane)) = next.tile_of(what) else { return noun.into() };
        let Some(layout) = next.space(&sp).map(|s| s.layout.clone()) else { return noun.into() };
        let anchor = match spot {
            FilesSpot::Tile { pane, .. } => layout.leaves().into_iter().find(|p| p.id == *pane && p.content().is_none()).and_then(|p| p.agent_id),
            _ => None,
        };
        format!("{noun} · {}", super::workspace::whereabouts(&layout, &pane, anchor.as_deref()).words(&name))
    }

    /// No files dragged over this window (any more).
    pub fn clear_files_hover(&mut self, cx: &mut Context<Self>) {
        if self.files_hint.take().is_some() {
            cx.notify();
        }
    }

    /// Where files would land now (tests, debug).
    /// The main area as the last frame drew it.
    #[cfg(test)]
    pub fn files_boxes_area(&self) -> Option<Bounds<Pixels>> {
        self.files_boxes.get().0
    }

    pub fn files_hint(&self) -> Option<&FilesHint> {
        self.files_hint.as_ref()
    }

    /// Keeps tile `pane`'s box (`kind`) for [`zone_at`].
    pub(super) fn files_measure(&self, pane: &str, kind: Kind) -> impl IntoElement {
        let boxes = self.files_boxes.clone();
        let id = pane.to_string();
        canvas(move |b, _, _: &mut App| boxes.set_pane(&id, b, kind.clone()), |_, _, _, _| {})
            .absolute()
            .size_full()
    }

    /// The files about to land on tile `pane`.
    pub(super) fn files_pane_hint(&self, pane: &str, has_head: bool, t: &Theme) -> Option<AnyElement> {
        let h = self.files_hint.as_ref()?;
        let (zone, replace) = match &h.spot {
            FilesSpot::Tile { pane: p, side } if p == pane => (Some(*side), false),
            FilesSpot::Take(p) if p == pane => (None, false),
            FilesSpot::Replace(p) if p == pane => (None, true),
            _ => return None,
        };
        Some(indicator(zone, &h.label, if has_head { 38. } else { 6. }, replace, t))
    }

    /// The files about to land along an edge of the main area.
    pub(super) fn files_edge_hint(&self, t: &Theme) -> Option<AnyElement> {
        let h = self.files_hint.as_ref()?;
        let FilesSpot::Edge(side) = &h.spot else { return None };
        let share = super::workspace::EDGE_SHARE as f32;
        let el = div().absolute();
        let el = match side {
            Side::Left => el.left_0().top_0().bottom_0().w(gpui::relative(share)),
            Side::Right => el.right_0().top_0().bottom_0().w(gpui::relative(share)),
            Side::Top => el.left_0().right_0().top_0().h(gpui::relative(share)),
            Side::Bottom => el.left_0().right_0().bottom_0().h(gpui::relative(share)),
        };
        Some(el.debug_selector(|| "files-drop-edge".into()).child(indicator(None, &h.label, 0., false, t)).into_any_element())
    }

    /// Tile `pane` showing agent `owner`'s files.
    pub(super) fn render_files_pane(
        &mut self,
        pane: &str,
        owner: &str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = crate::explorer::dock::view_for(owner, window, cx);
        let what = Content::Files(owner.to_string());
        let keys = crate::explorer::dock::viewer(owner, cx).map(|v| v.focus_handle(cx));
        if let Some(k) = &keys {
            self.follow_tile_keys(&what, k, window, cx);
        }
        let body: AnyElement = match view {
            Some(v) => {
                // Its agent was removed: kept for unsaved edits, says so.
                let bar = crate::explorer::dock::viewer(owner, cx).and_then(|e| crate::explorer::dock::orphan_bar(owner, &e, t, cx));
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .children(bar)
                    .child(div().flex_1().min_w_0().min_h_0().flex().child(v))
                    .into_any_element()
            }
            None => {
                let o = owner.to_string();
                div()
                    .m_auto()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .text_color(t.text_3)
                    .child("These files can't be read from here (their agent is gone).")
                    .child(
                        crate::kit::small_btn("files-tile-close", "Close", t).on_click(cx.listener(move |this, _, _, cx| {
                            let o = o.clone();
                            this.update_ui(|s| s.undock_files(&o).forget_files(&o), cx);
                        })),
                    )
                    .into_any_element()
            }
        };
        self.tile_shell(pane, what, format!("files-tile-{owner}"), body, keys, t, cx)
    }

    /// Debug builds, `tile …` (bench commands): `terminal <folder>` (a
    /// terminal there), `files` (the focused agent's files right of its
    /// terminal), `press files|terminal|web` (a press in the middle of
    /// that tile of the active space, through the window's mouse events:
    /// nothing is clicked on the screen), `state` (on stderr: the focused
    /// and maximised tiles, where the keys are, the viewers' tabs).
    #[cfg(debug_assertions)]
    pub fn debug_tile(&mut self, cmd: &str, window: &mut Window, cx: &mut Context<Self>) {
        let say = |s: String| eprintln!("pitwall-debug: tile {s}");
        let mut words = cmd.splitn(2, ' ');
        let (what, arg) = (words.next().unwrap_or_default(), words.next().unwrap_or_default().trim());
        let Some(space) = self.active_space().cloned() else { return say("no space".into()) };
        match what {
            "terminal" => self.open_terminal(arg.to_string(), window, cx),
            // (Once this window's update is done: placing updates it.)
            "files" => match space.focused_agent() {
                Some(a) => cx.defer(move |cx| match crate::explorer::dock::place(&a, crate::explorer::dock::FilesPlace::Next(Side::Right), None, cx) {
                    Ok(at) => eprintln!("pitwall-debug: tile files of {a}: {at}"),
                    Err(e) => eprintln!("pitwall-debug: tile files of {a}: {e}"),
                }),
                None => say("no focused agent".into()),
            },
            "press" => {
                let pane = space.layout.leaves().into_iter().find(|p| match arg {
                    "files" => p.files.is_some(),
                    "web" => p.web.is_some(),
                    _ => p.agent_id.is_some(),
                });
                let Some(b) = pane.and_then(|p| self.files_boxes.pane(&p.id)) else { return say(format!("no {arg} tile drawn")) };
                let position = b.center();
                window.defer(cx, move |window, cx| {
                    let (button, modifiers) = (gpui::MouseButton::Left, gpui::Modifiers::default());
                    let down = gpui::MouseDownEvent { position, button, modifiers, click_count: 1, first_mouse: false };
                    window.dispatch_event(gpui::PlatformInput::MouseDown(down), cx);
                    let up = gpui::MouseUpEvent { position, button, modifiers, click_count: 1 };
                    window.dispatch_event(gpui::PlatformInput::MouseUp(up), cx);
                });
            }
            "state" => {
                let leaves = space.layout.leaves();
                let name = |id: Option<&String>| {
                    let p = id.and_then(|id| leaves.iter().find(|p| &p.id == id));
                    p.map(|p| match (&p.agent_id, &p.files, &p.web, &p.inbox, &p.view) {
                        (Some(a), ..) => format!("terminal {a}"),
                        (_, Some(f), ..) => format!("files {f}"),
                        (_, _, Some(w), ..) => format!("web {w}"),
                        (_, _, _, Some(e), _) => format!("inbox {e}"),
                        (_, _, _, _, Some(v)) => format!("view {v}"),
                        _ if p.settings => "settings".into(),
                        _ if p.device.is_some() => format!("device {}", p.device.clone().unwrap_or_default()),
                        _ if p.editor => "editor".into(),
                        _ => "empty".into(),
                    })
                };
                let keys: Vec<String> = leaves
                    .iter()
                    .filter_map(|p| p.files.as_ref())
                    .filter_map(|f| crate::explorer::dock::viewer(f, cx).map(|v| (f, v)))
                    .map(|(f, v)| {
                        let v = v.read(cx);
                        format!("{f}: keys {}, tabs {:?}, active {:?}", v.focus_handle(cx).contains_focused(window, cx), v.tabs(), v.active())
                    })
                    .collect();
                say(format!(
                    "focused {:?}, maximised {:?}, focused agent {:?}, viewers [{}]",
                    name(space.focused_pane_id.as_ref()),
                    name(space.maximized_pane_id.as_ref()),
                    space.focused_agent(),
                    keys.join("; ")
                ));
            }
            _ => say(format!("what? {cmd}")),
        }
    }

    /// Tile `what`'s content taking the keys (`keys`, or anything in it)
    /// makes it the focused tile, as a press in it does: a click in a web
    /// page (the native view's, which GPUI never sees), a field.
    pub(super) fn follow_tile_keys(&mut self, what: &Content, keys: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        if self.tile_keys.0.get(what).is_some_and(|(k, _)| k == keys) {
            return;
        }
        // (Those whose tile is gone go now.)
        let gone: Vec<Content> = self.tile_keys.0.keys().filter(|c| self.ui.tile_of(c).is_none()).cloned().collect();
        for c in gone {
            self.tile_keys.0.remove(&c);
        }
        let c = what.clone();
        let sub = cx.on_focus_in(keys, window, move |this, _, cx| {
            let here = this.active_space().map(|s| s.id.clone());
            if let Some((_, pane)) = this.ui.tile_of(&c).filter(|(space, _)| Some(space) == here.as_ref()) {
                this.focus_tile(&pane, cx);
            }
        });
        self.tile_keys.0.insert(what.clone(), (keys.clone(), sub));
    }

    /// A files, web or inbox tile around `body`: its box (for
    /// [`zone_at`]), an agent dropped on a side splits it, and where
    /// dragged tiles land. It takes part in the tile keys as a terminal
    /// does: a press anywhere in it (its controls too) makes it the space's
    /// focused tile (⌘⏎ maximises it), shown by its border, and gives
    /// `keys` the keyboard unless something in it has it already (a press
    /// in a field then takes it on).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tile_shell(
        &mut self,
        pane: &str,
        what: Content,
        selector: String,
        body: AnyElement,
        keys: Option<FocusHandle>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hint = self.drop_hint.as_ref().filter(|(p, _)| p == pane).and_then(|(_, z)| *z);
        let (move_id, drop_id) = (pane.to_string(), pane.to_string());
        let focused = self.active_space().and_then(|s| s.focused_pane_id.as_deref()) == Some(pane);
        let focus_id = pane.to_string();
        div()
            .id(SharedString::from(format!("pane-{pane}")))
            .debug_selector(move || selector.clone())
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .overflow_hidden()
            .bg(t.bg)
            .border_1()
            .border_color(if focused {
                t.text_4
            } else if t.is_glass() {
                t.g.line_strong
            } else {
                t.line
            })
            .capture_any_mouse_down(cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                if e.button != gpui::MouseButton::Left {
                    return;
                }
                this.focus_tile(&focus_id, cx);
                if let Some(k) = keys.as_ref().filter(|k| !k.contains_focused(window, cx)) {
                    window.focus(k, cx);
                }
            }))
            .child(self.files_measure(pane, Kind::Tile(what)))
            // An agent dragged here: on a side only (the tile stays).
            .on_drag_move::<AgentDrag>(cx.listener(move |this, e: &DragMoveEvent<AgentDrag>, _, cx| {
                let b = e.bounds;
                let was = this.drop_hint.as_ref().is_some_and(|(p, _)| *p == move_id);
                if b.contains(&e.event.position) {
                    let fx = f32::from(e.event.position.x - b.left()) / f32::from(b.size.width).max(1.);
                    let fy = f32::from(e.event.position.y - b.top()) / f32::from(b.size.height).max(1.);
                    let next = tree::drop_zone(fx as f64, fy as f64).map(|z| (move_id.clone(), Some(z)));
                    if this.drop_hint != next {
                        this.drop_hint = next;
                        cx.notify();
                    }
                } else if was {
                    this.drop_hint = None;
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(move |this, d: &AgentDrag, window, cx| {
                let zone = this.drop_hint.take().filter(|(p, _)| *p == drop_id).and_then(|(_, z)| z);
                if zone.is_some() {
                    this.drop_on_pane(&drop_id, &d.id, zone, window, cx);
                }
            }))
            .child(body)
            .when_some(hint, |d, zone| d.child(indicator(Some(zone), &format!("Split {}", side_word(zone)), 6., false, t)))
            .children(self.files_pane_hint(pane, false, t))
            .into_any_element()
    }
}

fn side_word(s: Side) -> &'static str {
    match s {
        Side::Left => "left",
        Side::Right => "right",
        Side::Top => "above",
        Side::Bottom => "below",
    }
}

/// `drop_indicator`'s look: the part of the tile (or area) that `zone`
/// takes (`None`: all of it), with `text`. `replace`: the files take the
/// tile's place (what is in it leaves): the whole tile, more strongly.
fn indicator(zone: Option<Side>, text: &str, top: f32, replace: bool, t: &Theme) -> AnyElement {
    let (l, tp, r, b) = match zone {
        None => (0., 0., 0., 0.),
        Some(Side::Left) => (0., 0., 0.5, 0.),
        Some(Side::Right) => (0.5, 0., 0., 0.),
        Some(Side::Top) => (0., 0., 0., 0.5),
        Some(Side::Bottom) => (0., 0.5, 0., 0.),
    };
    let (bg, ring) = if replace { (t.text.opacity(0.22), t.text.opacity(0.85)) } else { (t.text.opacity(0.10), t.text.opacity(0.45)) };
    let label = div()
        .px(px(9.))
        .py(px(3.))
        .rounded(px(5.))
        .bg(t.surface_3)
        .text_color(t.text)
        .child(crate::kit::tracked(&text.to_uppercase(), 11., 0.04).font_weight(FontWeight::SEMIBOLD));
    div()
        .absolute()
        .top(px(top))
        .left(px(6.))
        .right(px(6.))
        .bottom(px(6.))
        .debug_selector(move || if replace { "files-drop-replace".into() } else { "files-drop-indicator".into() })
        .child(
            div()
                .absolute()
                .left(gpui::relative(l))
                .top(gpui::relative(tp))
                .right(gpui::relative(r))
                .bottom(gpui::relative(b))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .bg(bg)
                .border_2()
                .border_color(ring)
                .child(label),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn zones_around_tiles_along_edges_and_in_tiles() {
        let area = b(0., 0., 1000., 600.);
        let panes = vec![
            ("a".to_string(), b(0., 0., 500., 600.), Kind::Agent),
            ("e".to_string(), b(500., 0., 500., 300.), Kind::Empty),
            ("f".to_string(), b(500., 300., 500., 300.), Kind::Tile(files("x"))),
        ];
        let at = |x, y, owner| zone_at(point(px(x), px(y)), area, &panes, &files(owner));
        // The area's edges: a column or row of its own.
        assert_eq!(at(5., 300., "x"), Some(FilesSpot::Edge(Side::Left)));
        assert_eq!(at(995., 100., "x"), Some(FilesSpot::Edge(Side::Right)));
        assert_eq!(at(250., 590., "x"), Some(FilesSpot::Edge(Side::Bottom)));
        // A terminal's side bands split it (64 px deep at most here)…
        assert_eq!(at(450., 300., "x"), Some(FilesSpot::Tile { pane: "a".into(), side: Side::Right }));
        assert_eq!(at(250., 60., "x"), Some(FilesSpot::Tile { pane: "a".into(), side: Side::Top }));
        // …and all the rest of it is its place: replaced.
        assert_eq!(at(250., 300., "x"), Some(FilesSpot::Replace("a".into())));
        assert_eq!(at(400., 300., "x"), Some(FilesSpot::Replace("a".into())));
        assert_eq!(at(120., 100., "x"), Some(FilesSpot::Replace("a".into())));
        // An empty tile: taken, anywhere in it.
        assert_eq!(at(750., 150., "x"), Some(FilesSpot::Take("e".into())));
        assert_eq!(at(970., 150., "x"), Some(FilesSpot::Take("e".into())));
        // Over its own tile: nothing; over another agent's files: its sides,
        // and its middle swaps them when its own are in this space.
        assert_eq!(at(750., 450., "x"), None);
        assert_eq!(at(540., 450., "y"), Some(FilesSpot::Tile { pane: "f".into(), side: Side::Left }));
        assert_eq!(at(750., 450., "y"), None);
        let mut with_y = panes.clone();
        with_y[1].2 = Kind::Tile(files("y"));
        assert_eq!(zone_at(point(px(750.), px(450.)), area, &with_y, &files("y")), Some(FilesSpot::Replace("f".into())));
        // Outside the area.
        assert_eq!(at(1200., 300., "x"), None);
        // A web tile dragged: the same places; over files, their sides,
        // and their middle swaps when the web tile is in this space.
        let web = Content::Web("w".into());
        let at_web = |x, y, panes: &[PaneBox]| zone_at(point(px(x), px(y)), area, panes, &web);
        assert_eq!(at_web(250., 300., &panes), Some(FilesSpot::Replace("a".into())));
        assert_eq!(at_web(995., 100., &panes), Some(FilesSpot::Edge(Side::Right)));
        assert_eq!(at_web(540., 450., &panes), Some(FilesSpot::Tile { pane: "f".into(), side: Side::Left }));
        assert_eq!(at_web(750., 450., &panes), None);
        let mut with_web = panes.clone();
        with_web[1].2 = Kind::Tile(web.clone());
        assert_eq!(at_web(750., 450., &with_web), Some(FilesSpot::Replace("f".into())));
        assert_eq!(at_web(750., 150., &with_web), None, "over itself");
    }

    fn files(owner: &str) -> Content {
        Content::Files(owner.into())
    }

    #[test]
    fn side_bands_are_a_fifth_of_a_tile_between_28_and_64_px() {
        let tile = b(0., 0., 1000., 100.);
        let at = |x, y| side_band(point(px(x), px(y)), tile);
        // Wide: 64 px bands left and right; short: 28 px top and bottom.
        assert_eq!(at(63., 50.), Some(Side::Left));
        assert_eq!(at(65., 50.), None);
        assert_eq!(at(940., 50.), Some(Side::Right));
        assert_eq!(at(500., 27.), Some(Side::Top));
        assert_eq!(at(500., 29.), None);
        assert_eq!(at(500., 75.), Some(Side::Bottom));
        // A corner: the side it is relatively nearer.
        assert_eq!(at(10., 20.), Some(Side::Left));
        // A small tile: never more than half of it.
        let small = b(0., 0., 40., 40.);
        assert_eq!(side_band(point(px(20.), px(20.)), small), None);
        assert_eq!(side_band(point(px(5.), px(20.)), small), Some(Side::Left));
    }
}
