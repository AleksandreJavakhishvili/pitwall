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
        // The fields' own keys first, as at start (⌘↵ sends in one).
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
    view.read_with(cx, |s, cx| s.inbox("demo", cx)).expect("an inbox view")
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

    // A wide tile shows the first (the blocking one): opened, remembered.
    let selected = |cx: &mut VisualTestContext| v.read_with(cx, |v, _| v.selected().map(str::to_string));
    assert_eq!(selected(cx).as_deref(), Some("t1"));
    assert_eq!(sent(&h, cx), [json!({"type": "open", "thread": "t1"})]);
    assert_eq!(cx.update(|_, cx| crate::ui_state::get(cx).extension("demo").selected).as_deref(), Some("t1"));
    // ↓ and ↑ go through the list.
    cx.simulate_keystrokes("down");
    frame(cx);
    assert_eq!(selected(cx).as_deref(), Some("t2"));
    cx.simulate_keystrokes("up");
    frame(cx);
    assert_eq!(selected(cx).as_deref(), Some("t1"));
    assert_eq!(sent(&h, cx).last(), Some(&json!({"type": "open", "thread": "t1"})));
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
    cx.simulate_click(close.center(), Modifiers::none());
    frame(cx);
    assert_eq!(inbox_panes(&view, cx), 0);
    assert!(view.read_with(cx, |s, cx| s.inbox("demo", cx).is_none()));
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

/// The platform's Maximise key, as `main_screen::bindings` has it (⌘⏎ on
/// macOS, Ctrl+Shift+Enter elsewhere: "cmd" is Super off macOS).
const MAXIMISE: &str = if cfg!(target_os = "macos") { "cmd-enter" } else { "ctrl-shift-enter" };

/// The platform's key a field sends with (⌘↵ / Ctrl+↵).
const SEND: &str = if cfg!(target_os = "macos") { "cmd-enter" } else { "ctrl-enter" };

/// Every key a word or a number is typed with, none of them a tile key
/// when a field has the keyboard (J, K, E, R, 1–9 are, in the tile).
const TYPED: &str = "the quick brown fox jumps over a lazy dog 0123456789 JKER";

/// A press in the middle of `which` field ("reply", "other"), as a real
/// click.
fn click_field(v: &Entity<InboxView>, which: &str, cx: &mut VisualTestContext) {
    let field = v.read_with(cx, |v, _| v.debug_field(which)).unwrap_or_else(|| panic!("a {which} field"));
    let at = field.read_with(cx, |i, _| i.drawn_bounds()).unwrap_or_else(|| panic!("the {which} field is drawn")).center();
    cx.simulate_click(at, Modifiers::none());
    frame(cx);
}

fn field_text(v: &Entity<InboxView>, which: &str, cx: &mut VisualTestContext) -> String {
    let field = v.read_with(cx, |v, _| v.debug_field(which)).unwrap();
    field.read_with(cx, |i, _| i.text().to_string())
}

/// What the adapter was sent that isn't a thread being opened or read.
fn acted(h: &Entity<Hub>, cx: &mut VisualTestContext) -> Vec<serde_json::Value> {
    sent(h, cx).into_iter().filter(|m| !matches!(m["type"].as_str(), Some("open" | "read" | "close"))).collect()
}

/// The inbox open on thread t1 with its ask showing, wide.
fn on_t1(cx: &mut TestAppContext) -> (Entity<MainScreen>, Entity<Hub>, Entity<InboxView>, &mut VisualTestContext) {
    let (view, h, cx) = setup(cx, 2000.);
    connect(&h, cx);
    cx.dispatch_action(OpenInbox);
    frame(cx);
    say(&h, detail_t1(true), cx);
    let v = inbox(&view, cx);
    assert_eq!(v.read_with(cx, |v, _| v.selected().map(str::to_string)).as_deref(), Some("t1"));
    let _ = sent(&h, cx);
    (view, h, v, cx)
}

/// The bug: a press in a field gave the keys back to the tile (its own
/// press came after the field's), so typing ran the tile's keys (J, K
/// moved, E archived, 1–9 answered) and the rest went nowhere. Typed into
/// the reply box right after a click: every character is there, and
/// nothing else happened.
#[gpui::test]
fn what_is_typed_in_the_reply_box_goes_in_it(cx: &mut TestAppContext) {
    let (_, h, v, cx) = on_t1(cx);
    click_field(&v, "reply", cx);
    assert!(cx.update(|window, cx| v.read(cx).typing(window, cx)), "the reply box has the keys");
    cx.simulate_input(TYPED);
    frame(cx);
    assert_eq!(field_text(&v, "reply", cx), TYPED);
    assert_eq!(v.read_with(cx, |v, _| v.selected().map(str::to_string)).as_deref(), Some("t1"), "J and K moved nothing");
    assert_eq!(acted(&h, cx), Vec::<serde_json::Value>::new(), "nothing archived or answered");
    assert!(cx.debug_bounds("inbox-row-t1").is_some(), "still listed");

    // The list refreshes under it: the keys stay in the box.
    connect(&h, cx);
    cx.simulate_input("!");
    frame(cx);
    assert_eq!(field_text(&v, "reply", cx), format!("{TYPED}!"));

    // Sent with ⌘↵: the box is empty and still has the keys.
    cx.simulate_keystrokes(SEND);
    frame(cx);
    let s = acted(&h, cx);
    assert_eq!(s.len(), 1, "{s:?}");
    assert_eq!((s[0]["type"].as_str(), s[0]["text"].as_str()), (Some("message"), Some(format!("{TYPED}!").as_str())));
    cx.simulate_input(TYPED);
    frame(cx);
    assert_eq!(field_text(&v, "reply", cx), TYPED);
    assert_eq!(acted(&h, cx), Vec::<serde_json::Value>::new());

    // Esc gives the keys back to the tile: J moves again.
    cx.simulate_keystrokes("escape");
    frame(cx);
    assert!(cx.update(|window, cx| !v.read(cx).typing(window, cx) && v.read(cx).focus_handle(cx).is_focused(window)));
    v.update(cx, |v, cx| v.set_filter(Filter::All, cx));
    frame(cx);
    cx.simulate_keystrokes("j");
    frame(cx);
    assert_eq!(v.read_with(cx, |v, _| v.selected().map(str::to_string)).as_deref(), Some("t2"), "the tile's keys again");
}

/// The same in an ask's "write your own answer" field, and after the
/// answer is sent.
#[gpui::test]
fn what_is_typed_as_your_own_answer_goes_in_it(cx: &mut TestAppContext) {
    let (_, h, v, cx) = on_t1(cx);
    click_field(&v, "other", cx);
    cx.simulate_input(TYPED);
    frame(cx);
    assert_eq!(field_text(&v, "other", cx), TYPED);
    assert_eq!(v.read_with(cx, |v, _| v.selected().map(str::to_string)).as_deref(), Some("t1"));
    assert_eq!(acted(&h, cx), Vec::<serde_json::Value>::new(), "1–9 picked nothing");

    // After the list refreshes.
    connect(&h, cx);
    say(&h, detail_t1(true), cx);
    cx.simulate_input("7");
    frame(cx);
    assert_eq!(field_text(&v, "other", cx), format!("{TYPED}7"));

    // ↵ answers with it; its card goes, the keys back to the tile.
    cx.simulate_keystrokes("enter");
    frame(cx);
    let s = acted(&h, cx);
    assert_eq!(s.len(), 1, "{s:?}");
    assert_eq!((s[0]["type"].as_str(), s[0]["text"].as_str()), (Some("answer"), Some(format!("{TYPED}7").as_str())), "{s:?}");
    assert!(cx.update(|window, cx| v.read(cx).focus_handle(cx).is_focused(window)), "the tile has the keys");
    // Then the reply box takes typing as before.
    click_field(&v, "reply", cx);
    cx.simulate_input(TYPED);
    frame(cx);
    assert_eq!(field_text(&v, "reply", cx), TYPED);
    assert_eq!(acted(&h, cx), Vec::<serde_json::Value>::new());
}

/// ⌘⏎ maximises the inbox as any tile: after ⌘I, and after a click in
/// it (it becomes the space's focused tile, with a focused border); its
/// bar's button does the same.
#[gpui::test]
fn the_inbox_tile_takes_part_in_the_tile_keys(cx: &mut TestAppContext) {
    let (view, h, cx) = setup(cx, 2000.);
    connect(&h, cx);
    cx.dispatch_action(OpenInbox);
    frame(cx);
    let pane = view.read_with(cx, |s, _| s.active_space().unwrap().layout.leaves().iter().find(|p| p.inbox.is_some()).unwrap().id.clone());
    let space = |view: &Entity<MainScreen>, cx: &mut VisualTestContext| view.read_with(cx, |s, _| s.active_space().cloned().unwrap());
    assert_eq!(space(&view, cx).focused_pane_id.as_deref(), Some(pane.as_str()), "⌘I focuses its tile");
    cx.simulate_keystrokes(MAXIMISE);
    frame(cx);
    assert_eq!(space(&view, cx).maximized_pane_id.as_deref(), Some(pane.as_str()), "maximised");
    assert!(cx.debug_bounds("inbox-row-t1").is_some(), "drawn maximised");
    cx.simulate_keystrokes(MAXIMISE);
    frame(cx);
    assert_eq!(space(&view, cx).maximized_pane_id, None, "restored");

    // The terminal focused, then a click in the inbox's list: its tile.
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    frame(cx);
    assert_ne!(space(&view, cx).focused_pane_id.as_deref(), Some(pane.as_str()));
    let empty = cx.debug_bounds("inbox-bar").expect("its bar");
    cx.simulate_click(gpui::point(empty.left() + gpui::px(200.), empty.center().y), Modifiers::none());
    frame(cx);
    assert_eq!(space(&view, cx).focused_pane_id.as_deref(), Some(pane.as_str()), "a click focuses its tile");
    cx.simulate_keystrokes(MAXIMISE);
    frame(cx);
    assert_eq!(space(&view, cx).maximized_pane_id.as_deref(), Some(pane.as_str()));
    // Its bar's Restore / Maximise.
    let b = cx.debug_bounds("inbox-maximize").expect("its maximise button");
    cx.simulate_click(b.center(), Modifiers::none());
    frame(cx);
    assert_eq!(space(&view, cx).maximized_pane_id, None);
    let b = cx.debug_bounds("inbox-maximize").unwrap();
    cx.simulate_click(b.center(), Modifiers::none());
    frame(cx);
    assert_eq!(space(&view, cx).maximized_pane_id.as_deref(), Some(pane.as_str()));
}

/// An inbox's place in a window of its own is kept in `ui.json`, and
/// gone with it when it is closed.
#[test]
fn an_inbox_in_a_window_is_kept_in_ui_json() {
    let raw = json!({"v": 1, "spaces": [{"id": "s1", "name": "All", "kind": "all", "members": [], "layout": {"type": "pane", "id": "p1", "agentId": null, "inbox": "demo"}}],
        "extensions": {"demo": {"enabled": true, "selected": "t9"}}});
    let ui = crate::ui_state::UiState::sanitize(raw).inbox_to_window("demo");
    assert!(ui.tile_of(&Content::Inbox("demo".into())).is_none(), "out of the layout");
    let back = serde_json::to_value(&ui).unwrap();
    assert_eq!(back["extensions"]["demo"]["window"], true);
    assert_eq!(back["extensions"]["demo"]["selected"], "t9", "its thread kept");
    let again = crate::ui_state::UiState::sanitize(back);
    assert!(again.extension("demo").window);
    let (docked, at) = again.clone().inbox_to_layout("s1", "demo", &super::workspace::FilesSpot::Auto, None);
    assert!(at.is_some());
    assert!(!docked.extension("demo").window && docked.tile_of(&Content::Inbox("demo".into())).is_some());
    let closed = again.close_inbox("demo");
    assert!(!closed.extension("demo").window && closed.extension("demo").enabled, "closed, still on");
    assert!(serde_json::to_value(&closed).unwrap()["extensions"]["demo"].get("window").is_none());
}

// ── moved between windows ────────────────────────────────────────────────

struct Main(Option<gpui::WindowHandle<crate::ui::MainView>>, Entity<AgentStore>);

impl gpui::Global for Main {}

fn show_main(cx: &mut gpui::App) -> gpui::WindowHandle<crate::ui::MainView> {
    if let Some(h) = cx.global::<Main>().0 {
        if cx.windows().iter().any(|w| w.window_id() == h.window_id()) {
            return h;
        }
    }
    let store = cx.global::<Main>().1.clone();
    let h = cx.open_window(Default::default(), |window, cx| cx.new(|cx| crate::ui::MainView::new(crate::ui::Content::Live { store }, None, window, cx))).unwrap();
    cx.global_mut::<Main>().0 = Some(h);
    h
}

/// Pitwall's windows (main, agents api and web side by side in a space of
/// it) with the demo extension on and its adapter connected; the inbox
/// opened with ⌘I. The space's id and the hub.
fn app(cx: &mut TestAppContext) -> (String, Entity<Hub>) {
    let h = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::set_found(vec![Found { folder: "demo".into(), manifest: Ok(manifest()) }], cx);
        crate::extensions::follow(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("demo", true));
        let store = cx.new(|_| AgentStore::new(vec![agent("api", "/work/alpha", "idle", 1, true), agent("web", "/work/alpha", "idle", 2, true)]));
        cx.set_global(Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: crate::ui::Content::Live { store: store.clone() }, root: None, main: None });
        crate::windows::init(crate::ui::Content::Live { store }, None, show_main, cx);
        show_main(cx);
        crate::windows::restore(cx);
        hub::hub(cx)
    });
    cx.run_until_parked();
    let space = cx.update(|cx| {
        let (ui, id) = crate::ui_state::get(cx).create_custom_space(crate::windows::MAIN);
        let ui = ui.place_agent(&id, "api", None, None).place_agent(&id, "web", None, Some(super::tree::Side::Right));
        crate::ui_state::update(cx, |s| *s = ui);
        let sid = id.clone();
        main_screen(cx).update(cx, |s, cx| s.show_space(&sid, cx));
        id
    });
    frames(cx);
    let _ = h.read_with(cx, |h, _| h.ends.get("demo").map(|e| e.sent()));
    for line in [
        json!({"type": "hello", "protocol": 1}),
        json!({"type": "status", "state": "ready"}),
        json!({"type": "threads", "threads": [{"id": "t1", "title": "Which database?", "from": {"agent": "alpha", "session": "web"}, "unread": 1, "openAsks": 1, "blocking": true, "lastActivity": "2026-01-02T10:00:00Z"}]}),
    ] {
        h.read_with(cx, |h, _| h.ends["demo"].say(line));
        frames(cx);
    }
    let main = cx.update(|cx| crate::windows::main_window(crate::windows::MAIN, cx).unwrap());
    // At thread t1 (as from web's ask badge).
    main.update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.open_inbox(None, Some("t1"), window, cx))).unwrap();
    frames(cx);
    (space, h)
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

fn inbox_tiles(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| crate::ui_state::get(cx).spaces.iter().map(|s| s.layout.leaves().iter().filter(|p| p.inbox.is_some()).count()).sum())
}

/// Out into a window of its own and back, and moved in the layout: the
/// same inbox throughout (its view, its thread, the adapter: nothing
/// opened again), never two. ⌘I finds it in its window.
#[gpui::test]
fn the_inbox_moves_to_a_window_and_back_as_the_same_inbox(cx: &mut TestAppContext) {
    let (space, h) = app(cx);
    assert_eq!(inbox_tiles(cx), 1);
    let v = cx.update(|cx| super::inbox_tile::get("demo", cx)).expect("its view");
    assert_eq!(v.read_with(cx, |v, _| v.selected().map(str::to_string)).as_deref(), Some("t1"));
    let started = h.read_with(cx, |h, _| h.get("demo").map(|s| s.run.clone()));
    let _ = h.read_with(cx, |h, _| h.ends["demo"].sent());
    let same = |cx: &mut TestAppContext| cx.update(|cx| super::inbox_tile::get("demo", cx)).map(|e| e.entity_id()) == Some(v.entity_id());

    // The bar's "Open in a window of its own".
    cx.update(|cx| super::inbox_window::pop_out("demo", None, true, cx)).expect("popped out");
    frames(cx);
    cx.update(|cx| {
        assert!(crate::windows::inbox_window("demo", cx).is_some(), "a window");
        assert!(crate::ui_state::get(cx).extension("demo").window, "kept in ui.json");
        assert_eq!(super::inbox_window::described("demo", cx), "a window of its own");
    });
    assert_eq!(inbox_tiles(cx), 0, "out of the layout");
    assert!(same(cx), "the same view");
    // ⌘I (the palette's "Open … inbox" is the same call) finds it there.
    let main = cx.update(|cx| crate::windows::main_window(crate::windows::MAIN, cx).unwrap());
    main.update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.open_inbox(Some("demo"), None, window, cx))).unwrap();
    frames(cx);
    assert_eq!(inbox_tiles(cx), 0, "not placed again");
    let w = cx.update(|cx| crate::windows::inbox_window("demo", cx)).unwrap();
    let focused = w.update(cx, |_, window, cx| v.read(cx).focus_handle(cx).is_focused(window)).unwrap();
    assert!(focused, "its window's inbox has the keys");

    // Dock…: right of api's terminal; the window closes.
    cx.update(|cx| super::inbox_window::place("demo", crate::explorer::dock::FilesPlace::Next(super::tree::Side::Right), Some("api"), cx)).expect("docked");
    frames(cx);
    cx.update(|cx| {
        assert!(crate::windows::inbox_window("demo", cx).is_none(), "its window closed");
        let ui = crate::ui_state::get(cx);
        assert!(!ui.extension("demo").window);
        assert_eq!(ui.tile_of(&Content::Inbox("demo".into())).map(|(s, _)| s), Some(space.clone()));
        let at = super::inbox_window::described("demo", cx);
        assert!(at.contains("between api's and web's terminals"), "{at}");
    });
    assert_eq!(inbox_tiles(cx), 1);
    assert!(same(cx));
    // Moved in the layout: a column at the right edge; in web's place.
    for (place, of, words) in [
        (crate::explorer::dock::FilesPlace::Edge(super::tree::Side::Right), None, "right of web's terminal"),
        (crate::explorer::dock::FilesPlace::Replace, Some("web"), "in place of web's terminal"),
    ] {
        let at = cx.update(|cx| super::inbox_window::place("demo", place, of, cx)).expect("moved");
        frames(cx);
        assert!(at.contains(words), "{at}");
        assert_eq!(inbox_tiles(cx), 1, "never two");
        assert!(same(cx));
    }
    // A tile's bar dragged out of every window, let go: a window there.
    let r = cx.update(|cx| crate::explorer::drag::finish_what(&Content::Inbox("demo".into()), gpui::point(gpui::px(-5000.), gpui::px(-5000.)), crate::explorer::drag::From::Tile, cx));
    assert_eq!(r.expect("moved").as_deref(), Some("a window of its own"));
    frames(cx);
    assert!(cx.update(|cx| crate::windows::inbox_window("demo", cx).is_some()));
    assert!(same(cx));

    // Through all of it: the adapter never restarted, the thread never
    // opened again, still t1.
    assert_eq!(h.read_with(cx, |h, _| h.get("demo").map(|s| s.run.clone())), started);
    let s = h.read_with(cx, |h, _| h.ends["demo"].sent());
    assert!(!s.iter().any(|m| m["type"] == "open" || m["type"] == "close"), "{s:?}");
    assert_eq!(v.read_with(cx, |v, _| v.selected().map(str::to_string)).as_deref(), Some("t1"));

    // Its window closed by the user: the inbox closes.
    cx.update(|cx| super::inbox_tile::close("demo", cx));
    frames(cx);
    cx.update(|cx| {
        assert!(crate::windows::inbox_window("demo", cx).is_none());
        assert!(super::inbox_tile::get("demo", cx).is_none());
        assert!(!crate::ui_state::get(cx).extension("demo").window);
    });
}

/// An inbox left in a window of its own opens in one again at the next
/// start.
#[gpui::test]
fn an_inbox_in_a_window_comes_back_after_a_restart(cx: &mut TestAppContext) {
    let _ = app(cx);
    cx.update(|cx| super::inbox_window::pop_out("demo", None, false, cx)).expect("popped out");
    frames(cx);
    // "Restarted": the window and the view gone from memory, ui.json kept.
    cx.update(|cx| {
        if let Some(h) = crate::windows::inbox_window("demo", cx) {
            let _ = h.update(cx, |w, window, _| {
                w.closing();
                window.remove_window();
            });
        }
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(crate::windows::inbox_window("demo", cx).is_none());
        assert!(crate::ui_state::get(cx).extension("demo").window);
    });
    cx.update(super::inbox_window::restore);
    frames(cx);
    cx.update(|cx| assert!(crate::windows::inbox_window("demo", cx).is_some(), "open again"));
    assert_eq!(inbox_tiles(cx), 0);
}

/// Two connections in one inbox: all their threads listed, each row
/// naming its connection, the offline one said in a line of its own; the
/// bar's menu (each connection with the profiles that use it) lists one
/// connection's, kept in `ui.json`; while the menu is open the tile's
/// single keys do nothing. Back to one connection: no menu, no labels,
/// and the kept choice no longer applies.
#[gpui::test]
fn two_connections_share_the_inbox_with_a_filter(cx: &mut TestAppContext) {
    use pitwall_core::extensions::{SettingDef, SettingKind};
    let t = pitwall_core::testing::TempDir::new("inbox-conns");
    let paths = pitwall_core::paths::Paths::new(t.path().join("pw"));
    let man = Manifest {
        settings: vec![SettingDef { required: true, ..SettingDef::new("server", SettingKind::Url, "Server") }],
        ..manifest()
    };
    let (h, acme, side) = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        init(cx);
        let m = crate::accounts::model(cx);
        m.update(cx, |m, _| m.set_paths(paths));
        let mut ids = Vec::new();
        for (name, server) in [("Acme", "https://acme.example"), ("Side", "https://side.example")] {
            let c = m.update(cx, |m, cx| m.add_connection("demo", name, cx)).unwrap();
            m.update(cx, |m, cx| m.set_connection_value(&c.id, "server", Some(server), cx)).unwrap();
            ids.push(c.id);
        }
        let p = m.update(cx, |m, cx| m.add_profile("Work", cx)).unwrap();
        m.update(cx, |m, cx| m.set_profile_pick(&p.id, "ext:demo", Some(&ids[0]), cx)).unwrap();
        crate::extensions::set_found(vec![Found { folder: "demo".into(), manifest: Ok(man) }], cx);
        crate::extensions::follow(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("demo", true));
        (hub::hub(cx), ids[0].clone(), ids[1].clone())
    });
    let store = cx.new(|_| AgentStore::new(vec![agent("web", "/work/alpha", "idle", 2, true)]));
    let s = store.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(2000.), gpui::px(900.)));
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    frame(cx);
    let (ea, es) = (format!("demo/{acme}"), format!("demo/{side}"));
    let tell = |end: &str, line: serde_json::Value, cx: &mut VisualTestContext| {
        h.read_with(cx, |h, _| h.ends[end].say(line));
        frame(cx);
    };
    for e in [&ea, &es] {
        tell(e, json!({"type": "hello", "protocol": 1}), cx);
    }
    tell(&ea, json!({"type": "status", "state": "ready"}), cx);
    tell(
        &ea,
        json!({"type": "threads", "threads": [
            {"id": "a1", "title": "Which database?", "from": {"agent": "alpha", "session": "web"}, "unread": 1, "openAsks": 1, "blocking": true, "lastActivity": "2026-01-02T10:00:00Z"},
            {"id": "a2", "title": "Done", "from": {"agent": "alpha", "session": "web"}, "lastActivity": "2026-01-01T10:00:00Z"}
        ]}),
        cx,
    );
    tell(&es, json!({"type": "status", "state": "error", "message": "Can't reach the server"}), cx);
    cx.dispatch_action(OpenInbox);
    frame(cx);
    let v = inbox(&view, cx);
    let a1 = format!("{acme}:a1");

    // All connections: Acme's threads, labelled; Side said offline.
    assert!(cx.debug_bounds("inbox-conn").is_some(), "the connections menu's button");
    assert!(cx.debug_bounds(sel(format!("inbox-row-{a1}"))).is_some());
    assert!(cx.debug_bounds(sel(format!("inbox-row-conn-{a1}"))).is_some(), "the row names its connection");
    assert!(cx.debug_bounds(sel(format!("inbox-offline-{side}"))).is_some(), "Side's line");
    assert!(cx.debug_bounds(sel(format!("inbox-offline-{acme}"))).is_none());

    // The menu open: J, E and 1 do nothing to the list; Esc closes it.
    v.update(cx, |v, cx| v.set_filter(Filter::All, cx));
    let b = cx.debug_bounds("inbox-conn").unwrap();
    cx.simulate_click(b.center(), Modifiers::none());
    frame(cx);
    assert!(cx.debug_bounds("inbox-conn-menu").is_some(), "open");
    let profiles = cx.update(|_, cx| crate::accounts::list(cx).unwrap().profiles_of(&acme));
    assert_eq!(profiles, vec!["Work".to_string()], "Acme shown with its profile");
    let before = v.read_with(cx, |v, _| v.selected().map(str::to_string));
    let acted_on = |cx: &mut VisualTestContext| h.read_with(cx, |h, _| h.ends[&ea].sent()).into_iter().filter(|m| !matches!(m["type"].as_str(), Some("hello" | "open" | "read" | "close"))).count();
    let n = acted_on(cx);
    cx.simulate_keystrokes("j e 1");
    frame(cx);
    assert_eq!(v.read_with(cx, |v, _| v.selected().map(str::to_string)), before, "J moved nothing");
    assert_eq!(acted_on(cx), n, "nothing archived or answered");
    assert!(v.read_with(cx, |v, _| v.conn_menu_open()));
    cx.simulate_keystrokes("escape");
    frame(cx);
    assert!(!v.read_with(cx, |v, _| v.conn_menu_open()), "Esc closes it");

    // Side only: its line, none of Acme's threads; kept in ui.json.
    cx.simulate_click(b.center(), Modifiers::none());
    frame(cx);
    let item = cx.debug_bounds("inbox-conn-item-2").expect("Side in the menu");
    cx.simulate_click(item.center(), Modifiers::none());
    frame(cx);
    cx.update(|_, cx| assert_eq!(crate::ui_state::get(cx).extension("demo").connection.as_deref(), Some(side.as_str())));
    assert!(cx.debug_bounds(sel(format!("inbox-row-{a1}"))).is_none());
    assert!(cx.debug_bounds(sel(format!("inbox-offline-{side}"))).is_some());
    // Acme only: its threads, unlabelled; Side's line gone.
    v.update(cx, |v, cx| v.set_connection(Some(acme.clone()), cx));
    frame(cx);
    assert!(cx.debug_bounds(sel(format!("inbox-row-{a1}"))).is_some());
    assert!(cx.debug_bounds(sel(format!("inbox-row-conn-{a1}"))).is_none(), "one connection listed: no label");
    assert!(cx.debug_bounds(sel(format!("inbox-offline-{side}"))).is_none());
    assert_eq!(v.read_with(cx, |v, cx| v.listed(cx).len()), 2);

    // Side removed: one connection, nothing of this shows; a choice of a
    // connection that is gone lists all.
    v.update(cx, |v, cx| v.set_connection(Some(side.clone()), cx));
    cx.update(|_, cx| crate::accounts::model(cx).update(cx, |m, cx| m.remove(&side, cx))).unwrap();
    frame(cx);
    assert!(cx.debug_bounds("inbox-conn").is_none(), "no menu with one connection");
    assert!(cx.debug_bounds(sel(format!("inbox-row-{a1}"))).is_some(), "all listed");
    assert!(cx.debug_bounds(sel(format!("inbox-row-conn-{a1}"))).is_none(), "no labels");
}
