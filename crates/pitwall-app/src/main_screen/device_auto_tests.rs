//! A device's tile beside the agent that started it, in Pitwall's windows
//! (made-up agents and devices, no engine, nothing streamed): placed right
//! of the agent's terminal without the keys, never twice, an agent's
//! `show_device`, the offer's "Open in Pitwall", the settings.

use gpui::{AppContext, Modifiers, TestAppContext, VisualTestContext};
use pitwall_proto::code;

use super::*;
use crate::agents::tests::agent;
use crate::device::{Device, Platform, State};

fn main_view(cx: &mut TestAppContext) -> gpui::WindowHandle<crate::ui::MainView> {
    cx.update(|cx| crate::windows::main_window(crate::windows::MAIN, cx).unwrap())
}

fn frames(cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update(|cx| {
            for w in cx.windows() {
                let _ = w.update(cx, |_, window, _| window.refresh());
            }
        });
        cx.run_until_parked();
    }
}

/// Pitwall's main window, agents api and web in a space of it, api's
/// terminal focused.
fn app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        init(cx);
        crate::ui_state::init(None, cx);
        crate::settings::init(cx, None, None);
        crate::palette::init(cx);
        let store = cx.new(|_| AgentStore::new(vec![agent("api", "/work/alpha", "idle", 1, true), agent("web", "/work/alpha", "idle", 2, true)]));
        let show_main = super::inbox_tests::show_main;
        cx.set_global(super::inbox_tests::Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: crate::ui::Content::Live { store: store.clone() }, root: None, main: None });
        crate::windows::init(crate::ui::Content::Live { store }, None, show_main, cx);
        show_main(cx);
        crate::windows::restore(cx);
    });
    cx.run_until_parked();
    let h = main_view(cx);
    VisualTestContext::from_window(h.into(), cx).simulate_resize(gpui::size(gpui::px(1500.), gpui::px(800.)));
    cx.update(|cx| {
        let (ui, id) = crate::ui_state::get(cx).create_custom_space(crate::windows::MAIN);
        let ui = ui.place_agent(&id, "api", None, None);
        crate::ui_state::update(cx, |s| *s = ui);
        let h = crate::windows::main_window(crate::windows::MAIN, cx).unwrap();
        let s = h.update(cx, |v, _, _| v.screen().unwrap()).unwrap();
        s.update(cx, |s, cx| s.show_space(&id, cx));
    });
    frames(cx);
    main_view(cx).update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.show_agent("api", window, cx))).unwrap();
    frames(cx);
}

fn iphone() -> Device {
    Device { platform: Platform::Ios, id: "AAAA".into(), name: "iPhone 17 (iOS 26.1)".into(), virtual_device: true, state: State::Ready }
}

fn device_tiles(cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| crate::ui_state::get(cx).spaces.iter().flat_map(|s| s.layout.leaves().into_iter().filter_map(|p| p.device.clone())).collect())
}

fn focused(cx: &mut TestAppContext) -> Option<gpui::FocusHandle> {
    main_view(cx).update(cx, |_, window, cx| window.focused(cx)).unwrap()
}

/// Opening on its own: right of api's terminal, the keys and the window
/// in front where they were; again (or a second boot of it while open):
/// the same tile; an agent whose terminal isn't shown: nothing (the app
/// offers it instead).
#[gpui::test]
fn a_device_opens_right_of_its_agent_without_the_keys(cx: &mut TestAppContext) {
    app(cx);
    let before = focused(cx);
    let front = cx.update(|cx| cx.active_window().map(|w| w.window_id()));
    let (at, reused, note) = cx.update(|cx| device_auto::show_beside("ios:AAAA", "api", false, cx)).expect("placed");
    frames(cx);
    assert!(at.contains("right of api's terminal"), "{at}");
    assert_eq!((reused, note), (false, None));
    assert_eq!(device_tiles(cx), ["ios:AAAA"]);
    assert_eq!(focused(cx), before, "the keys stay");
    assert_eq!(cx.update(|cx| cx.active_window().map(|w| w.window_id())), front, "nothing comes forward");
    let (_, reused, _) = cx.update(|cx| device_auto::show_beside("ios:AAAA", "api", false, cx)).expect("there");
    assert!(reused);
    assert_eq!(device_tiles(cx), ["ios:AAAA"], "never two");
    assert!(cx.update(|cx| device_auto::show_beside("android:emulator-5554", "web", false, cx)).is_err(), "web's terminal isn't shown");
    assert_eq!(device_tiles(cx), ["ios:AAAA"]);
    // Closed by the user: gone (opening it again is the watch's to refuse).
    cx.update(|cx| device_tile::close("ios:AAAA", cx));
    frames(cx);
    assert!(device_tiles(cx).is_empty());
}

/// An agent's `show_device`: the tile beside it, the answer says which
/// and where and nothing of the screen; an agent whose terminal isn't
/// shown still gets it shown, said so; refused while off.
#[gpui::test]
fn an_agent_shows_a_device(cx: &mut TestAppContext) {
    app(cx);
    let before = focused(cx);
    let r = cx.update(|cx| device_auto::show_for_agent(&iphone(), "api", true, cx)).expect("shown");
    frames(cx);
    assert_eq!((r.device.as_str(), r.name.as_str(), r.reused, r.waited), ("ios:AAAA", "iPhone 17 (iOS 26.1)", false, true));
    assert!(r.at.starts_with("the device tile “iPhone 17 (iOS 26.1)” in space"), "{}", r.at);
    assert!(r.at.contains("right of api's terminal"), "{}", r.at);
    let mut keys: Vec<String> = serde_json::to_value(&r).unwrap().as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["at", "device", "inBackground", "name", "note", "reused", "waited", "window"]);
    assert_eq!(focused(cx), before, "the keys stay");
    let r = cx.update(|cx| device_auto::show_for_agent(&iphone(), "api", false, cx)).expect("shown");
    assert!(r.reused);
    assert_eq!(device_tiles(cx).len(), 1);
    // web isn't shown in any window: in the space shown, and said.
    let pixel = Device { platform: Platform::Android, id: "emulator-5554".into(), name: "Pixel 8".into(), virtual_device: true, state: State::Ready };
    let r = cx.update(|cx| device_auto::show_for_agent(&pixel, "web", false, cx)).expect("shown");
    assert!(r.note.as_deref().is_some_and(|n| n.contains("isn't shown in any open Pitwall window")), "{r:?}");
    assert_eq!(device_tiles(cx).len(), 2);
    // Off: refused.
    cx.update(|cx| {
        let def = pitwall_proto::settings::find(device_auto::SHOW_SETTING).unwrap();
        crate::ui_state::update(cx, |s| *s = crate::settings::data::with(s, def, &serde_json::Value::Bool(false)));
    });
    let e = cx.update(|cx| device_auto::show_for_agent(&iphone(), "api", false, cx)).unwrap_err();
    assert_eq!(e.code, code::OFF);
    assert!(e.message.contains("agents.showDevice"));
}

/// A boot nobody's: the quiet offer; its "Open in Pitwall" opens the tile
/// (from inside the toast's own click, deferred).
#[gpui::test]
fn the_offer_opens_the_device_when_asked(cx: &mut TestAppContext) {
    app(cx);
    cx.update(|cx| crate::windows::device_offer("ios:AAAA", "iPhone 17 started".into(), "Open its screen in Pitwall to watch and use it.".into(), Some("api".into()), cx));
    frames(cx);
    let screen = main_view(cx).update(cx, |v, _, _| v.screen().unwrap()).unwrap();
    assert_eq!(screen.read_with(cx, |s, _| s.toast_titles()), ["iPhone 17 started"]);
    assert!(device_tiles(cx).is_empty(), "only offered");
    let b = VisualTestContext::from_window(main_view(cx).into(), cx).debug_bounds("toast-act-0").expect("Open in Pitwall is drawn");
    VisualTestContext::from_window(main_view(cx).into(), cx).simulate_click(b.center(), Modifiers::none());
    frames(cx);
    assert_eq!(device_tiles(cx), ["ios:AAAA"]);
    assert!(screen.read_with(cx, |s, _| s.toast_titles()).is_empty(), "answered");
    // Nobody's: Open in Pitwall puts it in the space shown.
    cx.update(|cx| crate::windows::device_offer("android:emulator-5554", "Pixel 8 started".into(), String::new(), None, cx));
    frames(cx);
    let b = VisualTestContext::from_window(main_view(cx).into(), cx).debug_bounds("toast-act-0").expect("drawn");
    VisualTestContext::from_window(main_view(cx).into(), cx).simulate_click(b.center(), Modifiers::none());
    frames(cx);
    assert_eq!(device_tiles(cx).len(), 2);
}

#[test]
fn the_settings_read_as_modes() {
    use crate::device::auto::Mode;
    let def = pitwall_proto::settings::find(device_auto::AUTO_SETTING).unwrap();
    assert_eq!(def.default_value(), serde_json::json!("beside"));
    for (w, m) in [("beside", Mode::Beside), ("offer", Mode::Offer), ("off", Mode::Off)] {
        assert!(def.validate(&serde_json::json!(w)).is_ok());
        assert_eq!(Mode::parse(w), m);
    }
    assert!(def.validate(&serde_json::json!("always")).is_err());
    assert_eq!(pitwall_proto::settings::find(device_auto::SHOW_SETTING).unwrap().default_value(), serde_json::json!(true));
}
