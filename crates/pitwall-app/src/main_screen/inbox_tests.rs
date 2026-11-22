//! Inbox tiles in a test window, with the adapter's other end driven by
//! the test (`Hub::ends`): opened with ⌘I as a tile and focused again,
//! its threads drawn, an answer shown at once and taken back when it
//! fails, a narrow tile taking turns, the states, an agent's ask badge,
//! and the tile back from `ui.json`.

use gpui::{AppContext, Focusable, Modifiers, TestAppContext, VisualTestContext};
use pitwall_core::extensions::{Adapter, Found, Manifest, MatchAgents};
use serde_json::json;

use super::inbox_tile::{Filter, InboxView};
use super::tree::Content;
use super::*;
use crate::agents::tests::agent;
use crate::extensions::hub::{self, Hub, Run};

fn sel(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn manifest() -> Manifest {
    Manifest {
        adapter: Some(Adapter { command: vec!["node".into(), "a.mjs".into()] }),
        match_agents: Some(MatchAgents { agent: "{project}".into(), session: "{agent_id}".into() }),
        icon: Some("inbox".into()),
        ..Manifest::named("demo", "Demo")
    }
}

fn setup(cx: &mut TestAppContext, width: f32) -> (Entity<MainScreen>, Entity<Hub>, &mut VisualTestContext) {
    let h = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        init(cx);
        crate::extensions::set_found(vec![Found { folder: "demo".into(), manifest: Ok(manifest()) }], cx);
        crate::extensions::follow(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("demo", true));
        hub::hub(cx)
    });
    let store = cx.new(|_| AgentStore::new(vec![agent("web", "/work/alpha", "idle", 2, true), agent("api", "/work/alpha", "idle", 1, true)]));
    let s = store.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(900.)));
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    frame(cx);
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

/// The adapter says hello and lists two threads: one from agent "web"
/// (project alpha) with a blocking ask, one with nothing open.
fn connect(h: &Entity<Hub>, cx: &mut VisualTestContext) {
    let _ = sent(h, cx);
    say(h, json!({"type": "hello", "protocol": 1}), cx);
    say(h, json!({"type": "status", "state": "ready"}), cx);
    say(
        h,
        json!({"type": "threads", "threads": [
            {"id": "t1", "title": "Which database?", "from": {"agent": "alpha", "session": "web"}, "unread": 1, "openAsks": 1, "blocking": true, "lastActivity": "2026-01-02T10:00:00Z"},
            {"id": "t2", "title": "Done with the refactor", "from": {"agent": "alpha", "session": "api"}, "lastActivity": "2026-01-03T10:00:00Z"}
        ]}),
        cx,
    );
}

fn detail_t1(open: bool) -> serde_json::Value {
    json!({"type": "thread", "thread": {"id": "t1", "title": "Which database?", "from": {"agent": "alpha", "session": "web"}, "canReply": true, "messages": [
        {"id": "m1", "author": {"kind": "agent", "name": "alpha"}, "time": "2026-01-02T10:00:00Z", "text": "I need to pick a **database**.", "detail": "| a | b |\n|---|---|\n| 1 | 2 |",
         "asks": [{"number": 1, "question": "Postgres or SQLite?", "open": open, "blocking": true, "suggestions": [{"label": "Postgres", "description": "Shared, for production"}, {"label": "SQLite", "description": "One file"}]}]}
    ]}})
}

fn inbox(view: &Entity<MainScreen>, cx: &mut VisualTestContext) -> Entity<InboxView> {
    view.read_with(cx, |s, _| s.inbox("demo")).expect("an inbox view")
}

fn inbox_panes(view: &Entity<MainScreen>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |s, _| s.active_space().unwrap().layout.leaves().iter().filter(|p| p.inbox.is_some()).count())
}

#[gpui::test]
fn the_inbox_opens_as_a_tile_and_answers_at_once(cx: &mut TestAppContext) {
    let (view, h, cx) = setup(cx, 2000.);
    connect(&h, cx);
    cx.dispatch_action(OpenInbox);
    frame(cx);
    view.read_with(cx, |s, _| {
        let layout = &s.active_space().unwrap().layout;
        assert_eq!(layout.leaves().len(), 2);
        assert_eq!(layout.leaves()[1].content(), Some(Content::Inbox("demo".into())), "right of the terminal");
    });
    let v = inbox(&view, cx);
    assert!(cx.update(|window, cx| v.read(cx).focus_handle(cx).is_focused(window)), "it has the keys");
    // Again: the same tile, focused.
    cx.dispatch_action(OpenInbox);
    frame(cx);
    assert_eq!(inbox_panes(&view, cx), 1, "one tile per extension");

    // Needs you: only the thread that asks.
    assert!(cx.debug_bounds("inbox-row-t1").is_some());
    assert!(cx.debug_bounds("inbox-row-t2").is_none());
    v.update(cx, |v, cx| v.set_filter(Filter::All, cx));
    frame(cx);
    assert!(cx.debug_bounds("inbox-row-t2").is_some());

    // ↓ shows the first (the blocking one): opened, remembered, marked read.
    cx.simulate_keystrokes("down");
    frame(cx);
    assert_eq!(v.read_with(cx, |v, _| v.selected().map(str::to_string)).as_deref(), Some("t1"));
    assert_eq!(sent(&h, cx), [json!({"type": "open", "thread": "t1"})]);
    assert_eq!(cx.update(|_, cx| crate::ui_state::get(cx).extension("demo").selected).as_deref(), Some("t1"));
    say(&h, detail_t1(true), cx);
    let marked = sent(&h, cx);
    assert_eq!(marked.len(), 0, "read already: {marked:?}");
    assert!(cx.debug_bounds("inbox-ask-1").is_some(), "the ask is a card");
    assert!(cx.debug_bounds("inbox-pick-1-1").is_some());
    assert!(cx.debug_bounds("inbox-details-m1").is_some(), "its details fold");

    // 2 picks SQLite: sent, and shown answered before the adapter says so.
    cx.simulate_keystrokes("2");
    frame(cx);
    let s = sent(&h, cx);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0]["type"], "answer");
    assert_eq!(s[0]["picks"], json!([1]));
    let req = s[0]["req"].as_u64().unwrap();
    assert!(cx.debug_bounds("inbox-ask-1").is_none(), "answered at once");

    // It failed: the card is back, with why.
    say(&h, json!({"type": "result", "req": req, "ok": false, "message": "That question isn't open any more."}), cx);
    assert!(cx.debug_bounds("inbox-ask-1").is_some(), "rolled back");
    assert!(cx.debug_bounds("inbox-note").is_some(), "with why");

    // A click on a suggestion this time; it goes, and the thread says so.
    let pick = cx.debug_bounds("inbox-pick-1-0").expect("Postgres");
    cx.simulate_click(pick.center(), Modifiers::none());
    frame(cx);
    let s = sent(&h, cx);
    assert_eq!(s[0]["picks"], json!([0]));
    let req = s[0]["req"].as_u64().unwrap();
    say(&h, json!({"type": "result", "req": req, "ok": true}), cx);
    assert!(cx.debug_bounds("inbox-ask-1").is_none(), "still answered until the thread says so");
    say(&h, detail_t1(false), cx);
    assert!(cx.debug_bounds("inbox-ask-1").is_none());
    assert!(cx.debug_bounds("inbox-note").is_none());

    // E archives: out of the list at once.
    cx.update(|window, cx| window.focus(&v.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("e");
    frame(cx);
    let s = sent(&h, cx);
    assert!(s.iter().any(|m| m["type"] == "archive" && m["thread"] == "t1"), "{s:?}");
    assert!(cx.debug_bounds("inbox-row-t1").is_none());

    // The close button: out of the layout.
    let close = cx.debug_bounds("inbox-close").expect("its close button");
    let _ = close;
    view.update(cx, |s, cx| s.close_inbox("demo", cx));
    frame(cx);
    assert_eq!(inbox_panes(&view, cx), 0);
    assert!(view.read_with(cx, |s, _| s.inbox("demo").is_none()));
}

#[gpui::test]
fn a_narrow_inbox_shows_the_list_then_the_thread(cx: &mut TestAppContext) {
    let (view, h, cx) = setup(cx, 900.);
    connect(&h, cx);
    cx.dispatch_action(OpenInbox);
    frame(cx);
    let v = inbox(&view, cx);
    assert!(v.read_with(cx, |v, _| v.narrow()), "a narrow tile");
    let row = cx.debug_bounds("inbox-row-t1").expect("the list");
    cx.simulate_click(row.center(), Modifiers::none());
    frame(cx);
    say(&h, detail_t1(true), cx);
    assert!(v.read_with(cx, |v, _| v.pushed()));
    assert!(cx.debug_bounds("inbox-row-t1").is_none(), "the thread over the list");
    assert!(cx.debug_bounds("inbox-ask-1").is_some());
    cx.update(|window, cx| window.focus(&v.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("escape");
    frame(cx);
    assert!(cx.debug_bounds("inbox-row-t1").is_some(), "back to the list");
}

#[gpui::test]
fn the_inbox_says_what_is_going_on(cx: &mut TestAppContext) {
    let (view, h, cx) = setup(cx, 1500.);
    cx.dispatch_action(OpenInbox);
    frame(cx);
    assert!(cx.debug_bounds("inbox-loading").is_some(), "loading before its hello");
    say(&h, json!({"type": "hello", "protocol": 1}), cx);
    say(&h, json!({"type": "threads", "threads": []}), cx);
    assert!(cx.debug_bounds("inbox-state").is_some(), "nothing needs you");
    // It crashes: why, and Restart.
    h.read_with(cx, |h, _| {
        let _ = h.ends["demo"].events.unbounded_send(crate::extensions::adapter::Event::Stopped("It exited with code 3.".into()));
    });
    frame(cx);
    assert!(matches!(h.read_with(cx, |h, _| h.get("demo").unwrap().run.clone()), Run::Stopped(_)));
    let restart = cx.debug_bounds("inbox-restart").expect("Restart");
    cx.simulate_click(restart.center(), Modifiers::none());
    frame(cx);
    assert_eq!(h.read_with(cx, |h, _| h.get("demo").unwrap().run.clone()), Run::Starting);
    // Off: says so, with the way to turn it on.
    cx.update(|_, cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("demo", false)));
    frame(cx);
    assert!(cx.debug_bounds("inbox-settings").is_some());
    assert_eq!(inbox_panes(&view, cx), 1, "the tile stays");
}

#[gpui::test]
fn an_agents_open_asks_show_on_its_row_and_open_the_inbox(cx: &mut TestAppContext) {
    let (view, h, cx) = setup(cx, 1500.);
    assert!(cx.debug_bounds("ask-badge-web").is_none());
    connect(&h, cx);
    assert!(cx.debug_bounds("ask-badge-web").is_some(), "web's thread asks");
    assert!(cx.debug_bounds("ask-badge-api").is_none(), "api's doesn't");
    let badge = cx.debug_bounds(sel("ask-badge-web".into())).unwrap();
    cx.simulate_click(badge.center(), Modifiers::none());
    frame(cx);
    assert_eq!(inbox_panes(&view, cx), 1);
    let v = inbox(&view, cx);
    assert_eq!(v.read_with(cx, |v, _| v.selected().map(str::to_string)).as_deref(), Some("t1"), "at its oldest ask");
}

/// The tile and the thread it showed are back after a restart.
#[test]
fn an_inbox_tile_is_kept_in_ui_json() {
    let raw = json!({"v": 1, "spaces": [{"id": "s1", "name": "All", "kind": "all", "members": [],
        "layout": {"type": "pane", "id": "p1", "agentId": null, "inbox": "demo"}}],
        "extensions": {"demo": {"enabled": true, "selected": "t9"}}});
    let ui = crate::ui_state::UiState::sanitize(raw);
    assert_eq!(ui.tile_of(&Content::Inbox("demo".into())).map(|(_, p)| p).as_deref(), Some("p1"));
    assert_eq!(ui.extension("demo").selected.as_deref(), Some("t9"));
    let back = serde_json::to_value(&ui).unwrap();
    let layout = &back["spaces"].as_array().unwrap().iter().find(|s| s["id"] == "s1").unwrap()["layout"];
    assert_eq!(layout["inbox"], "demo");
    assert_eq!(back["extensions"]["demo"]["selected"], "t9");
}

/// With no agents, a space holding a tile (inbox, files, web) shows it;
/// only a space with nothing at all is "quiet".
#[gpui::test]
fn a_tile_shows_when_there_are_no_agents(cx: &mut TestAppContext) {
    let h = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        init(cx);
        crate::extensions::set_found(vec![Found { folder: "demo".into(), manifest: Ok(manifest()) }], cx);
        crate::extensions::follow(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("demo", true));
        hub::hub(cx)
    });
    let store = cx.new(|_| AgentStore::new(vec![]));
    let s = store.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(900.)));
    frame(cx);
    assert!(view.read_with(cx, |s, _| s.quiet(true, true)), "nothing at all: quiet");
    cx.dispatch_action(OpenInbox);
    frame(cx);
    connect(&h, cx);
    assert!(!view.read_with(cx, |s, _| s.quiet(true, true)), "an inbox tile: drawn");
    assert!(cx.debug_bounds("inbox-tile-demo").is_some());
    assert!(cx.debug_bounds("inbox-row-t1").is_some());
    // Files and web tiles count the same way.
    view.update(cx, |s, cx| s.close_inbox("demo", cx));
    frame(cx);
    assert!(view.read_with(cx, |s, _| s.quiet(true, true)));
    for c in [Content::Files("gone".into()), Content::Web("w1".into())] {
        view.update(cx, |s, cx| {
            let space = s.active_space().unwrap().id.clone();
            let c2 = c.clone();
            s.update_ui(|u| u.dock(&space, &c2, &super::workspace::FilesSpot::Auto, 0.5, None).0, cx);
        });
        frame(cx);
        assert!(!view.read_with(cx, |s, _| s.quiet(true, true)), "{c:?}");
        let c2 = c.clone();
        view.update(cx, |s, cx| s.update_ui(|u| u.undock(&c2), cx));
    }
}
