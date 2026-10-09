//! The top bar (React `TopBar.tsx`, `SpaceTabs.tsx`): sidebar toggle,
//! wordmark, space tabs, Search ⌘K, live counts, Review / Wall / Settings /
//! details buttons. The whole bar is the window's drag area.

use crate::kit::HoverText;
use gpui::{
    div, prelude::*, px, AnyElement, Context, Focusable, FontWeight, IntoElement, MouseButton,
    SharedString, Window,
};

use pitwall_proto::{AgentView, Status};

use crate::kit::icon;
use crate::kit::{InputEvent, TextInput};
use crate::kit::{icon_btn, icon_btn_state, kbd, label_t, tooltip, tracked};
use super::workspace::SpaceKind;
use super::{Breakpoint, MainScreen, Route, ScreenEvent};
use crate::menu::OpenSettings;
use crate::platform::decorations::{self, no_drag};
use crate::theme::{Theme, RADIUS, TOPBAR_H};

/// A dragged agent (sidebar row, pane header, chip).
#[derive(Clone, Debug)]
pub struct AgentDrag {
    pub id: String,
    pub name: SharedString,
}

impl AgentDrag {
    /// The floating label that follows the pointer (`.drag-ghost`): 12 px
    /// right of and 10 px under the cursor, wherever the drag was grabbed.
    pub fn ghost(
        &self,
        grab: gpui::Point<gpui::Pixels>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> gpui::Entity<AgentGhost> {
        let name = self.name.clone();
        cx.new(|_| AgentGhost { name, grab })
    }
}

/// The dragged agent's ghost (gpui draws it at the pointer minus `grab`).
pub struct AgentGhost {
    name: SharedString,
    grab: gpui::Point<gpui::Pixels>,
}

impl Render for AgentGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx);
        div()
            .pl(self.grab.x + px(12.))
            .pt(self.grab.y + px(10.))
            .child(
                div()
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(RADIUS)
                    .bg(t.surface_3)
                    // inset 0 0 0 1px color-mix(text 25%)
                    .border_1()
                    .border_color(t.text.opacity(0.25))
                    .shadow(vec![gpui::BoxShadow {
                        color: gpui::hsla(0., 0., 0., 0.35),
                        offset: gpui::point(px(0.), px(6.)),
                        blur_radius: px(20.),
                        spread_radius: px(0.),
                    }])
                    .opacity(0.95)
                    .whitespace_nowrap()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(self.name.clone()),
            )
    }
}

impl MainScreen {
    pub(super) fn render_topbar(
        &mut self,
        agents: &[AgentView],
        bp: Breakpoint,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let compact = bp.compact();
        let count = |s: Status| agents.iter().filter(|a| a.status == s).count();
        let (working, done, blocked) = (
            count(Status::Working),
            count(Status::Done),
            count(Status::Blocked),
        );
        let sidebar_open =
            self.sidebar_mode(bp) == super::SidebarMode::Full || self.overlay_sidebar;
        let right_open = self.route == Route::Space
            && self.focused_agent_id().is_some()
            && if bp.right_docked() {
                self.right_pinned
            } else {
                self.drawer_open
            };

        let engineer_focused = self
            .focused_agent_id()
            .is_some_and(|id| agents.iter().any(|a| a.id == id && a.engineer));

        // aria-pressed: open is the normal colour, closed is dimmer; Wall /
        // Review on are lit (`.wall-btn[data-on]`).
        let aria = |on: bool| if on { None } else { Some(false) };
        let lit = |on: bool| Some(on);
        let keys = crate::kit::keys;

        let left = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .flex_none()
            .child(
                icon_btn_state(
                    "tb-sidebar",
                    "sidebar",
                    keys(
                        if sidebar_open {
                            "Hide agents (⌘B)"
                        } else {
                            "Show agents (⌘B)"
                        },
                        false,
                    ),
                    false,
                    16.,
                    aria(sidebar_open),
                    t,
                )
                .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx)))
                .map(no_drag),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    // The mark is the P, so the word goes on from it: the
                    // 8 px gap less `.wordmark-rest`'s -4px margin (a
                    // negative margin upsets GPUI's flex sizing).
                    .gap(px(4.))
                    .px(px(4.))
                    .child(crate::kit::brand_mark(t))
                    .when(!compact, |d| {
                        d.child(
                            tracked("ITWALL", 17., 0.22)
                                .font_family(crate::kit::LABEL_FONT)
                                .font_weight(FontWeight::BOLD)
                                .text_color(t.text),
                        )
                    }),
            );

        let counts = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .mx(px(4.))
            .text_size(px(11.5))
            .font_family(crate::kit::MONO_FONT)
            .when(!agents.is_empty(), |d| {
                d.child(count_pill(
                    "◐",
                    t.green,
                    working,
                    "working",
                    compact,
                    false,
                    t,
                    "Agents working",
                ))
            })
            .when(done > 0, |d| {
                d.child(count_pill(
                    "⚑",
                    t.flag,
                    done,
                    "done",
                    compact,
                    false,
                    t,
                    "Finished, not looked at yet",
                ))
            })
            .when(blocked > 0, |d| {
                d.child(count_pill(
                    "▲",
                    t.amber,
                    blocked,
                    "needs you",
                    compact,
                    true,
                    t,
                    "Waiting on you",
                ))
            });

        let right = no_drag(div().id("topbar-right"))
            .flex()
            .items_center()
            .gap(px(6.))
            .flex_none()
            .child(
                div()
                    .id("tb-search")
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .h(px(28.))
                    .pl(px(9.))
                    .pr(px(6.))
                    .rounded(if t.is_glass() { px(999.) } else { RADIUS })
                    .border_1()
                    .border_color(t.line)
                    .bg(t.bg)
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .cursor_pointer()
                    .hover_text(t.text_2, |s| s.border_color(t.line_strong))
                    .tooltip(tooltip(keys("Command palette (⌘K)", false)))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ScreenEvent::OpenPalette)))
                    .child(crate::kit::icon("search", 14., t.text_3))
                    .when(!compact, |d| d.child("Search"))
                    .child(kbd("⌘K", t)),
            )
            .child(counts)
            .child(
                // The Race Engineer (docs/spec/engineer.md): lit while its
                // pane is the focused one.
                icon_btn_state(
                    "tb-engineer",
                    crate::engineer::ICON,
                    "Race Engineer: Pitwall's assistant (setup, agw, spaces, settings)",
                    false,
                    16.,
                    lit(engineer_focused),
                    t,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_engineer(window, cx))),
            )
            .child(
                icon_btn_state(
                    "tb-review",
                    "review",
                    keys("Review: what agents changed (⌘R)", false),
                    false,
                    16.,
                    lit(self.route == Route::Review),
                    t,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_route(Route::Review, cx))),
            )
            .child(
                icon_btn_state(
                    "tb-wall",
                    "wall",
                    keys("Wall: every agent at once (⌘E)", false),
                    false,
                    16.,
                    lit(self.route == Route::Wall),
                    t,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_route(Route::Wall, cx))),
            )
            .child(
                icon_btn("tb-settings", "gear", "Settings", false, t)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
            )
            .child(
                icon_btn_state(
                    "tb-details",
                    "panel",
                    keys(
                        if right_open {
                            "Hide details (⌘.)"
                        } else {
                            "Show details (⌘.)"
                        },
                        false,
                    ),
                    false,
                    16.,
                    aria(right_open),
                    t,
                )
                .on_click(cx.listener(|this, _, window, cx| this.toggle_details(window, cx))),
            );

        let bar = div()
            .id("topbar")
            .h(TOPBAR_H)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(10.))
            .pr(px(8.))
            .border_b_1()
            .border_color(t.line)
            .bg(t.surface)
            .child(left)
            .child(self.render_tabs(agents, t, window, cx))
            // `.topbar-right { margin-left: auto }`
            .child(div().flex_1())
            .child(right);
        decorations::title_bar(bar, window, t)
    }

    fn render_tabs(
        &mut self,
        agents: &[AgentView],
        t: &Theme,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mine: Vec<_> = self
            .ui
            .spaces_of(&self.label)
            .into_iter()
            .cloned()
            .collect();
        let active = if self.route == Route::Wall {
            None
        } else {
            self.active_space().map(|s| s.id.clone())
        };
        let mut tabs: Vec<AnyElement> = Vec::new();
        for s in mine {
            let shown = s.layout.agents();
            let members: Vec<&AgentView> = if s.kind == SpaceKind::All {
                agents.iter().filter(|a| shown.contains(&a.id)).collect()
            } else {
                s.members_of(agents)
            };
            let blocked = members.iter().any(|a| a.status == Status::Blocked);
            let done = !blocked && members.iter().any(|a| a.status == Status::Done);
            let is_active = active.as_deref() == Some(s.id.as_str());
            let tip: SharedString = match s.kind {
                SpaceKind::Project => s.project.clone().unwrap_or_default().into(),
                SpaceKind::All => "Every agent".into(),
                SpaceKind::Custom => "Custom space — drag agents here".into(),
            };
            let id = s.id.clone();
            let editing = self
                .rename
                .as_ref()
                .filter(|(sid, _)| *sid == s.id)
                .map(|(_, i)| i.clone());
            let (id_click, id_dbl, id_drop) = (id.clone(), id.clone(), id.clone());
            let name = s.name.clone();
            let can_edit = s.kind != SpaceKind::All;
            let tab = div()
                .id(SharedString::from(format!("tab-{}", s.id)))
                .group("tab")
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .h_full()
                .rounded_t(px(7.))
                .border_1()
                .border_b_0()
                // Glass: the selected tab is a lighter pane of glass.
                .border_color(match (is_active, t.is_glass()) {
                    (true, true) => t.g.line_strong,
                    (true, false) => t.line,
                    _ => gpui::transparent_black(),
                })
                .bg(match (is_active, t.is_glass()) {
                    (true, true) => t.g.surface_3,
                    (true, false) => t.bg,
                    _ => gpui::transparent_black(),
                })
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if is_active { t.text } else { t.text_3 })
                .cursor_pointer()
                .when(!is_active, |d| {
                    d.hover_text(t.text_2, |s| s.bg(t.surface_2))
                })
                .drag_over::<AgentDrag>({
                    let c = t.surface_3;
                    move |s, _, _, _| s.bg(c)
                })
                .on_drop(cx.listener(move |this, d: &AgentDrag, window, cx| {
                    this.drop_on_space(&id_drop, &d.id, window, cx);
                }))
                .tooltip(tooltip(tip))
                .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                    if e.click_count() >= 2 && can_edit {
                        this.start_rename(&id_dbl, window, cx);
                        return;
                    }
                    this.active = id_click.clone();
                    this.set_route(Route::Space, cx);
                    cx.notify();
                }))
                .when(blocked, |d| {
                    d.child(crate::kit::motion::bounce_in(
                        SharedString::from(format!("tab-b-{}", s.id)),
                        crate::kit::text_glow(
                            div().text_size(px(10.)).text_color(t.amber).child("▲"),
                            t.amber_glow,
                            8.,
                        ),
                    ))
                })
                .when(done, |d| {
                    let finish = t.finish;
                    d.child(crate::kit::motion::keyframes(
                        SharedString::from(format!("tab-d-{}", s.id)),
                        crate::kit::motion::FLAG,
                        1,
                        move |p| {
                            let (o, sc) = crate::kit::motion::flag_pop(p);
                            div()
                                .w(px(10.))
                                .flex()
                                .justify_center()
                                .opacity(o)
                                .text_size(px(10. * sc))
                                .text_color(finish)
                                .child("⚑")
                                .into_any_element()
                        },
                    ))
                })
                .child(match editing {
                    Some(input) => div().w(px(120.)).child(input).into_any_element(),
                    None => label_t(
                        name.clone(),
                        if is_active { t.text } else { t.text_3 },
                        12.5,
                        0.1,
                    )
                    .into_any_element(),
                })
                .when(can_edit, |d| {
                    let (mv, cl) = (id.clone(), id.clone());
                    d.child(
                        div()
                            .flex()
                            .gap(px(1.))
                            .mr(px(-4.))
                            .when(!is_active, |d| {
                                d.invisible().group_hover_probed("tab", |s| s.visible())
                            })
                            .child(
                                tab_btn(
                                    &format!("tab-mv-{}", s.id),
                                    "window",
                                    crate::kit::keys("Move to new window (⌘⇧N)", false),
                                    t,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.move_space(mv.clone(), cx);
                                    },
                                )),
                            )
                            .child(
                                tab_btn(&format!("tab-x-{}", s.id), "x", "Close space".into(), t)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        let id = cl.clone();
                                        this.update_ui(|s| s.close_space(&id), cx);
                                    })),
                            ),
                    )
                });
            tabs.push(tab.into_any_element());
        }
        tabs.push(
            div()
                .id("tab-new")
                .flex()
                .items_center()
                .px(px(8.))
                .h_full()
                .rounded_t(px(7.))
                .text_color(t.text_3)
                .cursor_pointer()
                .hover_text(t.text_2, |s| s.bg(t.surface_2))
                .drag_over::<AgentDrag>({
                    let c = t.surface_3;
                    move |s, _, _, _| s.bg(c)
                })
                .on_drop(cx.listener(|this, d: &AgentDrag, window, cx| {
                    this.new_space(cx);
                    let id = this.active.clone();
                    this.drop_on_space(&id, &d.id, window, cx);
                }))
                .tooltip(tooltip("New space (or drop an agent here)"))
                .on_click(cx.listener(|this, _, _, cx| this.new_space(cx)))
                .child(icon("plus", 14., t.text_3))
                .into_any_element(),
        );
        no_drag(div().id("tabs"))
            .flex()
            .items_end()
            .gap(px(2.))
            .h_full()
            .pt(px(7.))
            .min_w_0()
            .overflow_x_scroll()
            .children(tabs)
    }

    fn start_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .ui
            .space(id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| TextInput::new(cx, &name, ""));
        input.update(cx, |i, cx| i.select_all_text(cx));
        let sid = id.to_string();
        self._subs.push(cx.subscribe_in(
            &input,
            window,
            move |this, input, e: &InputEvent, window, cx| match e {
                InputEvent::Submit | InputEvent::SubmitShift | InputEvent::Blur => {
                    let name = input.read(cx).text().to_string();
                    if this.rename.as_ref().is_some_and(|(s, _)| *s == sid) {
                        this.rename = None;
                        let id = sid.clone();
                        this.update_ui(|s| s.rename_space(&id, &name), cx);
                        window.focus(&this.focus);
                    }
                }
                InputEvent::Cancel => {
                    this.rename = None;
                    window.focus(&this.focus);
                    cx.notify();
                }
                InputEvent::Changed => {}
            },
        ));
        window.focus(&input.focus_handle(cx));
        self.rename = Some((id.to_string(), input));
        cx.notify();
    }

    /// An agent dropped on a space tab (or "+"): auto-place and switch there.
    pub(super) fn drop_on_space(
        &mut self,
        space: &str,
        agent: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fit = if self.active_space().map(|s| s.id.as_str()) == Some(space) {
            self.fit()
        } else {
            None
        };
        let (space, agent) = (space.to_string(), agent.to_string());
        let mut outcome = super::workspace::DropOutcome::Noop;
        let label = self.label.clone();
        self.update_ui(
            |s| {
                let (s, o, _) = s.drop_on_space(&space, &agent, fit);
                outcome = o;
                s.set_wall(&label, false)
            },
            cx,
        );
        if self.ui.window_of_space(&space) == self.label {
            self.active = space;
        }
        self.set_route(Route::Space, cx);
        self.after_drop(&agent, outcome, window, cx);
    }

    pub(super) fn after_drop(
        &mut self,
        agent: &str,
        outcome: super::workspace::DropOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if outcome == super::workspace::DropOutcome::Chip {
            let name = self
                .agents(cx)
                .iter()
                .find(|a| a.id == agent)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "Agent".into());
            self.toast(
                super::strip::Toast::info(
                    &format!("{name} added to the chip strip"),
                    Some("No room for another tile at this density. Click the chip to swap it in, or pick a denser layout.".into()),
                ),
                cx,
            );
        }
        self.changes_follow(cx);
        window.focus(&self.focus);
        cx.notify();
    }
}

fn tab_btn(id: &str, name: &str, tip: String, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id.to_string()))
        .group("tab-btn")
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .hover_probed(|s| s.bg(t.surface_3))
        .tooltip(tooltip(tip))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(icon(name, 12., t.text_4).group_hover_text("tab-btn", t.text, |s| s))
}

#[allow(clippy::too_many_arguments)]
fn count_pill(
    g: &'static str,
    color: gpui::Hsla,
    n: usize,
    word: &'static str,
    compact: bool,
    loud: bool,
    t: &Theme,
    tip: &'static str,
) -> AnyElement {
    let pill = div()
        .id(word)
        .flex()
        .items_center()
        .gap(px(5.))
        .h(px(24.))
        .px(px(8.))
        .rounded(px(12.))
        .text_color(if loud { t.amber } else { t.text_2 })
        .when(loud, |d| {
            d.bg(t.amber_soft)
                .border_1()
                .border_color(t.amber.opacity(0.35))
                .shadow(t.amber_glow(14., 0.))
                .font_weight(FontWeight::SEMIBOLD)
        })
        .tooltip(tooltip(tip))
        .child(div().text_size(px(11.)).text_color(color).child(g))
        // React writes `<glyph/> {n} working`: the text starts with a space.
        .child(if compact {
            format!(" {n}")
        } else {
            format!(" {n} {word}")
        });
    // `.count-loud` bounces twice when it shows.
    if loud {
        crate::kit::motion::bounce_in("count-loud", pill).into_any_element()
    } else {
        pill.into_any_element()
    }
}

