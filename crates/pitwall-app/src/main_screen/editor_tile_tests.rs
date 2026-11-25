//! The editor's tile in Pitwall's windows (made-up agents, no engine):
//! opened as the palette does, its files back after a restart in a tile
//! and in a window of its own, closing it asks about unsaved edits.

use gpui::{Focusable, TestAppContext, VisualTestContext};

use super::tree::{Content, Surface, EDITOR_KEY};
use super::*;
use crate::agents::tests::agent;

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

/// Pitwall's main window, agent api alone in a space.
fn app(cx: &mut TestAppContext) {
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
    VisualTestContext::from_window(main_view(cx).into(), cx).simulate_resize(gpui::size(gpui::px(1400.), gpui::px(800.)));
    cx.update(|cx| {
        let (ui, id) = crate::ui_state::get(cx).create_custom_space(crate::windows::MAIN);
        let ui = ui.place_agent(&id, "api", None, None);
        crate::ui_state::update(cx, |s| *s = ui);
        let h = crate::windows::main_window(crate::windows::MAIN, cx).unwrap();
        let s = h.update(cx, |v, _, _| v.screen().unwrap()).unwrap();
        s.update(cx, |s, cx| s.show_space(&id, cx));
    });
    frames(cx);
}

fn tiles(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| crate::ui_state::get(cx).spaces.iter().map(|s| s.layout.leaves().iter().filter(|p| p.editor).count()).sum())
}

fn folder(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pw-editor-tile-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn tabs(cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| super::editor_tile::editor(cx).map(|e| e.read(cx).tabs().iter().map(|t| t.name()).collect()).unwrap_or_default())
}

/// "New file" in the palette: the editor right of api's terminal with a
/// new file, the keys in it; a file opened from the viewer joins it.
/// After a "restart" (the tile made again from `ui.json`) its files are
/// open again.
#[gpui::test]
fn the_editor_tile_opens_and_comes_back_after_a_restart(cx: &mut TestAppContext) {
    app(cx);
    let d = folder("restart");
    let f = d.join("notes.md");
    std::fs::write(&f, "# Notes\n").unwrap();
    main_view(cx).update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.new_file_in_editor(window, cx))).unwrap();
    frames(cx);
    assert_eq!(tiles(cx), 1);
    assert_eq!(tabs(cx), vec!["Untitled-1"]);
    let e = cx.update(|cx| super::editor_tile::editor(cx)).unwrap();
    assert!(main_view(cx).update(cx, |_, window, cx| e.read(cx).focus_handle(cx).contains_focused(window, cx)).unwrap(), "the new file has the keys");
    let p = f.to_string_lossy().into_owned();
    cx.update(|cx| super::editor_tile::open_path(&p, Some(1), cx));
    frames(cx);
    assert_eq!(tabs(cx), vec!["Untitled-1", "notes.md"]);
    assert_eq!(tiles(cx), 1, "the same tile");
    let at = cx.update(|cx| super::inbox_window::described(EDITOR_KEY, cx));
    assert!(at.starts_with("the editor tile in space"), "{at}");

    // "Restarted": the tile made again from ui.json.
    let saved = cx.update(|cx| serde_json::to_value(crate::ui_state::get(cx)).unwrap());
    assert_eq!(saved["editor"]["tabs"][0], p.as_str());
    cx.update(|cx| cx.set_global(super::editor_tile::Tile(None)));
    frames(cx);
    assert_eq!(tabs(cx), vec!["notes.md"], "files kept; a new file's text is not");

    // A window of its own, and back after a restart there.
    cx.update(|cx| super::inbox_window::pop_out(EDITOR_KEY, None, true, cx)).expect("popped out");
    frames(cx);
    assert_eq!(tiles(cx), 0);
    cx.update(|cx| {
        assert!(crate::ui_state::get(cx).editor_window);
        assert_eq!(super::inbox_window::title(EDITOR_KEY, cx), "Editor");
        assert!(super::editor_tile::placed(cx));
    });
    cx.update(|cx| {
        if let Some(h) = crate::windows::inbox_window(EDITOR_KEY, cx) {
            let _ = h.update(cx, |w, window, _| {
                w.closing();
                window.remove_window();
            });
        }
        cx.set_global(super::editor_tile::Tile(None));
    });
    cx.run_until_parked();
    cx.update(super::inbox_window::restore);
    frames(cx);
    cx.update(|cx| assert!(crate::windows::inbox_window(EDITOR_KEY, cx).is_some(), "open again"));
    assert_eq!(tabs(cx), vec!["notes.md"]);
    let _ = std::fs::remove_dir_all(&d);
}

/// The tile's close button asks about unsaved edits; Don't Save closes it
/// and forgets its files.
#[gpui::test]
fn closing_the_tile_asks_first(cx: &mut TestAppContext) {
    app(cx);
    main_view(cx).update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.new_file_in_editor(window, cx))).unwrap();
    frames(cx);
    let code = cx.update(|cx| super::editor_tile::editor(cx).and_then(|e| e.read(cx).active_code())).unwrap();
    cx.update(|cx| code.update(cx, |c, cx| c.debug_type("draft", cx)));
    let tile = cx.update(|cx| super::editor_tile::get(cx)).unwrap();
    main_view(cx).update(cx, |_, window, cx| tile.update(cx, |t, cx| t.request_close(window, cx))).unwrap();
    cx.run_until_parked();
    let vcx = VisualTestContext::from_window(main_view(cx).into(), cx);
    vcx.simulate_prompt_answer("Cancel");
    frames(cx);
    assert_eq!(tiles(cx), 1, "kept");
    main_view(cx).update(cx, |_, window, cx| tile.update(cx, |t, cx| t.request_close(window, cx))).unwrap();
    cx.run_until_parked();
    let vcx = VisualTestContext::from_window(main_view(cx).into(), cx);
    vcx.simulate_prompt_answer("Don't Save");
    frames(cx);
    assert_eq!(tiles(cx), 0);
    cx.update(|cx| {
        assert!(!super::editor_tile::placed(cx));
        assert!(super::editor_tile::get(cx).is_none());
        assert!(crate::ui_state::get(cx).editor.tabs.is_empty());
        assert!(Surface::of(&Content::Editor) == Some(Surface::Editor));
    });
}
