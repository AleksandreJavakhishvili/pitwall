use gpui::{TestAppContext, VisualTestContext};

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
fn submit_label_follows_the_desktop() {
    assert_eq!(submit_keys(true), "⌘↵");
    assert_eq!(submit_keys(false), "Ctrl+↵");
}

#[test]
fn failures_say_what_failed() {
    assert_eq!(Op::Add("x".into()).what(), "queue prompt");
    assert_eq!(Op::Remove("q".into()).what(), "remove queue item");
    assert_eq!(Op::SendNow("q".into()).what(), "send prompt");
    assert_eq!(Op::AutoSend(true).what(), "change auto-send");
}

fn next_up(cx: &mut TestAppContext) -> (Entity<NextUp>, &mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::code_view::register(cx);
    });
    cx.add_window_view(NextUp::new)
}

#[gpui::test]
fn drafts_stay_with_their_agent(cx: &mut TestAppContext) {
    let (view, cx) = next_up(cx);
    let (a, b) = (
        agent("a", "/work/alpha", "idle", 1, true),
        agent("b", "/work/alpha", "working", 2, true),
    );
    view.update(cx, |v, cx| {
        v.set_agent(&a, cx);
        v.field
            .update(cx, |f, cx| f.set_text("first line\nsecond", cx));
        v.set_agent(&b, cx);
        assert_eq!(v.draft(cx), "", "b has no draft");
        v.field.update(cx, |f, cx| f.set_text("for b", cx));
        v.set_agent(&a, cx);
        assert_eq!(v.draft(cx), "first line\nsecond", "kept verbatim");
        v.set_agent(&b, cx);
        assert_eq!(v.draft(cx), "for b");
    });
}

#[gpui::test]
fn queueing_without_an_engine_keeps_nothing_back(cx: &mut TestAppContext) {
    let (view, cx) = next_up(cx);
    let a = agent("a", "/work/alpha", "idle", 1, true);
    view.update_in(cx, |v, window, cx| {
        v.set_agent(&a, cx);
        // Blank text is never queued.
        v.field.update(cx, |f, cx| f.set_text("  \n ", cx));
        v.add(window, cx);
        assert_eq!(v.draft(cx), "  \n ");
        // Real text leaves the box (the engine call is skipped here).
        v.field.update(cx, |f, cx| f.set_text("ship it", cx));
        v.add(window, cx);
        assert_eq!(v.draft(cx), "");
    });
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.drafts.get("a").map(String::as_str),
            Some(""),
            "the draft is gone too"
        )
    });
}
