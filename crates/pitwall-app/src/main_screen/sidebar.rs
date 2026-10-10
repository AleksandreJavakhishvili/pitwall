//! The sidebar (React `Sidebar.tsx`, `AgentRow.tsx`, `RailItem.tsx`):
//! agents grouped by machine and project, needs-you first; collapsible
//! groups with a summary; project "+" and right-click menus; agent context
//! menu; ⌘1–9 hints; drag sources; ↑/↓/⏎ when it has keyboard focus; an
//! icon rail on narrow windows.

use crate::kit::Ellipsis as _;
use crate::kit::HoverText;
use gpui::{
    div, point, prelude::*, px, AnyElement, Context, FontWeight, IntoElement, MouseButton,
    MouseDownEvent, SharedString, Window,
};

use pitwall_proto::{AgentView, Status};

use crate::kit::{icon, MONO_FONT};
use super::menu::MenuItem;
use super::model::{initials, machine_heading, summarize, with_projects};
use crate::kit::keys;
use super::topbar::AgentDrag;
use crate::kit::{
    chevron, chip, diffstat, icon_btn, kbd, label_fit, status_label, tooltip,
};
use super::{Confirm, MainScreen, SelectNext, SelectPrev, SidebarMode};
use crate::agents::{group_by_project, status_word, ProjectGroup};
use crate::theme::{Theme, RADIUS, SIDEBAR_W};

impl MainScreen {
    fn sidebar_groups(&self, agents: &[AgentView]) -> Vec<ProjectGroup> {
        with_projects(group_by_project(agents), &self.projects)
    }

    /// Agents in sidebar order, skipping collapsed groups (keyboard moves).
    fn visible_rows(&self, agents: &[AgentView]) -> Vec<String> {
        self.sidebar_groups(agents)
            .into_iter()
            .filter(|g| !self.ui.collapsed.contains(&g.key))
            .flat_map(|g| g.agents.into_iter().map(|a| a.id))
            .collect()
    }

    fn move_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.visible_rows(self.agents(cx));
        if rows.is_empty() {
            return;
        }
        let current = self.cursor.clone().or_else(|| self.focused_agent_id());
        let at = current.and_then(|c| rows.iter().position(|r| *r == c));
        let next = match at {
            Some(i) => (i as isize + delta).rem_euclid(rows.len() as isize) as usize,
            None => 0,
        };
        self.cursor = Some(rows[next].clone());
        cx.notify();
    }

    pub(super) fn render_sidebar(
        &mut self,
        mode: SidebarMode,
        agents: &[AgentView],
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if mode == SidebarMode::Rail {
            return self.render_rail(agents, t, cx).into_any_element();
        }
        let groups = self.sidebar_groups(agents);
        let focused = self.focused_agent_id();
        let kb = self.sidebar_focus.is_focused(window);
        let mut index = 0usize;
        let mut list: Vec<AnyElement> = Vec::new();
        for (gi, g) in groups.iter().enumerate() {
            let collapsed = self.ui.collapsed.contains(&g.key);
            let start = index;
            index += g.agents.len();
            // `.project-group + .project-group { margin-top: 4px }`
            if gi > 0 {
                list.push(div().h(px(4.)).flex_none().into_any_element());
            }
            if let Some(h) = machine_heading(&groups, gi) {
                list.push(
                    div()
                        .px(px(14.))
                        .pt(px(10.))
                        .pb(px(2.))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text_4)
                        .child(h.to_uppercase())
                        .into_any_element(),
                );
            }
            list.push(self.project_head(g, collapsed, t, cx).into_any_element());
            if g.agents.is_empty() {
                if !collapsed {
                    let project = g.project.clone();
                    // `.project-empty`: one quiet dashed row where an agent
                    // row would be; the whole row opens New agent.
                    let group = SharedString::from(format!("project-empty-{}", g.key));
                    list.push(
                        div()
                            .id(SharedString::from(format!("empty-{}", g.key)))
                            .group(group.clone())
                            .mx(px(6.))
                            .mb(px(4.))
                            .h(px(32.))
                            .px(px(9.))
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .rounded(RADIUS)
                            .border_1()
                            .border_dashed()
                            .border_color(t.line_strong)
                            .text_size(px(12.5))
                            .text_color(t.text_3)
                            .cursor_pointer()
                            .hover_text(t.text, |s| s.border_color(t.text_4).bg(t.surface_2))
                            .tooltip(tooltip("New agent in this project"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_new_agent(Some(project.clone()), window, cx)
                            }))
                            .child(icon("plus", 12., t.text_3).group_hover_text(group, t.text, |s| s))
                            .child("New agent")
                            .child(
                                div()
                                    .ml_auto()
                                    .text_size(px(11.5))
                                    .text_color(t.text_4)
                                    .child("No agents yet"),
                            )
                            .into_any_element(),
                    );
                }
            } else if collapsed {
                list.push(
                    div()
                        .pl(px(32.))
                        .pr(px(14.))
                        .pb(px(6.))
                        .text_size(px(12.))
                        .text_color(t.text_3)
                        .child(summarize(&g.agents))
                        .into_any_element(),
                );
            } else {
                for (i, a) in g.agents.iter().enumerate() {
                    let selected = focused.as_deref() == Some(a.id.as_str());
                    let cursor = kb && self.cursor.as_deref() == Some(a.id.as_str());
                    list.push(
                        self.agent_row(a, start + i, selected, cursor, g.can_create, t, cx)
                            .into_any_element(),
                    );
                    list.extend(self.wt_agent(a, t, cx));
                }
                list.extend(self.wt_others(&g.key, &g.agents, t, cx));
            }
        }
        let elsewhere: Vec<AnyElement> = self.render_elsewhere(t, cx).into_iter().collect();
        if !groups.is_empty() && !elsewhere.is_empty() {
            list.push(div().h(px(4.)).flex_none().into_any_element());
        }
        list.extend(elsewhere);

        div()
            .id("sidebar")
            .key_context("PwSidebar")
            .track_focus(&self.sidebar_focus)
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_cursor(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_cursor(1, cx)))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| {
                if let Some(id) = this.cursor.clone() {
                    this.show_agent(&id, window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| window.focus(&this.sidebar_focus)),
            )
            .w(SIDEBAR_W)
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(t.surface)
            // In a floating glass slab: its rounded shape (GPUI clips square).
            .when(crate::theme::glass_regions(cx), |d| {
                d.rounded(px(crate::kit::PANEL_RADIUS))
            })
            .border_r_1()
            .border_color(t.line)
            .child(
                crate::kit::scroll_area("sidebar-scroll-area", t, move |h| {
                    div()
                        .id("sidebar-scroll")
                        .track_scroll(h)
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .pt(px(6.))
                        .pb(px(8.))
                        .children(list)
                        .into_any_element()
                })
                .fill()
                .flex_1()
                .min_h_0(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pr(px(8.))
                    .child(
                        div()
                            .id("new-agent-btn")
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .m(px(8.))
                            .mr_0()
                            .px(px(10.))
                            .py(px(8.))
                            .rounded(RADIUS)
                            .border_1()
                            .border_dashed()
                            .border_color(t.line_strong)
                            .text_color(t.text_2)
                            .font_weight(crate::kit::WEIGHT_550)
                            .cursor_pointer()
                            .hover_text(t.text, |s| s.border_color(t.text_4).bg(t.surface_2))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_new_agent(None, window, cx)
                            }))
                            .child(icon("plus", 16., t.text_2))
                            .child(div().flex_1().child("New agent"))
                            .child(kbd("⌘N", t)),
                    )
                    // `.new-terminal-btn`: 36 px, dashed, text-2.
                    .child({
                        let (fg, bg, edge) = (t.text, t.surface_2, t.text_4);
                        div()
                            .id("new-terminal-btn")
                            .group("new-terminal-btn")
                            .flex_none()
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(RADIUS)
                            .border_1()
                            .border_dashed()
                            .border_color(t.line_strong)
                            .cursor_pointer()
                            .hover_probed(move |s| s.bg(bg).border_color(edge))
                            .tooltip(tooltip(keys(
                                "New terminal here (⌘T) · ⌘⇧T to choose a folder",
                                false,
                            )))
                            .on_click(cx.listener(|this, _, window, cx| {
                                let here = this.here(cx);
                                this.open_terminal(here, window, cx)
                            }))
                            .child(
                                icon("terminal", 16., t.text_2).group_hover_text(
                                    "new-terminal-btn",
                                    fg,
                                    |s| s,
                                ),
                            )
                    }),
            )
            .into_any_element()
    }

    fn project_head(
        &self,
        g: &ProjectGroup,
        collapsed: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let key = g.key.clone();
        let title: SharedString = match (&g.machine, g.can_create) {
            (Some(m), false) => format!("{} on {}", g.project, m.label).into(),
            _ => g.project.clone().into(),
        };
        let (p_menu, p_click, p_space, d_space, p_remove) = (
            g.project.clone(),
            g.project.clone(),
            g.project.clone(),
            g.display.clone(),
            g.project.clone(),
        );
        let can_create = g.can_create;
        div()
            .id(SharedString::from(format!("group-{}", g.key)))
            .group("project-head")
            .flex()
            .items_center()
            .pl(px(8.))
            .pr(px(6.))
            .pt(px(6.))
            .pb(px(2.))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    if can_create {
                        this.project_menu(&p_menu, e.position, window, cx);
                    }
                }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("toggle-{}", g.key)))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(4.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .tooltip(tooltip(title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let k = key.clone();
                        this.update_ui(|s| s.toggle_collapsed(&k), cx);
                    }))
                    .child(chevron("chev", !collapsed, 12., t.text_4))
                    .child(
                        div().min_w_0().overflow_hidden().child(
                            label_fit(
                                crate::kit::project_name(&g.display).to_string(),
                                t.text_3,
                                12.,
                                22,
                            )
                                .group_hover_text("project-head", t.text_2, |s| s),
                        ),
                    )
                    .when(g.blocked > 0, |d| {
                        d.child(
                            crate::kit::text_glow(
                                div()
                                    .font_family(MONO_FONT)
                                    .text_size(px(10.5))
                                    .text_color(t.amber)
                                    .child(format!("▲ {}", g.blocked)),
                                t.amber_glow,
                                8.,
                            )
                            .flex_none(),
                        )
                    })
                    .child(
                        div()
                            .flex_none()
                            .font_family(MONO_FONT)
                            .text_size(px(11.))
                            .text_color(t.text_3)
                            .child(g.agents.len().to_string()),
                    ),
            )
            .when(g.can_create, |d| {
                d.child(
                    div()
                        .invisible()
                        .group_hover_probed("project-head", |s| s.visible())
                        .child(
                            icon_btn(
                                format!("add-{}", g.key),
                                "plus",
                                "New terminal or agent here",
                                true,
                                t,
                            )
                            .on_click(cx.listener(
                                move |this, e: &gpui::ClickEvent, window, cx| {
                                    let at = e.position();
                                    this.project_menu(
                                        &p_click,
                                        point(at.x, at.y + px(4.)),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        ),
                )
            })
            .map(|d| {
                if g.agents.is_empty() {
                    d.child(
                        icon_btn(
                            format!("rm-{}", g.key),
                            "x",
                            "Remove from Pitwall (files are not touched)",
                            true,
                            t,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_project(p_remove.clone(), cx)
                        })),
                    )
                } else {
                    d.child(
                        div()
                            .invisible()
                            .group_hover_probed("project-head", |s| s.visible())
                            .child(
                                icon_btn(
                                    format!("space-{}", g.key),
                                    "grid",
                                    "Open as space",
                                    true,
                                    t,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.open_project_space(&p_space, &d_space, cx)
                                    },
                                )),
                            ),
                    )
                }
            })
    }

    fn project_menu(
        &mut self,
        project: &str,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (p1, p2) = (project.to_string(), project.to_string());
        self.open_menu(
            at,
            vec![
                MenuItem::new("terminal", "New terminal here", move |this, window, cx| {
                    this.open_terminal(p1.clone(), window, cx)
                }),
                MenuItem::new("plus", "New agent…", move |this, window, cx| {
                    this.open_new_agent(Some(p2.clone()), window, cx)
                }),
            ],
            window,
            cx,
        );
    }

    fn agent_menu(
        &mut self,
        a: &AgentView,
        can_create: bool,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut items = Vec::new();
        if can_create {
            let cwd = a.cwd.clone();
            items.push(MenuItem::new(
                "terminal",
                "Open terminal here",
                move |this, window, cx| this.open_terminal(cwd.clone(), window, cx),
            ));
        }
        let id = a.id.clone();
        items.push(MenuItem::new(
            "trash",
            "Remove agent…",
            move |this, _, cx| this.open_remove(&id, cx),
        ));
        self.open_menu(at, items, window, cx);
    }

    /// Demo only: the agent menu next to the sidebar.
    pub(super) fn demo_agent_menu(
        &mut self,
        a: &AgentView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.agent_menu(a, true, point(px(150.), px(120.)), window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn agent_row(
        &self,
        a: &AgentView,
        index: usize,
        selected: bool,
        cursor: bool,
        can_create: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let where_: Option<String> = self.ui.locate(&a.id).map(|l| {
            if l.window != self.label {
                "other window".into()
            } else {
                l.space_name
            }
        });
        let mut tip = a.name.clone();
        if let Some(w) = &where_ {
            tip.push_str(&format!(" — in {w}"));
        }
        if let Some(d) = &a.status_detail {
            tip.push('\n');
            tip.push_str(d);
        }
        let blocked = a.status == Status::Blocked;
        let dim = matches!(a.status, Status::Exited | Status::Stopped);
        let (id_click, id_menu) = (a.id.clone(), a.clone());
        let drag = AgentDrag {
            id: a.id.clone(),
            name: a.name.clone().into(),
        };
        div()
            .id(SharedString::from(format!("row-{}", a.id)))
            .group("agent-row")
            .relative()
            .mx(px(6.))
            .flex()
            .items_start()
            .gap(px(9.))
            .pl(px(9.))
            .pr(px(if a.status == Status::Done { 14. } else { 8. }))
            .py(px(7.))
            .rounded(RADIUS)
            .cursor_pointer()
            .when(dim, |d| d.opacity(0.6))
            .map(|d| {
                if blocked {
                    d.bg(t.amber_soft)
                } else if selected {
                    d.bg(t.surface_3)
                } else {
                    d.hover_probed(|s| s.bg(t.surface_2))
                }
            })
            .when(cursor, |d| d.border_1().border_color(t.focus))
            .tooltip(tooltip(tip))
            .on_drag(drag, |d, grab, w, cx| d.ghost(grab, w, cx))
            .on_click(
                cx.listener(move |this, _, window, cx| this.show_agent(&id_click, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    this.agent_menu(&id_menu, can_create, e.position, window, cx)
                }),
            )
            // The left edge: amber when blocked, a light bar when selected.
            .when(blocked || selected, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(if blocked { 6. } else { 8. }))
                        .bottom(px(if blocked { 6. } else { 8. }))
                        .w(px(if blocked { 3. } else { 2. }))
                        .rounded(px(2.))
                        .bg(if blocked { t.amber } else { t.text_2 }),
                )
            })
            // `inset 0 0 0 1px` amber: a ring that takes no room.
            .when(blocked, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(RADIUS)
                        .border_1()
                        .border_color(t.amber.opacity(0.3)),
                )
            })
            // "Needs you": a halo twice (`m-halo`), then the steady amber.
            .when(blocked, |d| {
                let (amber, glow) = (t.amber.opacity(0.6), t.amber_glow);
                d.child(crate::kit::motion::keyframes(
                    SharedString::from(format!("halo-{}", a.id)),
                    crate::kit::motion::HALO,
                    2,
                    move |p| {
                        let o = if p >= 1. { 0. } else { crate::kit::motion::halo(p) };
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(RADIUS)
                            .border_1()
                            .border_color(amber)
                            .shadow(vec![gpui::BoxShadow {
                                color: glow,
                                offset: gpui::point(px(0.), px(0.)),
                                blur_radius: px(18.),
                                spread_radius: px(-4.),
                            }])
                            .opacity(o)
                            .into_any_element()
                    },
                ))
            })
            // The chequered edge of "done", popping in (`m-flag`).
            .when(a.status == Status::Done, |d| {
                let flag = t.flag;
                d.child(
                    div()
                        .absolute()
                        .right_0()
                        .top(px(7.))
                        .bottom(px(7.))
                        .w(px(6.))
                        .child(crate::kit::motion::keyframes(
                            SharedString::from(format!("flag-{}", a.id)),
                            crate::kit::motion::FLAG_ROW,
                            1,
                            move |p| {
                                let (o, s) = crate::kit::motion::flag_pop(p);
                                checker(flag, s).opacity(0.55 * o).into_any_element()
                            },
                        )),
                )
            })
            .child(div().mt(px(1.)).child(crate::kit::glyph_play(format!("row-{}", a.id), a.status, t, false)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .ellipsis()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(13.))
                                    .text_color(t.text)
                                    .child(crate::kit::one_line(a.name.clone())),
                            )
                            .when(a.engineer, |d| d.child(chip("engineer", t)))
                            .child(status_label(a.status, t)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .text_size(px(11.5))
                            .text_color(t.text_3)
                            .min_w_0()
                            .child(chip(a.location.clone(), t))
                            .child(div().flex_1().min_w_0().ellipsis().child(crate::kit::one_line(
                                if a.agent_in_terminal {
                                    format!("{} · in terminal", a.kind_name)
                                } else {
                                    a.kind_name.clone()
                                },
                            )))
                            .child(
                                div()
                                    .group_hover_probed("agent-row", |s| s.invisible())
                                    .child(diffstat(a.added, a.removed, t)),
                            ),
                    )
                    .when(blocked, |d| {
                        d.children(a.status_detail.clone().map(|det| {
                            div()
                                .ellipsis()
                                .text_size(px(11.))
                                .font_family(crate::kit::MONO_FONT)
                                .text_color(t.amber)
                                .child(crate::kit::one_line(det))
                        }))
                    }),
            )
            .when(index < 9, |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(8.))
                        .bottom(px(7.))
                        .text_size(px(10.))
                        .text_color(t.text_4)
                        .invisible()
                        .group_hover_probed("agent-row", |s| s.visible())
                        .child(keys(&format!("⌘{}", index + 1), true)),
                )
            })
    }

    fn render_rail(
        &mut self,
        agents: &[AgentView],
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let groups = group_by_project(agents);
        let focused = self.focused_agent_id();
        let rail = div()
            .id("rail")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .py(px(8.))
            .gap(px(2.))
            .overflow_y_scroll()
            .children(groups.iter().enumerate().map(|(gi, g)| {
                let title: SharedString = match (&g.machine, g.can_create) {
                    (Some(m), false) => format!("{} · {}", g.display, m.label).into(),
                    _ => g.display.clone().into(),
                };
                div()
                    .id(SharedString::from(format!("rail-g-{}", g.key)))
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .py(px(6.))
                    .when(gi > 0, |d| d.border_t_1().border_color(t.line))
                    .tooltip(tooltip(title))
                    .children(g.agents.iter().map(|a| {
                        let id = a.id.clone();
                        let on = focused.as_deref() == Some(a.id.as_str());
                        let blocked = a.status == Status::Blocked;
                        let tip = format!(
                            "{} · {}{}",
                            a.name,
                            status_word(a.status),
                            a.status_detail
                                .as_ref()
                                .map(|d| format!(": {d}"))
                                .unwrap_or_default()
                        );
                        div()
                            .id(SharedString::from(format!("rail-{}", a.id)))
                            .relative()
                            .size(px(36.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(if blocked { t.amber_soft } else { t.surface_2 })
                            .text_color(if blocked {
                                t.amber
                            } else if on {
                                t.text
                            } else {
                                t.text_2
                            })
                            .when(on, |d| d.border_1().border_color(t.text_3))
                            .when(matches!(a.status, Status::Exited | Status::Stopped), |d| {
                                d.opacity(0.55)
                            })
                            .cursor_pointer()
                            .hover_text(t.text, |s| s.bg(t.surface_3))
                            .tooltip(tooltip(tip))
                            .on_drag(
                                AgentDrag {
                                    id: a.id.clone(),
                                    name: a.name.clone().into(),
                                },
                                |d, grab, w, cx| d.ghost(grab, w, cx),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_agent(&id, window, cx)
                            }))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(initials(&a.name)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .right(px(-2.))
                                    .top(px(-2.))
                                    .size(px(9.))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(t.surface)
                                    .bg(match a.status {
                                        Status::Working => t.green,
                                        Status::Blocked => t.amber,
                                        Status::Done => t.flag,
                                        Status::Idle => t.text_3,
                                        _ => t.text_4,
                                    }),
                            )
                    }))
            }))
            .child(
                div()
                    .id("rail-new")
                    .mt(px(8.))
                    .size(px(36.))
                    .rounded(px(8.))
                    .border_1()
                    .border_dashed()
                    .border_color(t.line_strong)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(t.text_3)
                    .cursor_pointer()
                    .hover_text(t.text, |s| s)
                    .tooltip(tooltip(keys("New agent (⌘N)", false)))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_new_agent(None, window, cx)),
                    )
                    .child(icon("plus", 16., t.text_3)),
            );
        crate::kit::scroll_area("rail-area", t, move |h| rail.track_scroll(h).into_any_element())
            .fill()
            .w(px(56.))
            .h_full()
            .flex_none()
            .bg(t.surface)
            .border_r_1()
            .border_color(t.line)
    }
}

/// The chequered edge of a "done" row (`repeating-conic-gradient` of 3 px
/// squares), filling its box; `scale` shrinks it about the centre.
fn checker(color: gpui::Hsla, scale: f32) -> gpui::Div {
    div().size_full().child(
        gpui::canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                let cell = 3. * scale;
                let (w, h) = (f32::from(b.size.width) * scale, f32::from(b.size.height) * scale);
                let c = b.center();
                let (x0, y0) = (f32::from(c.x) - w / 2., f32::from(c.y) - h / 2.);
                let rows = (h / cell).ceil() as usize;
                for r in 0..rows {
                    for k in 0..2 {
                        if (r + k) % 2 == 0 {
                            let y = y0 + r as f32 * cell;
                            let hh = cell.min(y0 + h - y);
                            window.paint_quad(gpui::fill(
                                gpui::Bounds::new(
                                    gpui::point(px(x0 + k as f32 * cell), px(y)),
                                    gpui::size(px(cell), px(hh)),
                                ),
                                color,
                            ));
                        }
                    }
                }
            },
        )
        .size_full(),
    )
}
