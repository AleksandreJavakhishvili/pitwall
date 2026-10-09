//! Windows in a test app: moving a space out and back, closing a window,
//! routing an agent to the window that shows it, and windows.json across a
//! "restart" (made-up agents, no engine).

use gpui::{AppContext, Entity, TestAppContext, WindowHandle};

use super::*;
use crate::agents::tests::agent;
use crate::agents::AgentStore;
use crate::main_screen::workspace::ALL_SPACE;

struct Main(Option<WindowHandle<MainView>>, Entity<AgentStore>);

impl Global for Main {}

fn show_main(cx: &mut App) -> WindowHandle<MainView> {
    if let Some(h) = cx.global::<Main>().0 {
        if cx.windows().iter().any(|w| w.window_id() == h.window_id()) {
            return h;
        }
    }
    let store = cx.global::<Main>().1.clone();
    let h = cx
        .open_window(Default::default(), |window, cx| {
            cx.new(|cx| MainView::new(Content::Live { store }, None, window, cx))
        })
        .unwrap();
    cx.global_mut::<Main>().0 = Some(h);
    h
}

fn screen_of(label: &str, cx: &mut App) -> Entity<MainScreen> {
    let h = state(cx).handle(label).expect(label);
    h.read(cx).unwrap().screen.clone().unwrap()
}

/// An app with two agents, a custom space holding "web", and main open.
fn app(cx: &mut TestAppContext, root: Option<PathBuf>) -> String {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::main_screen::init(cx);
        crate::ui_state::init(root.clone(), cx);
        let store = cx.new(|_| {
            AgentStore::new(vec![
                agent("api", "/work/alpha", "idle", 1, true),
                agent("web", "/work/alpha", "blocked", 2, true),
            ])
        });
        cx.set_global(Main(None, store.clone()));
        init(Content::Live { store }, root, show_main, cx);
        show_main(cx);
        restore(cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let ui = crate::ui_state::get(cx);
        if let Some(sp) = ui.spaces.iter().find(|s| s.id != ALL_SPACE) {
            return sp.id.clone();
        }
        let (ui, id) = ui.create_custom_space(MAIN);
        let ui = ui.place_agent(&id, "web", None, None);
        crate::ui_state::update(cx, |s| *s = ui);
        id
    })
}

#[gpui::test]
fn a_space_moves_to_a_new_window_and_comes_back_on_close(cx: &mut TestAppContext) {
    let space = app(cx, None);
    cx.update(|cx| move_to_new_window(&space, MAIN, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(labels(cx), [MAIN, "pitwall-1"]);
        let ui = crate::ui_state::get(cx);
        assert_eq!(ui.window_of_space(&space), "pitwall-1");
        // Not duplicated: main's tabs no longer have it.
        assert!(ui.spaces_of(MAIN).iter().all(|s| s.id != space));
        let second = screen_of("pitwall-1", cx);
        assert_eq!(
            second.read(cx).focused_agent_id().as_deref(),
            Some("web"),
            "the new window shows the space it owns"
        );
    });
    // "All" never leaves main.
    cx.update(|cx| move_to_new_window(ALL_SPACE, MAIN, cx));
    cx.update(|cx| assert_eq!(labels(cx).len(), 2));
    // Closing the window gives the space back.
    cx.update(|cx| {
        let h = state(cx).handle("pitwall-1").unwrap();
        h.update(cx, |_, window, _| window.remove_window()).unwrap();
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(labels(cx), [MAIN]);
        assert_eq!(crate::ui_state::get(cx).window_of_space(&space), MAIN);
    });
}

#[gpui::test]
fn a_window_whose_space_closed_stays_open(cx: &mut TestAppContext) {
    let space = app(cx, None);
    cx.update(|cx| move_to_new_window(&space, MAIN, cx));
    cx.run_until_parked();
    cx.update(|cx| crate::ui_state::update(cx, |ui| *ui = ui.clone().close_space(&space)));
    cx.run_until_parked();
    cx.update(|cx| {
        // As in the Tauri app: "No spaces in this window", still open.
        assert_eq!(labels(cx), [MAIN, "pitwall-1"]);
        assert!(crate::ui_state::get(cx).spaces_of("pitwall-1").is_empty());
    });
}

#[gpui::test]
fn showing_an_agent_of_another_window_focuses_it_there(cx: &mut TestAppContext) {
    let space = app(cx, None);
    cx.update(|cx| move_to_new_window(&space, MAIN, cx));
    cx.run_until_parked();
    // Main shows "api"; "web" lives in pitwall-1.
    let main = cx.update(|cx| state(cx).handle(MAIN).unwrap());
    main.update(cx, |view, window, cx| {
        let s = view.screen.clone().unwrap();
        s.update(cx, |s, cx| s.show_agent("web", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let second = state(cx).handle("pitwall-1").unwrap();
        assert_eq!(
            cx.active_window().map(|w| w.window_id()),
            Some(second.window_id()),
            "the owning window comes forward"
        );
        let ui = crate::ui_state::get(cx);
        assert_eq!(ui.locate("web").unwrap().window, "pitwall-1", "not moved");
        assert!(ui
            .spaces_of(MAIN)
            .iter()
            .all(|s| !s.layout.agents().contains(&"web".into())));
    });
}

#[gpui::test]
fn windows_come_back_after_a_restart(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("pw-windows-restart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let space = app(cx, Some(dir.clone()));
    cx.update(|cx| move_to_new_window(&space, MAIN, cx));
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    let saved = file::load(&dir);
    let second = saved
        .iter()
        .find(|e| e.label == "pitwall-1")
        .expect("saved");
    assert_eq!(second.space_id.as_deref(), Some(space.as_str()));
    assert!(second.bounds.is_some());
    let ui: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("ui.json")).unwrap()).unwrap();
    assert_eq!(ui["windowOf"][&space], "pitwall-1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn a_restart_reopens_saved_windows_and_reclaims_lost_ones(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("pw-windows-reopen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // As a previous run (or the Tauri app) left them.
    let mut ui = crate::main_screen::workspace::UiState::default();
    let (next, a) = ui.create_custom_space(MAIN);
    let (next, b) = next.create_custom_space(MAIN);
    ui = ownership::move_space(next, &a, "pitwall-2");
    ui = ownership::move_space(ui, &b, "pitwall-9");
    ui.wall = vec!["pitwall-2".into()];
    crate::main_screen::workspace::save(&dir, &ui).unwrap();
    file::save(
        &dir,
        &[
            file::WindowEntry {
                label: MAIN.into(),
                space_id: None,
                bounds: Some(file::Bounds {
                    x: 0,
                    y: 0,
                    width: 2000,
                    height: 1400,
                }),
            },
            file::WindowEntry {
                label: "pitwall-2".into(),
                space_id: Some(a.clone()),
                bounds: Some(file::Bounds {
                    x: 200,
                    y: 100,
                    width: 1600,
                    height: 1000,
                }),
            },
            // Its space was closed: it reopens empty, as in the Tauri app.
            file::WindowEntry {
                label: "pitwall-3".into(),
                space_id: Some("space-closed".into()),
                bounds: None,
            },
        ],
    )
    .unwrap();
    app(cx, Some(dir.clone()));
    cx.update(|cx| {
        assert_eq!(labels(cx), [MAIN, "pitwall-2", "pitwall-3"]);
        let ui = crate::ui_state::get(cx);
        assert_eq!(ui.window_of_space(&a), "pitwall-2");
        assert_eq!(ui.window_of_space(&b), MAIN, "pitwall-9 is gone");
        assert_eq!(
            ui.wall,
            vec!["pitwall-2".to_string()],
            "per-window Wall kept"
        );
        assert_eq!(
            screen_of("pitwall-2", cx).read(cx).route(),
            crate::main_screen::Route::Wall
        );
        assert_eq!(
            screen_of(MAIN, cx).read(cx).route(),
            crate::main_screen::Route::Space
        );
    });
    let _ = std::fs::remove_dir_all(&dir);
}
