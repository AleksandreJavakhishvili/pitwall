//! The Settings tile in Pitwall's windows (made-up agents, no engine):
//! handed over from the dialog, opened as the palette does, its full size
//! at every width (the page list over the page when narrow), its pages,
//! focus and ⌘⏎, a window of its own and back, back after a restart at
//! the same page; one Settings at a time; an agent's `show_settings`.

use gpui::{AppContext, Focusable, Modifiers, TestAppContext, VisualTestContext};
use pitwall_proto::settings::Page;
use pitwall_proto::{code, SettingsShow};

use super::tree::{Content, Surface, SETTINGS_KEY};
use super::*;
use crate::agents::tests::agent;

/// The platform's Maximise key (⌘⏎ on macOS, Ctrl+Shift+Enter elsewhere).
const MAXIMISE: &str = if cfg!(target_os = "macos") { "cmd-enter" } else { "ctrl-shift-enter" };

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

/// Pitwall's main window (agent api alone in a space of it), Settings'
/// keys bound; the window `width` wide. The space's id.
fn app(cx: &mut TestAppContext, width: f32) -> String {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        init(cx);
        crate::ui_state::init(None, cx);
        crate::settings::init(cx, None, None);
        crate::palette::init(cx);
        let store = cx.new(|_| AgentStore::new(vec![agent("api", "/work/alpha", "idle", 1, true)]));
        let show_main = super::inbox_tests::show_main;
        cx.set_global(super::inbox_tests::Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: crate::ui::Content::Live { store: store.clone() }, root: None, main: None });
        crate::windows::init(crate::ui::Content::Live { store }, None, show_main, cx);
        show_main(cx);
        crate::windows::restore(cx);
    });
    cx.run_until_parked();
    resize(cx, width);
    let space = cx.update(|cx| {
        let (ui, id) = crate::ui_state::get(cx).create_custom_space(crate::windows::MAIN);
        let ui = ui.place_agent(&id, "api", None, None);
        crate::ui_state::update(cx, |s| *s = ui);
        let sid = id.clone();
        let h = crate::windows::main_window(crate::windows::MAIN, cx).unwrap();
        let s = h.update(cx, |v, _, _| v.screen().unwrap()).unwrap();
        s.update(cx, |s, cx| s.show_space(&sid, cx));
        id
    });
    frames(cx);
    space
}

fn resize(cx: &mut TestAppContext, width: f32) {
    let h = main_view(cx);
    VisualTestContext::from_window(h.into(), cx).simulate_resize(gpui::size(gpui::px(width), gpui::px(800.)));
    frames(cx);
}

/// The main window's drawn bounds of `sel`.
fn bounds(cx: &mut TestAppContext, sel: &'static str) -> Option<gpui::Bounds<gpui::Pixels>> {
    let h = main_view(cx);
    VisualTestContext::from_window(h.into(), cx).debug_bounds(sel)
}

fn click(cx: &mut TestAppContext, sel: &'static str) {
    let b = bounds(cx, sel).unwrap_or_else(|| panic!("{sel} is drawn"));
    let h = main_view(cx);
    VisualTestContext::from_window(h.into(), cx).simulate_click(b.center(), Modifiers::none());
    frames(cx);
}

fn keys(cx: &mut TestAppContext, k: &str) {
    let h = main_view(cx);
    VisualTestContext::from_window(h.into(), cx).simulate_keystrokes(k);
    frames(cx);
}

fn dialog_open(cx: &mut TestAppContext) -> bool {
    main_view(cx).update(cx, |v, _, cx| v.layer_open(cx)).unwrap()
}

fn tiles(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| crate::ui_state::get(cx).spaces.iter().map(|s| s.layout.leaves().iter().filter(|p| p.settings).count()).sum())
}

fn tile_focused(cx: &mut TestAppContext) -> bool {
    let t = cx.update(|cx| super::settings_tile::get(cx)).expect("the tile");
    main_view(cx).update(cx, |_, window, cx| t.read(cx).focus_handle(cx).contains_focused(window, cx)).unwrap()
}

/// The dialog's "Open as a tile": the same Settings (its unsaved fields
/// with it) in a tile right of api's, the dialog closed, the keys there.
/// Then Settings' shortcut focuses the tile instead of opening the dialog,
/// and "Open Settings as a tile" opens no second one.
#[gpui::test]
fn the_dialog_becomes_a_tile_and_there_is_one_settings(cx: &mut TestAppContext) {
    app(cx, 1500.);
    main_view(cx).update(cx, |v, _, cx| v.open_settings(cx)).unwrap();
    frames(cx);
    assert!(dialog_open(cx));
    let dialog = main_view(cx).update(cx, |v, _, cx| v.layer_settings(cx)).unwrap().expect("the dialog's Settings");
    assert!(bounds(cx, "settings-to-tile").is_some(), "the button in the dialog's head");
    click(cx, "settings-to-tile");
    assert!(!dialog_open(cx), "the dialog closed");
    assert_eq!(tiles(cx), 1);
    let tile = cx.update(|cx| super::settings_tile::get(cx)).expect("the tile");
    assert_eq!(tile.read_with(cx, |t, _| t.settings().entity_id()), dialog.entity_id(), "the dialog's own Settings");
    assert!(tile_focused(cx), "the keys in the tile");
    assert!(bounds(cx, "settings-to-tile").is_none(), "no such button in a tile");
    let at = cx.update(|cx| super::inbox_window::described(SETTINGS_KEY, cx));
    assert!(at.starts_with("the Settings tile in space") && at.contains("right of api's terminal"), "{at}");

    // Settings' shortcut (and every "Settings: <page>"): the tile, no dialog.
    main_view(cx).update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.show_agent("api", window, cx))).unwrap();
    frames(cx);
    main_view(cx).update(cx, |v, _, cx| v.open_settings(cx)).unwrap();
    frames(cx);
    assert!(!dialog_open(cx), "never a second Settings");
    assert!(tile_focused(cx));
    // The palette's "Open Settings as a tile": the same tile, focused.
    main_view(cx).update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.open_settings_tile(None, window, cx))).unwrap();
    frames(cx);
    assert_eq!(tiles(cx), 1, "never two");
    assert_eq!(cx.update(|cx| super::settings_tile::get(cx)).map(|t| t.entity_id()), Some(tile.entity_id()));

    // Closed: the shortcut opens the dialog again.
    click(cx, "settings-tile-close");
    assert_eq!(tiles(cx), 0);
    assert!(cx.update(|cx| super::settings_tile::get(cx)).is_none());
    main_view(cx).update(cx, |v, _, cx| v.open_settings(cx)).unwrap();
    frames(cx);
    assert!(dialog_open(cx));
    // Opened as a tile from the palette with the dialog open: the dialog
    // goes into it.
    let dialog = main_view(cx).update(cx, |v, _, cx| v.layer_settings(cx)).unwrap().unwrap();
    main_view(cx).update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.open_settings_tile(None, window, cx))).unwrap();
    frames(cx);
    assert!(!dialog_open(cx), "the dialog closed");
    assert_eq!(tiles(cx), 1);
    let _ = dialog;
}

/// Its page list and page at the tile's full size, wide and narrow (the
/// list over the page), never the dialog's fixed width; a page picked in
/// the tile; ⌘⏎ maximises and restores it.
#[gpui::test]
fn the_tile_fills_its_slot_and_has_every_page(cx: &mut TestAppContext) {
    app(cx, 1500.);
    main_view(cx).update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.open_settings_tile(None, window, cx))).unwrap();
    frames(cx);
    assert_eq!(tiles(cx), 1);
    // Side by side with api's terminal at 1500 and 1100 (narrow), under
    // it at 800 (a small window stacks them).
    let mut seen = vec![];
    for width in [1500., 1100., 800.] {
        resize(cx, width);
        let pane = bounds(cx, "settings-tile").expect("the tile");
        let tiled = bounds(cx, "settings-tiled").expect("Settings in it");
        let nav = bounds(cx, "settings-nav").expect("the page list");
        let page = bounds(cx, "settings-page-body").expect("the page");
        let near = |a: gpui::Pixels, b: gpui::Pixels| (f32::from(a) - f32::from(b)).abs() < 1.5;
        assert!(near(tiled.size.width, pane.size.width), "{width}: its full width ({tiled:?} in {pane:?})");
        assert!(near(tiled.bottom(), pane.bottom()), "{width}: its full height");
        let narrow = f32::from(tiled.size.width) < 560.;
        seen.push(narrow);
        let t = cx.update(|cx| super::settings_tile::get(cx)).unwrap();
        assert_eq!(t.read_with(cx, |t, cx| t.settings().read(cx).narrow()), narrow, "{width}");
        if narrow {
            assert!(near(nav.size.width, tiled.size.width) && page.top() >= nav.bottom() - gpui::px(1.), "{width}: the list over the page ({nav:?}, {page:?})");
        } else {
            assert!(near(nav.top(), tiled.top()) && page.left() >= nav.right() - gpui::px(1.), "{width}: the list beside the page");
            assert!(near(page.right(), tiled.right()), "{width}: the page to the tile's edge");
        }
        assert!(page.size.height > gpui::px(100.), "{width}: room for the page");
    }
    assert_eq!(seen, [false, true, false], "wide, narrow, wide again");
    resize(cx, 1500.);

    // A page picked in the tile: shown (and remembered).
    click(cx, "settings-page-agents");
    assert_eq!(cx.update(|cx| crate::settings::data::page(cx)), Page::Agents);
    // agents.showSettings is on it, beside showFiles and showWeb, and
    // turns off with a click.
    let t = cx.update(|cx| super::settings_tile::get(cx)).unwrap();
    for _ in 0..12 {
        if bounds(cx, "show-settings").is_some() {
            break;
        }
        t.read_with(cx, |t, _| t.settings()).update(cx, |v, cx| {
            v.scroll_by(gpui::px(200.));
            cx.notify();
        });
        frames(cx);
    }
    assert!(cx.update(|cx| super::settings_tile::enabled(cx)), "on unless turned off");
    click(cx, "show-settings");
    assert!(!cx.update(|cx| super::settings_tile::enabled(cx)), "turned off in the tile");
    click(cx, "settings-page-extensions");
    assert_eq!(cx.update(|cx| crate::settings::data::page(cx)), Page::Extensions);

    // Focused by a click, and ⌘⏎ maximises it like any tile.
    main_view(cx).update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.show_agent("api", window, cx))).unwrap();
    frames(cx);
    let focused_pane = |cx: &mut TestAppContext| cx.update(|cx| {
        let ui = crate::ui_state::get(cx);
        let (space, pane) = ui.tile_of(&Content::Settings).unwrap();
        ui.space(&space).and_then(|s| s.focused_pane_id.clone()) == Some(pane)
    });
    assert!(!focused_pane(cx));
    click(cx, "settings-tile-bar");
    assert!(focused_pane(cx), "the space's focused tile");
    assert!(tile_focused(cx), "with the keys");
    let maximized = |cx: &mut TestAppContext| cx.update(|cx| {
        let ui = crate::ui_state::get(cx);
        let (space, pane) = ui.tile_of(&Content::Settings).unwrap();
        ui.space(&space).and_then(|s| s.maximized_pane_id.clone()) == Some(pane)
    });
    let before = bounds(cx, "settings-tile").unwrap();
    keys(cx, MAXIMISE);
    assert!(maximized(cx), "maximised");
    let tile = bounds(cx, "settings-tile").unwrap();
    assert!(tile.size.width > before.size.width + gpui::px(200.), "the whole space: {tile:?} (was {before:?})");
    assert!(tile_focused(cx), "still with the keys");
    keys(cx, MAXIMISE);
    assert!(!maximized(cx), "restored");
    // Its own button does the same.
    click(cx, "settings-tile-maximize");
    assert!(maximized(cx));
    click(cx, "settings-tile-maximize");
    assert!(!maximized(cx));
}

/// Out into a window of its own and back into the layout, the same
/// Settings; after a "restart" it is where it was, at the same page.
#[gpui::test]
fn the_tile_moves_to_a_window_and_comes_back_after_a_restart(cx: &mut TestAppContext) {
    app(cx, 1500.);
    main_view(cx).update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.open_settings_tile(None, window, cx))).unwrap();
    frames(cx);
    cx.update(|cx| crate::settings::data::set_page(cx, Page::Rules));
    let tile = cx.update(|cx| super::settings_tile::get(cx)).unwrap();
    let same = |cx: &mut TestAppContext| cx.update(|cx| super::settings_tile::get(cx)).map(|e| e.entity_id()) == Some(tile.entity_id());

    cx.update(|cx| super::inbox_window::pop_out(SETTINGS_KEY, None, true, cx)).expect("popped out");
    frames(cx);
    cx.update(|cx| {
        assert!(crate::windows::inbox_window(SETTINGS_KEY, cx).is_some(), "a window");
        assert!(crate::ui_state::get(cx).settings_window, "kept in ui.json");
        assert_eq!(super::inbox_window::title(SETTINGS_KEY, cx), "Settings");
        assert_eq!(super::inbox_window::described(SETTINGS_KEY, cx), "a window of its own");
        assert!(super::settings_tile::placed(cx));
    });
    assert_eq!(tiles(cx), 0);
    assert!(same(cx));
    // Settings' shortcut finds it there.
    main_view(cx).update(cx, |v, _, cx| v.open_settings(cx)).unwrap();
    frames(cx);
    assert!(!dialog_open(cx));
    let w = cx.update(|cx| crate::windows::inbox_window(SETTINGS_KEY, cx)).unwrap();
    assert!(w.update(cx, |_, window, cx| tile.read(cx).focus_handle(cx).contains_focused(window, cx)).unwrap(), "its window's Settings has the keys");
    // Back right of api's terminal; its window closes.
    let at = cx.update(|cx| super::inbox_window::place(SETTINGS_KEY, crate::explorer::dock::FilesPlace::Next(super::tree::Side::Right), Some("api"), cx)).expect("docked");
    frames(cx);
    assert!(at.contains("right of api's terminal"), "{at}");
    cx.update(|cx| {
        assert!(crate::windows::inbox_window(SETTINGS_KEY, cx).is_none());
        assert!(!crate::ui_state::get(cx).settings_window);
    });
    assert_eq!(tiles(cx), 1);
    assert!(same(cx));
    // A tile's bar let go outside every window: a window there.
    let r = cx.update(|cx| crate::explorer::drag::finish_what(&Content::Settings, gpui::point(gpui::px(-5000.), gpui::px(-5000.)), crate::explorer::drag::From::Tile, cx));
    assert_eq!(r.expect("moved").as_deref(), Some("a window of its own"));
    frames(cx);
    assert!(same(cx));

    // "Restarted" with it in its window: open again, at the same page.
    cx.update(|cx| {
        if let Some(h) = crate::windows::inbox_window(SETTINGS_KEY, cx) {
            let _ = h.update(cx, |w, window, _| {
                w.closing();
                window.remove_window();
            });
        }
    });
    cx.run_until_parked();
    let saved = cx.update(|cx| serde_json::to_value(crate::ui_state::get(cx)).unwrap());
    assert_eq!(saved["settingsWindow"], true);
    cx.update(super::inbox_window::restore);
    frames(cx);
    cx.update(|cx| {
        assert!(crate::windows::inbox_window(SETTINGS_KEY, cx).is_some(), "open again");
        assert_eq!(crate::settings::data::page(cx), Page::Rules, "at the same page");
    });
}

/// A layout with the Settings tile reads back with it, at its page; one
/// saved before it reads the same.
#[test]
fn layouts_keep_the_settings_tile() {
    let n = super::tree::Node::content_pane(&Content::Settings);
    let json = serde_json::to_value(&n).unwrap();
    assert_eq!(json["settings"], true);
    let back: super::tree::Node = serde_json::from_value(json).unwrap();
    assert_eq!(back.leaves()[0].content(), Some(Content::Settings));
    let old = r#"{"type":"pane","id":"a","agentId":"x"}"#;
    let n: super::tree::Node = serde_json::from_str(old).unwrap();
    assert_eq!(serde_json::to_string(&n).unwrap(), old, "written back unchanged");
    assert_eq!(Surface::parse(SETTINGS_KEY), Surface::Settings);
    assert_eq!(Surface::Settings.key(), SETTINGS_KEY);
    let ui = UiState::default().surface_to_window(&Surface::Settings);
    assert!(ui.surface_in_window(&Surface::Settings));
    let ui: UiState = serde_json::from_value(serde_json::to_value(&ui).unwrap()).unwrap();
    assert!(ui.settings_window, "kept");
    assert!(!ui.close_surface(&Surface::Settings).settings_window);
    // The page Settings shows is kept with it.
    let mut ui = UiState::default();
    ui.rest.insert("settingsPage".into(), serde_json::json!("extensions"));
    let ui: UiState = serde_json::from_value(serde_json::to_value(&ui).unwrap()).unwrap();
    assert_eq!(crate::settings::data::page_of(&ui), Page::Extensions);
}

/// An agent's `show_settings`: a tile right of its terminal at the page,
/// then the same tile switched to another page; the keys and the window
/// in front never move; nothing about Settings in the answer; a dialog
/// left open goes into the tile; refused while `agents.showSettings` is
/// off or for a page there isn't.
#[gpui::test]
fn an_agent_shows_settings_without_taking_anything(cx: &mut TestAppContext) {
    app(cx, 1500.);
    main_view(cx).update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.show_agent("api", window, cx))).unwrap();
    frames(cx);
    let focus_before = main_view(cx).update(cx, |_, window, cx| window.focused(cx)).unwrap();
    let front_before = cx.update(|cx| cx.active_window().map(|w| w.window_id()));
    let ask = |page: Option<&str>, ext: Option<&str>| SettingsShow { agent_id: "api".into(), page: page.map(String::from), extension: ext.map(String::from), secret: None, connection: None };

    let r = cx.update(|cx| super::settings_tile::show_for_agent(&ask(Some("agents"), None), cx)).expect("shown");
    frames(cx);
    assert_eq!((r.page.as_str(), r.reused, r.note.as_deref()), ("agents", false, None));
    assert!(r.at.contains("right of api's terminal"), "{}", r.at);
    assert_eq!(tiles(cx), 1);
    assert_eq!(cx.update(|cx| crate::settings::data::page(cx)), Page::Agents);
    // Nothing of what Settings holds: where it is, and nothing else.
    let mut keys: Vec<String> = serde_json::to_value(&r).unwrap().as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["at", "inBackground", "note", "page", "reused", "window"]);
    // The keys and the window in front where they were.
    assert_eq!(main_view(cx).update(cx, |_, window, cx| window.focused(cx)).unwrap(), focus_before, "the keys stay");
    assert!(!tile_focused(cx));
    assert_eq!(cx.update(|cx| cx.active_window().map(|w| w.window_id())), front_before, "nothing comes forward");

    // Again, another page and an extension's card: the same tile.
    let tile = cx.update(|cx| super::settings_tile::get(cx)).unwrap();
    let r = cx.update(|cx| super::settings_tile::show_for_agent(&ask(None, Some("demo")), cx)).expect("shown");
    frames(cx);
    assert_eq!((r.page.as_str(), r.reused), ("extensions", true));
    assert_eq!(tiles(cx), 1, "never two");
    assert_eq!(cx.update(|cx| super::settings_tile::get(cx)).map(|t| t.entity_id()), Some(tile.entity_id()));
    assert_eq!(main_view(cx).update(cx, |_, window, cx| window.focused(cx)).unwrap(), focus_before);

    // Not a page: refused, nothing changes.
    let e = cx.update(|cx| super::settings_tile::show_for_agent(&ask(Some("secrets"), None), cx)).unwrap_err();
    assert_eq!(e.code, code::BAD_PARAMS);
    assert_eq!(cx.update(|cx| crate::settings::data::page(cx)), Page::Extensions);

    // A dialog the user left open: it goes into the tile.
    cx.update(super::settings_tile::close);
    frames(cx);
    main_view(cx).update(cx, |v, _, cx| v.open_settings(cx)).unwrap();
    frames(cx);
    let dialog = main_view(cx).update(cx, |v, _, cx| v.layer_settings(cx)).unwrap().unwrap();
    cx.update(|cx| super::settings_tile::show_for_agent(&ask(Some("general"), None), cx)).expect("shown");
    frames(cx);
    assert!(!dialog_open(cx), "one Settings");
    let tile = cx.update(|cx| super::settings_tile::get(cx)).unwrap();
    assert_eq!(tile.read_with(cx, |t, _| t.settings().entity_id()), dialog.entity_id());

    // Off: refused, nothing opened.
    cx.update(super::settings_tile::close);
    frames(cx);
    cx.update(|cx| {
        let def = pitwall_proto::settings::find(super::settings_tile::SETTING).unwrap();
        crate::ui_state::update(cx, |s| *s = crate::settings::data::with(s, def, &serde_json::Value::Bool(false)));
        assert!(!super::settings_tile::enabled(cx));
    });
    let e = cx.update(|cx| super::settings_tile::show_for_agent(&ask(Some("agents"), None), cx)).unwrap_err();
    assert_eq!(e.code, code::OFF);
    assert!(e.message.contains("agents.showSettings"));
    assert_eq!(tiles(cx), 0);
}
