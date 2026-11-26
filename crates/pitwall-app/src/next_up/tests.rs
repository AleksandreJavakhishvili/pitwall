use gpui::{div, Render, TestAppContext, Window};

use super::*;
use crate::agents::tests::agent;

#[test]
fn relative_times_match_the_react_app() {
    const NOW: u64 = 1_800_000_000_000;
    assert_eq!(rel_time(NOW - 3_000, NOW), "just now");
    assert_eq!(rel_time(NOW + 5_000, NOW), "just now", "clock skew");
    assert_eq!(rel_time(NOW - 42_000, NOW), "42s ago");
    assert_eq!(rel_time(NOW - 4 * 60_000, NOW), "4m ago");
    assert_eq!(rel_time(NOW - 2 * 3_600_000, NOW), "2h ago");
    assert_eq!(rel_time(NOW - 3 * 86_400_000, NOW), "3d ago");
}

#[test]
fn absolute_time_reads_like_to_locale_string() {
    let s = absolute_time(1_800_000_000_000);
    // "M/D/YYYY, H:MM:SS AM|PM" in the local time zone.
    let (date, time) = s.split_once(", ").expect("date, time");
    assert_eq!(date.split('/').count(), 3);
    assert!(time.ends_with(" AM") || time.ends_with(" PM"), "{s}");
}

#[test]
fn failures_say_what_failed() {
    assert_eq!(Op::Remove("q".into()).what(), "remove queue item");
    assert_eq!(Op::SendNow("q".into()).what(), "send prompt");
    assert_eq!(Op::AutoSend(true).what(), "change auto-send");
}

fn item(id: &str, text: &str, from: Option<(&str, &str)>) -> pitwall_proto::QueueItem {
    pitwall_proto::QueueItem {
        id: id.into(),
        text: text.into(),
        origin: from.map(|(e, n)| pitwall_proto::QueueOrigin { extension: e.into(), name: n.into() }),
    }
}

#[test]
fn the_chip_counts_what_waits() {
    let mut a = agent("a", "/work/alpha", "idle", 1, true);
    assert_eq!(chip_label(&a), None, "nothing queued: no chip");
    a.queue = vec![item("q1", "one", None), item("q2", "two", None)];
    assert_eq!(chip_label(&a).as_deref(), Some("2 queued"));
}

/// The popover's content, alone in a window.
struct Host(Shown);

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).clone();
        div().child(body(&self.0, &t, &WeakEntity::new_invalid()))
    }
}

fn shown(a: AgentView) -> Shown {
    Shown { agent: a, senders: vec![], icons: HashMap::new(), now: 1_800_000_000_000 }
}

fn draw(cx: &mut TestAppContext, s: Shown) -> &mut gpui::VisualTestContext {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
    });
    let (_, cx) = cx.add_window_view(|_, _| Host(s));
    cx.run_until_parked();
    cx
}

/// An extension's item says whose it is and that it waits for Send; the
/// user's own item says nothing of the kind.
#[gpui::test]
fn an_extensions_item_is_marked(cx: &mut TestAppContext) {
    let mut a = agent("a", "/work/alpha", "idle", 1, true);
    a.queue = vec![item("q1", "mine", None), item("q2", "fix the failing test", Some(("ci", "CI")))];
    let cx = draw(cx, shown(a));
    assert!(cx.debug_bounds("queue-from-1").is_some(), "the extension's item is marked");
    assert!(cx.debug_bounds("queue-from-0").is_none(), "the user's isn't");
    assert!(cx.debug_bounds("queue-item-1").is_some());
}

/// Send now only while the agent runs; Remove always; no compose box,
/// and the last prompt sent is one line at the foot.
#[gpui::test]
fn send_now_only_while_it_runs(cx: &mut TestAppContext) {
    let mut a = agent("a", "/work/alpha", "working", 1, true);
    a.running = false;
    a.queue = vec![item("q1", "mine", None)];
    a.last_sent = Some("run the tests\nthen lint".into());
    a.last_sent_at = Some(1_800_000_000_000 - 4 * 60_000);
    let cx = draw(cx, shown(a));
    assert!(cx.debug_bounds("next-up").is_some());
    assert!(cx.debug_bounds("next-up-auto-send").is_some());
    assert!(cx.debug_bounds("next-up-last-sent").is_some());
    assert!(cx.debug_bounds("queue-send-0").is_none(), "not running: no Send now");
}

#[gpui::test]
fn a_running_agent_offers_send_now(cx: &mut TestAppContext) {
    let mut a = agent("a", "/work/alpha", "working", 1, true);
    a.running = true;
    a.queue = vec![item("q1", "mine", None)];
    let cx = draw(cx, shown(a));
    assert!(cx.debug_bounds("queue-send-0").is_some());
    assert!(cx.debug_bounds("next-up-last-sent").is_none(), "nothing sent yet: no line");
}
