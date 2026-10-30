//! An agent pane's Preview in a test window, with a fake web view
//! (`crate::preview::fake`): what the view is asked as the pane is drawn,
//! covered, hidden, and as its terminal prints.

use gpui::{AppContext, Modifiers, TestAppContext, VisualTestContext};

use super::*;
use crate::agents::tests::agent;
use crate::preview::fake::{self, Log};
use crate::preview::Server;
use crate::webview::WebEvent;

const VITE: &str = "http://localhost:5173/";

fn setup(
    cx: &mut TestAppContext,
) -> (Entity<MainScreen>, Entity<AgentStore>, fake::Shared, &mut VisualTestContext) {
    let log = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        init(cx);
        fake::install(cx)
    });
    let store = cx.new(|_| {
        let mut web = agent("web", "/work/alpha", "idle", 2, true);
        web.preview_urls = vec![VITE.into()];
        web.preview_seen = 1;
        AgentStore::new(vec![web, agent("api", "/work/alpha", "idle", 1, true)])
    });
    let s = store.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(1500.), gpui::px(900.)));
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    frame(cx);
    (view, store, log, cx)
}

/// Draw a frame (the views are placed after it) and settle.
fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn calls(log: &fake::Shared, cx: &mut VisualTestContext) -> Vec<String> {
    frame(cx);
    Log::take(log)
}

fn set_web(store: &Entity<AgentStore>, cx: &mut VisualTestContext, f: impl FnOnce(&mut AgentView)) {
    store.update(cx, |s, cx| {
        f(s.agents.iter_mut().find(|a| a.id == "web").expect("web"));
        cx.notify();
    });
}

/// Click the pane header's Preview chip.
fn click_chip(cx: &mut VisualTestContext) {
    let chip = cx.debug_bounds("preview-web").expect("the chip");
    cx.simulate_click(chip.center(), Modifiers::none());
}

/// The middle of the toolbar's layout button.
fn layout(cx: &mut VisualTestContext) -> Point<Pixels> {
    cx.debug_bounds("pv-layout-web").expect("the layout button").center()
}

/// The bounds the fake was given last ("bounds x y w h").
fn last_bounds(calls: &[String]) -> Option<(f32, f32, f32, f32)> {
    let b = calls.iter().rev().find(|c| c.starts_with("bounds "))?;
    let v: Vec<f32> = b[7..].split(' ').map(|n| n.parse().unwrap()).collect();
    Some((v[0], v[1], v[2], v[3]))
}

#[gpui::test]
fn preview_opens_beside_its_terminal_and_moves_only_when_its_place_does(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    assert!(Log::take(&log).is_empty(), "nothing made before the click");
    click_chip(cx);
    let c = calls(&log, cx);
    assert_eq!(c[0], format!("create {VITE}"));
    let (x, y, w, h) = last_bounds(&c).expect("placed");
    assert!(c.contains(&"show".to_string()), "{c:?}");
    assert!(x > 500. && w > 200. && h > 300. && y > 60., "the right half under the toolbar: {c:?}");
    assert!(view.read_with(cx, |s, _| s.previews.is_open("web")));
    // Frames where nothing moved ask nothing.
    assert!(calls(&log, cx).is_empty());
    assert!(calls(&log, cx).is_empty());
    // The layout button: on the left (the terminal on the right), then
    // below (remembered in ui.json), then back on the right.
    let at = layout(cx);
    cx.simulate_click(at, Modifiers::none());
    let c = calls(&log, cx);
    let (xl, yl, wl, _) = last_bounds(&c).expect("moved");
    assert!(xl + wl < x && yl == y, "on the left: {c:?}");
    assert!(view.read_with(cx, |s, _| s.ui.preview_left && !s.ui.preview_below));
    let at = layout(cx);
    cx.simulate_click(at, Modifiers::none());
    let c = calls(&log, cx);
    let (_, y2, w2, _) = last_bounds(&c).expect("moved");
    assert!(y2 > y + 100. && w2 > w, "below, full width: {c:?}");
    assert_eq!(c.iter().filter(|c| c.starts_with("bounds")).count(), 1, "once: {c:?}");
    let at = layout(cx);
    cx.simulate_click(at, Modifiers::none());
    let c = calls(&log, cx);
    assert_eq!(last_bounds(&c).map(|b| (b.0, b.1)), Some((x, y)), "on the right again: {c:?}");
    // The chip again closes it, and the view goes.
    click_chip(cx);
    assert_eq!(calls(&log, cx), ["drop"]);
    assert!(view.read_with(cx, |s, _| !s.previews.is_open("web")));
}

#[gpui::test]
fn a_menu_over_the_page_swaps_it_for_its_snapshot(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    click_chip(cx);
    let (x, y, w, h) = last_bounds(&calls(&log, cx)).expect("placed");
    // A context menu opening over the page.
    let at = gpui::point(gpui::px(x + w / 2.), gpui::px(y + h / 2.));
    view.update_in(cx, |s, window, cx| {
        let items = vec![super::menu::MenuItem::new("", "Something", |_, _, _| {})];
        s.open_menu(at, items, window, cx)
    });
    assert_eq!(calls(&log, cx), ["snapshot"], "the page stays until its picture is ready");
    assert_eq!(Log::answer_snapshots(&log), 1);
    assert_eq!(calls(&log, cx), ["hide"]);
    assert!(calls(&log, cx).is_empty(), "covered: settled");
    // The menu closes: the page is back.
    cx.dispatch_action(crate::menu::Dismiss);
    assert_eq!(calls(&log, cx), ["show"]);
    // A menu elsewhere (over the sidebar) leaves it alone.
    view.update_in(cx, |s, window, cx| {
        let items = vec![super::menu::MenuItem::new("", "Something", |_, _, _| {})];
        s.open_menu(gpui::point(gpui::px(20.), gpui::px(120.)), items, window, cx)
    });
    assert!(calls(&log, cx).is_empty());
}

#[gpui::test]
fn the_page_follows_what_its_terminal_prints(cx: &mut TestAppContext) {
    let (view, store, log, cx) = setup(cx);
    click_chip(cx);
    calls(&log, cx);
    // The dev server restarted: its address printed again.
    set_web(&store, cx, |a| a.preview_seen = 2);
    assert_eq!(calls(&log, cx), ["reload"]);
    // It came back on another port.
    set_web(&store, cx, |a| {
        a.preview_urls = vec!["http://localhost:5174/".into(), VITE.into()];
        a.preview_seen = 3;
    });
    assert_eq!(calls(&log, cx), ["navigate http://localhost:5174/"]);
    // A link inside the page: followed, shown in the toolbar.
    Log::emit(&log, WebEvent::Url("http://localhost:5174/about".into()));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |s, _| s.previews.page_url("web")).as_deref(), Some("http://localhost:5174/about"));
    // The agent stops: "Server stopped" instead of the page.
    set_web(&store, cx, |a| {
        a.running = false;
        a.preview_urls.clear();
    });
    assert_eq!(calls(&log, cx), ["hide"]);
    assert_eq!(view.read_with(cx, |s, _| s.previews.server("web")), Some(Server::Stopped));
    assert!(cx.debug_bounds("preview-pane-web").is_some(), "the preview stays, calmly");
    // Running again and printing its address: back on it.
    set_web(&store, cx, |a| {
        a.running = true;
        a.preview_urls = vec!["http://localhost:5174/".into()];
        a.preview_seen = 1;
    });
    let c = calls(&log, cx);
    assert_eq!(c.first().map(String::as_str), Some("navigate http://localhost:5174/"));
    assert!(c.contains(&"show".to_string()), "{c:?}");
}

#[gpui::test]
fn a_failed_load_shows_calmly_and_the_next_printing_retries(cx: &mut TestAppContext) {
    let (view, store, log, cx) = setup(cx);
    click_chip(cx);
    calls(&log, cx);
    Log::emit(&log, WebEvent::Failed { url: VITE.into(), error: "Could not connect to the server.".into() });
    assert_eq!(calls(&log, cx), ["hide"], "no engine error page");
    assert!(matches!(view.read_with(cx, |s, _| s.previews.server("web")), Some(Server::Unreachable(_))));
    set_web(&store, cx, |a| a.preview_seen = 2);
    let c = calls(&log, cx);
    assert_eq!(c[0], "reload");
    assert!(c.contains(&"show".to_string()), "{c:?}");
}

#[gpui::test]
fn a_hidden_preview_pauses_then_lets_its_view_go(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    click_chip(cx);
    calls(&log, cx);
    Log::emit(&log, WebEvent::Url("http://localhost:5173/settings".into()));
    // Another route: the pane isn't drawn.
    view.update(cx, |s, cx| s.toggle_route(Route::Wall, cx));
    assert_eq!(calls(&log, cx), ["hide"]);
    cx.executor().advance_clock(crate::preview::DROP_AFTER / 2);
    assert!(calls(&log, cx).is_empty());
    cx.executor().advance_clock(crate::preview::DROP_AFTER);
    cx.run_until_parked();
    assert_eq!(Log::take(&log), ["drop"]);
    assert!(view.read_with(cx, |s, _| !s.previews.has_view("web") && s.previews.is_open("web")));
    // Shown again: made again, on the page it was on.
    view.update(cx, |s, cx| s.toggle_route(Route::Wall, cx));
    let c = calls(&log, cx);
    assert_eq!(c[0], "create http://localhost:5173/settings");
    assert!(c.contains(&"show".to_string()), "{c:?}");
}

#[gpui::test]
fn shown_again_before_the_timer_keeps_its_view(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    click_chip(cx);
    calls(&log, cx);
    view.update(cx, |s, cx| s.toggle_route(Route::Wall, cx));
    calls(&log, cx);
    view.update(cx, |s, cx| s.toggle_route(Route::Wall, cx));
    assert_eq!(calls(&log, cx), ["show"]);
    cx.executor().advance_clock(crate::preview::DROP_AFTER * 2);
    cx.run_until_parked();
    assert!(Log::take(&log).is_empty(), "the timer went with the hiding");
}

#[gpui::test]
fn the_preview_goes_with_its_agent(cx: &mut TestAppContext) {
    let (view, store, log, cx) = setup(cx);
    click_chip(cx);
    calls(&log, cx);
    store.update(cx, |s, cx| {
        s.agents.retain(|a| a.id != "web");
        cx.notify();
    });
    assert_eq!(calls(&log, cx), ["drop"]);
    assert!(view.read_with(cx, |s, _| !s.previews.is_open("web")));
}

#[gpui::test]
fn without_a_web_view_the_browser_opens_it(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    log.borrow_mut().fail = Some("No web view on this platform".into());
    click_chip(cx);
    frame(cx);
    assert_eq!(cx.opened_url().as_deref(), Some(VITE));
    view.read_with(cx, |s, _| {
        assert!(!s.previews.is_open("web"));
        assert_eq!(s.toasts.len(), 1, "the reason, once");
    });
    click_chip(cx);
    frame(cx);
    view.read_with(cx, |s, _| assert_eq!(s.toasts.len(), 1, "not again"));
}

#[gpui::test]
fn the_page_takes_the_keys_and_pitwalls_shortcuts_still_work(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    click_chip(cx);
    calls(&log, cx);
    // A click into the page.
    Log::emit(&log, WebEvent::Focused);
    cx.run_until_parked();
    let focus = view.read_with(cx, |s, _| s.previews.focus_of("web").unwrap());
    assert!(cx.update(|window, _| focus.is_focused(window)));
    // ⌘B (Ctrl+Shift+B elsewhere) seen by the page first: the sidebar folds.
    let before = view.read_with(cx, |s, _| s.sidebar_collapsed);
    let key = if cfg!(target_os = "macos") { "cmd-b" } else { "ctrl-shift-b" };
    Log::emit(&log, WebEvent::Shortcut(gpui::Keystroke::parse(key).unwrap()));
    cx.run_until_parked();
    assert_ne!(view.read_with(cx, |s, _| s.sidebar_collapsed), before);
    // Esc: the keys back to the screen, the page let go.
    Log::take(&log);
    Log::emit(&log, WebEvent::Shortcut(gpui::Keystroke::parse("escape").unwrap()));
    cx.run_until_parked();
    assert!(Log::take(&log).contains(&"blur".to_string()));
    let screen = view.read_with(cx, |s, _| s.focus.clone());
    assert!(cx.update(|window, _| screen.is_focused(window)));
    // Back in the page, then a click outside it (the terminal side).
    Log::emit(&log, WebEvent::Focused);
    frame(cx);
    Log::take(&log);
    let page = cx.debug_bounds("preview-pane-web").expect("the page side");
    cx.simulate_click(page.origin - gpui::point(gpui::px(100.), gpui::px(-200.)), Modifiers::none());
    assert!(!cx.update(|window, _| focus.is_focused(window)), "focus moved");
    frame(cx);
    let c = Log::take(&log);
    assert!(c.contains(&"blur".to_string()), "{c:?} {page:?}");
    assert!(!cx.update(|window, _| focus.is_focused(window)));
}

#[gpui::test]
fn only_local_pages_show_in_the_toolbar(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    click_chip(cx);
    calls(&log, cx);
    Log::emit(&log, WebEvent::Url("https://example.com/".into()));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |s, _| s.previews.page_url("web")).as_deref(), Some(VITE));
}
