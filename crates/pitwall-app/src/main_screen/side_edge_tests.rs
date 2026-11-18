//! The sidebar's edge in a test window: dragged with the mouse, held to its
//! range, double-clicked back to the default, saved and read back, and
//! kept through a collapse.

use gpui::{point, AppContext, Modifiers, MouseButton, Point, TestAppContext, VisualTestContext};

use super::*;
use crate::agents::tests::agent;
use crate::theme::{SIDEBAR_MAX_W, SIDEBAR_MIN_W, SIDEBAR_W};

/// A 1500 × 900 window (a docked sidebar) whose `ui.json` holds `width`.
fn setup(cx: &mut TestAppContext, width: Option<f32>) -> (Entity<MainScreen>, &mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        init(cx);
        let store = crate::ui_state::store(cx);
        store.update(cx, |s, cx| {
            let mut next = s.state.clone();
            next.sidebar_width = width;
            s.set(next, cx);
        });
    });
    let store = cx.new(|_| {
        AgentStore::new(vec![
            agent("api", "/work/alpha", "idle", 1, true),
            agent("web", "/work/alpha", "blocked", 2, true),
        ])
    });
    let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(store.clone(), window, cx));
    cx.simulate_resize(gpui::size(px(1500.), px(900.)));
    cx.run_until_parked();
    (view, cx)
}

/// Where the edge's grab bar is drawn (a fresh frame).
fn edge(cx: &mut VisualTestContext) -> gpui::Bounds<Pixels> {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.debug_bounds("sidebar-edge").expect("the sidebar's edge is drawn")
}

/// The sidebar's width now, and the width it was last drawn at.
fn width(view: &Entity<MainScreen>, cx: &mut VisualTestContext) -> (f32, f32) {
    view.update_in(cx, |s, window, _| (f32::from(s.sidebar_width(window)), s.side_drawn_w))
}

fn saved(cx: &mut VisualTestContext) -> Option<f32> {
    cx.update(|_, cx| crate::ui_state::store(cx).read(cx).state.sidebar_width)
}

/// Press the edge's middle, move the pointer by `dx` in steps, let go.
fn drag_edge(cx: &mut VisualTestContext, dx: f32) -> Point<Pixels> {
    let from = edge(cx).center();
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    let to = from + point(px(dx), px(0.));
    for i in 1..=4 {
        cx.simulate_mouse_move(from + point(px(dx * i as f32 / 4.), px(0.)), MouseButton::Left, Modifiers::none());
    }
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    to
}

#[gpui::test]
fn dragging_the_edge_sets_the_width_and_it_is_saved(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, None);
    let default = f32::from(SIDEBAR_W);
    assert_eq!(width(&view, cx), (default, default));
    let bar = edge(cx);
    assert!((f32::from(bar.center().x) - default).abs() <= 1., "the bar straddles the border: {bar:?}");

    // The width follows while the button is down, before it is let go.
    let from = bar.center();
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(from + point(px(60.), px(0.)), MouseButton::Left, Modifiers::none());
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert_eq!(width(&view, cx), (default + 60., default + 60.), "live while dragging");
    assert_eq!(saved(cx), None, "saved once let go");
    cx.simulate_mouse_move(from + point(px(100.), px(0.)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(from + point(px(100.), px(0.)), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();

    assert_eq!(width(&view, cx).0, default + 100.);
    assert_eq!(saved(cx), Some(default + 100.));
    let bar = edge(cx);
    assert!((f32::from(bar.center().x) - (default + 100.)).abs() <= 1., "the bar moved with it: {bar:?}");
    assert_eq!(width(&view, cx).1, default + 100., "the sidebar is drawn that wide");
    // The pointer moving on after the drag changes nothing.
    cx.simulate_mouse_move(point(px(900.), px(400.)), None, Modifiers::none());
    assert_eq!(width(&view, cx).0, default + 100.);
}

#[gpui::test]
fn the_width_stays_in_its_range(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, None);
    drag_edge(cx, -400.);
    assert_eq!(width(&view, cx).0, SIDEBAR_MIN_W);
    assert_eq!(saved(cx), Some(SIDEBAR_MIN_W));
    drag_edge(cx, 1200.);
    assert_eq!(width(&view, cx).0, SIDEBAR_MAX_W, "1500 wide: the cap, under half the window");

    // A narrower window: at most half of it, the user's width kept for
    // when it grows again.
    cx.simulate_resize(gpui::size(px(1100.), px(900.)));
    cx.run_until_parked();
    assert_eq!(width(&view, cx).0, 550.);
    assert_eq!(saved(cx), Some(SIDEBAR_MAX_W));
    cx.simulate_resize(gpui::size(px(1500.), px(900.)));
    cx.run_until_parked();
    assert_eq!(width(&view, cx).0, SIDEBAR_MAX_W);
}

#[gpui::test]
fn a_double_click_on_the_edge_puts_the_default_back(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, Some(420.));
    assert_eq!(width(&view, cx).0, 420., "read from ui.json when the window opens");
    let at = edge(cx).center();
    for clicks in 1..=2 {
        cx.simulate_event(gpui::MouseDownEvent {
            position: at,
            button: MouseButton::Left,
            modifiers: Modifiers::none(),
            click_count: clicks,
            first_mouse: false,
        });
        cx.simulate_event(gpui::MouseUpEvent {
            position: at,
            button: MouseButton::Left,
            modifiers: Modifiers::none(),
            click_count: clicks,
        });
    }
    cx.run_until_parked();
    assert_eq!(width(&view, cx).0, f32::from(SIDEBAR_W));
    assert_eq!(saved(cx), None, "the default isn't written");
}

#[gpui::test]
fn collapsing_and_expanding_keeps_the_width(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, None);
    drag_edge(cx, 120.);
    let w = f32::from(SIDEBAR_W) + 120.;
    view.update_in(cx, |s, window, cx| s.toggle_sidebar(window, cx));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(view.read_with(cx, |s, _| s.sidebar_collapsed));
    assert!(cx.debug_bounds("sidebar-edge").is_none(), "no edge without a sidebar");
    view.update_in(cx, |s, window, cx| s.toggle_sidebar(window, cx));
    assert_eq!(width(&view, cx).0, w);
    assert!((f32::from(edge(cx).center().x) - w).abs() <= 1.);
    assert_eq!(saved(cx), Some(w));
}

#[gpui::test]
fn the_saved_width_reads_back_from_ui_json(cx: &mut TestAppContext) {
    let mut s = super::workspace::UiState { sidebar_width: Some(388.), ..Default::default() };
    let text = serde_json::to_value(&s).unwrap();
    assert_eq!(text["sidebarWidth"], 388.);
    assert_eq!(super::workspace::UiState::sanitize(text).sidebar_width, Some(388.));
    s.sidebar_width = None;
    let text = serde_json::to_value(&s).unwrap();
    assert!(text.get("sidebarWidth").is_none(), "the default isn't written");
    // Anything else there: the default.
    let odd = serde_json::json!({"v": 1, "sidebarWidth": "wide"});
    assert_eq!(super::workspace::UiState::sanitize(odd).sidebar_width, None);

    // A window opened with it.
    let (view, cx) = setup(cx, Some(388.));
    assert_eq!(width(&view, cx).0, 388.);
    // The debug hook drags it the same way.
    view.update_in(cx, |s, window, cx| s.debug_sidebar("300", window, cx));
    assert_eq!(width(&view, cx).0, 300.);
    assert_eq!(saved(cx), Some(300.));
    view.update_in(cx, |s, window, cx| s.debug_sidebar("reset", window, cx));
    assert_eq!(width(&view, cx).0, f32::from(SIDEBAR_W));
}

#[test]
fn the_width_is_clamped() {
    use crate::theme::sidebar_width;
    assert_eq!(sidebar_width(None, 1500.), SIDEBAR_W);
    assert_eq!(sidebar_width(Some(10.), 1500.), px(SIDEBAR_MIN_W));
    assert_eq!(sidebar_width(Some(5000.), 3000.), px(SIDEBAR_MAX_W));
    assert_eq!(sidebar_width(Some(600.), 1000.), px(500.));
    // A window too narrow for even the narrowest: that one.
    assert_eq!(sidebar_width(Some(300.), 300.), px(SIDEBAR_MIN_W));
    assert_eq!(sidebar_width(Some(f32::NAN), 1500.), SIDEBAR_W);
}
