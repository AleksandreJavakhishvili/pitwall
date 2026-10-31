//! The sidebar (React `Sidebar.tsx`, `AgentRow.tsx`, `RailItem.tsx`):
//! agents grouped by machine, then project (or by machine, account, then
//! project: Settings → General → Group agents by; the machine level only
//! with 2+ machines or another machine's Pitwall), in the order the user
//! drags them
//! (accounts, projects, and agents within a project); collapsible groups
//! with a summary; project "+" and right-click menus; agent context menu;
//! ⌘1–9 hints; drag sources; ↑/↓/⏎ when it has keyboard focus; an icon
//! rail on narrow windows.
//!
//! Grouped by account, dropping an agent or a project on another account
//! moves it there: its agents restart on that login at once, conversations
//! with them (as the agent menu's "Run on account" and a project's
//! Accounts…). Drags stay within one machine; another machine's Pitwall
//! is read-only (nothing is dragged in, out or within it).

use crate::kit::Ellipsis as _;
use crate::kit::HoverText;
use gpui::{
    div, point, prelude::*, px, AnyElement, App, Context, Div, DragMoveEvent, Entity,
    Focusable as _, FontWeight, IntoElement, MouseButton, MouseDownEvent, SharedString, Stateful,
    Window,
};

use pitwall_proto::{AgentView, Status};

use crate::kit::{icon, InputEvent, TextInput, MONO_FONT};
use super::menu::MenuItem;
use super::model::{
    base_key, by_machine, by_machine_account, in_order, initials, machine_level, reorder, section_of, summarize, with_paired, with_projects, AccountSection,
    MachineSection,
};
use super::workspace::SidebarGroup;
use crate::kit::keys;
use super::topbar::{AgentDrag, AgentGhost};
use crate::kit::{
    chevron, chip, diffstat, icon_btn, kbd, label_fit, status_label, tooltip,
};
use super::strip::Toast;
use super::{Confirm, MainScreen, Move, SelectNext, SelectPrev, SidebarMode};
use crate::agents::{group_by_project, status_word, ProjectGroup};
use crate::theme::{Theme, RADIUS, SIDEBAR_W};

/// An agent's name being edited in its sidebar row, and why the last try
/// failed.
pub(super) struct AgentRenaming {
    pub(super) id: String,
    pub(super) input: Entity<TextInput>,
    pub(super) error: Option<String>,
    _sub: gpui::Subscription,
}

impl MainScreen {
    fn sidebar_groups(&self, agents: &[AgentView]) -> Vec<ProjectGroup> {
        in_order(with_projects(group_by_project(agents), &self.projects), &self.ui.sidebar_order)
    }

    /// The sidebar's machines: their projects, or their accounts (in
    /// Settings → Agents → Accounts' order) and their projects, as dragged;
    /// paired machines with nothing to show too.
    pub(super) fn machine_sections(&self, agents: &[AgentView], cx: &App) -> Vec<MachineSection> {
        let sections = match self.ui.sidebar_group {
            SidebarGroup::Project => by_machine(self.sidebar_groups(agents)),
            SidebarGroup::Account => {
                let mut sections = by_machine_account(agents, &self.projects, &crate::accounts::ids(cx));
                for s in sections.iter_mut().flat_map(|m| m.accounts.iter_mut()) {
                    s.groups = in_order(std::mem::take(&mut s.groups), &self.ui.sidebar_order);
                }
                sections
            }
        };
        let paired: Vec<_> = crate::peers::model(cx)
            .map(|m| m.read(cx).entries.iter().filter_map(|e| Some((e.paired.id.clone(), e.paired.label.clone(), e.link.clone()?))).collect())
            .unwrap_or_default();
        with_paired(sections, &paired)
    }

    /// Every project group in sidebar order, folded ones too (⌘1–9).
    pub(super) fn sidebar_list(&self, agents: &[AgentView], cx: &App) -> Vec<ProjectGroup> {
        self.machine_sections(agents, cx)
            .into_iter()
            .flat_map(|m| m.groups.into_iter().chain(m.accounts.into_iter().flat_map(|s| s.groups)))
            .collect()
    }

    /// Agents in sidebar order, skipping collapsed groups (keyboard moves).
    pub(super) fn visible_rows(&self, agents: &[AgentView], cx: &App) -> Vec<String> {
        let sections = self.machine_sections(agents, cx);
        let level = machine_level(&sections);
        let open = |k: &String| !self.ui.collapsed.contains(k);
        sections
            .into_iter()
            .filter(|m| !level || open(&m.key))
            .flat_map(|m| m.groups.into_iter().chain(m.accounts.into_iter().filter(|s| open(&s.key)).flat_map(|s| s.groups)))
            .filter(|g| open(&g.key))
            .flat_map(|g| g.agents.into_iter().map(|a| a.id))
            .collect()
    }

    fn move_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.visible_rows(self.agents(cx), cx);
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
        let focused = self.focused_agent_id();
        let kb = self.sidebar_focus.is_focused(window);
        let mut index = 0usize;
        let mut list: Vec<AnyElement> = Vec::new();
        let sections = self.machine_sections(agents, cx);
        // This Mac alone: no machine level, as before there were others.
        let level = machine_level(&sections);
        for (mi, m) in sections.iter().enumerate() {
            if level {
                let collapsed = self.ui.collapsed.contains(&m.key);
                if mi > 0 {
                    list.push(div().h(px(8.)).flex_none().into_any_element());
                }
                list.push(self.machine_head(m, collapsed, t, cx));
                let all = m.agents();
                let quiet = if collapsed {
                    index += all.len();
                    (!all.is_empty()).then(|| summarize(&all))
                } else if m.groups.is_empty() && m.accounts.is_empty() {
                    // A paired machine with nothing to show.
                    let connected = m.link.as_ref().is_some_and(|l| l.state == pitwall_proto::peer::LinkState::Connected);
                    Some(if connected { "No agents running" } else { "Its agents show once it's reachable" }.to_string())
                } else {
                    None
                };
                if let Some(q) = quiet {
                    list.push(div().pl(px(32.)).pr(px(14.)).pb(px(6.)).text_size(px(12.)).text_color(t.text_4).child(q).into_any_element());
                }
                if collapsed {
                    continue;
                }
            }
            list.extend(self.render_groups(&m.groups, &mut index, &focused, kb, t, cx));
            for (si, sec) in m.accounts.iter().enumerate() {
                let collapsed = self.ui.collapsed.contains(&sec.key);
                if si > 0 {
                    list.push(div().h(px(6.)).flex_none().into_any_element());
                }
                list.push(self.account_head(sec, collapsed, m.home, t, cx).into_any_element());
                if collapsed {
                    index += sec.agents;
                } else {
                    // Its projects sit a step in, under the account.
                    let rows = self.render_groups(&sec.groups, &mut index, &focused, kb, t, cx);
                    list.push(div().pl(px(10.)).flex().flex_col().children(rows).into_any_element());
                }
            }
        }
        let any = !sections.is_empty();
        let mut elsewhere: Vec<AnyElement> = self.render_elsewhere(t, cx).into_iter().collect();
        elsewhere.extend(self.render_peers(t, cx));
        if any && !elsewhere.is_empty() {
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

    /// Project groups and their agent rows (`index`: rows so far).
    fn render_groups(
        &mut self,
        groups: &[ProjectGroup],
        index: &mut usize,
        focused: &Option<String>,
        kb: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut list: Vec<AnyElement> = Vec::new();
        for (gi, g) in groups.iter().enumerate() {
            let collapsed = self.ui.collapsed.contains(&g.key);
            let start = *index;
            *index += g.agents.len();
            // `.project-group + .project-group { margin-top: 4px }`
            if gi > 0 {
                list.push(div().h(px(4.)).flex_none().into_any_element());
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
                        self.agent_row(a, &g.key, start + i, selected, cursor, g.can_create, t, cx)
                            .into_any_element(),
                    );
                    list.extend(self.wt_agent(a, t, cx));
                }
                list.extend(self.wt_others(&g.key, &g.agents, t, cx));
            }
        }
        list
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
        let spot = Spot::Group {
            key: g.key.clone(),
            block: block(g),
            account: section_of(&g.key).map(String::from),
        };
        let drag = ProjectDrag {
            key: g.key.clone(),
            project: g.project.clone(),
            name: crate::kit::project_name(&g.display).to_string().into(),
            block: block(g),
            account: section_of(&g.key).map(String::from),
        };
        let mark = self.side_mark(&spot.key(), t, cx);
        // Another machine's Pitwall: read-only, nothing is dragged.
        let ro = g.machine.as_ref().is_some_and(|m| m.link.is_some());
        let el = div()
            .id(SharedString::from(format!("group-{}", g.key)))
            .group("project-head")
            .relative()
            .when(!ro, |d| d.on_drag(drag, |d, grab, _, cx| cx.new(|_| AgentGhost::named(d.name.clone(), grab))))
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
            });
        if ro {
            return el;
        }
        self.drop_spot(el, spot, cx).when_some(mark, |d, m| d.child(m))
    }

    /// A machine's heading (2+ machines): an icon with how it's doing,
    /// its name and what it is (this Mac, an agw machine, or another
    /// machine's Pitwall: how the link is, view only), needs-you and agent
    /// counts; folds what is on it.
    fn machine_head(&self, m: &MachineSection, collapsed: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let key = m.key.clone();
        let (agents, blocked) = (m.agents().len(), m.blocked());
        div()
            .id(SharedString::from(format!("machine-{}", m.key)))
            .group("machine-head")
            .mx(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(4.))
            .pr(px(8.))
            .py(px(5.))
            .rounded(RADIUS)
            .cursor_pointer()
            .hover_probed(|s| s.bg(t.surface_2))
            // After the hover: tests keep one selector, the last.
            .debug_selector({
                let k = m.key.clone();
                move || format!("machine-head-{k}")
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                let k = key.clone();
                this.update_ui(|s| s.toggle_collapsed(&k), cx);
            }))
            .child(chevron(SharedString::from(format!("machine-chev-{}", m.key)), !collapsed, 10., t.text_4))
            .child(machine_badge(m, 24., t))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .min_w_0()
                            .ellipsis()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text_2)
                            .group_hover_text("machine-head", t.text, |s| s)
                            .child(crate::kit::one_line(m.label.clone())),
                    )
                    .child(super::peers::machine_sub(&m.label, m.home, m.link.as_ref(), t)),
            )
            .when(blocked > 0, |d| {
                d.child(
                    crate::kit::text_glow(
                        div().font_family(MONO_FONT).text_size(px(10.5)).text_color(t.amber).child(format!("▲ {blocked}")),
                        t.amber_glow,
                        8.,
                    )
                    .flex_none(),
                )
            })
            .when(agents > 0, |d| {
                d.child(
                    div()
                        .flex_none()
                        .px(px(6.))
                        .rounded_full()
                        .bg(t.surface_2)
                        .font_family(MONO_FONT)
                        .text_size(px(10.5))
                        .text_color(t.text_3)
                        .child(agents.to_string()),
                )
            })
            .into_any_element()
    }

    /// An account's header in the sidebar grouped by account: its name and
    /// login, needs-you and agent counts, "Open as space"; folds its
    /// projects; right-click for its menu.
    /// On another machine than this Mac (`home`), accounts aren't dragged
    /// or dropped on; another machine's Pitwall's have no menu either.
    fn account_head(&self, sec: &AccountSection, collapsed: bool, home: bool, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let default = sec.account == pitwall_core::accounts::DEFAULT;
        let peer = sec.peer_name.is_some();
        let (name, sub) = if let Some(n) = &sec.peer_name {
            // The machine's heading already says it's view only.
            (n.clone(), String::new())
        } else if default {
            ("Default".to_string(), "your own logins".to_string())
        } else {
            let m = crate::accounts::model(cx);
            let (name, kind) = {
                let m = m.read(cx);
                let a = m.list.get(&sec.account);
                (a.map(|a| a.name.clone()).unwrap_or_else(|| sec.account.clone()), a.map(|a| a.kind.clone()).unwrap_or_default())
            };
            let login = m.update(cx, |m, _| m.login(&kind, Some(&sec.account)));
            let kind_name = m.read(cx).kind(&kind).map(|k| k.name).unwrap_or(kind);
            (name, login.map(|l| format!("{kind_name} · {l}")).unwrap_or(kind_name))
        };
        let (key, k_menu, k_space) = (sec.key.clone(), sec.account.clone(), sec.account.clone());
        let (n_menu, n_space) = (name.clone(), name.clone());
        let spot = Spot::Account(sec.account.clone());
        let mark = self.side_mark(&spot.key(), t, cx);
        let drag = AccountDrag { id: sec.account.clone(), name: name.clone().into() };
        let el = div()
            .id(SharedString::from(format!("acct-{}", sec.key)))
            .group("account-head")
            .relative()
            // Default stays first; only this Mac's accounts move.
            .when(!default && home, |d| {
                d.on_drag(drag, |d, grab, _, cx| cx.new(|_| AgentGhost::named(d.name.clone(), grab)))
            })
            .flex()
            .items_center()
            .pl(px(8.))
            .pr(px(6.))
            .pt(px(8.))
            .pb(px(2.))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    if !peer {
                        this.account_menu(&k_menu, &n_menu, e.position, window, cx);
                    }
                }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("acct-toggle-{}", sec.key)))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(4.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .tooltip(tooltip(if sub.is_empty() { name.clone() } else { format!("{name} — {sub}") }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let k = key.clone();
                        this.update_ui(|s| s.toggle_collapsed(&k), cx);
                    }))
                    .child(chevron(SharedString::from(format!("acct-chev-{}", sec.key)), !collapsed, 12., t.text_3))
                    .child(account_avatar(&name, default, t))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text_2)
                            .group_hover_text("account-head", t.text, |s| s)
                            .child(name),
                    )
                    .when(!sub.is_empty(), |d| d.child(div().min_w_0().ellipsis().text_size(px(11.)).text_color(t.text_4).child(crate::kit::one_line(sub))))
                    .child(div().flex_1())
                    .when(sec.blocked > 0, |d| {
                        d.child(
                            crate::kit::text_glow(
                                div().font_family(MONO_FONT).text_size(px(10.5)).text_color(t.amber).child(format!("▲ {}", sec.blocked)),
                                t.amber_glow,
                                8.,
                            )
                            .flex_none(),
                        )
                    })
                    .child(div().flex_none().font_family(MONO_FONT).text_size(px(11.)).text_color(t.text_3).child(sec.agents.to_string())),
            )
            .when(sec.agents > 0, |d| {
                d.child(
                    div().invisible().group_hover_probed("account-head", |s| s.visible()).child(
                        icon_btn(format!("acct-space-{}", sec.key), "grid", "Open as space", true, t)
                            .on_click(cx.listener(move |this, _, _, cx| this.open_account_space(&k_space, &n_space, cx))),
                    ),
                )
            });
        if !home {
            return el;
        }
        self.drop_spot(el, spot, cx).when_some(mark, |d, m| d.child(m))
    }

    /// An account header's right-click menu.
    fn account_menu(&mut self, account: &str, name: &str, at: gpui::Point<gpui::Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let (a, n) = (account.to_string(), name.to_string());
        let items = vec![
            MenuItem::new("grid", "Open as space", move |this, _, cx| this.open_account_space(&a, &n, cx)),
            MenuItem::new("", "Group by project", |this, _, cx| this.set_sidebar_group(SidebarGroup::Project, cx)),
        ];
        self.open_menu(at, items, window, cx);
    }

    fn project_menu(
        &mut self,
        project: &str,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (p1, p2) = (project.to_string(), project.to_string());
        let mut items = vec![
            MenuItem::new("terminal", "New terminal here", move |this, window, cx| {
                this.open_terminal(p1.clone(), window, cx)
            }),
            MenuItem::new("plus", "New agent…", move |this, window, cx| {
                this.open_new_agent(Some(p2.clone()), window, cx)
            }),
        ];
        // Which login its new agents use (agents with more than one).
        if !crate::accounts::model(cx).read(cx).kinds().is_empty() {
            let p = project.to_string();
            items.push(MenuItem::new("", "Accounts…", move |this, window, cx| {
                this.open_project_accounts(p.clone(), window, cx)
            }));
            items.push(MenuItem::new("", "Group by account", |this, _, cx| {
                this.set_sidebar_group(SidebarGroup::Account, cx)
            }));
        }
        self.open_menu(at, items, window, cx);
    }

    pub(super) fn agent_menu(
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
        // Which login it runs on (Settings → Agents → Accounts).
        let local = a.machine.provider == pitwall_core::provider::ProviderId::LOCAL;
        let list = crate::accounts::model(cx).read(cx).of_kind(&a.kind);
        if local && !list.is_empty() && !a.engineer {
            items.push(MenuItem::header("Run on account"));
            let current = a.account.clone();
            let mut push = |id: Option<String>, name: String| {
                let on = current == id;
                let (agent, pick, label) = (a.id.clone(), id.clone(), name.clone());
                items.push(MenuItem::new(if on { "check" } else { "" }, &label, move |this, _, cx| {
                    if !on {
                        this.switch_account(agent.clone(), pick.clone(), name.clone(), cx)
                    }
                }));
            };
            push(None, "Default".into());
            for acc in list {
                push(Some(acc.id.clone()), acc.name.clone());
            }
        }
        // Another machine's agent: nothing of it is kept here to rename or
        // remove (and its caps allow nothing else).
        if crate::agents::mirrored(a) {
            if !items.is_empty() {
                self.open_menu(at, items, window, cx);
            }
            return;
        }
        let rn = a.id.clone();
        items.push(MenuItem::new("", "Rename…", move |this, window, cx| this.start_agent_rename(&rn, window, cx)));
        // Named by the user, while its conversation has a title of its own.
        if !a.name_auto && a.session_title.is_some() {
            let id = a.id.clone();
            items.push(MenuItem::new("", "Use session title", move |this, _, cx| this.use_session_title(&id, cx)));
        }
        let id = a.id.clone();
        items.push(MenuItem::new(
            "trash",
            "Remove agent…",
            move |this, _, cx| this.open_remove(&id, cx),
        ));
        self.open_menu(at, items, window, cx);
    }

    /// "Rename…" (or a double-click on its row): the agent's name becomes a
    /// field in place. Enter saves, Esc cancels, leaving it saves a valid name.
    pub(super) fn start_agent_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.store.read(cx).agents.iter().find(|a| a.id == id && !crate::agents::mirrored(a)).map(|a| a.name.clone()) else {
            return;
        };
        self.menu = None;
        let input = cx.new(|cx| TextInput::new(cx, &name, "Name"));
        input.update(cx, |i, cx| i.select_all_text(cx));
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::Submit | InputEvent::SubmitShift => this.finish_agent_rename(true, window, cx),
            InputEvent::Blur => this.finish_agent_rename(false, window, cx),
            InputEvent::Cancel => {
                this.agent_rename = None;
                window.focus(&this.focus);
                cx.notify();
            }
            InputEvent::Changed => {
                if let Some(r) = this.agent_rename.as_mut().filter(|r| r.error.is_some()) {
                    r.error = None;
                    cx.notify();
                }
            }
        });
        window.focus(&input.focus_handle(cx));
        self.agent_rename = Some(AgentRenaming { id: id.to_string(), input, error: None, _sub: sub });
        cx.notify();
    }

    /// Save the name being edited. A bad one stays in the field with why
    /// (`keep`), or is dropped (the field lost focus).
    pub(super) fn finish_agent_rename(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.agent_rename.as_mut() else { return };
        let typed = r.input.read(cx).text().to_string();
        // Unchanged: nothing to pin (a name following its title keeps doing so).
        let current = self.store.read(cx).agents.iter().find(|a| a.id == r.id).map(|a| a.name.clone());
        if current.as_deref() == Some(typed.trim()) {
            self.agent_rename = None;
            window.focus(&self.focus);
            cx.notify();
            return;
        }
        let saved = pitwall_core::engine::check_name(&typed).map(str::to_string).and_then(|name| match &self.engine {
            Some(engine) => engine.rename(&r.id, &name),
            None => Err("Pitwall's engine isn't running".into()),
        });
        match saved {
            Ok(view) => {
                self.agent_rename = None;
                crate::agents::patch(view, cx);
                window.focus(&self.focus);
            }
            Err(e) if keep => r.error = Some(e),
            Err(_) => self.agent_rename = None,
        }
        cx.notify();
    }

    /// "Use session title": the name follows the agent's conversation title again.
    pub(super) fn use_session_title(&mut self, id: &str, cx: &mut Context<Self>) {
        self.menu = None;
        if let Some(view) = self.engine.as_ref().and_then(|e| e.use_session_title(id).ok()) {
            crate::agents::patch(view, cx);
        }
        cx.notify();
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
        group: &str,
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
        // Grouped by account, the heading already says it.
        let account = crate::accounts::agent_account(cx, a).filter(|_| self.ui.sidebar_group != SidebarGroup::Account);
        let (id_click, id_menu) = (a.id.clone(), a.clone());
        let renaming = self.agent_rename.as_ref().filter(|r| r.id == a.id);
        let rename_error = renaming.and_then(|r| r.error.clone());
        let rename_input = renaming.map(|r| r.input.clone());
        let drag = AgentDrag {
            id: a.id.clone(),
            name: a.name.clone().into(),
        };
        let spot = Spot::Row { id: a.id.clone(), group: group.to_string() };
        let el = div()
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
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                if e.click_count() >= 2 {
                    this.start_agent_rename(&id_click, window, cx);
                } else {
                    this.show_agent(&id_click, window, cx);
                }
            }))
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
                    .debug_selector(|| format!("row-body-{}", a.id))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(match rename_input {
                                // Clicks and drags in the field stay in it.
                                Some(input) => div()
                                    .id(SharedString::from(format!("row-rename-{}", a.id)))
                                    .flex_1()
                                    .min_w_0()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                    .on_click(|_, _, cx| cx.stop_propagation())
                                    .child(input)
                                    .into_any_element(),
                                None => div()
                                    .flex_1()
                                    .min_w_0()
                                    .ellipsis()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(13.))
                                    .text_color(t.text)
                                    .child(crate::kit::one_line(a.name.clone()))
                                    .into_any_element(),
                            })
                            .when(a.engineer, |d| d.child(chip("engineer", t)))
                            .when_some(account, |d, n| d.child(chip(n, t)))
                            // Another machine's Pitwall: watched, not driven.
                            .when(crate::agents::mirrored(a), |d| {
                                d.child(
                                    div()
                                        .id(SharedString::from(format!("row-view-only-{}", a.id)))
                                        .flex_none()
                                        .tooltip(tooltip(format!("View only: it runs on {}", a.machine.label)))
                                        .child(icon("eye", 12., t.text_4)),
                                )
                            })
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
                            .child(crate::kit::agent_logo(&a.kind, &a.kind_name, 12., t))
                            .child(div().flex_1().min_w_0().ml(px(-2.)).ellipsis().child(crate::kit::one_line(
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
                    .when_some(rename_error, |d, e| d.child(crate::kit::error_text(e, t)))
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
            });
        // Another machine's Pitwall: read-only, nothing lands on it.
        if crate::agents::mirrored(a) {
            return el;
        }
        let mark = self.side_mark(&spot.key(), t, cx);
        self.drop_spot(el, spot, cx).when_some(mark, |d, m| d.child(m))
    }

    fn render_rail(
        &mut self,
        agents: &[AgentView],
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // With 2+ machines, each machine's projects under its badge.
        let sections = self.machine_sections(agents, cx);
        let level = machine_level(&sections);
        let groups: Vec<(Option<&MachineSection>, &ProjectGroup)> = sections
            .iter()
            .flat_map(|m| {
                let mut first = true;
                m.all_groups().map(move |g| (std::mem::take(&mut first).then_some(m).filter(|_| level), g))
            })
            .collect();
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
            .children(groups.iter().enumerate().map(|(gi, (head, g))| {
                let title: SharedString = match (&g.machine, g.can_create) {
                    (Some(m), false) => format!("{} · {}", g.display, m.label).into(),
                    _ => g.display.clone().into(),
                };
                let head = head.map(|m| {
                    let what = match &m.link {
                        Some(l) => format!("{} · {} · view only", m.label, crate::peers::link_text(l, crate::peers::now_ms())),
                        None => m.label.clone(),
                    };
                    div()
                        .id(SharedString::from(format!("rail-m-{}", m.key)))
                        .debug_selector({
                            let k = m.key.clone();
                            move || format!("rail-m-{k}")
                        })
                        .w_full()
                        .flex()
                        .justify_center()
                        .pt(px(if gi > 0 { 10. } else { 2. }))
                        .pb(px(2.))
                        .when(gi > 0, |d| d.border_t_1().border_color(t.line_strong))
                        .tooltip(tooltip(what))
                        .child(machine_badge(m, 22., t))
                });
                let new_machine = head.is_some();
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .children(head)
                    .child(
                div()
                    .id(SharedString::from(format!("rail-g-{}", g.key)))
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .py(px(6.))
                    .when(gi > 0 && !new_machine, |d| d.border_t_1().border_color(t.line))
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
                            .debug_selector({
                                let id = a.id.clone();
                                move || format!("rail-{id}")
                            })
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
                    })),
                    )
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

/// Where a drag over the sidebar would land: the row or header under the
/// pointer ([`Spot::key`]) and how.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SideDrop {
    target: String,
    how: Landing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Landing {
    /// Above it, in its list.
    Before,
    /// Below it (on a project's header: first in that project).
    After,
    /// Onto it: to its account.
    Into,
}

/// A project dragged in the sidebar.
#[derive(Clone, Debug)]
pub struct ProjectDrag {
    /// Its group key (with its account section's, grouped by account).
    key: String,
    project: String,
    name: SharedString,
    block: (bool, String),
    /// Its account section (grouped by account).
    account: Option<String>,
}

/// An account dragged in the sidebar grouped by account.
#[derive(Clone, Debug)]
pub struct AccountDrag {
    id: String,
    name: SharedString,
}

/// What is dragged.
enum Payload {
    Agent(String),
    Project(ProjectDrag),
    Account(String),
}

/// What a drag is over.
#[derive(Clone)]
enum Spot {
    /// An agent's row in group `group` (its whole key).
    Row { id: String, group: String },
    /// A project's header.
    Group { key: String, block: (bool, String), account: Option<String> },
    /// An account's header (`default`: Default).
    Account(String),
}

impl Spot {
    fn key(&self) -> String {
        match self {
            Spot::Row { id, .. } => format!("row:{id}"),
            Spot::Group { key, .. } => format!("group:{key}"),
            Spot::Account(id) => format!("acct:{id}"),
        }
    }

    /// The account a drop "into" it moves to.
    fn account(&self) -> Option<&str> {
        match self {
            Spot::Row { group, .. } => section_of(group),
            Spot::Group { account, .. } => account.as_deref(),
            Spot::Account(id) => Some(id),
        }
    }
}

/// Which projects a project can be put between: this Mac's, or one
/// machine's.
fn block(g: &ProjectGroup) -> (bool, String) {
    let label = g.machine.as_ref().filter(|_| !g.can_create).map(|m| m.label.clone());
    (g.can_create, label.unwrap_or_default())
}

/// The sibling `item` goes before when dropped `how` on `target`
/// (`None`: last).
fn before_of(siblings: &[String], item: &str, target: &str, how: Landing) -> Option<String> {
    let rest: Vec<&String> = siblings.iter().filter(|s| *s != item).collect();
    match how {
        Landing::Before => Some(target.to_string()),
        _ => rest.iter().position(|s| *s == target).and_then(|i| rest.get(i + 1)).map(|s| s.to_string()),
    }
}

/// Agents moving to an account, and the project a login starts in.
struct ToAccount {
    mv: Move,
    project: String,
}

impl MainScreen {
    /// Make `el` a place to drop on in the sidebar: it follows where a drag
    /// over it would land ([`SideDrop`]) and acts on the drop.
    fn drop_spot(&self, el: Stateful<Div>, spot: Spot, cx: &mut Context<Self>) -> Stateful<Div> {
        let (s1, s2, s3) = (spot.clone(), spot.clone(), spot.clone());
        let (s4, s5, s6) = (spot.clone(), spot.clone(), spot);
        el.on_drag_move::<AgentDrag>(cx.listener(move |this, e: &DragMoveEvent<AgentDrag>, _, cx| {
            let p = Payload::Agent(e.drag(cx).id.clone());
            this.side_track(p, &s1, e.bounds, e.event.position, cx)
        }))
        .on_drag_move::<ProjectDrag>(cx.listener(move |this, e: &DragMoveEvent<ProjectDrag>, _, cx| {
            let p = Payload::Project(e.drag(cx).clone());
            this.side_track(p, &s2, e.bounds, e.event.position, cx)
        }))
        .on_drag_move::<AccountDrag>(cx.listener(move |this, e: &DragMoveEvent<AccountDrag>, _, cx| {
            let p = Payload::Account(e.drag(cx).id.clone());
            this.side_track(p, &s3, e.bounds, e.event.position, cx)
        }))
        .on_drop(cx.listener(move |this, d: &AgentDrag, window, cx| {
            this.side_dropped(Payload::Agent(d.id.clone()), &s4, window, cx)
        }))
        .on_drop(cx.listener(move |this, d: &ProjectDrag, window, cx| {
            this.side_dropped(Payload::Project(d.clone()), &s5, window, cx)
        }))
        .on_drop(cx.listener(move |this, d: &AccountDrag, window, cx| {
            this.side_dropped(Payload::Account(d.id.clone()), &s6, window, cx)
        }))
    }

    fn side_track(
        &mut self,
        p: Payload,
        spot: &Spot,
        b: gpui::Bounds<gpui::Pixels>,
        at: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let key = spot.key();
        let next = if b.contains(&at) {
            self.landing(&p, spot, at.y < b.center().y, cx).map(|how| SideDrop { target: key, how })
        } else if self.side_drop.as_ref().is_some_and(|d| d.target == key) {
            None
        } else {
            return;
        };
        if self.side_drop != next {
            self.side_drop = next;
            cx.notify();
        }
    }

    /// The line or ring showing where a drag would land on `key`.
    fn side_mark(&self, key: &str, t: &Theme, cx: &App) -> Option<AnyElement> {
        let d = self.side_drop.as_ref().filter(|d| d.target == key && cx.has_active_drag())?;
        let mark = match d.how {
            Landing::Into => div().absolute().inset_0().rounded(RADIUS).border_1().border_color(t.focus).bg(t.focus.opacity(0.08)),
            how => {
                let line = div().absolute().left(px(6.)).right(px(6.)).h(px(2.)).rounded(px(1.)).bg(t.focus);
                if how == Landing::Before { line.top(px(-1.)) } else { line.bottom(px(-1.)) }
            }
        };
        Some(mark.into_any_element())
    }

    /// The group (its whole key) agent `id` shows in.
    fn group_of(&self, id: &str, cx: &App) -> Option<String> {
        self.sidebar_list(self.agents(cx), cx)
            .into_iter()
            .find(|g| g.agents.iter().any(|a| a.id == id))
            .map(|g| g.key)
    }

    /// What dropping `p` on `spot` would do: put it in its place among its
    /// siblings, or move it to another account; `None`: nothing.
    fn landing(&self, p: &Payload, spot: &Spot, upper: bool, cx: &mut App) -> Option<Landing> {
        let line = if upper { Landing::Before } else { Landing::After };
        // Another machine's Pitwall's agents stay where they are.
        if let Payload::Agent(id) = p {
            if self.agents(cx).iter().any(|a| a.id == *id && crate::agents::mirrored(a)) {
                return None;
            }
        }
        match (p, spot) {
            (Payload::Account(a), Spot::Account(b)) if a != b => {
                Some(if b == pitwall_core::accounts::DEFAULT { Landing::After } else { line })
            }
            (Payload::Project(d), Spot::Group { key, block, account, .. }) if *key != d.key => {
                if *account == d.account {
                    (*block == d.block).then_some(line)
                } else if block.0 {
                    // To another of this Mac's accounts (never a machine's).
                    self.project_to(d, account.as_deref()?, cx).map(|_| Landing::Into)
                } else {
                    None
                }
            }
            (Payload::Project(d), Spot::Account(to)) => self.project_to(d, to, cx).map(|_| Landing::Into),
            (Payload::Agent(id), Spot::Row { id: row, group }) if id != row => {
                let from = self.group_of(id, cx)?;
                if from == *group {
                    Some(line)
                } else if base_key(&from) == base_key(group) {
                    self.agent_to(id, spot.account()?, cx).map(|_| Landing::Into)
                } else {
                    None
                }
            }
            (Payload::Agent(id), Spot::Group { key, .. }) => {
                let from = self.group_of(id, cx)?;
                if from == *key {
                    Some(Landing::After)
                } else if base_key(&from) == base_key(key) {
                    self.agent_to(id, spot.account()?, cx).map(|_| Landing::Into)
                } else {
                    None
                }
            }
            (Payload::Agent(id), Spot::Account(to)) => self.agent_to(id, to, cx).map(|_| Landing::Into),
            _ => None,
        }
    }

    /// Agent `id` to account `to` (`default`: Default): only an agent on
    /// this Mac whose tool has accounts, onto one of that tool's.
    fn agent_to(&self, id: &str, to: &str, cx: &mut App) -> Option<ToAccount> {
        let a = self.agents(cx).iter().find(|a| a.id == id)?.clone();
        let local = a.machine.provider == pitwall_core::provider::ProviderId::LOCAL;
        if !local || a.engineer || a.terminal || super::workspace::account_key(&a) == to {
            return None;
        }
        let m = crate::accounts::model(cx);
        let m = m.read(cx);
        m.kind(&a.kind)?;
        let (account, name) = if to == pitwall_core::accounts::DEFAULT {
            (None, "Default".to_string())
        } else {
            let acc = m.list.get(to).filter(|acc| acc.kind == a.kind)?;
            (Some(acc.id.clone()), acc.name.clone())
        };
        Some(ToAccount { mv: Move { ids: vec![a.id], account, name, kind: a.kind }, project: a.project })
    }

    /// A project, from its account section, to account `to`: its tool is
    /// the one either account is for; its agents of that tool go along.
    fn project_to(&self, d: &ProjectDrag, to: &str, cx: &mut App) -> Option<ToAccount> {
        use pitwall_core::accounts::DEFAULT;
        let from = d.account.as_deref()?;
        if from == to || !d.block.0 {
            return None;
        }
        let m = crate::accounts::model(cx);
        let m = m.read(cx);
        let kind = match (from == DEFAULT, to == DEFAULT) {
            (true, true) => return None,
            (false, _) => {
                let k = m.list.get(from)?.kind.clone();
                if to != DEFAULT && m.list.get(to)?.kind != k {
                    return None;
                }
                k
            }
            (true, false) => m.list.get(to)?.kind.clone(),
        };
        let (account, name) = match m.list.get(to) {
            Some(a) => (Some(a.id.clone()), a.name.clone()),
            None => (None, "Default".to_string()),
        };
        let ids = self
            .agents(cx)
            .iter()
            .filter(|a| a.project == d.project && a.kind == kind && a.account != account && !a.engineer)
            .map(|a| a.id.clone())
            .collect();
        Some(ToAccount { mv: Move { ids, account, name, kind }, project: d.project.clone() })
    }

    fn side_dropped(&mut self, p: Payload, spot: &Spot, window: &mut Window, cx: &mut Context<Self>) {
        let Some(how) = self.side_drop.take().filter(|d| d.target == spot.key()).map(|d| d.how) else {
            return;
        };
        cx.notify();
        match (p, spot, how) {
            (Payload::Account(a), Spot::Account(b), _) => {
                let ids: Vec<String> = crate::accounts::ids(cx).into_iter().filter(|x| *x != a).collect();
                let before = if b == pitwall_core::accounts::DEFAULT {
                    ids.first().cloned()
                } else {
                    before_of(&ids, &a, b, how)
                };
                let m = crate::accounts::model(cx);
                if let Err(e) = m.update(cx, |m, cx| m.move_before(&a, before.as_deref(), cx)) {
                    self.failed("move the account", e, cx);
                }
            }
            (Payload::Project(d), _, Landing::Into) => {
                if let Some(to) = spot.account().and_then(|to| self.project_to(&d, to, cx)) {
                    self.move_project(to, window, cx);
                }
            }
            (Payload::Project(d), Spot::Group { key, .. }, how) => {
                let siblings: Vec<String> = match d.account {
                    None => self.sidebar_groups(self.agents(cx)),
                    // Its account section on its machine.
                    Some(_) => self
                        .machine_sections(self.agents(cx), cx)
                        .into_iter()
                        .flat_map(|m| m.accounts)
                        .find(|s| s.groups.iter().any(|g| g.key == d.key))
                        .map(|s| s.groups)
                        .unwrap_or_default(),
                }
                .iter()
                .map(|g| base_key(&g.key).to_string())
                .collect();
                let (item, target) = (base_key(&d.key), base_key(key));
                let before = before_of(&siblings, item, target, how);
                self.update_ui(|mut s| {
                    s.sidebar_order = reorder(&s.sidebar_order, &siblings, item, before.as_deref());
                    s
                }, cx);
            }
            (Payload::Agent(id), _, Landing::Into) => {
                if let Some(to) = spot.account().and_then(|to| self.agent_to(&id, to, cx)) {
                    self.move_agents(to.mv, Some(to.project), window, cx);
                }
            }
            (Payload::Agent(id), Spot::Row { group, .. } | Spot::Group { key: group, .. }, how) => {
                let siblings: Vec<String> = self
                    .sidebar_list(self.agents(cx), cx)
                    .into_iter()
                    .find(|g| g.key == *group)
                    .map(|g| g.agents.into_iter().map(|a| a.id).collect())
                    .unwrap_or_default();
                let before = match spot {
                    Spot::Row { id: target, .. } => before_of(&siblings, &id, target, how),
                    // On its project's header: first.
                    _ => siblings.iter().find(|s| **s != id).cloned(),
                };
                self.update_ui(|mut s| {
                    s.sidebar_order = reorder(&s.sidebar_order, &siblings, &id, before.as_deref());
                    s
                }, cx);
            }
            _ => {}
        }
    }

    /// A project's new agents of a tool use another account, and its
    /// running ones of that tool restart there (as its Accounts…).
    fn move_project(&mut self, to: ToAccount, window: &mut Window, cx: &mut Context<Self>) {
        let ToAccount { mv, project } = to;
        let m = crate::accounts::model(cx);
        if let Err(e) = m.update(cx, |m, cx| m.set_project_default(&project, &mv.kind, mv.account.as_deref(), cx)) {
            return self.failed("change the project's account", e, cx);
        }
        if mv.ids.is_empty() {
            let folder = crate::kit::project_name(&project).to_string();
            let tool = m.read(cx).kind(&mv.kind).map(|k| k.name).unwrap_or(mv.kind.clone());
            self.toast(Toast::info(&format!("{folder} uses {}", mv.name), Some(format!("New {tool} agents there start on it."))), cx);
        }
        self.move_agents(mv, Some(project), window, cx);
    }
}

#[cfg(test)]
impl MainScreen {
    /// Tests: drag agent `id` over the upper or lower half of `row`'s row and
    /// drop it there; whether the drop was taken.
    pub(super) fn drop_agent_on_row(&mut self, id: &str, row: &str, upper: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(group) = self.group_of(row, cx) else { return false };
        let (p, spot) = (Payload::Agent(id.into()), Spot::Row { id: row.into(), group });
        let Some(how) = self.landing(&p, &spot, upper, cx) else { return false };
        self.side_drop = Some(SideDrop { target: spot.key(), how });
        self.side_dropped(p, &spot, window, cx);
        true
    }
}

/// A machine's icon (a laptop for this Mac and paired Macs, a server for
/// an agw machine) in a `size` tile, with a dot for how it's doing: this
/// Mac is always up, another machine's Pitwall shows its link.
fn machine_badge(m: &MachineSection, size: f32, t: &Theme) -> Div {
    let dot = match &m.link {
        Some(l) => Some(super::peers::link_color(l.state, t)),
        None => m.home.then_some(t.green),
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .rounded(px(6.))
        .bg(t.surface_2)
        .border_1()
        .border_color(t.line)
        .flex()
        .items_center()
        .justify_center()
        .child(icon(if m.home || m.link.is_some() { "laptop" } else { "server" }, size * 0.56, t.text_2))
        .when_some(dot, |d, c| {
            d.child(div().absolute().right(px(-2.)).bottom(px(-2.)).size(px(8.)).rounded_full().border_2().border_color(t.surface).bg(c))
        })
}

/// An account's initials ("Work Laptop" → "WL"), the person icon for the
/// default (the user's own logins).
fn account_avatar(name: &str, default: bool, t: &Theme) -> AnyElement {
    if default {
        return div().flex_none().size(px(16.)).flex().items_center().justify_center().child(icon("user", 12., t.text_3)).into_any_element();
    }
    div()
        .flex_none()
        .size(px(16.))
        .rounded(px(4.))
        .bg(t.surface_3)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(8.5))
        .font_weight(FontWeight::BOLD)
        .text_color(t.text_2)
        .child(avatar_letters(name))
        .into_any_element()
}

/// Up to two letters for an account: the first of its first two words.
fn avatar_letters(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| c.is_whitespace() || c == '-' || c == '_').filter(|w| !w.is_empty()).collect();
    match words.as_slice() {
        [] => "?".into(),
        [one] => one.chars().take(2).collect::<String>().to_uppercase(),
        [a, b, ..] => a.chars().take(1).chain(b.chars().take(1)).collect::<String>().to_uppercase(),
    }
}

#[cfg(test)]
#[test]
fn avatar_letters_take_the_first_two_words() {
    assert_eq!(avatar_letters("Work Laptop"), "WL");
    assert_eq!(avatar_letters("client-acme"), "CA");
    assert_eq!(avatar_letters("personal"), "PE");
    assert_eq!(avatar_letters("  "), "?");
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
