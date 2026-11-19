//! Web tiles in a test window, with a fake web view
//! (`crate::preview::fake`): what the view is asked as the tile is opened,
//! drawn, covered, dragged, hidden and closed; an agent's Preview as its
//! terminal prints; a page typed into the address field.

use gpui::{AppContext, Modifiers, MouseButton, TestAppContext, VisualTestContext};

use super::*;
use crate::agents::tests::agent;
use crate::preview::fake::{self, Log};
use crate::preview::Server;
use crate::webview::WebEvent;
use super::workspace::{FilesSpot, WebTile};

/// A debug selector made at run time.
fn sel(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

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

/// Agent "web"'s Preview tile.
fn preview(view: &Entity<MainScreen>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |s, _| s.ui.preview_of("web")).expect("a Preview tile")
}

/// The bounds the fake was given last ("bounds x y w h").
fn last_bounds(calls: &[String]) -> Option<(f32, f32, f32, f32)> {
    let b = calls.iter().rev().find(|c| c.starts_with("bounds "))?;
    let v: Vec<f32> = b[7..].split(' ').map(|n| n.parse().unwrap()).collect();
    Some((v[0], v[1], v[2], v[3]))
}

/// The layout's web tiles.
fn web_ids(view: &Entity<MainScreen>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |s, _| s.active_space().map(|sp| sp.layout.web_ids()).unwrap_or_default())
}

#[gpui::test]
fn preview_opens_in_a_tile_beside_its_terminal_and_moves_only_when_its_place_does(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    assert!(Log::take(&log).is_empty(), "nothing made before the click");
    click_chip(cx);
    let c = calls(&log, cx);
    assert_eq!(c[0], format!("create {VITE}"));
    let (x, y, w, h) = last_bounds(&c).expect("placed");
    assert!(c.contains(&"show".to_string()), "{c:?}");
    assert!(x > 500. && w > 200. && h > 300. && y > 60., "a tile on the right, under its bar: {c:?}");
    let id = preview(&view, cx);
    // A tile of the layout, not inside the terminal's: the terminal keeps
    // its own tile, the page has its own.
    view.read_with(cx, |s, _| {
        let layout = &s.active_space().unwrap().layout;
        assert_eq!(layout.leaves().len(), 2);
        assert!(layout.find_by_agent("web").is_some());
        assert!(s.webs.is_open(&id));
        assert_eq!(s.ui.web_tiles[&id].agent.as_deref(), Some("web"));
    });
    let term = cx.debug_bounds(sel(format!("web-pane-{id}"))).expect("the page's tile");
    assert!(term.origin.x > gpui::px(500.));
    // Frames where nothing moved ask nothing.
    assert!(calls(&log, cx).is_empty());
    assert!(calls(&log, cx).is_empty());
    // The chip again closes it, and the view goes.
    click_chip(cx);
    assert_eq!(calls(&log, cx), ["drop"]);
    assert!(view.read_with(cx, |s, _| !s.webs.is_open(&id) && s.ui.preview_of("web").is_none()));
    assert!(web_ids(&view, cx).is_empty());
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

/// The tile dragged by its bar to the left edge of the space: a snapshot
/// while it is dragged (the drop zones show), the layout changed when let
/// go, the page live again in its new place.
#[gpui::test]
fn a_web_tile_dragged_by_its_bar_moves_in_the_layout(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    click_chip(cx);
    let (x, ..) = last_bounds(&calls(&log, cx)).expect("placed");
    let id = preview(&view, cx);
    // Pressed on the bar's grip, then moved: a drag.
    let grab = cx.debug_bounds(sel(format!("web-grip-{id}"))).expect("the grip").center();
    cx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(grab + gpui::point(gpui::px(20.), gpui::px(20.)), MouseButton::Left, Modifiers::none());
    assert!(cx.update(|_, cx| cx.has_active_drag()), "a drag started");
    assert_eq!(calls(&log, cx), ["snapshot"], "the page gives way while dragged");
    Log::answer_snapshots(&log);
    assert_eq!(calls(&log, cx), ["hide"]);
    // Over the space's left edge: its drop zone says where.
    let area = cx.debug_bounds("files-drop-edge");
    assert!(area.is_none(), "no edge zone before it is over one");
    let terminal = cx.debug_bounds("pane-web-agent").map(|b| b.origin.x).unwrap_or(gpui::px(0.));
    let _ = terminal;
    let left = view.read_with(cx, |s, _| s.files_boxes_area()).expect("the area");
    let edge = gpui::point(left.left() + gpui::px(4.), left.center().y);
    cx.simulate_mouse_move(edge, MouseButton::Left, Modifiers::none());
    frame(cx);
    let hint = view.read_with(cx, |s, _| s.files_hint().map(|h| (h.spot.clone(), h.label.clone())));
    assert!(matches!(hint, Some((FilesSpot::Edge(tree::Side::Left), _))), "{hint:?}");
    assert!(hint.unwrap().1.starts_with("Web page"), "said as a web page");
    assert!(cx.debug_bounds("files-drop-edge").is_some(), "the zone is drawn (under no live page)");
    cx.simulate_mouse_up(edge, MouseButton::Left, Modifiers::none());
    let c = calls(&log, cx);
    let (x2, ..) = last_bounds(&c).expect("moved");
    assert!(x2 < x, "now on the left: {c:?}");
    assert!(c.contains(&"show".to_string()), "live again: {c:?}");
    view.read_with(cx, |s, _| {
        let layout = &s.active_space().unwrap().layout;
        let first = layout.leaves()[0].clone();
        assert_eq!(first.web.as_deref(), Some(id.as_str()), "the leftmost tile");
        assert!(s.files_hint().is_none());
    });
}

#[gpui::test]
fn the_page_follows_what_its_terminal_prints(cx: &mut TestAppContext) {
    let (view, store, log, cx) = setup(cx);
    click_chip(cx);
    calls(&log, cx);
    let id = preview(&view, cx);
    // The dev server restarted: its address printed again.
    set_web(&store, cx, |a| a.preview_seen = 2);
    assert_eq!(calls(&log, cx), ["reload"]);
    // It came back on another port.
    set_web(&store, cx, |a| {
        a.preview_urls = vec!["http://localhost:5174/".into(), VITE.into()];
        a.preview_seen = 3;
    });
    assert_eq!(calls(&log, cx), ["navigate http://localhost:5174/"]);
    // A link inside the page: followed, shown in the bar, kept for a restart.
    Log::emit(&log, WebEvent::Url("http://localhost:5174/about".into()));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |s, _| s.webs.page_url(&id)).as_deref(), Some("http://localhost:5174/about"));
    assert_eq!(view.read_with(cx, |s, _| s.ui.web_tiles[&id].url.clone()), "http://localhost:5174/about");
    // The agent stops: "Server stopped" instead of the page.
    set_web(&store, cx, |a| {
        a.running = false;
        a.preview_urls.clear();
    });
    assert_eq!(calls(&log, cx), ["hide"]);
    assert_eq!(view.read_with(cx, |s, _| s.webs.server(&id)), Some(Server::Stopped));
    assert!(cx.debug_bounds(sel(format!("web-pane-{id}"))).is_some(), "the tile stays, calmly");
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
    let id = preview(&view, cx);
    Log::emit(&log, WebEvent::Failed { url: VITE.into(), error: "Could not connect to the server.".into() });
    assert_eq!(calls(&log, cx), ["hide"], "no engine error page");
    assert!(matches!(view.read_with(cx, |s, _| s.webs.server(&id)), Some(Server::Unreachable(_))));
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
    let id = preview(&view, cx);
    Log::emit(&log, WebEvent::Url("http://localhost:5173/settings".into()));
    // Another route: the tile isn't drawn.
    view.update(cx, |s, cx| s.toggle_route(Route::Wall, cx));
    assert_eq!(calls(&log, cx), ["hide"]);
    cx.executor().advance_clock(crate::preview::DROP_AFTER / 2);
    assert!(calls(&log, cx).is_empty());
    cx.executor().advance_clock(crate::preview::DROP_AFTER);
    cx.run_until_parked();
    assert_eq!(Log::take(&log), ["drop"]);
    assert!(view.read_with(cx, |s, _| !s.webs.has_view(&id) && s.webs.is_open(&id)));
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
    let id = preview(&view, cx);
    store.update(cx, |s, cx| {
        s.agents.retain(|a| a.id != "web");
        cx.notify();
    });
    assert_eq!(calls(&log, cx), ["drop"]);
    assert!(view.read_with(cx, |s, _| !s.webs.is_open(&id) && s.ui.web_tiles.is_empty()));
    assert!(web_ids(&view, cx).is_empty());
}

#[gpui::test]
fn without_a_web_view_the_browser_opens_it(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    log.borrow_mut().fail = Some("No web view on this platform".into());
    click_chip(cx);
    frame(cx);
    assert_eq!(cx.opened_url().as_deref(), Some(VITE));
    view.read_with(cx, |s, _| {
        assert!(s.ui.preview_of("web").is_none(), "no tile");
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
    let id = preview(&view, cx);
    // A click into the page.
    Log::emit(&log, WebEvent::Focused);
    cx.run_until_parked();
    let focus = view.read_with(cx, |s, _| s.webs.focus_of(&id).unwrap());
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
    // Back in the page, then a click outside it (the terminal's tile).
    Log::emit(&log, WebEvent::Focused);
    frame(cx);
    Log::take(&log);
    let page = cx.debug_bounds(sel(format!("web-pane-{id}"))).expect("the page's tile");
    cx.simulate_click(page.origin - gpui::point(gpui::px(100.), gpui::px(-200.)), Modifiers::none());
    assert!(!cx.update(|window, _| focus.is_focused(window)), "focus moved");
    frame(cx);
    let c = Log::take(&log);
    assert!(c.contains(&"blur".to_string()), "{c:?} {page:?}");
}

#[gpui::test]
fn only_local_pages_show_in_a_previews_bar(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    click_chip(cx);
    calls(&log, cx);
    let id = preview(&view, cx);
    Log::emit(&log, WebEvent::Url("https://example.com/".into()));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |s, _| s.webs.page_url(&id)).as_deref(), Some(VITE));
}

// ── pages the user opens ─────────────────────────────────────────────────

/// ⌘L (Ctrl+Shift+L elsewhere).
fn open_web(cx: &mut VisualTestContext) {
    cx.dispatch_action(OpenWebPage);
    frame(cx);
}

/// The one web tile that isn't a Preview.
fn page_tile(view: &Entity<MainScreen>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |s, _| s.ui.web_tiles.iter().find(|(_, t)| t.agent.is_none()).map(|(id, _)| id.clone()))
        .expect("a web page tile")
}

/// Type `text` into tile `id`'s address field and press Enter.
fn go(view: &Entity<MainScreen>, id: &str, text: &str, cx: &mut VisualTestContext) {
    let field = view.read_with(cx, |s, _| s.webs.field_of(id)).expect("its field");
    field.update(cx, |f, cx| f.set_text(text, cx));
    field.update(cx, |_, cx| cx.emit(crate::kit::InputEvent::Submit));
    frame(cx);
}

#[gpui::test]
fn open_web_page_puts_a_tile_right_of_the_focused_one_and_takes_the_keys(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    open_web(cx);
    let id = page_tile(&view, cx);
    assert!(Log::take(&log).is_empty(), "no web view before an address");
    view.read_with(cx, |s, _| {
        let layout = &s.active_space().unwrap().layout;
        assert_eq!(layout.leaves().len(), 2);
        assert_eq!(layout.leaves()[1].web.as_deref(), Some(id.as_str()), "right of the terminal");
        assert!(layout.find_by_agent("web").is_some(), "the terminal stays");
    });
    // Its address field has the keys; typing goes there.
    let field = view.read_with(cx, |s, _| s.webs.field_of(&id)).unwrap();
    assert!(cx.update(|window, cx| field.read(cx).is_focused(window)));
    cx.simulate_input("example.com");
    // (Enter: the field's own binding, not bound in this test app.)
    field.update(cx, |_, cx| cx.emit(crate::kit::InputEvent::Submit));
    frame(cx);
    let c = Log::take(&log);
    assert_eq!(c[0], "create https://example.com");
    assert!(c.contains(&"show".to_string()), "{c:?}");
    assert_eq!(view.read_with(cx, |s, _| s.ui.web_tiles[&id].url.clone()), "https://example.com");
    // ⌘L in the tile: its address again, all selected, the page let go.
    Log::emit(&log, WebEvent::Focused);
    cx.run_until_parked();
    Log::take(&log);
    open_web(cx);
    assert!(cx.update(|window, cx| field.read(cx).is_focused(window)));
    assert!(Log::take(&log).contains(&"blur".to_string()));
    assert_eq!(view.read_with(cx, |s, _| s.ui.web_tiles.len()), 1, "no second tile");
}

#[gpui::test]
fn text_that_isnt_an_address_is_said_and_nothing_loads(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    open_web(cx);
    let id = page_tile(&view, cx);
    go(&view, &id, "how to center a div", cx);
    assert!(Log::take(&log).is_empty());
    let note = view.read_with(cx, |s, _| s.webs.error(&id)).expect("said");
    assert!(note.contains("isn't a web address"), "{note}");
    assert!(cx.debug_bounds(sel(format!("web-note-{id}"))).is_some());
    go(&view, &id, "file:///etc/passwd", cx);
    assert!(Log::take(&log).is_empty(), "no file pages");
    go(&view, &id, "localhost:3000", cx);
    assert_eq!(Log::take(&log).first().map(String::as_str), Some("create http://localhost:3000"));
    assert!(view.read_with(cx, |s, _| s.webs.error(&id)).is_none());
    // Somewhere else, in the same view.
    go(&view, &id, "https://example.com/docs", cx);
    assert_eq!(Log::take(&log), ["navigate https://example.com/docs", "focus"]);
}

#[gpui::test]
fn a_page_keeps_its_view_while_hidden_and_closing_frees_it(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    open_web(cx);
    let id = page_tile(&view, cx);
    go(&view, &id, "example.com", cx);
    Log::take(&log);
    view.update(cx, |s, cx| s.toggle_route(Route::Wall, cx));
    assert_eq!(calls(&log, cx), ["hide"]);
    cx.executor().advance_clock(crate::preview::DROP_AFTER * 3);
    cx.run_until_parked();
    assert!(Log::take(&log).is_empty(), "a page you opened stays loaded");
    view.update(cx, |s, cx| s.toggle_route(Route::Wall, cx));
    assert_eq!(calls(&log, cx), ["show"]);
    // Its close button: out of the layout, the view freed.
    let close = cx.debug_bounds(sel(format!("web-close-{id}"))).expect("close");
    cx.simulate_click(close.center(), Modifiers::none());
    assert_eq!(calls(&log, cx), ["drop"]);
    assert!(web_ids(&view, cx).is_empty());
    assert!(view.read_with(cx, |s, _| s.ui.web_tiles.is_empty() && !s.webs.is_open(&id)));
}

#[gpui::test]
fn a_page_load_error_and_a_download_are_said_in_the_tile(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    open_web(cx);
    let id = page_tile(&view, cx);
    go(&view, &id, "example.com", cx);
    Log::take(&log);
    Log::emit(&log, WebEvent::Failed { url: "https://example.com".into(), error: "A server with the specified hostname could not be found.".into() });
    assert_eq!(calls(&log, cx), ["hide"]);
    assert!(view.read_with(cx, |s, _| s.webs.error(&id)).unwrap().contains("hostname"));
    let retry = cx.debug_bounds(sel(format!("web-retry-{id}")));
    assert!(retry.is_none(), "(a button without a debug selector)");
    view.update(cx, |s, cx| s.web_reload(&id, cx));
    let c = calls(&log, cx);
    assert_eq!(c, ["reload", "show"]);
    Log::emit(&log, WebEvent::Download("https://example.com/file.zip".into()));
    cx.run_until_parked();
    assert!(view.read_with(cx, |s, _| s.webs.error(&id)).unwrap().contains("Downloads"));
}

/// Restarted: the tile is in `ui.json` with its page; the first frame
/// that draws it makes its view there.
#[gpui::test]
fn a_page_comes_back_after_a_restart(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    view.update(cx, |s, cx| {
        let space = s.active_space().unwrap().id.clone();
        s.update_ui(|ui| ui.open_web(&space, "kept", WebTile { url: "https://example.com/a".into(), agent: None }, &FilesSpot::Edge(tree::Side::Bottom), None).0, cx);
    });
    let c = calls(&log, cx);
    assert_eq!(c[0], "create https://example.com/a");
    assert!(c.contains(&"show".to_string()), "{c:?}");
    let (_, y, _, _) = last_bounds(&c).unwrap();
    assert!(y > 400., "a row along the bottom: {c:?}");
}

#[gpui::test]
fn with_web_pages_off_no_view_is_made(cx: &mut TestAppContext) {
    let (view, _, log, cx) = setup(cx);
    cx.update(|_, cx| {
        let def = pitwall_proto::settings::find(super::web_tile::SETTING).unwrap();
        crate::ui_state::update(cx, |s| *s = crate::settings::data::with(s, def, &serde_json::Value::Bool(false)));
    });
    frame(cx);
    open_web(cx);
    assert!(view.read_with(cx, |s, _| s.ui.web_tiles.is_empty()), "no tile");
    assert_eq!(view.read_with(cx, |s, _| s.toasts.len()), 1, "said why");
    click_chip(cx);
    frame(cx);
    assert_eq!(cx.opened_url().as_deref(), Some(VITE), "the Preview opens in the browser");
    assert!(Log::take(&log).is_empty());
}
