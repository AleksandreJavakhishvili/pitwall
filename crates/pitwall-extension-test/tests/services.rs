//! The harness on the tile host's committed services tile: fake `net`
//! answers, the database in the test's temporary folder, captured
//! notifications, files and pages it opened, and its toolbar button.

use std::path::{Path, PathBuf};
use std::time::Duration;

use pitwall_extension_test::{BadgeTone, Harness, Level, ServiceInfo, ServiceState, ToolbarBadge};
use pitwall_tile_host::UiRequest;

fn services() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/services")
}

fn status(t: &pitwall_extension_test::Session) -> String {
    t.find_key("status").and_then(|f| f.text().map(String::from)).unwrap_or_default()
}

#[test]
fn net_reaches_only_granted_hosts() {
    let mut t = Harness::load(services()).respond("https://api.example.com/items?x=1", 200, "two items").open("main");
    // Not granted yet: refused before anything leaves.
    t.click_key("do/fetch");
    assert!(status(&t).starts_with("refused: not allowed"), "{}", status(&t));
    assert!(t.requests().is_empty());
    assert!(t.logged(Level::Info, "it isn't granted “net”"));
    t.grant("net").click_key("do/fetch");
    assert_eq!(status(&t), "fetched 200: two items");
    let sent = t.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].method.as_str(), sent[0].url.as_str()), ("GET", "https://api.example.com/items?x=1"));
    assert!(sent[0].headers.iter().any(|h| h.name == "Accept" && h.value == "text/plain"));
    // A host the manifest doesn't list stays out of reach, granted or not.
    t.click_key("do/fetch-other");
    assert!(status(&t).contains("not allowed to reach other.example.org"), "{}", status(&t));
    assert_eq!(t.requests().len(), 1);
    // Taken back: refused again.
    t.revoke("net").click_key("do/fetch");
    assert!(status(&t).starts_with("refused"), "{}", status(&t));
}

#[test]
fn its_own_server_needs_no_net_grant() {
    let mut t = Harness::load(services()).own_server("127.0.0.1:8790").respond("http://127.0.0.1:8790/api/ping", 200, "pong").open("main");
    t.click_key("do/fetch-own");
    assert_eq!(status(&t), "fetched 200: pong");
    // Without a running server it has no address to ask.
    let mut off = Harness::load(services()).open("main");
    off.click_key("do/fetch-own");
    assert_eq!(status(&off), "no server address");
}

#[test]
fn the_database_migrates_keeps_and_rolls_back() {
    let mut t = Harness::load(services()).open("main");
    assert_eq!(status(&t), "schema 2");
    t.click_key("do/add").click_key("do/add");
    assert!(t.find_text("Notes: 2").is_some(), "{}", t.text_snapshot());
    assert!(t.database().is_file());
    // A failing step undoes the whole transaction.
    t.click_key("do/tx");
    assert!(status(&t).starts_with("transaction: "), "{}", status(&t));
    assert!(t.find_text("Notes: 2").is_some(), "{}", t.text_snapshot());
    // A transaction left open when the call returns is rolled back.
    t.click_key("do/open-tx");
    assert!(t.logged(Level::Warn, "was still open when the call returned"), "{}", t.log_text());
    t.click_key("do/add");
    assert!(t.find_text("Notes: 3").is_some(), "{}", t.text_snapshot());
    // Kept across a restart; the migrations already ran.
    t.restart();
    assert_eq!(status(&t), "schema 2");
    assert!(t.find_text("Notes: 3").is_some(), "{}", t.text_snapshot());
}

#[test]
fn notifications_need_the_grant_and_repeat_once() {
    let mut t = Harness::load(services()).open("main");
    t.click_key("do/notify");
    assert_eq!(status(&t), "notify: not allowed: the permission isn't granted");
    assert!(t.notifications().is_empty());
    t.grant("notify").click_key("do/notify").click_key("do/notify");
    assert_eq!(status(&t), "notify: ok");
    // The same words within ten minutes are shown once.
    assert_eq!(t.notifications(), vec![("Build failed".to_string(), "api: 3 tests failed".to_string())]);
    t.advance(Duration::from_secs(11 * 60)).click_key("do/notify");
    assert_eq!(t.notifications().len(), 2);
}

#[test]
fn files_and_pages_open_through_pitwall() {
    let mut t = Harness::load(services()).open("main");
    t.click_key("do/open-file");
    assert_eq!(status(&t), "file: not allowed: the permission isn't granted");
    t.grant("show").click_key("do/open-file").click_key("do/open-web");
    assert_eq!(status(&t), "web: ok");
    let asked: Vec<_> = t.ui_requests().iter().filter(|r| matches!(r, UiRequest::OpenFile { .. } | UiRequest::OpenWeb { .. })).cloned().collect();
    assert!(matches!(&asked[0], UiRequest::OpenFile { path, line: Some(12) } if path == "/tmp/project/src/main.rs"), "{asked:?}");
    assert!(matches!(&asked[1], UiRequest::OpenWeb { url } if url == "https://example.com/docs"), "{asked:?}");
}

#[test]
fn the_toolbar_button_badge_tooltip_and_popover() {
    let mut t = Harness::load(services()).open("main");
    t.click_key("do/badge").click_key("do/badge");
    let tb = t.toolbar();
    assert_eq!(tb.badge, Some((ToolbarBadge::Count(2), BadgeTone::Attention)));
    assert_eq!(tb.tooltip.as_deref(), Some("2 unread"));
    assert!(!tb.popover_open);
    // Its popover opens on a click and closes when the tile asks.
    t.click_toolbar(Some("api"));
    assert!(t.toolbar().popover_open);
    assert!(t.find_key_in(pitwall_tile_host::POPOVER_VIEW, "title").is_some());
    assert_eq!(status(&t), "clicked api");
    t.click_in_popover("close");
    assert!(!t.toolbar().popover_open);
    // A second click on the button closes it too.
    t.click_toolbar(None).click_toolbar(None);
    assert!(!t.toolbar().popover_open);
    t.click_key("do/hide");
    assert_eq!(t.toolbar().shown, Some(false));
}

#[test]
fn it_hears_its_server_change_and_asks_for_a_restart() {
    let mut t = Harness::load(services()).own_server("127.0.0.1:8790").open("main");
    t.set_service(ServiceInfo { state: ServiceState::Failed, address: None, version: None, why: Some("exited with 1".into()) });
    assert_eq!(status(&t), "service Failed -");
    t.click_key("do/restart");
    assert!(t.ui_requests().iter().any(|r| matches!(r, UiRequest::RestartService)));
}
