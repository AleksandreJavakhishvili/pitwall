//! The centre (React `App.tsx` centre, `SpaceView.tsx`, `LayoutView.tsx`,
//! `PaneView.tsx`, `PaneHeader.tsx`, `ChipStrip.tsx`, `StoppedOverlay.tsx`,
//! `EmptyState.tsx`): the empty state, another screen's view, or the active
//! space — its bar (presets, density, maximise), the split tree of panes with
//! draggable dividers and drop zones, and the chip strip.

use crate::kit::Ellipsis as _;
use crate::kit::HoverText;
use std::collections::HashMap;

use gpui::{
    canvas, div, prelude::*, px, AnyElement, App, Bounds, Context, DragMoveEvent, FontWeight, Hsla,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString,
    Window,
};

use pitwall_proto::{AgentView, Status};

use crate::kit::icon;
use super::menu::MenuItem;
use crate::kit::keys;
use super::topbar::AgentDrag;
use super::tree::{self, Dir, Node, Preset, Side};
use crate::kit::{
    button, chip, diffstat, glyph, icon_btn, kbd, label, small_btn, status_label, tooltip, BtnKind,
};
use super::workspace::{Density, Space, ALL_SPACE};
use super::{Breakpoint, DividerDrag, MainScreen, Route, TerminalArgs, TerminalSlot};
use crate::agents::status_word;
use crate::theme::{Theme, RADIUS, RADIUS_LG};

/// Minimum share per side when dragging a divider.
const MIN_FRAC: f64 = 0.08;
const DIVIDER: f32 = 6.;

impl MainScreen {
    pub(super) fn render_center(
        &mut self,
        agents: &[AgentView],
        bp: Breakpoint,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Switching space / Wall / Review: a short cross-fade in (`m-fade`).
        let fade_key: Option<SharedString> = match self.route {
            _ if agents.is_empty() && self.engine.is_some() => None,
            Route::Wall => Some("center-wall".into()),
            Route::Review => Some("center-review".into()),
            Route::Explorer => None,
            Route::Space => self
                .active_space()
                .map(|s| format!("center-space-{}", s.id).into()),
        };
        let body: AnyElement = if agents.is_empty() && self.engine.is_some() {
            empty_state(t, cx).into_any_element()
        } else if self.route != Route::Space {
            match self.parts.as_ref().filter(|_| super::part::cached(cx)) {
                Some(p) => super::part::slot(
                    &p.route,
                    gpui::StyleRefinement::default().flex_1().min_h_0().w_full(),
                ),
                None => self.render_route_body(t, window, cx),
            }
        } else if let Some(space) = self.active_space().cloned() {
            self.render_space(&space, agents, bp, t, window, cx)
                .into_any_element()
        } else {
            div()
                .m_auto()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(14.))
                .child(label("No spaces in this window", t.text, 24.))
                .child(
                    button("new-space", BtnKind::Primary, false, t)
                        .on_click(cx.listener(|this, _, _, cx| this.new_space(cx)))
                        .child("New space"),
                )
                .into_any_element()
        };
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            // Glass: the panes float on the window's glass (`.center`).
            .when(!t.is_glass(), |d| d.bg(t.bg))
            .child(match fade_key {
                Some(key) => crate::kit::motion::enter(
                    key,
                    crate::kit::motion::Fx::FADE,
                    div().flex_1().min_w_0().min_h_0().flex().flex_col().child(body),
                )
                .into_any_element(),
                None => body,
            })
    }

    /// The centre on the Wall, Review or Explorer route.
    pub(super) fn render_route_body(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.route_view(window, cx) {
            Some(v) => v.into_any_element(),
            None => route_placeholder(self.route, t, cx).into_any_element(),
        }
    }

    fn render_space(
        &mut self,
        space: &Space,
        agents: &[AgentView],
        bp: Breakpoint,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.drop_hint = None;
        }
        let f = self.fitted(space, agents);
        let bar = match self.parts.as_ref().filter(|_| super::part::cached(cx)) {
            // The bar is a cached view of its own (part.rs).
            Some(p) => super::part::slot(
                &p.bar,
                gpui::StyleRefinement::default().h(px(32.)).w_full().flex_none(),
            ),
            None => self.space_bar(space, &f, t, cx).into_any_element(),
        };
        let Fitted { tree, chips, pane_count, maximized, area, min, density, .. } = f;
        if let Some(a) = area {
            self.auto_font = super::tile_font::auto_fonts(&tree, a, density, self.ui.font_size);
        }

        let me = cx.entity().downgrade();
        let measure = canvas(
            move |bounds: Bounds<Pixels>, _, cx: &mut App| {
                let _ = me.update(cx, |s, cx| {
                    if s.area != Some(bounds.size) {
                        s.area = Some(bounds.size);
                        cx.notify();
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let sizes = area
            .map(|(w, h)| {
                tree::pane_rects(&tree, tree::Rect { x: 0., y: 0., w, h })
                    .into_iter()
                    .map(|(id, r)| (id, (r.w, r.h)))
                    .collect()
            })
            .unwrap_or_default();
        let ctx = PaneCtx {
            focused: space.focused_pane_id.clone(),
            maximized: maximized.is_some(),
            pane_count,
            members: chips.clone(),
            sizes,
            min,
        };
        if let Some(p) = self.parts.as_mut() {
            let shown: Vec<String> = tree.leaves().into_iter().map(|p| p.id).collect();
            p.keep_headers(&shown, window);
        }
        let area_el = div()
            .id("space-area")
            .flex_1()
            .min_h_0()
            .flex()
            .p(px(6.))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .child(measure)
                    .child(self.render_node(&tree, &ctx, agents, t, window, cx)),
            );

        let _ = bp;
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(bar)
            .child(area_el)
            .when(!chips.is_empty(), |d| match self.parts.as_ref().filter(|_| super::part::cached(cx)) {
                // A cached view of its own (part.rs): 28 px chips + 8 px below.
                Some(p) => d.child(super::part::slot(
                    &p.chips,
                    gpui::StyleRefinement::default().h(px(36.)).w_full().flex_none(),
                )),
                None => d.child(self.chip_strip(&chips, &space.id, t, cx)),
            })
    }

    /// The chip strip of the active space (the chips part, part.rs).
    pub(super) fn chips_now(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(space) = self.active_space().cloned() else {
            return div().into_any_element();
        };
        let agents = self.agents(cx).to_vec();
        let f = self.fitted(&space, &agents);
        self.chip_strip(&f.chips, &space.id, t, cx).into_any_element()
    }

    /// The space's bar: name, shown / folded, density, presets, move.
    pub(super) fn space_bar(&self, space: &Space, f: &Fitted, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let space_id = space.id.clone();
        div()
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(12.))
            .pr(px(8.))
            .border_b_1()
            .border_color(if t.is_glass() { t.g.line } else { t.line })
            .child(label(space.name.clone(), t.text_3, 12.))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(t.text_3)
                    .child(if f.hidden.is_empty() {
                        format!("{} shown", f.pane_count)
                    } else {
                        format!(
                            "{} shown · {} folded (too small at this density)",
                            f.pane_count,
                            f.hidden.len()
                        )
                    }),
            )
            .child(div().flex_1())
            .when(f.maximized.is_some(), |d| {
                d.child(
                    small_btn("restore-layout", "Restore layout", t)
                        .child(icon("restore", 12., t.text))
                        .flex_row_reverse()
                        .tooltip(tooltip(keys("Restore (⌘⏎)", false)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.with_active(|s, id| s.toggle_maximize(id, None), cx)
                        })),
                )
            })
            .child(self.density_button(space, t, cx))
            .child(
                // `.presets` (Glass: glass surface-2 in a hairline ring).
                div()
                    .flex()
                    .gap(px(1.))
                    .rounded(RADIUS)
                    .map(|d| {
                        if t.is_glass() {
                            // The inset ring as a border, inside the 2 px.
                            d.p(px(1.)).bg(t.g.surface_2).border_1().border_color(t.g.line)
                        } else {
                            d.p(px(2.)).bg(t.surface_2)
                        }
                    })
                    .children(f.presets.clone().into_iter().map(|p| {
                        let tip = if p == Preset::Auto {
                            "Auto grid: fit all agents evenly, the rest as chips".to_string()
                        } else {
                            format!("Tile {}", p.label())
                        };
                        let group = SharedString::from(format!("preset-{}", p.label()));
                        // `.preset-btn`: text-4 (text-3 on glass), text on hover.
                        let ink = if t.is_glass() { t.text_3 } else { t.text_4 };
                        let eid = gpui::ElementId::Name(group.clone());
                        let pressing = crate::kit::motion::pressing(&eid);
                        let el = div()
                            .id(eid.clone())
                            .group(group.clone())
                            .w(px(28.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .when(!pressing, |d| d.hover_probed(|s| s.bg(t.surface_3)));
                        crate::kit::motion::pressable(
                            el,
                            &eid,
                            false,
                            crate::kit::motion::Skin::new(Some(t.surface_3), None, px(4.)),
                        )
                            .tooltip(tooltip(tip))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.apply_preset(p, window, cx)
                            }))
                            .child(preset_icon(p, ink, t.text, group))
                    })),
            )
            .when(space.id != ALL_SPACE, |d| {
                d.child(
                    icon_btn(
                        "space-move",
                        "window",
                        keys("Move to new window (⌘⇧N)", false),
                        true,
                        t,
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.move_space(space_id.clone(), cx)),
                    ),
                )
            })
    }

    /// What `space` shows at the current area: the fitted pane tree, what
    /// folded into chips, and the presets that fit.
    pub(super) fn fitted(&self, space: &Space, agents: &[AgentView]) -> Fitted {
        let density = self.ui.density_of(space);
        let min = super::workspace::fold_min(density, self.ui.font_size);
        let area = self
            .area
            .map(|a| (f32::from(a.width) as f64, f32::from(a.height) as f64));
        let maximized = space
            .maximized_pane_id
            .as_deref()
            .and_then(|m| space.layout.find_node(m).cloned());
        let base = maximized.clone().unwrap_or(space.layout.clone());
        let (tree, hidden) = match area {
            Some(a) => tree::fit_layout(&base, a, min, space.focused_pane_id.as_deref()),
            None => (base, vec![]),
        };
        // A divider being dragged shows its live sizes.
        let tree = match &self.divider {
            Some(d) => tree::set_split_sizes(&tree, &d.split, &d.live),
            None => tree,
        };
        let visible = tree.agents();
        let chips: Vec<AgentView> = space
            .members_of(agents)
            .into_iter()
            .filter(|a| !visible.contains(&a.id))
            .cloned()
            .collect();
        Fitted {
            pane_count: space.layout.agents().len(),
            presets: tree::available_presets(area, min),
            tree,
            hidden,
            chips,
            maximized,
            area,
            min,
            density,
        }
    }

    /// A pane's header (the pane header part, part.rs): its agent in the
    /// active space now.
    pub(super) fn header_of(&self, pane_id: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(space) = self.active_space().cloned() else {
            return div().into_any_element();
        };
        let agent = match space.layout.find_node(pane_id) {
            Some(Node::Pane { agent_id: Some(a), .. }) => self.agents(cx).iter().find(|x| x.id == *a).cloned(),
            _ => None,
        };
        let Some(a) = agent else {
            return div().into_any_element();
        };
        let ctx = PaneCtx {
            focused: space.focused_pane_id.clone(),
            maximized: space.maximized_pane_id.is_some(),
            pane_count: 0,
            members: vec![],
            sizes: HashMap::new(),
            min: super::workspace::fold_min(self.ui.density_of(&space), self.ui.font_size),
        };
        self.pane_header(&a, pane_id, &ctx, t, cx).into_any_element()
    }

    fn density_button(&self, space: &Space, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let current = match space.density {
            Some(d) => d.label().to_string(),
            None => format!("Default ({})", self.ui.density.label()),
        };
        let global = self.ui.density;
        let space_id = space.id.clone();
        div()
            .id("density")
            .flex()
            .items_center()
            .gap(px(14.))
            .h(px(22.))
            .px(px(6.))
            .max_w(px(170.))
            .rounded(px(4.))
            .border_1()
            .border_color(t.line)
            .bg(t.surface_2)
            .text_size(px(11.5))
            .text_color(t.text_3)
            .cursor_pointer()
            .hover_text(t.text, |s| s.border_color(t.line_strong))
            .child(div().ellipsis().child(crate::kit::one_line(current)))
            .child(crate::kit::chevron_at(true, 11., t.text_3))
            .tooltip(tooltip("Tile density for this space (smallest tile before it folds into a chip)"))
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                let mut items = vec![{
                    let id = space_id.clone();
                    MenuItem::new(
                        "",
                        &format!("Default ({})", global.label()),
                        move |this, _, cx| this.update_ui(|s| s.set_density(None, Some(&id)), cx),
                    )
                }];
                for d in Density::ALL {
                    let id = space_id.clone();
                    let crate::theme::Cells { cols: c, rows: r } = d.cells();
                    items.push(MenuItem::new(
                        "",
                        &format!("{} · {c}×{r}", d.label()),
                        move |this, _, cx| {
                            this.update_ui(|s| s.set_density(Some(d), Some(&id)), cx)
                        },
                    ));
                }
                let at = e.position();
                this.open_menu(gpui::point(at.x, at.y + px(6.)), items, window, cx);
            }))
    }

    fn render_node(
        &mut self,
        node: &Node,
        ctx: &PaneCtx,
        agents: &[AgentView],
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            Node::Pane { id, agent_id } => {
                let agent = agent_id
                    .as_ref()
                    .and_then(|a| agents.iter().find(|x| x.id == *a))
                    .cloned();
                self.render_pane(id, agent, ctx, t, window, cx)
                    .into_any_element()
            }
            Node::Split {
                id,
                dir,
                children,
                sizes,
            } => {
                let row = *dir == Dir::Row;
                let mut kids: Vec<AnyElement> = Vec::new();
                for (i, c) in children.iter().enumerate() {
                    if i > 0 {
                        let split = id.clone();
                        let initial = sizes.clone();
                        let index = i - 1;
                        kids.push(
                            div()
                                .id(SharedString::from(format!("div-{id}-{i}")))
                                .flex_none()
                                .when(row, |d| d.w(px(DIVIDER)).h_full().cursor_col_resize())
                                .when(!row, |d| d.h(px(DIVIDER)).w_full().cursor_row_resize())
                                .flex()
                                .items_center()
                                .justify_center()
                                .group("divider")
                                .child(
                                    div()
                                        .rounded(px(2.))
                                        .when(row, |d| d.w(px(2.)).h(gpui::relative(0.4)))
                                        .when(!row, |d| d.h(px(2.)).w(gpui::relative(0.4)))
                                        .group_hover_probed("divider", |s| s.bg(t.text_4)),
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.divider_down(
                                            &split,
                                            index,
                                            row,
                                            initial.clone(),
                                            e,
                                            cx,
                                        );
                                    }),
                                )
                                .into_any_element(),
                        );
                    }
                    let share = sizes.get(i).copied().unwrap_or(0.) as f32;
                    let mut cell = div().flex().min_w_0().min_h_0().flex_basis(px(0.));
                    cell.style().flex_grow = Some(share);
                    cell.style().flex_shrink = Some(1.);
                    kids.push(
                        cell.child(self.render_node(c, ctx, agents, t, window, cx))
                            .into_any_element(),
                    );
                }
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .when(!row, |d| d.flex_col())
                    .children(kids)
                    .into_any_element()
            }
        }
    }

    fn divider_down(
        &mut self,
        split: &str,
        index: usize,
        row: bool,
        initial: Vec<f64>,
        e: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(area) = self.area else { return };
        let Some(sp) = self.active_space() else {
            return;
        };
        let full = tree::Rect {
            x: 0.,
            y: 0.,
            w: f32::from(area.width) as f64,
            h: f32::from(area.height) as f64,
        };
        let total = split_rect(&sp.layout, full, split)
            .map(|r| if row { r.w } else { r.h })
            .unwrap_or(if row { full.w } else { full.h });
        self.divider = Some(DividerDrag {
            split: split.to_string(),
            index,
            row,
            start: if row { e.position.x } else { e.position.y },
            total: total.max(1.),
            live: initial.clone(),
            initial,
        });
        cx.notify();
    }

    pub(super) fn divider_move(
        &mut self,
        e: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(d) = self.divider.as_mut() else {
            return;
        };
        let pos = if d.row { e.position.x } else { e.position.y };
        let delta = f32::from(pos - d.start) as f64 / d.total;
        d.live = tree::resized(&d.initial, d.index, delta, MIN_FRAC);
        cx.notify();
    }

    pub(super) fn divider_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.divider.take() else { return };
        self.with_active(
            |s, id| {
                let layout = s
                    .space(id)
                    .map(|sp| tree::set_split_sizes(&sp.layout, &d.split, &d.live));
                match layout {
                    Some(l) => s.set_layout(id, l),
                    None => s,
                }
            },
            cx,
        );
        cx.notify();
    }

    fn render_pane(
        &mut self,
        pane_id: &str,
        agent: Option<AgentView>,
        ctx: &PaneCtx,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focused = ctx.focused.as_deref() == Some(pane_id);
        let status = agent.as_ref().map(|a| a.status);
        let hint = self
            .drop_hint
            .as_ref()
            .filter(|(p, _)| p == pane_id)
            .map(|(_, z)| *z);
        let (focus_id, move_id, drop_id) = (
            pane_id.to_string(),
            pane_id.to_string(),
            pane_id.to_string(),
        );
        let has_agent = agent.is_some();
        let own_id = agent.as_ref().map(|a| a.id.clone());
        let border = match status {
            Some(Status::Blocked) => t.amber.opacity(0.55),
            Some(Status::Done) => t.finish.opacity(0.65),
            _ if focused => t.text_4,
            // Glass: the centre's hairlines are glass line-strong.
            _ if t.is_glass() => t.g.line_strong,
            _ => t.line,
        };
        let body: AnyElement = match &agent {
            Some(a) => div()
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .child(match self.parts.as_mut().filter(|_| super::part::cached(cx)) {
                    // The header is a cached view of its own (part.rs).
                    Some(p) => {
                        let me = cx.entity();
                        let h = p.header(pane_id, &me, cx);
                        super::part::slot(&h, gpui::StyleRefinement::default().h(px(34.)).w_full().flex_none())
                    }
                    None => self.pane_header(a, pane_id, ctx, t, cx).into_any_element(),
                })
                .flex_col()
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(terminal_element(
                            a,
                            focused,
                            self.focus.is_focused(window),
                            self.tile_font(&a.id),
                            window,
                            cx,
                        ))
                        .when(!a.running, |d| d.child(stopped_overlay(a, t, cx))),
                )
                .into_any_element(),
            None => self.empty_pane(pane_id, ctx, t, cx).into_any_element(),
        };
        div()
            .id(SharedString::from(format!("pane-{pane_id}")))
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .overflow_hidden()
            .bg(t.term_bg)
            .border_1()
            .border_color(border)
            .when(status == Some(Status::Blocked), |d| {
                d.shadow(t.amber_glow(22., -6.))
            })
            .when(status == Some(Status::Done), |d| {
                // The finish bar pops in (`m-flag`).
                d.child(
                    div().absolute().top_0().left_0().right_0().h(px(4.)).child(
                        crate::kit::motion::flag_in(
                            SharedString::from(format!("pane-flag-{pane_id}")),
                            crate::kit::motion::FLAG_ROW,
                            div().size_full().bg(t.finish),
                        ),
                    ),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this
                        .active_space()
                        .and_then(|s| s.focused_pane_id.as_deref())
                        != Some(focus_id.as_str())
                    {
                        let p = focus_id.clone();
                        this.with_active(|s, id| s.focus_pane(id, &p), cx);
                        this.changes_follow(cx);
                    }
                }),
            )
            .on_drag_move::<AgentDrag>(cx.listener(
                move |this, e: &DragMoveEvent<AgentDrag>, _, cx| {
                    let b = e.bounds;
                    // No preview over the dragged agent's own pane (`self`).
                    let own = own_id.as_deref() == Some(e.drag(cx).id.as_str());
                    let inside = b.contains(&e.event.position) && !own;
                    let was = this.drop_hint.as_ref().is_some_and(|(p, _)| *p == move_id);
                    if inside {
                        let fx = f32::from(e.event.position.x - b.left())
                            / f32::from(b.size.width).max(1.);
                        let fy = f32::from(e.event.position.y - b.top())
                            / f32::from(b.size.height).max(1.);
                        let zone = if has_agent {
                            tree::drop_zone(fx as f64, fy as f64)
                        } else {
                            None
                        };
                        let next = Some((move_id.clone(), zone));
                        if this.drop_hint != next {
                            this.drop_hint = next;
                            cx.notify();
                        }
                    } else if was {
                        this.drop_hint = None;
                        cx.notify();
                    }
                },
            ))
            .on_drop(cx.listener(move |this, d: &AgentDrag, window, cx| {
                let zone = this
                    .drop_hint
                    .take()
                    .filter(|(p, _)| *p == drop_id)
                    .and_then(|(_, z)| z);
                this.drop_on_pane(&drop_id, &d.id, zone, window, cx);
            }))
            .child(body)
            .when_some(hint, |d, zone| {
                let no_room = has_agent
                    && match (zone, ctx.sizes.get(pane_id)) {
                        (Some(Side::Left | Side::Right), Some((w, _))) => w / 2. < ctx.min.min_w,
                        (Some(Side::Top | Side::Bottom), Some((_, h))) => h / 2. < ctx.min.min_h,
                        _ => false,
                    };
                d.child(drop_indicator(zone, has_agent, no_room, t))
            })
    }

    fn drop_on_pane(
        &mut self,
        pane: &str,
        agent: &str,
        zone: Option<Side>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(space) = self.active_space().map(|s| s.id.clone()) else {
            return;
        };
        let fit = self.fit();
        let mut outcome = super::workspace::DropOutcome::Noop;
        let (pane, agent) = (pane.to_string(), agent.to_string());
        self.update_ui(
            |s| {
                let (s, o, _) = s.drop_on_pane(&space, &agent, &pane, zone, fit);
                outcome = o;
                s
            },
            cx,
        );
        self.after_drop(&agent, outcome, window, cx);
    }

    fn pane_header(
        &self,
        a: &AgentView,
        pane_id: &str,
        ctx: &PaneCtx,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (max_id, close_id, dbl_id) = (
            pane_id.to_string(),
            pane_id.to_string(),
            pane_id.to_string(),
        );
        let (stop_id, rm_id) = (a.id.clone(), a.id.clone());
        // An agent's pane can always close (the agent keeps running).
        let can_close = true;
        let tint = match a.status {
            Status::Blocked => Some(t.amber_soft),
            Status::Done => Some(t.finish_soft),
            _ => None,
        };
        let machine_tip = format!("{} ({})", a.machine.label, a.machine.provider);
        let branch_tip = if a.worktree {
            format!("Separate worktree: {}", a.cwd_display)
        } else {
            a.cwd_display.clone()
        };
        div()
            .id(SharedString::from(format!("head-{pane_id}")))
            .h(px(34.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(10.))
            .pr(px(6.))
            .border_b_1()
            .border_color(t.line)
            .bg(t.surface)
            .when_some(tint, |d, c| d.bg(c))
            .min_w_0()
            .tooltip(tooltip("Double-click to maximise · drag to move"))
            .on_drag(
                AgentDrag {
                    id: a.id.clone(),
                    name: a.name.clone().into(),
                },
                |d, grab, w, cx| d.ghost(grab, w, cx),
            )
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                if e.click_count() == 2 {
                    let p = dbl_id.clone();
                    this.with_active(|s, id| s.toggle_maximize(id, Some(&p)), cx);
                }
            }))
            .child(crate::kit::glyph_play(format!("pane-{}", a.id), a.status, t, false))
            .child(
                // `.pane-name`: 650, 13 px.
                div()
                    .flex_none()
                    .font_weight(crate::kit::WEIGHT_650)
                    .text_size(px(13.))
                    .child(a.name.clone()),
            )
            .when(a.engineer, |d| d.child(chip("engineer", t)))
            .child(
                status_label(a.status, t).when(a.status == Status::Done, |d| {
                    d.px(px(6.)).py(px(1.)).rounded(px(4.)).bg(t.finish_soft)
                }),
            )
            .when(a.status == Status::Blocked, |d| {
                d.children(a.status_detail.clone().map(|det| {
                    div()
                        .min_w_0()
                        .ellipsis()
                        .text_size(px(11.5))
                        .font_family(crate::kit::MONO_FONT)
                        .text_color(t.amber)
                        .child(crate::kit::one_line(det))
                }))
            })
            // `.pane-meta`: `flex: 0 1000 auto`, so it gives way before the
            // blocked detail, clipping its items whole (no ellipsis).
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .overflow_hidden()
                    .map(|mut d| {
                        d.style().flex_shrink = Some(1000.);
                        d
                    })
                    .text_size(px(11.5))
                    .text_color(t.text_3)
                    .child(
                        div()
                            .id(SharedString::from(format!("m-{pane_id}")))
                            .tooltip(tooltip(machine_tip))
                            .child(chip(a.machine.label.clone(), t)),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("p-{pane_id}")))
                            .flex_none()
                            .flex()
                            .tooltip(tooltip(a.cwd.clone()))
                            .child(crate::kit::one_line(crate::kit::project_name(&a.project_display).to_string())),
                    )
                    .child(div().text_color(t.text_4).child("·"))
                    .child(div().flex_none().child(a.kind_name.clone()))
                    .when_some(a.branch.clone(), |d, b| {
                        d.child(div().text_color(t.text_4).child("·")).child(
                            div()
                                .id(SharedString::from(format!("b-{pane_id}")))
                                .flex_none()
                                .font_family(crate::kit::MONO_FONT)
                                .tooltip(tooltip(branch_tip.clone()))
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(icon("branch", 12., t.text_3))
                                .child(crate::kit::one_line(b)),
                        )
                    })
                    .child(diffstat(a.added, a.removed, t)),
            )
            .child(div().flex_1())
            .children(crate::rules::stale::button(&a.id, t, cx, {
                let id = a.id.clone();
                move |this: &mut Self, r, cx| match r {
                    Ok(()) => this.restart_agent(id.clone(), cx),
                    Err(e) => this.failed("re-apply rules", e, cx),
                }
            }))
            .child(
                div()
                    .flex()
                    .gap(px(1.))
                    .flex_none()
                    .when(a.caps.stop, |d| {
                        d.child(
                            pane_btn(format!("stop-{pane_id}"), "stop", "Stop process", t)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.stop_agent(stop_id.clone(), cx)
                                })),
                        )
                    })
                    .child(
                        pane_btn(format!("rm-{pane_id}"), "trash", "Remove agent", t).on_click(
                            cx.listener(move |this, _, _, cx| this.open_remove(&rm_id, cx)),
                        ),
                    )
                    .child(
                        pane_btn(
                            format!("max-{pane_id}"),
                            if ctx.maximized { "restore" } else { "maximize" },
                            keys(
                                if ctx.maximized {
                                    "Restore (⌘⏎)"
                                } else {
                                    "Maximise (⌘⏎)"
                                },
                                false,
                            ),
                            t,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let p = max_id.clone();
                            this.with_active(|s, id| s.toggle_maximize(id, Some(&p)), cx)
                        })),
                    )
                    .when(can_close, |d| {
                        d.child(
                            pane_btn(
                                format!("close-{pane_id}"),
                                "x",
                                "Close pane (agent keeps running)",
                                t,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let p = close_id.clone();
                                    this.with_active(|s, id| s.close_pane(id, &p), cx)
                                },
                            )),
                        )
                    }),
            )
    }

    fn empty_pane(
        &self,
        pane_id: &str,
        ctx: &PaneCtx,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let can_close = ctx.pane_count > 1;
        let close_id = pane_id.to_string();
        div()
            .relative()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .p_4()
            .when(can_close, |d| {
                d.child(div().absolute().top(px(4.)).right(px(4.)).child(
                    icon_btn(format!("close-{pane_id}"), "x", "Close pane", true, t).on_click(
                        cx.listener(move |this, _, _, cx| {
                            let p = close_id.clone();
                            this.with_active(|s, id| s.close_pane(id, &p), cx)
                        }),
                    ),
                ))
            })
            .child(label("Empty pane", t.text_2, 12.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .child(if ctx.members.is_empty() {
                        "Drag an agent here from the sidebar."
                    } else {
                        "Drag an agent here from the sidebar, or pick one:"
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(6.))
                    .max_w(px(420.))
                    .children(ctx.members.iter().take(8).map(|m| {
                        let (pane, agent) = (pane_id.to_string(), m.id.clone());
                        div()
                            .id(SharedString::from(format!("pick-{pane_id}-{}", m.id)))
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .h(px(24.))
                            .px(px(8.))
                            .rounded(RADIUS)
                            .border_1()
                            .border_color(t.line_strong)
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover_probed(|s| s.bg(t.surface_3))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.drop_on_pane(&pane, &agent, None, window, cx)
                            }))
                            .child(glyph(m.status, t, true))
                            .child(m.name.clone())
                    })),
            )
    }

    fn chip_strip(
        &self,
        chips: &[AgentView],
        space: &str,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("chip-strip")
            .flex()
            .flex_none()
            .gap(px(6.))
            .px(px(8.))
            .pb(px(8.))
            .overflow_x_scroll()
            .children(chips.iter().map(|a| {
                let elsewhere = self
                    .ui
                    .locate(&a.id)
                    .filter(|l| l.space_id != space)
                    .map(|l| {
                        if l.window == self.label {
                            l.space_name
                        } else {
                            "other window".into()
                        }
                    });
                let tip = match &elsewhere {
                    Some(w) => format!("{} is shown in {w}", a.name),
                    None => format!("Show {}", a.name),
                };
                let id = a.id.clone();
                let eid = gpui::ElementId::Name(SharedString::from(format!("chip-{}", a.id)));
                let pressing = crate::kit::motion::pressing(&eid);
                let blocked = a.status == Status::Blocked;
                let skin = crate::kit::motion::Skin::new(
                    Some(if blocked { t.amber_soft } else { t.surface_3 }),
                    Some(if blocked { t.amber.opacity(0.45) } else { t.line_strong }),
                    px(14.),
                );
                let chip = div()
                    .id(eid.clone())
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .pl(px(9.))
                    .pr(px(11.))
                    .rounded(px(14.))
                    .bg(if t.is_glass() { t.g.surface_2 } else { t.surface_2 })
                    .border_1()
                    .border_color(if t.is_glass() { t.g.line_strong } else { t.line })
                    .text_size(px(12.))
                    .font_weight(crate::kit::WEIGHT_550)
                    .text_color(t.text_2)
                    .cursor_pointer()
                    .map(|d| {
                        if pressing {
                            d.hover_text(t.text, |s| s)
                        } else {
                            d.hover_text(t.text, |s| s.bg(t.surface_3).border_color(t.line_strong))
                        }
                    })
                    .when(blocked, |d| {
                        d.bg(t.amber_soft)
                            .text_color(t.amber)
                            .border_color(t.amber.opacity(0.45))
                            .shadow(t.amber_glow(12., -3.))
                    });
                crate::kit::motion::pressable(chip, &eid, false, skin)
                    .when(matches!(a.status, Status::Exited | Status::Stopped), |d| {
                        d.opacity(0.6)
                    })
                    .tooltip(tooltip(tip))
                    .on_drag(
                        AgentDrag {
                            id: a.id.clone(),
                            name: a.name.clone().into(),
                        },
                        |d, grab, w, cx| d.ghost(grab, w, cx),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.show_agent(&id, window, cx)),
                    )
                    .child(crate::kit::glyph_play(format!("chip-{}", a.id), a.status, t, true))
                    .child(a.name.clone())
                    .when_some(elsewhere, |d, w| {
                        d.child(
                            div()
                                .text_size(px(10.5))
                                .text_color(t.text_4)
                                .child(format!("↗ {w}")),
                        )
                    })
            }))
    }
}

/// A pane header action (`.icon-btn-sm` with a 13 px icon).
fn pane_btn(
    id: String,
    name: &str,
    tip: impl Into<SharedString>,
    t: &Theme,
) -> gpui::Stateful<gpui::Div> {
    // Buttons in the header keep their own clicks: no drag, no
    // double-click maximise from them (`dnd.ts`, `PaneHeader.tsx`).
    crate::kit::icon_btn_state(id, name, tip, true, 13., None, t)
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// See [`MainScreen::fitted`].
pub(super) struct Fitted {
    pub tree: Node,
    pub hidden: Vec<tree::Pane>,
    pub chips: Vec<AgentView>,
    pub pane_count: usize,
    pub presets: Vec<Preset>,
    pub maximized: Option<Node>,
    pub area: Option<(f64, f64)>,
    pub min: tree::MinSize,
    pub density: crate::theme::Density,
}

struct PaneCtx {
    focused: Option<String>,
    maximized: bool,
    pane_count: usize,
    /// Agents offered in empty panes (members not shown).
    members: Vec<AgentView>,
    /// Each pane's size (px) and the density's smallest tile: a split that
    /// would go below it lands as a chip (the drop says "No room").
    sizes: HashMap<String, (f64, f64)>,
    min: tree::MinSize,
}

/// The rect of split `id` inside `rect`.
fn split_rect(node: &Node, rect: tree::Rect, id: &str) -> Option<tree::Rect> {
    match node {
        Node::Pane { .. } => None,
        Node::Split {
            id: sid,
            dir,
            children,
            sizes,
        } => {
            if sid == id {
                return Some(rect);
            }
            let mut offset = 0.;
            for (i, c) in children.iter().enumerate() {
                let f = sizes.get(i).copied().unwrap_or(0.);
                let sub = match dir {
                    Dir::Row => tree::Rect {
                        x: rect.x + offset * rect.w,
                        y: rect.y,
                        w: f * rect.w,
                        h: rect.h,
                    },
                    Dir::Col => tree::Rect {
                        x: rect.x,
                        y: rect.y + offset * rect.h,
                        w: rect.w,
                        h: f * rect.h,
                    },
                };
                if let Some(r) = split_rect(c, sub, id) {
                    return Some(r);
                }
                offset += f;
            }
            None
        }
    }
}

/// The terminal for a pane: `pitwall-term-view` once it is set
/// ([`TerminalSlot`]), else a placeholder naming the agent.
fn terminal_element(
    a: &AgentView,
    focused: bool,
    screen_focused: bool,
    font: f64,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    if let Some(slot) = cx.try_global::<TerminalSlot>().cloned() {
        return (slot.0)(
            &TerminalArgs {
                agent: a,
                focused,
                screen_focused,
                font_size: font,
            },
            window,
            cx,
        );
    }
    let t = crate::theme::theme(cx).clone();
    div()
        .flex_1()
        .flex()
        .flex_col()
        .p(px(10.))
        .gap(px(2.))
        .font_family(crate::kit::MONO_FONT)
        .text_size(px(12.))
        .text_color(t.text_3)
        .child(format!("{}@{} {}", a.name, a.machine.label, a.cwd_display))
        .child(format!(
            "{} · {} · {}×{}",
            a.kind_name,
            status_word(a.status),
            a.cols,
            a.rows
        ))
        .child(
            div()
                .text_color(t.text_4)
                .child("The terminal (pitwall-term-view) is drawn here."),
        )
        .into_any_element()
}

fn stopped_overlay(a: &AgentView, t: &Theme, cx: &mut Context<MainScreen>) -> impl IntoElement {
    let word = if a.status == Status::Stopped {
        "is stopped"
    } else {
        "has exited"
    };
    let resume = a.caps.resume;
    let verb = if resume { "Resume" } else { "Restart" };
    let (label_text, note) = match &a.restart_as {
        Some(k) => (
            format!("{verb} {k}"),
            if resume {
                format!(" Starts the shell and picks up {k}'s previous session in it.")
            } else {
                format!(" Starts the shell and {k} in it.")
            },
        ),
        None => (
            verb.to_string(),
            if resume {
                " Resume picks up the previous session.".to_string()
            } else {
                String::new()
            },
        ),
    };
    let reason = if a.status == Status::Stopped {
        if a.caps.remove_keeps_session {
            format!("Its session on {} isn't running.", a.machine.label)
        } else {
            "Not running since Pitwall restarted.".into()
        }
    } else {
        "The process ended.".into()
    };
    let (restart_id, rm_id) = (a.id.clone(), a.id.clone());
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(t.term_bg.opacity(0.82))
        .child(
            div()
                .max_w(px(380.))
                .p(px(18.))
                .rounded(RADIUS_LG)
                .bg(t.raised)
                .border_1()
                .border_color(t.line_strong)
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{} {word}", a.name)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.text_3)
                        .child(format!("{reason}{note}")),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .when(a.caps.restart, |d| {
                            d.child(
                                button(SharedString::from(format!("restart-{}", a.id)), BtnKind::Primary, false, t)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.restart_agent(restart_id.clone(), cx)
                                    }))
                                    .child(icon("restart", 14., t.bg))
                                    .child(label_text),
                            )
                        })
                        .child(
                            button(SharedString::from(format!("remove-{}", a.id)), BtnKind::Ghost, false, t)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.open_remove(&rm_id, cx)),
                                )
                                .child("Remove"),
                        ),
                ),
        )
}

/// `.drop-indicator`: where a dragged agent lands (`.drop-label` inside).
fn drop_indicator(zone: Option<Side>, has_agent: bool, no_room: bool, t: &Theme) -> impl IntoElement {
    let text = match zone {
        None if has_agent => "Swap".to_string(),
        None => "Show here".to_string(),
        Some(_) if no_room => "No room · adds as a chip".to_string(),
        Some(s) => format!(
            "Split {}",
            match s {
                Side::Left => "left",
                Side::Right => "right",
                Side::Top => "top",
                Side::Bottom => "bottom",
            }
        ),
    };
    // Insets as in space.css: 38 px from the top (under the pane head),
    // 6 px elsewhere; an empty pane's whole area. The edges are fractions of
    // that box and ease between zones (`transition: all 0.08s ease-out`).
    let (l, tp, r, b) = match zone {
        None => (0., 0., 0., 0.),
        Some(Side::Left) => (0., 0., 0.5, 0.),
        Some(Side::Right) => (0.5, 0., 0., 0.),
        Some(Side::Top) => (0., 0., 0., 0.5),
        Some(Side::Bottom) => (0., 0.5, 0., 0.),
    };
    let tint = if no_room { t.amber } else { t.text };
    let (bg, ring) = (tint.opacity(0.10), tint.opacity(if no_room { 0.55 } else { 0.45 }));
    let label = div()
        .px(px(9.))
        .py(px(3.))
        .rounded(px(5.))
        .bg(t.surface_3)
        .text_color(t.text)
        // `.drop-label`: Inter 600, 11 px, upper case, 0.04em.
        .child(crate::kit::tracked(&text.to_uppercase(), 11., 0.04).font_weight(FontWeight::SEMIBOLD));
    div()
        .absolute()
        .top(px(if has_agent { 38. } else { 6. }))
        .left(px(6.))
        .right(px(6.))
        .bottom(px(6.))
        .child(crate::kit::motion::tween_many(
            "drop-zone",
            vec![l, tp, r, b],
            std::time::Duration::from_millis(80),
            crate::kit::motion::Ease::EaseOut,
            move |e| {
                div()
                    .absolute()
                    .left(gpui::relative(e[0]))
                    .top(gpui::relative(e[1]))
                    .right(gpui::relative(e[2]))
                    .bottom(gpui::relative(e[3]))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .bg(bg)
                    .border_2()
                    .border_color(ring)
                    .child(label)
                    .into_any_element()
            },
        ))
}


/// A preset's mini layout (`PresetIcon`), drawn from its pane rects.
fn preset_icon(p: Preset, ink: Hsla, hover: Hsla, group: SharedString) -> impl IntoElement {
    let (w, h) = (17., 13.);
    let rects: Vec<tree::Rect> = if p == Preset::Auto {
        let mut v = vec![];
        for x in [1., 5.5, 10.] {
            for y in [1., 5.] {
                v.push(tree::Rect {
                    x,
                    y,
                    w: 3.5,
                    h: 3.,
                });
            }
        }
        v.push(tree::Rect {
            x: 1.,
            y: 9.,
            w: 16.,
            h: 4.,
        });
        v
    } else {
        tree::pane_rects(
            &tree::build_preset(p, &[]),
            tree::Rect { x: 0., y: 0., w, h },
        )
        .into_iter()
        .map(|(_, r)| tree::Rect {
            x: r.x + 1.,
            y: r.y + 1.,
            w: (r.w - 1.).max(0.5),
            h: (r.h - 1.).max(0.5),
        })
        .collect()
    };
    div()
        .relative()
        .w(px(18.))
        .h(px(14.))
        .children(rects.into_iter().enumerate().map(move |(i, r)| {
            div()
                .absolute()
                .left(px(r.x as f32))
                .top(px(r.y as f32))
                .w(px(r.w as f32))
                .h(px(r.h as f32))
                .rounded(px(0.8))
                .bg(ink)
                .group_hover_probed(group.clone(), |s| s.bg(hover))
                .when(p == Preset::Auto && i == 6, |d| d.opacity(0.45))
        }))
}

fn empty_state(t: &Theme, cx: &mut Context<MainScreen>) -> impl IntoElement {
    div()
        .m_auto()
        .max_w(px(440.))
        .p(px(32.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(14.))
        .child(
            div()
                .flex()
                .gap(px(4.))
                .p(px(6.))
                .rounded(px(6.))
                .bg(t.surface_2)
                .border_1()
                .border_color(t.line_strong)
                .children(["P1", "—", "BOX"].into_iter().enumerate().map(|(i, s)| {
                    div()
                        .px(px(9.))
                        .py(px(3.))
                        .rounded(px(3.))
                        .bg(t.bg)
                        .text_size(px(14.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(if i == 2 { t.amber } else { t.text_2 })
                        .child(s)
                })),
        )
        .child(label("The pit wall is quiet", t.text, 24.))
        .child(
            div()
                .text_center()
                .text_color(t.text_3)
                .line_height(px(20.))
                .child(
                    "Start a coding agent in one of your projects. Pitwall shows its terminal, what it changed, and tells you when it needs you.",
                ),
        )
        .child(
            button("empty-new-agent", BtnKind::Primary, false, t)
                .on_click(cx.listener(|this, _, window, cx| this.open_new_agent(None, window, cx)))
                .child("+ New agent")
                .child(kbd("⌘N", t)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(t.text_4)
                .child(kbd("⌘K", t))
                .child("opens the command palette any time."),
        )
}

fn route_placeholder(route: Route, t: &Theme, cx: &mut Context<MainScreen>) -> impl IntoElement {
    let (title, what) = match route {
        Route::Wall => ("Wall", "every agent, live · view only"),
        Route::Review => ("Review", "what agents changed"),
        Route::Explorer => ("Files", "read-only file viewer"),
        Route::Space => ("", ""),
    };
    div()
        .flex_1()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(32.))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .border_b_1()
                .border_color(t.line)
                .child(label(title, t.text, 12.))
                .child(div().text_size(px(11.5)).text_color(t.text_3).child(what))
                .child(div().flex_1())
                .child(
                    small_btn("route-back", "Back", t)
                        .on_click(cx.listener(|this, _, _, cx| this.set_route(Route::Space, cx)))
                        .child(kbd("esc", t)),
                ),
        )
        .child(div().m_auto().text_color(t.text_3).child(format!(
            "{title} is drawn by its own module (not registered in this build)."
        )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_rects_are_found() {
        let t = tree::build_preset(Preset::Three, &[]);
        let Node::Split { children, .. } = &t else {
            panic!()
        };
        let col = children[1].id().to_string();
        let r = split_rect(
            &t,
            tree::Rect {
                x: 0.,
                y: 0.,
                w: 1000.,
                h: 500.,
            },
            &col,
        )
        .unwrap();
        assert!((r.x - 600.).abs() < 1e-6 && (r.w - 400.).abs() < 1e-6);
    }
}
