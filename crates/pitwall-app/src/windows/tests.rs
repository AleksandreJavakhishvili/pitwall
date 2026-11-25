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
fn an_agents_files_go_beside_it_in_the_window_that_shows_it() {
    let order: Vec<String> = ["pitwall-2", MAIN, "pitwall-1"].map(String::from).to_vec();
    let s = |l: &[&str]| l.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // Shown in two: the frontmost of them.
    assert_eq!(frontmost_showing(&order, &s(&[MAIN, "pitwall-1"])), Some(MAIN));
    assert_eq!(frontmost_showing(&order, &s(&["pitwall-1"])), Some("pitwall-1"));
    // Shown in no open window: none (it opens in a window of its own).
    assert_eq!(frontmost_showing(&order, &s(&["pitwall-9"])), None);
    assert_eq!(frontmost_showing(&order, &[]), None);
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

/// Each split has a size for each child, summing to 1, and two children
/// or more; no tile, terminal or files tile twice.
fn assert_whole(layout: &crate::main_screen::tree::Node) {
    use crate::main_screen::tree::Node;
    fn rec(n: &Node, seen: &mut Vec<String>) {
        match n {
            Node::Pane { id, .. } => {
                assert!(!seen.contains(id), "tile {id} twice");
                seen.push(id.clone());
            }
            Node::Split { children, sizes, .. } => {
                assert!(children.len() >= 2);
                assert_eq!(children.len(), sizes.len());
                assert!((sizes.iter().sum::<f64>() - 1.0).abs() < 1e-6, "{sizes:?}");
                children.iter().for_each(|c| rec(c, seen));
            }
        }
    }
    rec(layout, &mut Vec::new());
    let mut a = layout.agents();
    let n = a.len();
    a.sort();
    a.dedup();
    assert_eq!(a.len(), n, "a terminal twice");
}

/// A file viewer window open when Pitwall quits opens again at the next
/// start, behind the rest, with its folder, tabs, open folders and place;
/// one the user closed doesn't, nor one whose agent is gone.
#[gpui::test]
fn files_windows_open_again_after_a_restart(cx: &mut TestAppContext) {
    use crate::main_screen::workspace::{DockedFiles, FilesWindow};
    let dir = std::env::temp_dir().join(format!("pw-files-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::main_screen::init(cx);
        crate::code_view::register(cx);
        crate::ui_state::init(Some(dir.clone()), cx);
        let mut agents = vec![agent("api", "/work/alpha", "idle", 1, true), agent("web", "/work/alpha", "idle", 2, true)];
        for a in &mut agents {
            a.caps.explorer = true;
        }
        let store = cx.new(|_| AgentStore::new(agents));
        cx.set_global(Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: Content::Live { store: store.clone() }, root: None, main: None });
        let source: std::sync::Arc<dyn crate::explorer::source::Source> = crate::explorer::source::fake::FakeSource::new(&[("a.txt", "a\n"), ("src/b.txt", "b\n")]);
        cx.set_global(crate::explorer::source::ExplorerSource(source.clone()));
        crate::explorer::dock::remember_env(Some(source), &store, cx);
        init(Content::Live { store }, Some(dir.clone()), show_main, cx);
        show_main(cx);
        restore(cx);
    });
    cx.run_until_parked();
    let open = |agent: &str, cx: &mut TestAppContext| {
        let h = cx.update(|cx| crate::explorer::window::open(agent, agent, format!("{agent} — Files"), false, cx)).expect("window");
        cx.run_until_parked();
        let _ = cx.update(|cx| {
            h.update(cx, |w, _, cx| {
                let v = w.viewer().clone();
                v.update(cx, |v, cx| v.open("a.txt".into(), None, cx));
                let tree = v.read(cx).tree();
                tree.update(cx, |t, cx| t.expand("src", cx));
            })
        });
        cx.run_until_parked();
        h
    };
    let close = |h: gpui::WindowHandle<crate::explorer::window::ViewerWindow>, cx: &mut TestAppContext| {
        let _ = cx.update(|cx| h.update(cx, |_, window, _| window.remove_window()));
        cx.run_until_parked();
    };
    let api = open("api", cx);
    let web = open("web", cx);
    let kept = cx.update(|cx| crate::ui_state::get(cx).files_windows.clone());
    assert_eq!(kept["api"].open, DockedFiles { key: "api".into(), tabs: vec!["a.txt".into()], active: Some("a.txt".into()) });
    assert_eq!(kept["api"].expanded, vec!["src".to_string()]);
    assert!(kept["api"].bounds.is_some());
    // web's closed by the user: forgotten.
    close(web, cx);
    assert!(cx.update(|cx| !crate::ui_state::get(cx).files_windows.contains_key("web")));
    // An agent removed since (as in an older ui.json).
    cx.update(|cx| {
        crate::ui_state::update(cx, |ui| {
            *ui = ui.clone().keep_files_window("gone", FilesWindow { open: DockedFiles { key: "gone".into(), ..Default::default() }, ..Default::default() })
        })
    });
    // Quitting: api's window closes with Pitwall, still kept.
    cx.update(|cx| set_quitting(true, cx));
    close(api, cx);
    cx.update(|cx| set_quitting(false, cx));
    let kept = cx.update(|cx| crate::ui_state::get(cx).files_windows.clone());
    assert!(kept.contains_key("api") && kept.contains_key("gone"), "{kept:?}");
    cx.executor().advance_clock(std::time::Duration::from_millis(800));
    cx.run_until_parked();
    let on_disk: crate::main_screen::workspace::UiState = serde_json::from_str(&std::fs::read_to_string(dir.join("ui.json")).unwrap()).unwrap();
    assert!(on_disk.files_windows.contains_key("api"));

    // "Restarted": the windows open again from what was kept.
    cx.update(crate::explorer::window::restore);
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(viewer_window_owners(cx), vec!["api".to_string()], "only api's");
        let h = viewer_window("api", cx).unwrap();
        let (tabs, active, expanded) = h
            .read_with(cx, |w, cx| {
                let v = w.viewer().read(cx);
                (v.tabs().to_vec(), v.active().map(str::to_string), v.tree().read(cx).is_open("src"))
            })
            .unwrap();
        assert_eq!((tabs, active.as_deref(), expanded), (vec!["a.txt".to_string()], Some("a.txt"), true));
        let ui = crate::ui_state::get(cx);
        assert!(!ui.files_windows.contains_key("gone"), "a removed agent's is forgotten");
        assert!(ui.files_windows.contains_key("api"));
    });
    let _ = std::fs::remove_dir_all(&dir);
}


/// Files put in a terminal's place (the tool's `where: replace`, the
/// "Replace …'s terminal" menu entry): the same tile, place and size; the
/// terminal leaves the layout but its agent is still there, and comes back
/// when shown (beside the files); kept in `ui.json`; the layout stays whole
/// through moving the files and popping them out. A files window's bar
/// drags it as the grip does.
#[gpui::test]
fn files_replace_a_terminal_that_comes_back_when_shown(cx: &mut TestAppContext) {
    use crate::explorer::dock::{self, FilesPlace};
    use crate::main_screen::tree::{self, Side};
    use pitwall_proto::show::{OpenedFile, ViewerPlace, ViewerShow};
    let dir = std::env::temp_dir().join(format!("pw-files-replace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(dir.join("api")).unwrap();
    std::fs::write(dir.join("api/a.txt"), "a\n").unwrap();
    // (Its real path: the temporary folder may be behind a link.)
    let api_dir = dir.join("api").canonicalize().unwrap().to_string_lossy().into_owned();
    let store = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::main_screen::init(cx);
        crate::code_view::register(cx);
        crate::ui_state::init(Some(home.clone()), cx);
        let mut agents = vec![agent("api", &api_dir, "idle", 1, true), agent("web", &api_dir, "idle", 2, true)];
        for a in &mut agents {
            a.caps.explorer = true;
        }
        let store = cx.new(|_| AgentStore::new(agents));
        cx.set_global(Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: Content::Live { store: store.clone() }, root: None, main: None });
        let source: std::sync::Arc<dyn crate::explorer::source::Source> = crate::explorer::source::fake::FakeSource::new(&[("a.txt", "a\n")]);
        cx.set_global(crate::explorer::source::ExplorerSource(source.clone()));
        crate::explorer::dock::remember_env(Some(source), &store, cx);
        init(Content::Live { store: store.clone() }, Some(home.clone()), show_main, cx);
        show_main(cx);
        restore(cx);
        store
    });
    cx.run_until_parked();
    // A space in main: api's terminal, web's right of it.
    let space = cx.update(|cx| {
        let (ui, id) = crate::ui_state::get(cx).create_custom_space(MAIN);
        let ui = ui.place_agent(&id, "api", None, None).place_agent(&id, "web", None, Some(Side::Right));
        crate::ui_state::update(cx, |s| *s = ui);
        screen_of(MAIN, cx).update(cx, |s, cx| s.show_space(&id, cx));
        id
    });
    cx.run_until_parked();
    let layout = |cx: &mut TestAppContext| cx.update(|cx| crate::ui_state::get(cx).space(&space).unwrap().layout.clone());
    let rects = |l: &tree::Node| tree::pane_rects(l, tree::Rect { x: 0., y: 0., w: 1., h: 1. }).into_iter().map(|(id, r)| (id, (r.x, r.y, r.w, r.h))).collect::<Vec<_>>();
    let before = layout(cx);
    let web_pane = before.find_by_agent("web").unwrap().id;

    // The tool, asked for the files instead of web's terminal.
    let req = ViewerShow {
        agent_id: "api".into(),
        files: vec![OpenedFile { path: format!("{api_dir}/a.txt"), line: None, column: None }],
        folder: None,
        root: None,
        place: Some(ViewerPlace::Replace),
        of: Some("web".into()),
        note: None,
    };
    let shown = cx.update(|cx| crate::explorer::show::show(&req, cx)).expect("shown");
    cx.run_until_parked();
    let at = shown.at.unwrap_or_default();
    assert!(at.contains("in place of web's terminal (it keeps running"), "{at}");
    let after = layout(cx);
    assert_eq!(after.find_files("api").map(|p| p.id), Some(web_pane.clone()), "web's tile, now the files'");
    assert!(after.find_by_agent("web").is_none());
    assert_eq!(rects(&after), rects(&before), "the same place and size");
    assert_whole(&after);
    assert!(cx.update(|cx| store.read(cx).agent("web").is_some_and(|a| a.running)), "web is still there");
    // The same again: added to, not moved.
    let again = cx.update(|cx| crate::explorer::show::show(&req, cx)).expect("again");
    assert_eq!(again.place, ViewerPlace::Replace);
    assert_eq!(layout(cx), after);

    // Kept in ui.json for the next start.
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    let kept: crate::main_screen::workspace::UiState = serde_json::from_str(&std::fs::read_to_string(home.join("ui.json")).unwrap()).unwrap();
    assert_eq!(kept.files_tile("api"), Some((space.clone(), web_pane.clone())));
    assert!(kept.space(&space).unwrap().layout.find_by_agent("web").is_none());
    assert_eq!(kept.docked_files["api"].tabs, ["a.txt"]);

    // web shown again (the sidebar): beside the files, never over them.
    let h = cx.update(|cx| state(cx).handle(MAIN).unwrap());
    h.update(cx, |view, window, cx| view.screen.clone().unwrap().update(cx, |s, cx| s.show_agent("web", window, cx))).unwrap();
    cx.run_until_parked();
    let back = layout(cx);
    assert!(back.find_by_agent("web").is_some());
    assert_eq!(back.find_files("api").map(|p| p.id), Some(web_pane.clone()));
    assert_whole(&back);

    // The menu's entries: its own terminal first; picked, as a click does.
    let names = cx.update(|cx| dock::replaceable("api", cx));
    assert_eq!(names, [("api".to_string(), "api".to_string()), ("web".to_string(), "web".to_string())]);
    let api_pane = back.find_by_agent("api").unwrap().id;
    // (As its row in the Move… menu does: `explorer::tests` clicks it.)
    let v = cx.update(|cx| dock::viewer("api", cx)).expect("its tile's viewer");
    v.update(cx, |_, cx| cx.emit(crate::explorer::viewer::ViewerEvent::Place(FilesPlace::Replace, Some("api".into()))));
    cx.run_until_parked();
    let own = layout(cx);
    assert_eq!(own.find_files("api").map(|p| p.id), Some(api_pane));
    assert!(own.find_by_agent("api").is_none() && own.find_by_agent("web").is_some());
    assert_whole(&own);

    // Moved along an edge, then out to a window: still whole.
    cx.update(|cx| dock::place("api", FilesPlace::Edge(Side::Bottom), None, cx)).expect("moved");
    cx.run_until_parked();
    assert_whole(&layout(cx));
    cx.update(|cx| dock::place("api", FilesPlace::Window, None, cx)).expect("popped out");
    cx.run_until_parked();
    let out = layout(cx);
    assert_whole(&out);
    assert!(out.files_owners().is_empty());
    assert_eq!(out.agents(), ["web"]);
    // api comes back like any terminal not in the space (into the
    // focused tile, as always).
    h.update(cx, |view, window, cx| view.screen.clone().unwrap().update(cx, |s, cx| s.show_agent("api", window, cx))).unwrap();
    cx.run_until_parked();
    assert!(layout(cx).find_by_agent("api").is_some());
    assert_whole(&layout(cx));

    // The files window's bar: pressed and dragged, as its grip (the window
    // moves; let go over no layout, it stays a window).
    let w = cx.update(|cx| crate::windows::viewer_window("api", cx)).expect("a files window");
    let wcx = &mut gpui::VisualTestContext::from_window(*w, cx);
    wcx.run_until_parked();
    wcx.update(|window, _| window.refresh());
    wcx.run_until_parked();
    let bar = wcx.debug_bounds("ex-viewer-hint").expect("its bar").center();
    let drags = crate::platform::FLOAT_DRAGS.with(|n| n.get());
    wcx.simulate_mouse_down(bar, gpui::MouseButton::Left, gpui::Modifiers::none());
    wcx.simulate_mouse_move(gpui::point(bar.x + gpui::px(14.), bar.y), gpui::MouseButton::Left, gpui::Modifiers::none());
    wcx.simulate_mouse_up(gpui::point(bar.x + gpui::px(14.), bar.y), gpui::MouseButton::Left, gpui::Modifiers::none());
    wcx.run_until_parked();
    assert_eq!(crate::platform::FLOAT_DRAGS.with(|n| n.get()), drags + 1, "the window moves with the pointer");
    assert!(wcx.cx.update(|cx| crate::windows::viewer_window("api", cx).is_some()));
    assert!(wcx.cx.update(|cx| crate::ui_state::get(cx).files_tile("api").is_none()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// What `v`'s tree lists: each entry's path and change letter.
fn tree_listing(v: &Entity<crate::explorer::viewer::Viewer>, cx: &App) -> Vec<String> {
    let tree = v.read(cx).tree();
    let rows = tree.read(cx).state().rows();
    rows.iter().filter_map(|r| r.entry()).map(|e| format!("{}{}", e.path, e.status.map(|s| format!(" {s:?}")).unwrap_or_default())).collect()
}

/// The report: an agent whose viewer is rooted at its home folder, its
/// repository and the repository's worktrees two levels below. Asked for a
/// worktree (by its branch from a files tile on the right, by its path in
/// a window of its own), the viewer is rooted at that worktree, not at the
/// home folder with the worktree a folder in it, and shows its git status.
#[gpui::test]
fn a_worktree_below_the_agents_folder_is_the_root_in_a_tile_and_in_a_window(cx: &mut TestAppContext) {
    use crate::explorer::show::roots_tests::Fixture;
    use pitwall_core::exec::LocalExec;
    use pitwall_proto::show::{ViewerOpen, ViewerPlace};
    let fx = Fixture::new("tile-window");
    let dir = std::env::temp_dir().join(format!("pw-wt-root-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let home = dir.clone();
    let agent_home = fx.home.clone();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::main_screen::init(cx);
        crate::code_view::register(cx);
        crate::ui_state::init(Some(home.clone()), cx);
        let mut road = agent("road", &agent_home, "idle", 1, true);
        road.caps.explorer = true;
        let store = cx.new(|_| AgentStore::new(vec![road]));
        cx.set_global(Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: Content::Live { store: store.clone() }, root: None, main: None });
        let source: std::sync::Arc<dyn crate::explorer::source::Source> = crate::explorer::source::fake::DiskSource::new(&[("road", &agent_home)]);
        cx.set_global(crate::explorer::source::ExplorerSource(source.clone()));
        crate::explorer::dock::remember_env(Some(source), &store, cx);
        init(Content::Live { store: store.clone() }, Some(home.clone()), show_main, cx);
        show_main(cx);
        restore(cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let (ui, id) = crate::ui_state::get(cx).create_custom_space(MAIN);
        let ui = ui.place_agent(&id, "road", None, None);
        crate::ui_state::update(cx, |s| *s = ui);
        screen_of(MAIN, cx).update(cx, |s, cx| s.show_space(&id, cx));
    });
    cx.run_until_parked();
    // As the server resolves it (the agent works in the repository).
    let resolved = |req: ViewerOpen| {
        let x = LocalExec;
        let req = ViewerOpen { cwd: Some(fx.repo.clone()), ..req };
        let (worktrees, inside) = pitwall_core::show::names_for(&x, &fx.home, Some(&fx.repo), &req);
        let place = pitwall_core::show::Place { exec: &x, cwd: &fx.home, now: Some(&fx.repo), worktrees: &worktrees, inside: &inside, local: true };
        pitwall_core::show::resolve(&place, "road", &req).expect("resolved").0
    };
    let key = crate::explorer::source::path_key(&fx.chicken);
    let check = |v: &Entity<crate::explorer::viewer::Viewer>, cx: &mut TestAppContext| {
        assert_eq!(v.read_with(cx, |v, cx| (v.agent_id().to_string(), v.root(cx))), (key.clone(), Some(fx.chicken.clone())));
        v.read_with(cx, |v, _| v.tree()).update(cx, |t, cx| t.expand("src", cx));
        cx.run_until_parked();
        let listed = cx.update(|cx| tree_listing(v, cx));
        assert!(listed.contains(&"src/game.rs M".to_string()) && listed.contains(&"README.md".to_string()), "{listed:?}");
    };

    // (2), (3): its branch; a files tile right of the terminal.
    let show = resolved(ViewerOpen { root: Some("feat/chicken-town".into()), place: Some(ViewerPlace::Right), ..Default::default() });
    let shown = cx.update(|cx| crate::explorer::show::show(&show, cx)).expect("shown");
    cx.run_until_parked();
    assert_eq!((shown.place, shown.root.as_str()), (ViewerPlace::Right, fx.chicken.as_str()));
    let tile = cx.update(|cx| crate::explorer::dock::viewer("road", cx)).expect("a files tile");
    check(&tile, cx);
    // Kept with the tile for the next start.
    assert_eq!(cx.update(|cx| crate::ui_state::get(cx).docked_files["road"].key.clone()), key);

    // (1), (4): its path; a window of its own (the tile's viewer, moved).
    let show = resolved(ViewerOpen { root: Some(fx.chicken.clone()), place: Some(ViewerPlace::Window), ..Default::default() });
    let shown = cx.update(|cx| crate::explorer::show::show(&show, cx)).expect("shown");
    cx.run_until_parked();
    assert_eq!((shown.place, shown.root.as_str()), (ViewerPlace::Window, fx.chicken.as_str()));
    let w = cx.update(|cx| crate::windows::viewer_window("road", cx)).expect("a files window");
    let v = w.read_with(cx, |w, _| w.viewer().clone()).unwrap();
    check(&v, cx);
    assert_eq!(w.read_with(cx, |w, _| w.key().to_string()).unwrap(), key);
    let _ = std::fs::remove_dir_all(&dir);
}
