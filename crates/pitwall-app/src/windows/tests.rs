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

#[gpui::test]
fn a_tab_dragged_out_opens_its_window_where_it_was_let_go(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton, VisualTestContext};
    let space = app(cx, None);
    let main = cx.update(|cx| state(cx).handle(MAIN).unwrap());
    let mut vcx = VisualTestContext::from_window(main.into(), cx);
    vcx.simulate_resize(size(px(1500.), px(900.)));
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let selector: &'static str = Box::leak(format!("tab-name-{space}").into_boxed_str());
    let grab = vcx.debug_bounds(selector).expect("the tab").center();
    let at = point(grab.x + px(30.), px(400.));
    vcx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::none());
    for p in [point(grab.x + px(6.), grab.y), at, at] {
        vcx.simulate_mouse_move(p, MouseButton::Left, Modifiers::none());
    }
    let (b, view) = vcx.update(|window, _| (window.bounds(), window.viewport_size()));
    vcx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(labels(cx), [MAIN, "pitwall-1"]);
        assert_eq!(crate::ui_state::get(cx).window_of_space(&space), "pitwall-1");
        // Its top-left where the tab was let go, kept on the display.
        let released = point(
            b.origin.x + at.x,
            b.origin.y + (b.size.height - view.height) + at.y,
        );
        let (dw, dh) = window_state::DEFAULT_SIZE;
        let area = cx.primary_display().unwrap().bounds();
        let second = state(cx).handle("pitwall-1").unwrap();
        let opened = second.update(cx, |_, window, _| window.bounds()).unwrap();
        assert_eq!(opened.origin, fit_on(released, size(px(dw), px(dh)), area));
        assert_ne!(
            opened.origin,
            point(b.origin.x + px(28.), b.origin.y + px(28.)),
            "not cascaded"
        );
    });
}

#[test]
fn a_dropped_window_is_kept_on_its_display() {
    let area = Bounds::new(point(px(0.), px(0.)), size(px(1920.), px(1080.)));
    let s = size(px(1400.), px(900.));
    let fit = |x: f32, y: f32| fit_on(point(px(x), px(y)), s, area);
    assert_eq!(fit(100., 50.), point(px(100.), px(50.)));
    assert_eq!(fit(1800., 1000.), point(px(520.), px(180.)));
    assert_eq!(fit(-300., -40.), point(px(0.), px(0.)));
    // Bigger than the display: its top-left corner.
    let big = fit_on(point(px(50.), px(50.)), size(px(2400.), px(1400.)), area);
    assert_eq!(big, point(px(0.), px(0.)));
}

#[test]
fn a_point_on_screen_finds_the_frontmost_window_there() {
    let frame =
        |x: f32, y: f32, w: f32, h: f32| Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
    let id = |n: u64| WindowId::from(n);
    let open = vec![
        (MAIN.to_string(), id(1)),
        ("pitwall-1".to_string(), id(2)),
        ("pitwall-2".to_string(), id(3)),
    ];
    // The system's order wins; windows it doesn't list come after, newest first.
    assert_eq!(
        front_to_back(&open, Some(vec![id(2), id(1)]), Some(id(2))),
        ["pitwall-1", MAIN, "pitwall-2"]
    );
    // No order from the platform: the active window, then the newest.
    assert_eq!(
        front_to_back(&open, None, Some(id(1))),
        [MAIN, "pitwall-2", "pitwall-1"]
    );
    assert_eq!(
        front_to_back(&open, None, None),
        ["pitwall-2", "pitwall-1", MAIN]
    );

    // main at the left, pitwall-1 over its right half, pitwall-2 apart.
    let frames = |order: &[&str]| -> Vec<(String, Bounds<Pixels>)> {
        order
            .iter()
            .map(|l| {
                let b = match *l {
                    MAIN => frame(0., 0., 800., 600.),
                    "pitwall-1" => frame(400., 100., 800., 600.),
                    _ => frame(1300., 0., 500., 400.),
                };
                (l.to_string(), b)
            })
            .collect()
    };
    let front = frames(&["pitwall-1", MAIN, "pitwall-2"]);
    let at = |x: f32, y: f32| point(px(x), px(y));
    assert_eq!(
        window_at(at(500., 200.), &front),
        Some("pitwall-1"),
        "overlap: the front one"
    );
    assert_eq!(window_at(at(100., 200.), &front), Some(MAIN));
    assert_eq!(window_at(at(1400., 20.), &front), Some("pitwall-2"));
    assert_eq!(window_at(at(1250., 500.), &front), None, "the desktop");
    let main_first = frames(&[MAIN, "pitwall-1", "pitwall-2"]);
    assert_eq!(window_at(at(500., 200.), &main_first), Some(MAIN));
}

/// Main (holding the custom space, active) and an empty `pitwall-1` to its
/// right on screen, apart; a context to drag main's tabs in.
fn two_windows(
    cx: &mut TestAppContext,
) -> (String, WindowHandle<MainView>, gpui::VisualTestContext) {
    let space = app(cx, None);
    let main = cx.update(|cx| state(cx).handle(MAIN).unwrap());
    let mut vcx = gpui::VisualTestContext::from_window(main.into(), cx);
    vcx.simulate_resize(size(px(1000.), px(700.)));
    let a = vcx.update(|window, _| window.bounds());
    let b = Bounds::new(
        point(a.right() + px(80.), a.top()),
        size(px(900.), px(600.)),
    );
    cx.update(|cx| build("pitwall-1", WindowBounds::Windowed(b), None, None, cx).unwrap());
    vcx.update(|window, _| {
        window.activate_window();
        window.refresh();
    });
    vcx.run_until_parked();
    (space, main, vcx)
}

/// Press on `space`'s tab in `vcx`'s window and drag it to `to` (on
/// screen), without letting go. Where that is in the window.
fn drag_tab_to(vcx: &mut gpui::VisualTestContext, space: &str, to: Point<Pixels>) -> Point<Pixels> {
    use gpui::{Modifiers, MouseButton};
    let selector: &'static str = Box::leak(format!("tab-name-{space}").into_boxed_str());
    let grab = vcx.debug_bounds(selector).expect("the tab").center();
    let (b, view) = vcx.update(|window, _| (window.bounds(), window.viewport_size()));
    let local = point(
        to.x - b.origin.x,
        to.y - b.origin.y - (b.size.height - view.height),
    );
    vcx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::none());
    for p in [point(grab.x + px(6.), grab.y), local, local] {
        vcx.simulate_mouse_move(p, MouseButton::Left, Modifiers::none());
    }
    local
}

#[gpui::test]
fn a_tab_let_go_over_another_window_moves_its_space_there(cx: &mut TestAppContext) {
    use crate::main_screen::SpaceRelease;
    let (space, main, mut vcx) = two_windows(cx);
    let second = cx.update(|cx| state(cx).handle("pitwall-1").unwrap());
    let b = cx.update(|cx| second.update(cx, |_, w, _| w.bounds()).unwrap());
    // Over the other window's top bar.
    let over = point(b.left() + px(200.), b.top() + px(16.));
    let local = drag_tab_to(&mut vcx, &space, over);
    cx.update(|cx| {
        let ghost = screen_of(MAIN, cx).read(cx).space_drag_release(cx);
        assert_eq!(
            ghost,
            Some(SpaceRelease::Into),
            "the ghost says Move to this window"
        );
        let incoming = screen_of("pitwall-1", cx)
            .read(cx)
            .space_incoming()
            .cloned();
        assert!(incoming.is_some(), "its tab bar shows where it lands");
    });
    vcx.simulate_mouse_up(local, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(labels(cx), [MAIN, "pitwall-1"], "no new window");
        let ui = crate::ui_state::get(cx);
        assert_eq!(ui.window_of_space(&space), "pitwall-1");
        assert!(ui.spaces_of(MAIN).iter().all(|s| s.id != space));
        let there = screen_of("pitwall-1", cx);
        assert_eq!(there.read(cx).focused_agent_id().as_deref(), Some("web"));
        assert_eq!(there.read(cx).route(), crate::main_screen::Route::Space);
        assert_eq!(
            there.read(cx).space_incoming(),
            None,
            "the highlight is gone"
        );
        assert_eq!(
            cx.active_window().map(|w| w.window_id()),
            Some(second.window_id()),
            "that window comes forward"
        );
        assert!(!cx.has_active_drag());
    });
    let _ = main;
}

#[gpui::test]
fn a_tab_let_go_on_the_desktop_or_its_own_bar_skips_the_other_window(cx: &mut TestAppContext) {
    use crate::main_screen::SpaceRelease;
    let (space, main, mut vcx) = two_windows(cx);
    let a = vcx.update(|window, _| window.bounds());
    let title = vcx.update(|window, _| a.size.height - window.viewport_size().height);
    // Its own top bar: nothing happens.
    let own = point(a.left() + px(600.), a.top() + title + px(16.));
    let local = drag_tab_to(&mut vcx, &space, own);
    cx.update(|cx| {
        let ghost = screen_of(MAIN, cx).read(cx).space_drag_release(cx);
        assert_eq!(ghost, Some(SpaceRelease::Stay));
        assert_eq!(screen_of("pitwall-1", cx).read(cx).space_incoming(), None);
    });
    vcx.simulate_mouse_up(local, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(labels(cx), [MAIN, "pitwall-1"]);
        assert_eq!(crate::ui_state::get(cx).window_of_space(&space), MAIN);
    });
    // Over the other window, then back over the desktop under main: the
    // highlight goes, and the space opens in a window of its own.
    let second = cx.update(|cx| state(cx).handle("pitwall-1").unwrap());
    let b = cx.update(|cx| second.update(cx, |_, w, _| w.bounds()).unwrap());
    let desktop = point(a.left() + px(300.), a.bottom() + px(60.));
    drag_tab_to(
        &mut vcx,
        &space,
        point(b.left() + px(100.), b.top() + px(16.)),
    );
    let local = point(desktop.x - a.origin.x, desktop.y - a.origin.y - title);
    vcx.simulate_mouse_move(local, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.update(|cx| {
        let ghost = screen_of(MAIN, cx).read(cx).space_drag_release(cx);
        assert_eq!(ghost, Some(SpaceRelease::NewWindow));
        assert_eq!(screen_of("pitwall-1", cx).read(cx).space_incoming(), None);
    });
    vcx.simulate_mouse_up(local, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(labels(cx), [MAIN, "pitwall-1", "pitwall-2"]);
        let ui = crate::ui_state::get(cx);
        assert_eq!(ui.window_of_space(&space), "pitwall-2");
        assert!(ui.spaces_of("pitwall-1").is_empty());
    });
    let _ = main;
}

#[gpui::test]
fn esc_clears_the_other_windows_highlight(cx: &mut TestAppContext) {
    let (space, _, mut vcx) = two_windows(cx);
    let second = cx.update(|cx| state(cx).handle("pitwall-1").unwrap());
    let b = cx.update(|cx| second.update(cx, |_, w, _| w.bounds()).unwrap());
    let local = drag_tab_to(
        &mut vcx,
        &space,
        point(b.left() + px(100.), b.top() + px(16.)),
    );
    cx.update(|cx| {
        assert!(screen_of("pitwall-1", cx)
            .read(cx)
            .space_incoming()
            .is_some())
    });
    vcx.simulate_keystrokes("escape");
    vcx.simulate_mouse_up(local, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(screen_of("pitwall-1", cx).read(cx).space_incoming(), None);
        assert_eq!(crate::ui_state::get(cx).window_of_space(&space), MAIN);
    });
}
