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
        assert!(b.iter().filter(|k| k.predicate().is_none()).all(|k| k
            .keystrokes()
            .iter()
            .all(|s| !s.modifiers().control || s.modifiers().shift)));
    }
}
