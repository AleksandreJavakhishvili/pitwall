//! What a tile reports and asks for beyond its views, headless, on the
//! tile host's questions fixture ("Release desk"): the status it reports
//! (`status.set`) on its card's pill and checklist, as an adapter's; and
//! its shortcut, off until the user grants it, then bound, shown in the
//! palette and changeable in its card.

use std::time::{Duration, Instant};

use gpui::{Keystroke, TestAppContext, VisualTestContext};
use pitwall_core::extensions::permissions;
use pitwall_core::testing::TempDir;

use super::group_tests::{init, server};
use super::shortcuts::{self, TileShortcut};
use super::TileView;
use crate::settings::extensions::{card_status, checklist, ServiceFacts, Step};

const WAIT: Duration = Duration::from_secs(60);

fn until(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let end = Instant::now() + WAIT;
    loop {
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < end, "no {what} within {WAIT:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn pill(cx: &mut VisualTestContext) -> (&'static str, String) {
    cx.update(|_, cx| {
        let m = crate::extensions::manifest_of("questions", cx).expect("the fixture");
        let (word, _, why, _) = card_status(&m, None, cx);
        (word, why)
    })
}

fn grant(name: &str, on: bool, cx: &mut VisualTestContext) {
    // As its card's switch does: the running tile hears it.
    cx.update(|_, cx| {
        crate::extensions::calls::decide("questions", name, on, cx);
        crate::extensions::calls::sync(cx);
    });
}

#[gpui::test]
fn a_tiles_status_is_its_cards_pill_and_connected_step(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-status");
    let root = t.path().join("data");
    let (engine, server) = server(&t);
    cx.executor().allow_parking();
    cx.update(|cx| init(&root, &engine, server, cx));
    let (_tile, cx) = cx.add_window_view(|_, cx| TileView::new("questions", "main", cx));
    // Without agents.read it can't read them: an error, in its words.
    grant(permissions::AGENTS_READ, false, cx);
    cx.update(|_, cx| crate::extensions::hub::hub(cx).update(cx, |h, cx| h.reconcile(cx)));
    until(cx, "Error", |cx| pill(cx).0 == "Error");
    assert!(pill(cx).1.contains("Can't read the agents"), "{:?}", pill(cx));
    // Granted, and restarted from its card: connected, its checklist's
    // last step done.
    grant(permissions::AGENTS_READ, true, cx);
    cx.update(|_, cx| crate::extensions::hub::hub(cx).update(cx, |h, cx| h.restart("questions", cx)));
    until(cx, "Connected", |cx| pill(cx).0 == "Connected");
    let list = cx.update(|_, cx| {
        let m = crate::extensions::manifest_of("questions", cx).unwrap();
        checklist(&m, &crate::ui_state::get(cx).extension("questions"), true, ServiceFacts::of(&m, cx))
    });
    assert_eq!(list.last(), Some(&(Step::Connected, true)), "{list:?}");
}

fn bound(keystroke: &str, cx: &mut VisualTestContext) -> bool {
    let ks = Keystroke::parse(shortcuts::forever(keystroke)).expect("a keystroke");
    let keystroke = keystroke.to_string();
    cx.update(move |_, cx| cx.all_bindings_for_input(&[ks]).iter().any(|b| b.action().as_any().downcast_ref::<TileShortcut>().is_some_and(|a| a.keystroke == keystroke)))
}

#[gpui::test]
fn a_tiles_shortcut_is_off_until_granted_then_bound_and_changeable(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-shortcut");
    let root = t.path().join("data");
    let (engine, server) = server(&t);
    cx.executor().allow_parking();
    cx.update(|cx| init(&root, &engine, server, cx));
    let (_tile, cx) = cx.add_window_view(|_, cx| TileView::new("questions", "main", cx));
    let mac = cfg!(target_os = "macos");
    let ks = |k: &str| pitwall_core::extensions::keys::shortcut_keystroke(k, mac).unwrap();
    // The permission screen offers it switched off; not granted, not bound.
    let m = cx.update(|_, cx| crate::extensions::manifest_of("questions", cx)).unwrap();
    assert_eq!(crate::settings::extensions::grants::initial_picks(&m, &Default::default()).get(permissions::SHORTCUT), Some(&false));
    cx.update(|_, cx| shortcuts::sync(cx));
    let s = cx.update(|_, cx| shortcuts::of_tile("questions", "main", cx)).unwrap();
    assert!(s.off.as_deref().is_some_and(|w| w.contains("isn't granted")), "{s:?}");
    assert!(!bound(&ks("cmd-shift-y"), cx));
    assert_eq!(cx.update(|_, cx| shortcuts::target(&ks("cmd-shift-y"), cx)), None);
    // Granted: bound, it opens the desk, and the palette shows its keys.
    grant(permissions::SHORTCUT, true, cx);
    cx.update(|_, cx| shortcuts::sync(cx));
    assert!(bound(&ks("cmd-shift-y"), cx));
    assert_eq!(cx.update(|_, cx| shortcuts::target(&ks("cmd-shift-y"), cx)), Some(("questions".into(), "main".into())));
    assert_eq!(cx.update(|_, cx| shortcuts::of_tile("questions", "main", cx)).unwrap().label(), if mac { "⌘⇧Y" } else { "Ctrl+Shift+Alt+Y" });
    // Other keys: Pitwall's own are refused; free ones replace its own.
    let err = cx.update(|_, cx| shortcuts::set_keys("questions", "main", "cmd-k", cx)).unwrap_err();
    assert!(err.contains("Pitwall's own"), "{err}");
    cx.update(|_, cx| shortcuts::set_keys("questions", "main", "cmd-shift-u", cx)).unwrap();
    assert!(bound(&ks("cmd-shift-u"), cx));
    assert_eq!(cx.update(|_, cx| shortcuts::target(&ks("cmd-shift-u"), cx)), Some(("questions".into(), "main".into())));
    assert_eq!(cx.update(|_, cx| shortcuts::target(&ks("cmd-shift-y"), cx)), None, "its old keys open nothing now");
    // Empty: the keys it suggests, again.
    cx.update(|_, cx| shortcuts::set_keys("questions", "main", "", cx)).unwrap();
    assert!(cx.update(|_, cx| crate::ui_state::get(cx).extension("questions").shortcuts.is_empty()));
    assert_eq!(cx.update(|_, cx| shortcuts::target(&ks("cmd-shift-y"), cx)), Some(("questions".into(), "main".into())));
    // Revoked: off again.
    grant(permissions::SHORTCUT, false, cx);
    assert_eq!(cx.update(|_, cx| shortcuts::target(&ks("cmd-shift-y"), cx)), None);
}
