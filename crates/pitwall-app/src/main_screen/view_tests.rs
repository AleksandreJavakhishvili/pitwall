//! View tiles in a test window, with the adapter's other end driven by
//! the test (`Hub::ends`): opened as a tile, what it is sent drawn (a
//! list with its detail, refused content said), its keys and buttons sent
//! back, maximised and closed; moved to a window of its own and back after
//! a restart; an extension's badge on an agent's row; an older layout
//! read unchanged.

use gpui::{AppContext, Focusable, Modifiers, TestAppContext, VisualTestContext};
use pitwall_core::extensions::{Adapter, Contributes, Found, Manifest, MatchAgents, ViewDef};
use serde_json::json;

use super::tree::{Content, Surface};
use super::*;
use crate::agents::tests::agent;
use crate::extensions::hub::{self, Hub};

/// The platform's Maximise key (⌘⏎ on macOS, Ctrl+Shift+Enter elsewhere).
const MAXIMISE: &str = if cfg!(target_os = "macos") { "cmd-enter" } else { "ctrl-shift-enter" };

const KEY: &str = "demo/runs";

fn manifest() -> Manifest {
    Manifest {
        adapter: Some(Adapter { command: vec!["node".into(), "a.mjs".into()] }),
        match_agents: Some(MatchAgents { agent: "{project}".into(), session: "{agent_id}".into() }),
        contributes: Contributes { views: vec![ViewDef { id: "runs".into(), title: "Runs".into(), icon: Some("list".into()) }], badges: true, ..Default::default() },
        ..Manifest::named("demo", "Demo")
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<MainScreen>, Entity<Hub>, &mut VisualTestContext) {
    let h = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        init(cx);
        crate::extensions::set_found(vec![Found { folder: "demo".into(), manifest: Ok(manifest()) }], cx);
        crate::extensions::follow(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("demo", true));
        hub::hub(cx)
    });
    let store = cx.new(|_| AgentStore::new(vec![agent("web", "/work/alpha", "idle", 2, true), agent("api", "/work/alpha", "idle", 1, true)]));
    let s = store.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(1500.), gpui::px(900.)));
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    frame(cx);
    say(&h, json!({"type": "hello", "protocol": 2}), cx);
    (view, h, cx)
}

fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn say(h: &Entity<Hub>, line: serde_json::Value, cx: &mut VisualTestContext) {
    h.read_with(cx, |h, _| h.ends["demo"].say(line));
    frame(cx);
}

fn sent(h: &Entity<Hub>, cx: &mut VisualTestContext) -> Vec<serde_json::Value> {
    h.read_with(cx, |h, _| h.ends.get("demo").map(|e| e.sent()).unwrap_or_default())
}

fn view_panes(view: &Entity<MainScreen>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |s, _| s.ui.spaces.iter().map(|sp| sp.layout.leaves().iter().filter(|p| p.view.as_deref() == Some(KEY)).count()).sum())
}

fn list() -> serde_json::Value {
    json!({"type": "view", "view": "runs", "root": {"kind": "list", "sections": [{"title": "Today", "items": [
        {"id": "r1", "title": "Build #12", "subtitle": "main", "tone": "bad", "badges": [{"text": "failed", "tone": "bad"}], "actions": [{"id": "retry", "label": "Retry", "primary": true}]},
        {"id": "r2", "title": "Build #11", "tone": "good"}
    ]}], "detail": {"kind": "detail", "title": "Build #12", "fields": [{"label": "Branch", "value": "main"}], "body": "It **failed**."}}})
}

/// "Open Runs": a tile; the adapter told to show it; the list and its
/// detail drawn; ↓ and Enter press the selected row's main button;
/// ⌘⏎ maximises it; a form is refused in the tile; closed, it is hidden.
#[gpui::test]
fn a_view_opens_as_a_tile_and_goes_both_ways(cx: &mut TestAppContext) {
    let (view, h, cx) = setup(cx);
    view.update_in(cx, |s, window, cx| s.open_view("demo", "runs", None, window, cx));
    frame(cx);
    assert_eq!(view_panes(&view, cx), 1);
    assert!(sent(&h, cx).iter().any(|l| *l == json!({"type": "show", "view": "runs"})));
    assert!(cx.debug_bounds("view-loading").is_some(), "loading until it is sent");
    say(&h, list(), cx);
    assert!(cx.debug_bounds("view-row-r1").is_some());
    assert!(cx.debug_bounds("view-row-r2").is_some());
    assert!(cx.debug_bounds("view-detail").is_some(), "the detail beside the list");
    // Keys: the first row, then its main button.
    cx.simulate_keystrokes("down");
    frame(cx);
    assert!(sent(&h, cx).iter().any(|l| *l == json!({"type": "select", "view": "runs", "item": "r1"})));
    assert!(cx.debug_bounds("view-action-r1-retry").is_some(), "the selected row's buttons");
    cx.simulate_keystrokes("enter");
    frame(cx);
    let action = sent(&h, cx).into_iter().rev().find(|l| l["type"] == "action").expect("an action");
    assert_eq!((action["item"].as_str(), action["action"].as_str()), (Some("r1"), Some("retry")));
    say(&h, json!({"type": "result", "req": action["req"], "ok": false, "message": "No runner free"}), cx);
    assert!(cx.debug_bounds("view-note").is_some(), "why it failed");
    // Maximised like any tile.
    cx.simulate_keystrokes(MAXIMISE);
    frame(cx);
    assert!(view.read_with(cx, |s, _| s.active_space().and_then(|sp| sp.maximized_pane_id.clone())).is_some());
    cx.simulate_keystrokes(MAXIMISE);
    frame(cx);
    // Something it can't draw: said in the tile.
    say(&h, json!({"type": "view", "view": "runs", "root": {"kind": "table"}}), cx);
    assert!(cx.debug_bounds("view-refused").is_some());
    // Opened again: focused where it is, never two.
    view.update_in(cx, |s, window, cx| s.open_view("demo", "runs", None, window, cx));
    frame(cx);
    assert_eq!(view_panes(&view, cx), 1);
    cx.update(|_, cx| super::view_tile::close(KEY, cx));
    frame(cx);
    assert_eq!(view_panes(&view, cx), 0);
    assert_eq!(sent(&h, cx).last(), Some(&json!({"type": "hide", "view": "runs"})));
}

/// A badge an extension puts on an agent's row (by its `matchAgents`
/// names), and a click on it opens the view it names.
#[gpui::test]
fn a_badge_shows_on_its_agents_row_and_opens_its_view(cx: &mut TestAppContext) {
    let (view, h, cx) = setup(cx);
    assert!(cx.debug_bounds("ext-mark-agent-web-0").is_none());
    say(&h, json!({"type": "badges", "badges": [{"agent": {"from": {"agent": "alpha", "session": "web"}}, "note": "failing", "tone": "bad", "open": {"view": "runs", "item": "r1"}}]}), cx);
    let mark = cx.debug_bounds("ext-mark-agent-web-0").expect("on web's row");
    assert!(cx.debug_bounds("ext-mark-agent-api-0").is_none(), "not api's");
    cx.simulate_click(mark.center(), Modifiers::none());
    frame(cx);
    assert_eq!(view_panes(&view, cx), 1);
    let v = cx.update(|_, cx| super::view_tile::get(KEY, cx)).expect("its tile");
    assert_eq!(v.read_with(cx, |v, _| v.selected().map(str::to_string)).as_deref(), Some("r1"));
    // Stopped: its badges go.
    h.read_with(cx, |h, _| {
        let _ = h.ends["demo"].events.unbounded_send(crate::extensions::adapter::Event::Stopped("It exited.".into()));
    });
    frame(cx);
    assert!(cx.debug_bounds("ext-mark-agent-web-0").is_none());
}

/// A layout saved before views (panes with `inbox`) reads the same, and
/// one with a view keeps it; an extension window kept under its key.
#[test]
fn layouts_keep_inboxes_and_views() {
    let old = r#"{"type":"split","id":"s","dir":"row","children":[{"type":"pane","id":"a","agentId":"x"},{"type":"pane","id":"b","agentId":null,"inbox":"demo"}],"sizes":[0.5,0.5]}"#;
    let n: super::tree::Node = serde_json::from_str(old).unwrap();
    assert_eq!(serde_json::to_string(&n).unwrap(), old, "written back unchanged");
    assert_eq!(n.find_pane("b").unwrap().content(), Some(Content::Inbox("demo".into())));
    let n = super::tree::Node::content_pane(&Surface::parse(KEY).content());
    let json = serde_json::to_value(&n).unwrap();
    assert_eq!(json["view"], json!(KEY));
    assert!(json.get("inbox").is_none());
    let back: super::tree::Node = serde_json::from_value(json).unwrap();
    assert_eq!(back.leaves()[0].content(), Some(Content::View(KEY.into())));
    assert_eq!(Surface::parse("demo"), Surface::Inbox("demo".into()));
    assert_eq!(Surface::parse(KEY), Surface::View("demo".into(), "runs".into()));
    let ui = UiState::default().surface_to_window(&Surface::parse(KEY));
    assert_eq!(ui.extension("demo").view_windows, ["runs"]);
    assert!(!ui.extension("demo").window, "not the inbox");
    assert!(!ui.clone().close_surface(&Surface::parse(KEY)).extensions.contains_key("demo"));
}

fn main_screen(cx: &mut gpui::App) -> Entity<MainScreen> {
    let h = crate::windows::main_window(crate::windows::MAIN, cx).unwrap();
    h.update(cx, |v, _, _| v.screen().unwrap()).unwrap()
}

fn frames(cx: &mut TestAppContext) {
    for _ in 0..2 {
        cx.update(|cx| {
            for w in cx.windows() {
                let _ = w.update(cx, |_, window, _| window.refresh());
            }
        });
        cx.run_until_parked();
    }
}

/// Out into a window of its own and back into the layout: the same tile;
/// left in its window, it opens in one again after a restart.
#[gpui::test]
fn a_view_moves_to_its_own_window_and_comes_back_after_a_restart(cx: &mut TestAppContext) {
    let h = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::set_found(vec![Found { folder: "demo".into(), manifest: Ok(manifest()) }], cx);
        crate::extensions::follow(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("demo", true));
        let store = cx.new(|_| AgentStore::new(vec![agent("api", "/work/alpha", "idle", 1, true)]));
        let show_main = super::inbox_tests::show_main;
        cx.set_global(super::inbox_tests::Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: crate::ui::Content::Live { store: store.clone() }, root: None, main: None });
        crate::windows::init(crate::ui::Content::Live { store }, None, show_main, cx);
        show_main(cx);
        crate::windows::restore(cx);
        hub::hub(cx)
    });
    frames(cx);
    h.read_with(cx, |h, _| h.ends["demo"].say(json!({"type": "hello", "protocol": 2})));
    frames(cx);
    let main = cx.update(|cx| crate::windows::main_window(crate::windows::MAIN, cx).unwrap());
    main.update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.open_view("demo", "runs", None, window, cx))).unwrap();
    frames(cx);
    let tile = cx.update(|cx| super::view_tile::get(KEY, cx)).expect("its tile");
    let same = |cx: &mut TestAppContext| cx.update(|cx| super::view_tile::get(KEY, cx)).map(|e| e.entity_id()) == Some(tile.entity_id());

    cx.update(|cx| super::inbox_window::pop_out(KEY, None, true, cx)).expect("popped out");
    frames(cx);
    cx.update(|cx| {
        assert!(crate::windows::inbox_window(KEY, cx).is_some(), "a window");
        assert!(crate::windows::inbox_window("demo", cx).is_none(), "not the inbox's");
        assert_eq!(crate::ui_state::get(cx).extension("demo").view_windows, ["runs"]);
        assert_eq!(super::inbox_window::described(KEY, cx), "a window of its own");
        assert_eq!(super::inbox_window::title(KEY, cx), "Demo — Runs");
    });
    assert!(same(cx));
    let w = cx.update(|cx| crate::windows::inbox_window(KEY, cx)).unwrap();
    let focused = w.update(cx, |_, window, cx| tile.read(cx).focus_handle(cx).is_focused(window)).unwrap();
    assert!(focused, "its window's view has the keys");
    // Back into the layout: the window closes, the same tile.
    cx.update(|cx| super::inbox_window::place(KEY, crate::explorer::dock::FilesPlace::Next(super::tree::Side::Right), Some("api"), cx)).expect("docked");
    frames(cx);
    cx.update(|cx| {
        assert!(crate::windows::inbox_window(KEY, cx).is_none());
        assert!(crate::ui_state::get(cx).tile_of(&Content::View(KEY.into())).is_some());
        assert!(super::inbox_window::described(KEY, cx).starts_with("the view tile “runs”"));
    });
    assert!(same(cx));
    // Left in a window, "restarted": open again.
    cx.update(|cx| super::inbox_window::pop_out(KEY, None, false, cx)).expect("popped out");
    frames(cx);
    cx.update(|cx| {
        if let Some(h) = crate::windows::inbox_window(KEY, cx) {
            let _ = h.update(cx, |w, window, _| {
                w.closing();
                window.remove_window();
            });
        }
    });
    cx.run_until_parked();
    cx.update(|cx| assert!(crate::windows::inbox_window(KEY, cx).is_none()));
    cx.update(super::inbox_window::restore);
    frames(cx);
    cx.update(|cx| assert!(crate::windows::inbox_window(KEY, cx).is_some(), "open again"));
    let _ = main_screen;
}
