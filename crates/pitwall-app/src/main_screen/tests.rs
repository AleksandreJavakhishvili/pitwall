//! The main screen in a test window: showing agents, ⌘J, ⌘1–9, routes,
//! spaces and the strip, with made-up agents and no engine.

use gpui::{AppContext, TestAppContext, VisualTestContext};

use super::workspace::ALL_SPACE;
use super::*;
use crate::agents::tests::agent;

fn screen(
    cx: &mut TestAppContext,
) -> (
    Entity<MainScreen>,
    Entity<AgentStore>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        init(cx);
    });
    let store = cx.new(|_| {
        AgentStore::new(vec![
            agent("api", "/work/alpha", "idle", 1, true),
            agent("web", "/work/alpha", "blocked", 2, true),
            agent("docs", "/work/beta", "done", 3, true),
            agent("ops", "/work/beta", "blocked", 4, true),
        ])
    });
    let s = store.clone();
    let (view, cx) =
        cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    (view, store, cx)
}

#[gpui::test]
fn first_run_places_the_first_agent_and_shows_on_click(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    view.update(cx, |s, _| {
        // Sidebar order: alpha (web blocked, api idle), beta (ops blocked, docs done).
        assert_eq!(
            s.ui.space(ALL_SPACE).unwrap().layout.agents(),
            vec!["web".to_string()]
        );
    });
    view.update_in(cx, |s, window, cx| s.show_agent("docs", window, cx));
    view.update(cx, |s, cx| {
        assert_eq!(s.focused_agent_id().as_deref(), Some("docs"));
        assert_eq!(s.selected(cx).unwrap().name, "docs");
    });
}

#[gpui::test]
fn jump_cycles_blocked_agents_in_sidebar_order(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    let order = |view: &Entity<MainScreen>, cx: &mut VisualTestContext| {
        view.update_in(cx, |s, window, cx| s.next_blocked(window, cx));
        view.read_with(cx, |s, _| s.focused_agent_id().unwrap())
    };
    assert_eq!(
        order(&view, cx),
        "ops",
        "web is focused first, so the next blocked is ops"
    );
    assert_eq!(order(&view, cx), "web");
    view.update_in(cx, |s, window, cx| s.select_index(1, window, cx));
    view.read_with(cx, |s, _| {
        assert_eq!(s.focused_agent_id().as_deref(), Some("api"))
    });
}

#[gpui::test]
fn routes_toggle_and_escape_goes_back(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    view.update(cx, |s, cx| s.toggle_route(Route::Wall, cx));
    view.read_with(cx, |s, _| {
        assert_eq!(s.route(), Route::Wall);
        assert!(
            s.ui.wall.contains(&MAIN.to_string()),
            "Wall mode is kept in ui.json"
        );
    });
    view.update_in(cx, |s, window, cx| s.show_agent("api", window, cx));
    view.read_with(cx, |s, _| {
        assert_eq!(s.route(), Route::Space, "showing an agent leaves Wall")
    });
    view.update(cx, |s, cx| s.toggle_route(Route::Review, cx));
    cx.dispatch_action(Dismiss);
    view.read_with(cx, |s, _| assert_eq!(s.route(), Route::Space));
}

#[gpui::test]
fn spaces_and_presets(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    view.update(cx, |s, cx| s.new_space(cx));
    let custom = view.read_with(cx, |s, _| s.active.clone());
    assert_ne!(custom, ALL_SPACE);
    view.update_in(cx, |s, window, cx| s.show_agent("api", window, cx));
    view.update_in(cx, |s, window, cx| s.apply_preset(Preset::Two, window, cx));
    view.read_with(cx, |s, _| {
        let sp = s.ui.space(&custom).unwrap();
        assert_eq!(sp.layout.leaves().len(), 2);
        assert_eq!(sp.members, vec!["api".to_string()]);
    });
    view.update(cx, |s, cx| s.open_project_space("/work/beta", "beta", cx));
    view.read_with(cx, |s, _| {
        let sp = s.active_space().unwrap();
        assert_eq!(sp.kind, workspace::SpaceKind::Project);
        assert_eq!(sp.layout.agents().len(), 2);
    });
}

#[gpui::test]
fn attention_becomes_a_toast_and_the_strip_leads_with_blocked(cx: &mut TestAppContext) {
    let (view, store, cx) = screen(cx);
    store.update(cx, |_, cx| {
        cx.emit(crate::agents::StoreEvent::Attention {
            agent_id: "ops".into(),
            name: "ops".into(),
            reason: "blocked",
            detail: None,
        })
    });
    cx.run_until_parked();
    view.read_with(cx, |s, cx| {
        assert_eq!(s.toasts.len(), 1);
        assert_eq!(s.toasts[0].title.as_ref(), "ops needs you");
        let strip = model::strip(&s.ordered(cx));
        assert_eq!(strip.lead.unwrap().id, "web");
        assert_eq!(strip.more, 1);
    });
}

#[gpui::test]
fn removed_agents_leave_the_layout(cx: &mut TestAppContext) {
    let (view, store, cx) = screen(cx);
    view.update_in(cx, |s, window, cx| s.show_agent("docs", window, cx));
    store.update(cx, |s, cx| {
        s.agents.retain(|a| a.id != "docs");
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |s, _| assert!(s.ui.locate("docs").is_none()));
}

#[test]
fn breakpoints_follow_the_react_ones() {
    assert_eq!(Breakpoint::of(2400.), Breakpoint::Xl);
    assert_eq!(Breakpoint::of(1500.), Breakpoint::Lg);
    assert_eq!(Breakpoint::of(1200.), Breakpoint::Md);
    assert_eq!(Breakpoint::of(800.), Breakpoint::Sm);
    assert_eq!(Breakpoint::of(500.), Breakpoint::Xs);
    assert!(Breakpoint::Md.full_sidebar() && !Breakpoint::Md.right_docked());
    assert_eq!(Breakpoint::Lg.max_per_row(), 3);
}

#[test]
fn every_shortcut_has_a_binding() {
    let b = bindings();
    for a in [
        &OpenPalette as &dyn gpui::Action,
        &NewAgent,
        &NewTerminal,
        &NewTerminalAt,
        &JumpNextBlocked,
        &ToggleSidebar,
        &ToggleDetails,
        &ToggleWall,
        &ToggleReview,
        &ToggleMaximize,
        &RefreshChanges,
        &MoveSpaceToWindow,
        &SelectAgent1,
        &SelectAgent9,
    ] {
        assert!(b.iter().any(|k| k.action().partial_eq(a)), "{a:?}");
    }
    if !cfg!(target_os = "macos") {
        // Ctrl alone belongs to the terminal.
        assert!(b
            .iter()
            .filter(|k| k.predicate().is_none())
            .all(|k| !k.keystrokes().iter().any(super::terminal_ctrl)));
    }
}

/// The hover pass (main screen): the pointer visits every hoverable
/// element (sidebar rows, the empty-project row, project heads, tabs, top
/// bar, pane headers and their buttons, chips, the right panel, the strip)
/// and nothing moves or changes size while it is over one; then the same
/// with the agent menu open.
#[gpui::test]
fn hovering_never_moves_anything(cx: &mut TestAppContext) {
    use crate::kit::hover::probe;
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        init(cx);
    });
    let store = cx.new(|_| AgentStore::new(demo::agents()));
    let s = store.clone();
    let (view, cx) =
        cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(1500.), gpui::px(900.)));
    view.update_in(cx, |s, window, cx| {
        s.projects.push(pitwall_core::onboarding::project_list::Project {
            path: "/work/empty".into(),
            display: "empty-proj".into(),
            is_git: true,
            added_at: 0,
        });
        s.apply_preset(tree::Preset::G2x2, window, cx);
        let first = s.agents(cx)[0].id.clone();
        s.show_agent(&first, window, cx);
    });
    cx.run_until_parked();
    let n = probe::assert_hover_keeps_layout(cx, "main screen");
    assert!(n > 30, "only {n} hoverable elements drawn");

    view.update_in(cx, |s, window, cx| {
        let a = s.agents(cx)[0].clone();
        s.demo_agent_menu(&a, window, cx);
    });
    cx.run_until_parked();
    probe::assert_hover_keeps_layout(cx, "agent menu");
}

#[gpui::test]
fn big_pieces_are_cached_parts_that_follow_the_layout(cx: &mut TestAppContext) {
    let (view, store, cx) = screen(cx);
    cx.run_until_parked();
    let shown = |view: &Entity<MainScreen>, cx: &mut VisualTestContext| {
        view.read_with(cx, |s, _| {
            let mut ids: Vec<String> = s
                .active_space()
                .unwrap()
                .layout
                .leaves()
                .into_iter()
                .map(|p| p.id)
                .collect();
            ids.sort();
            (ids, s.parts.as_ref().map(|p| p.header_ids()))
        })
    };
    let (panes, headers) = shown(&view, cx);
    assert_eq!(headers, Some(panes), "one header part per shown pane");
    // Another agent shown: its pane gets a header, a closed pane loses it.
    view.update_in(cx, |s, window, cx| s.show_agent("docs", window, cx));
    cx.run_until_parked();
    let (panes, headers) = shown(&view, cx);
    assert_eq!(headers, Some(panes));
    // A store change re-renders the parts with the screen (no stale rows).
    store.update(cx, |s, cx| {
        s.agents[0].status = pitwall_proto::Status::Working;
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |s, cx| assert_eq!(s.agents(cx)[0].status, pitwall_proto::Status::Working));
}

/// A view that stands in for a terminal in the pane body.
struct FakeTerm;

impl Render for FakeTerm {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().size_full()
    }
}

#[gpui::test]
fn a_terminal_frame_replays_the_parts_a_change_redraws_them(cx: &mut TestAppContext) {
    let (view, store, cx) = screen(cx);
    let term = cx.update(|_, cx| cx.new(|_| FakeTerm));
    let t = term.clone();
    cx.update(|_, cx| {
        set_terminal_slot(cx, move |_, _, _| {
            gpui::AnyView::from(t.clone())
                .cached(gpui::StyleRefinement::default().flex_1())
                .into_any_element()
        })
    });
    view.update_in(cx, |s, window, cx| s.show_agent("api", window, cx));
    cx.run_until_parked();
    let renders = |cx: &mut VisualTestContext| {
        view.read_with(cx, |s, cx| {
            let p = s.parts.as_ref().expect("parts");
            (p.sidebar.read(cx).renders, p.topbar.read(cx).renders)
        })
    };
    let before = renders(cx);
    assert!(before.0 > 0 && before.1 > 0, "parts drawn: {before:?}");
    // Output in a pane: the sidebar and the top bar replay their last paint.
    for _ in 0..3 {
        term.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    assert_eq!(renders(cx), before, "a terminal frame re-rendered a part");
    // An agent's status changes: the parts show it.
    store.update(cx, |s, cx| {
        s.agents[0].status = pitwall_proto::Status::Working;
        cx.notify();
    });
    cx.run_until_parked();
    let after = renders(cx);
    assert!(after.0 > before.0 && after.1 > before.1, "{before:?} -> {after:?}");
}
