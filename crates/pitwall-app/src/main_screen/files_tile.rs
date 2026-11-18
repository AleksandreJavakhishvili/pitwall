//! A files tile in the space's layout (a pane with `files`: an agent's file
//! viewer, `crate::explorer::dock`), and where files dragged over this
//! window would land: around any tile (splitting it), along an edge of the
//! main area (a column or row of its own), or in an empty tile; the middle
//! of a terminal's tile splits it along its longer side. Drawn as an agent being dragged
//! is (`drop_indicator`'s look); found from the boxes the last frame drew,
//! since the drag comes from another window (`crate::explorer::drag`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{canvas, div, prelude::*, px, AnyElement, App, Bounds, Context, DragMoveEvent, FontWeight, Pixels, Point, SharedString, Window};

use super::topbar::AgentDrag;
use super::tree::{self, Side};
use super::workspace::FilesSpot;
use super::MainScreen;
use crate::theme::Theme;

/// What a tile shows (where files may go on it).
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Agent,
    Empty,
    /// Whose files.
    Files(String),
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
    fn get(&self) -> (Option<Bounds<Pixels>>, Vec<PaneBox>) {
        let g = self.0.borrow();
        (g.area, g.panes.iter().map(|(id, (b, k))| (id.clone(), *b, k.clone())).collect())
    }
}

/// Where files dragged over this window land now.
#[derive(Debug, Clone, PartialEq)]
pub struct FilesHint {
    pub owner: String,
    pub spot: FilesSpot,
    /// Where they'd be, in the words the tile's place is said in once
    /// there ("Files · above api's terminal").
    pub label: String,
}

/// Along the main area's edge, this deep: its own column or row.
const EDGE_BAND: f32 = 24.;

/// Where files of agent `owner` dropped at `at` land: see the module.
pub fn zone_at(at: Point<Pixels>, area: Bounds<Pixels>, panes: &[PaneBox], owner: &str) -> Option<FilesSpot> {
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
    let fx = f32::from(at.x - b.left()) / f32::from(b.size.width).max(1.);
    let fy = f32::from(at.y - b.top()) / f32::from(b.size.height).max(1.);
    match kind {
        Kind::Files(o) if o == owner => None,
        Kind::Empty => Some(FilesSpot::Take(id.clone())),
        _ => match tree::drop_zone(fx as f64, fy as f64) {
            Some(side) => Some(FilesSpot::Tile { pane: id.clone(), side }),
            // The middle of a terminal's: split along its longer side.
            None if *kind == Kind::Agent => {
                let side = if b.size.width >= b.size.height { Side::Right } else { Side::Bottom };
                Some(FilesSpot::Tile { pane: id.clone(), side })
            }
            None => None,
        },
    }
}

impl MainScreen {
    /// Files of agent `owner` dragged at `at` (this window's coordinates):
    /// shows where they'd land and says so (with the space). `None`:
    /// nowhere here.
    pub fn files_hover(&mut self, owner: &str, at: Point<Pixels>, cx: &mut Context<Self>) -> Option<(String, FilesSpot)> {
        let space = self.active_space().map(|s| s.id.clone()).filter(|_| self.route == super::Route::Space);
        let spot = space.as_ref().and_then(|_| {
            let (area, panes) = self.files_boxes.get();
            zone_at(at, area?, &panes, owner)
        });
        let same = self.files_hint.as_ref().is_some_and(|h| h.owner == owner && Some(&h.spot) == spot.as_ref());
        if same {
            return Some((space?, spot?));
        }
        let hint = match (&space, &spot) {
            (Some(sp), Some(spot)) => Some(FilesHint { owner: owner.to_string(), spot: spot.clone(), label: self.landing_words(sp, owner, spot, cx) }),
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
    fn landing_words(&self, space: &str, owner: &str, spot: &FilesSpot, cx: &App) -> String {
        let (next, _) = self.ui.clone().dock_files(space, owner, spot, super::workspace::FILES_SHARE, self.fit_for(space));
        let Some((sp, pane)) = next.files_tile(owner) else { return "Files".into() };
        let Some(layout) = next.space(&sp).map(|s| s.layout.clone()) else { return "Files".into() };
        let anchor = match spot {
            FilesSpot::Tile { pane, .. } => layout.leaves().into_iter().find(|p| p.id == *pane && p.files.is_none()).and_then(|p| p.agent_id),
            _ => None,
        };
        let names: Vec<(String, String)> = self.agents(cx).iter().map(|a| (a.id.clone(), a.name.clone())).collect();
        let name = |id: &str| names.iter().find(|(i, _)| i == id).map_or_else(|| id.to_string(), |(_, n)| n.clone());
        format!("Files · {}", super::workspace::whereabouts(&layout, &pane, anchor.as_deref()).words(&name))
    }

    /// No files dragged over this window (any more).
    pub fn clear_files_hover(&mut self, cx: &mut Context<Self>) {
        if self.files_hint.take().is_some() {
            cx.notify();
        }
    }

    /// Where files would land now (tests, debug).
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
        let zone = match &h.spot {
            FilesSpot::Tile { pane: p, side } if p == pane => Some(*side),
            FilesSpot::Take(p) if p == pane => None,
            _ => return None,
        };
        Some(indicator(zone, &h.label, if has_head { 38. } else { 6. }, t))
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
        Some(el.debug_selector(|| "files-drop-edge".into()).child(indicator(None, &h.label, 0., t)).into_any_element())
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
        let hint = self.drop_hint.as_ref().filter(|(p, _)| p == pane).and_then(|(_, z)| *z);
        let (move_id, drop_id) = (pane.to_string(), pane.to_string());
        let body: AnyElement = match view {
            Some(v) => div().flex_1().min_w_0().min_h_0().flex().child(v).into_any_element(),
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
        div()
            .id(SharedString::from(format!("pane-{pane}")))
            .debug_selector({
                let o = owner.to_string();
                move || format!("files-tile-{o}")
            })
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
            .border_color(if t.is_glass() { t.g.line_strong } else { t.line })
            .child(self.files_measure(pane, Kind::Files(owner.to_string())))
            // An agent dragged here: on a side only (the files stay).
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
            .when_some(hint, |d, zone| d.child(indicator(Some(zone), &format!("Split {}", side_word(zone)), 6., t)))
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
/// takes (`None`: all of it), with `text`.
fn indicator(zone: Option<Side>, text: &str, top: f32, t: &Theme) -> AnyElement {
    let (l, tp, r, b) = match zone {
        None => (0., 0., 0., 0.),
        Some(Side::Left) => (0., 0., 0.5, 0.),
        Some(Side::Right) => (0.5, 0., 0., 0.),
        Some(Side::Top) => (0., 0., 0., 0.5),
        Some(Side::Bottom) => (0., 0.5, 0., 0.),
    };
    let (bg, ring) = (t.text.opacity(0.10), t.text.opacity(0.45));
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
        .debug_selector(|| "files-drop-indicator".into())
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
            ("f".to_string(), b(500., 300., 500., 300.), Kind::Files("x".into())),
        ];
        let at = |x, y, owner| zone_at(point(px(x), px(y)), area, &panes, owner);
        // The area's edges: a column or row of its own.
        assert_eq!(at(5., 300., "x"), Some(FilesSpot::Edge(Side::Left)));
        assert_eq!(at(995., 100., "x"), Some(FilesSpot::Edge(Side::Right)));
        assert_eq!(at(250., 590., "x"), Some(FilesSpot::Edge(Side::Bottom)));
        // A terminal's sides split it, and so does its middle.
        assert_eq!(at(400., 300., "x"), Some(FilesSpot::Tile { pane: "a".into(), side: Side::Right }));
        assert_eq!(at(250., 60., "x"), Some(FilesSpot::Tile { pane: "a".into(), side: Side::Top }));
        // (Taller than wide: split below.)
        assert_eq!(at(250., 300., "x"), Some(FilesSpot::Tile { pane: "a".into(), side: Side::Bottom }));
        // An empty tile: taken.
        assert_eq!(at(750., 150., "x"), Some(FilesSpot::Take("e".into())));
        // Over its own tile: nothing; over another agent's files: its sides.
        assert_eq!(at(750., 450., "x"), None);
        assert_eq!(at(560., 450., "y"), Some(FilesSpot::Tile { pane: "f".into(), side: Side::Left }));
        assert_eq!(at(750., 450., "y"), None);
        // Outside the area.
        assert_eq!(at(1200., 300., "x"), None);
    }
}
