//! The main screen in a test window: showing agents, ⌘J, ⌘1–9, routes,
//! spaces and the strip, with made-up agents and no engine.

use gpui::{AppContext, TestAppContext, VisualTestContext};

use super::workspace::ALL_SPACE;
use super::*;
use crate::agents::tests::agent;

fn screen(
    cx: &mut TestAppContext,
) -> (
    Entity<MainScreen>,
    Entity<AgentStore>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        init(cx);
    });
    let store = cx.new(|_| {
        AgentStore::new(vec![
            agent("api", "/work/alpha", "idle", 1, true),
            agent("web", "/work/alpha", "blocked", 2, true),
            agent("docs", "/work/beta", "done", 3, true),
            agent("ops", "/work/beta", "blocked", 4, true),
        ])
    });
    let s = store.clone();
    let (view, cx) =
        cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    (view, store, cx)
}

/// Liquid Glass with floating surfaces as glass pop-ups (`kit::pop`).
fn liquid_glass(cx: &mut VisualTestContext) {
    use crate::theme::{Appearance, GlassOffer, Inputs, Look, OsPrefs};
    cx.update(|_, cx| {
        let ap = Appearance::resolve(
            Inputs { look: Look::Glass, ..Inputs::default() },
            gpui::WindowAppearance::Dark,
            GlassOffer::Liquid,
            OsPrefs::default(),
        );
        cx.set_global(ap.theme());
        cx.set_global(ap);
        cx.set_global(crate::kit::pop::TestHosted);
    });
}

/// The window's glass pop-ups.
fn pops(cx: &mut VisualTestContext) -> Vec<gpui::AnyWindowHandle> {
    let parent = cx.update(|window, _| window.window_handle().window_id());
    let all = cx.windows();
    crate::kit::pop::open_pops()
        .into_iter()
        .filter(|(_, p, _)| p.window_id() == parent)
        .filter_map(|(id, _, _)| all.iter().find(|w| w.window_id() == id).copied())
        .collect()
}

#[gpui::test]
fn under_liquid_glass_the_menu_is_a_pop_up_with_the_keyboard(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    liquid_glass(cx);
    let a = view.read_with(cx, |s, cx| s.agents(cx)[0].clone());
    cx.run_until_parked();
    let before = cx.update(|window, cx| window.focused(cx));
    view.update_in(cx, |s, window, cx| s.demo_agent_menu(&a, None, window, cx));
    cx.run_until_parked();
    let open = pops(cx);
    assert_eq!(open.len(), 1, "the menu's pop-up");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), before, "the window's own focus stays where it was");
    assert_eq!(view.read_with(cx, |s, _| s.menu_labels()), ["Open terminal here", "Rename…", "Remove agent…"]);
    // The screen's window is still its own: GPUI gives an entity the
    // window that last touched it while drawing, and the pop-up doesn't.
    let parent = cx.update(|window, _| window.window_handle().window_id());
    assert_eq!(view.update_in(cx, |_, window, _| window.window_handle().window_id()), parent);

    // ↓ and Enter in the pop-up: "Rename…" runs in the screen's window.
    cx.update_window(open[0], |_, window, cx| window.dispatch_action(Box::new(SelectNext), cx)).unwrap();
    cx.run_until_parked();
    cx.update_window(open[0], |_, window, cx| window.dispatch_action(Box::new(Confirm), cx)).unwrap();
    cx.run_until_parked();
    view.read_with(cx, |s, _| {
        assert!(s.menu.is_none());
        assert!(s.agent_rename.is_some(), "renaming, in the sidebar");
    });
    assert!(pops(cx).is_empty());

    // Esc closes it.
    view.update_in(cx, |s, window, cx| s.demo_agent_menu(&a, None, window, cx));
    cx.run_until_parked();
    let first = pops(cx);
    cx.update_window(first[0], |_, window, cx| window.dispatch_action(Box::new(Dismiss), cx)).unwrap();
    cx.run_until_parked();
    view.read_with(cx, |s, _| assert!(s.menu.is_none()));
    assert!(pops(cx).is_empty());

    // A menu that closes late (it lost the keyboard to the click that
    // opened the next one) leaves the next one open.
    view.update_in(cx, |s, window, cx| s.demo_agent_menu(&a, None, window, cx));
    cx.run_until_parked();
    let first = pops(cx);
    let old = first[0];
    view.update_in(cx, |s, window, cx| {
        s.demo_agent_menu(&a, Some((300., 200.)), window, cx);
        old.update(cx, |_, window, cx| window.dispatch_action(Box::new(Dismiss), cx)).unwrap();
    });
    cx.run_until_parked();
    view.read_with(cx, |s, _| assert!(s.menu.is_some(), "the second menu is open"));
    let second = pops(cx);
    assert_eq!(second.len(), 1);
    assert_ne!(second[0].window_id(), first[0].window_id());
}

#[gpui::test]
fn under_liquid_glass_the_toasts_are_one_pop_up_without_the_keyboard(cx: &mut TestAppContext) {
    use crate::kit::pop::PAD;
    let (view, _, cx) = screen(cx);
    liquid_glass(cx);
    cx.run_until_parked();
    let open = |cx: &mut VisualTestContext| cx.update(|window, cx| crate::kit::passive::test_over(window, cx));
    assert!(open(cx).is_empty(), "no toast: no pop-up");
    let before = cx.update(|window, cx| window.focused(cx));
    let id = view.read_with(cx, |s, cx| s.agents(cx)[1].id.clone());
    view.update(cx, |s, cx| {
        s.toast(Toast::info("Saved", None), cx);
        s.toast(Toast::done("web is done".into(), id.clone()), cx);
    });
    crate::kit::passive::test_draw(cx);
    let stack = open(cx);
    assert_eq!(stack.len(), 1, "one window for the whole stack");
    assert!(pops(cx).is_empty(), "and not one that takes the keyboard");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), before);
    cx.update(|window, _| assert!(!crate::kit::pop::over(window)));
    // The screen's window is still its own.
    let parent = cx.update(|window, _| window.window_handle().window_id());
    assert_eq!(view.update_in(cx, |_, window, _| window.window_handle().window_id()), parent);

    let click = |x: f32, y: f32, cx: &mut VisualTestContext| {
        cx.update_window(stack[0], |_, window, cx| {
            let position = gpui::point(px(x), px(y));
            let (button, modifiers) = (gpui::MouseButton::Left, gpui::Modifiers::default());
            window.dispatch_event(gpui::PlatformInput::MouseDown(gpui::MouseDownEvent { position, button, modifiers, click_count: 1, first_mouse: false }), cx);
            window.dispatch_event(gpui::PlatformInput::MouseUp(gpui::MouseUpEvent { position, button, modifiers, click_count: 1 }), cx);
        })
        .unwrap();
        crate::kit::passive::test_draw(cx);
    };
    // The first toast's text (wherever its opening has moved it to by
    // now): it goes, having nothing to show.
    click(PAD + 100., PAD + 20., cx);
    assert_eq!(view.read_with(cx, |s, _| s.toast_titles()), ["web is done"]);
    assert_eq!(open(cx).len(), 1, "the same pop-up, with what is left");
    // The other one's text, now on top: its agent shows, in the screen's window.
    click(PAD + 100., PAD + 20., cx);
    view.read_with(cx, |s, _| {
        assert!(s.toast_titles().is_empty());
        assert_eq!(s.cursor.as_deref(), Some(id.as_str()), "a click jumps to the agent");
    });
    assert!(open(cx).is_empty(), "the last toast went: so did the pop-up");
}

#[gpui::test]
fn first_run_places_the_first_agent_and_shows_on_click(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    view.update(cx, |s, _| {
        // Sidebar order, oldest first in each project: alpha (api, web), beta (docs, ops).
        assert_eq!(
            s.ui.space(ALL_SPACE).unwrap().layout.agents(),
            vec!["api".to_string()]
        );
    });
    view.update_in(cx, |s, window, cx| s.show_agent("docs", window, cx));
    view.update(cx, |s, cx| {
        assert_eq!(s.focused_agent_id().as_deref(), Some("docs"));
        assert_eq!(s.selected(cx).unwrap().name, "docs");
    });
}

#[gpui::test]
fn jump_cycles_blocked_agents_in_sidebar_order(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    let order = |view: &Entity<MainScreen>, cx: &mut VisualTestContext| {
        view.update_in(cx, |s, window, cx| s.next_blocked(window, cx));
        view.read_with(cx, |s, _| s.focused_agent_id().unwrap())
    };
    assert_eq!(
        order(&view, cx),
        "web",
        "api is focused first, so the next blocked is web"
    );
    assert_eq!(order(&view, cx), "ops");
    assert_eq!(order(&view, cx), "web");
    view.update_in(cx, |s, window, cx| s.select_index(2, window, cx));
    view.read_with(cx, |s, _| {
        assert_eq!(s.focused_agent_id().as_deref(), Some("docs"))
    });
}

#[gpui::test]
fn routes_toggle_and_escape_goes_back(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    view.update(cx, |s, cx| s.toggle_route(Route::Wall, cx));
    view.read_with(cx, |s, _| {
        assert_eq!(s.route(), Route::Wall);
        assert!(
            s.ui.wall.contains(&MAIN.to_string()),
            "Wall mode is kept in ui.json"
        );
    });
    view.update_in(cx, |s, window, cx| s.show_agent("api", window, cx));
    view.read_with(cx, |s, _| {
        assert_eq!(s.route(), Route::Space, "showing an agent leaves Wall")
    });
    view.update(cx, |s, cx| s.toggle_route(Route::Review, cx));
    cx.dispatch_action(Dismiss);
    view.read_with(cx, |s, _| assert_eq!(s.route(), Route::Space));
}

#[gpui::test]
fn spaces_and_presets(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    view.update(cx, |s, cx| s.new_space(cx));
    let custom = view.read_with(cx, |s, _| s.active.clone());
    assert_ne!(custom, ALL_SPACE);
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    view.update_in(cx, |s, window, cx| s.apply_preset(Preset::Two, window, cx));
    view.read_with(cx, |s, _| {
        let sp = s.ui.space(&custom).unwrap();
        assert_eq!(sp.layout.leaves().len(), 2);
        assert_eq!(sp.members, vec!["web".to_string()]);
    });
    view.update(cx, |s, cx| s.open_project_space("/work/beta", "beta", cx));
    view.read_with(cx, |s, _| {
        let sp = s.active_space().unwrap();
        assert_eq!(sp.kind, workspace::SpaceKind::Project);
        assert_eq!(sp.layout.agents().len(), 2);
    });
}

#[gpui::test]
fn attention_becomes_a_toast_and_the_strip_leads_with_blocked(cx: &mut TestAppContext) {
    let (view, store, cx) = screen(cx);
    store.update(cx, |_, cx| {
        cx.emit(crate::agents::StoreEvent::Attention {
            agent_id: "ops".into(),
            name: "ops".into(),
            reason: "blocked",
            detail: None,
        })
    });
    cx.run_until_parked();
    view.read_with(cx, |s, cx| {
        assert_eq!(s.toasts.len(), 1);
        assert_eq!(s.toasts[0].title.as_ref(), "ops needs you");
        let strip = model::strip(&s.ordered(cx));
        assert_eq!(strip.lead.unwrap().id, "web");
        assert_eq!(strip.more, 1);
    });
}

#[gpui::test]
fn removed_agents_leave_the_layout(cx: &mut TestAppContext) {
    let (view, store, cx) = screen(cx);
    view.update_in(cx, |s, window, cx| s.show_agent("docs", window, cx));
    store.update(cx, |s, cx| {
        s.agents.retain(|a| a.id != "docs");
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |s, _| assert!(s.ui.locate("docs").is_none()));
}

#[test]
fn breakpoints_follow_the_react_ones() {
    assert_eq!(Breakpoint::of(2400.), Breakpoint::Xl);
    assert_eq!(Breakpoint::of(1500.), Breakpoint::Lg);
    assert_eq!(Breakpoint::of(1200.), Breakpoint::Md);
    assert_eq!(Breakpoint::of(800.), Breakpoint::Sm);
    assert_eq!(Breakpoint::of(500.), Breakpoint::Xs);
    assert!(Breakpoint::Md.full_sidebar() && !Breakpoint::Sm.full_sidebar());
    assert_eq!(Breakpoint::Lg.max_per_row(), 3);
}

#[test]
fn every_shortcut_has_a_binding() {
    let b = bindings();
    for a in [
        &OpenPalette as &dyn gpui::Action,
        &NewAgent,
        &NewTerminal,
        &NewTerminalAt,
        &JumpNextBlocked,
        &ToggleSidebar,
        &ToggleWall,
        &ToggleReview,
        &ToggleMaximize,
        &RefreshChanges,
        &MoveSpaceToWindow,
        &SelectAgent1,
        &SelectAgent9,
    ] {
        assert!(b.iter().any(|k| k.action().partial_eq(a)), "{a:?}");
    }
    if !cfg!(target_os = "macos") {
        // Ctrl alone belongs to the terminal.
        assert!(b
            .iter()
            .filter(|k| k.predicate().is_none())
            .all(|k| !k.keystrokes().iter().any(super::terminal_ctrl)));
    }
}

/// The details panel is gone, and its shortcuts with it: ⌘. (Ctrl+Shift+.
/// and Ctrl+> elsewhere) is bound to nothing, not handed to another action.
#[test]
fn the_details_shortcut_is_free() {
    let period = |s: &gpui::KeybindingKeystroke| {
        let m = s.modifiers();
        (s.key() == "." && (m.platform || m.control)) || (s.key() == ">" && m.control)
    };
    let mut all = bindings();
    all.extend(crate::kit::input::bindings());
    assert!(all.iter().all(|k| !k.keystrokes().iter().any(period)));
}

/// The hover pass (main screen): the pointer visits every hoverable
/// element (sidebar rows, the empty-project row, project heads, tabs, top
/// bar, pane headers and their buttons, chips, the strip)
/// and nothing moves or changes size while it is over one; then the same
/// with the agent menu open.
#[gpui::test]
fn hovering_never_moves_anything(cx: &mut TestAppContext) {
    use crate::kit::hover::probe;
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        init(cx);
    });
    let store = cx.new(|_| AgentStore::new(demo::agents()));
    let s = store.clone();
    let (view, cx) =
        cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(1500.), gpui::px(900.)));
    view.update_in(cx, |s, window, cx| {
        s.projects.push(pitwall_core::onboarding::project_list::Project {
            path: "/work/empty".into(),
            display: "empty-proj".into(),
            is_git: true,
            added_at: 0,
        });
        s.apply_preset(tree::Preset::G2x2, window, cx);
        let first = s.agents(cx)[0].id.clone();
        s.show_agent(&first, window, cx);
    });
    cx.run_until_parked();
    let n = probe::assert_hover_keeps_layout(cx, "main screen");
    assert!(n > 30, "only {n} hoverable elements drawn");

    view.update_in(cx, |s, window, cx| {
        let a = s.agents(cx)[0].clone();
        s.demo_agent_menu(&a, None, window, cx);
    });
    cx.run_until_parked();
    probe::assert_hover_keeps_layout(cx, "agent menu");
}

#[gpui::test]
fn big_pieces_are_cached_parts_that_follow_the_layout(cx: &mut TestAppContext) {
    let (view, store, cx) = screen(cx);
    cx.run_until_parked();
    let shown = |view: &Entity<MainScreen>, cx: &mut VisualTestContext| {
        view.read_with(cx, |s, _| {
            let mut ids: Vec<String> = s
                .active_space()
                .unwrap()
                .layout
                .leaves()
                .into_iter()
                .map(|p| p.id)
                .collect();
            ids.sort();
            (ids, s.parts.as_ref().map(|p| p.header_ids()))
        })
    };
    let (panes, headers) = shown(&view, cx);
    assert_eq!(headers, Some(panes), "one header part per shown pane");
    // Another agent shown: its pane gets a header, a closed pane loses it.
    view.update_in(cx, |s, window, cx| s.show_agent("docs", window, cx));
    cx.run_until_parked();
    let (panes, headers) = shown(&view, cx);
    assert_eq!(headers, Some(panes));
    // A store change re-renders the parts with the screen (no stale rows).
    store.update(cx, |s, cx| {
        s.agents[0].status = pitwall_proto::Status::Working;
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |s, cx| assert_eq!(s.agents(cx)[0].status, pitwall_proto::Status::Working));
}

#[gpui::test]
fn a_printed_dev_server_shows_a_preview_chip_while_the_agent_runs(cx: &mut TestAppContext) {
    let (view, store, cx) = screen(cx);
    cx.simulate_resize(gpui::size(gpui::px(1500.), gpui::px(900.)));
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    let chip = |cx: &mut VisualTestContext| {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.debug_bounds("preview-web").is_some()
    };
    assert!(!chip(cx), "nothing printed yet");
    let set = |store: &Entity<AgentStore>, urls: &[&str], running: bool, cx: &mut VisualTestContext| {
        store.update(cx, |s, cx| {
            let a = s.agents.iter_mut().find(|a| a.id == "web").unwrap();
            a.preview_urls = urls.iter().map(|u| u.to_string()).collect();
            a.running = running;
            cx.notify();
        });
    };
    set(&store, &["http://localhost:5173/", "http://localhost:3000"], false, cx);
    assert!(!chip(cx), "a stopped agent has none");
    set(&store, &["http://localhost:5173/", "http://localhost:3000"], true, cx);
    assert!(chip(cx));
}

/// A view that stands in for a terminal in the pane body.
struct FakeTerm;

impl Render for FakeTerm {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().size_full()
    }
}

#[gpui::test]
fn a_terminal_frame_replays_the_parts_a_change_redraws_them(cx: &mut TestAppContext) {
    let (view, store, cx) = screen(cx);
    let term = cx.update(|_, cx| cx.new(|_| FakeTerm));
    let t = term.clone();
    cx.update(|_, cx| {
        set_terminal_slot(cx, move |_, _, _| {
            gpui::AnyView::from(t.clone())
                .cached(gpui::StyleRefinement::default().flex_1())
                .into_any_element()
        })
    });
    view.update_in(cx, |s, window, cx| s.show_agent("api", window, cx));
    cx.run_until_parked();
    let renders = |cx: &mut VisualTestContext| {
        view.read_with(cx, |s, cx| {
            let p = s.parts.as_ref().expect("parts");
            (p.sidebar.read(cx).renders, p.topbar.read(cx).renders)
        })
    };
    let before = renders(cx);
    assert!(before.0 > 0 && before.1 > 0, "parts drawn: {before:?}");
    // Output in a pane: the sidebar and the top bar replay their last paint.
    for _ in 0..3 {
        term.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    assert_eq!(renders(cx), before, "a terminal frame re-rendered a part");
    // An agent's status changes: the parts show it.
    store.update(cx, |s, cx| {
        s.agents[0].status = pitwall_proto::Status::Working;
        cx.notify();
    });
    cx.run_until_parked();
    let after = renders(cx);
    assert!(after.0 > before.0 && after.1 > before.1, "{before:?} -> {after:?}");
}

/// A custom space holding "api" next to "All", in a window big enough for
/// the whole top bar; the screen's events are collected.
fn tab_drag_setup(
    cx: &mut TestAppContext,
) -> (
    Entity<MainScreen>,
    String,
    std::rc::Rc<std::cell::RefCell<Vec<ScreenEvent>>>,
    &mut VisualTestContext,
) {
    let (view, _, cx) = screen(cx);
    cx.simulate_resize(gpui::size(px(1500.), px(900.)));
    view.update(cx, |s, cx| s.new_space(cx));
    let space = view.read_with(cx, |s, _| s.active.clone());
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &ScreenEvent, _| seen.borrow_mut().push(e.clone()))
            .detach()
    });
    (view, space, events, cx)
}

/// Where `selector` is drawn (a fresh frame: cached parts replay without
/// their debug bounds).
fn drawn_at(cx: &mut VisualTestContext, selector: String) -> gpui::Bounds<Pixels> {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.debug_bounds(Box::leak(selector.clone().into_boxed_str()))
        .unwrap_or_else(|| panic!("{selector} not drawn"))
}

/// Press at `from`, drag through `path`, let go at its last point.
fn drag(cx: &mut VisualTestContext, from: gpui::Point<Pixels>, path: &[gpui::Point<Pixels>]) {
    use gpui::{Modifiers, MouseButton};
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    for p in path {
        cx.simulate_mouse_move(*p, MouseButton::Left, Modifiers::none());
    }
    // Twice at the end: the move that starts a drag isn't one it sees.
    let end = *path.last().unwrap();
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
}

fn release(cx: &mut VisualTestContext, at: gpui::Point<Pixels>) {
    cx.simulate_mouse_up(at, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn a_tab_dragged_away_from_the_bar_moves_to_a_new_window_there(cx: &mut TestAppContext) {
    let (view, space, events, cx) = tab_drag_setup(cx);
    let tab = drawn_at(cx, format!("tab-name-{space}"));
    let grab = tab.center();
    let below = gpui::point(grab.x + px(40.), px(500.));
    drag(cx, grab, &[grab + gpui::point(px(6.), px(0.)), below]);
    assert!(cx.update(|_, cx| cx.has_active_drag()));
    assert_eq!(
        view.read_with(cx, |s, cx| s.space_drag_release(cx)),
        Some(SpaceRelease::NewWindow),
        "the ghost says New window"
    );
    let (origin, title) = cx.update(|window, _| {
        let b = window.bounds();
        (b.origin, b.size.height - window.viewport_size().height)
    });
    release(cx, below);
    let at = gpui::point(origin.x + below.x, origin.y + title + below.y);
    assert_eq!(
        *events.borrow(),
        vec![ScreenEvent::MoveSpaceToWindow(space.clone(), Some(at))]
    );
    assert!(!cx.update(|_, cx| cx.has_active_drag()));

    // Let go out of the window (no drop target there) works the same.
    events.borrow_mut().clear();
    let out = gpui::point(px(-300.), grab.y);
    drag(cx, grab, &[grab + gpui::point(px(6.), px(0.)), out]);
    release(cx, out);
    assert!(matches!(
        events.borrow().as_slice(),
        [ScreenEvent::MoveSpaceToWindow(id, Some(_))] if *id == space
    ));
}

#[gpui::test]
fn a_tab_let_go_on_or_near_the_bar_stays(cx: &mut TestAppContext) {
    let (view, space, events, cx) = tab_drag_setup(cx);
    let tab = drawn_at(cx, format!("tab-name-{space}"));
    let grab = tab.center();
    // Along the bar, then just under it.
    for end in [
        gpui::point(grab.x + px(300.), grab.y),
        gpui::point(grab.x, TOPBAR_HEIGHT + px(20.)),
    ] {
        drag(cx, grab, &[grab + gpui::point(px(6.), px(0.)), end]);
        assert_eq!(view.read_with(cx, |s, cx| s.space_drag_release(cx)), Some(SpaceRelease::Stay));
        release(cx, end);
        assert!(!cx.update(|_, cx| cx.has_active_drag()));
    }
    // Esc cancels a drag that would have detached.
    let far = gpui::point(grab.x, px(600.));
    drag(cx, grab, &[grab + gpui::point(px(6.), px(0.)), far]);
    cx.simulate_keystrokes("escape");
    release(cx, far);
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    // "All" isn't dragged at all (it stays in the main window).
    let all = drawn_at(cx, format!("tab-name-{ALL_SPACE}"));
    drag(cx, all.center(), &[all.center() + gpui::point(px(6.), px(0.)), far]);
    assert!(!cx.update(|_, cx| cx.has_active_drag()));
    release(cx, far);
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    view.read_with(cx, |s, _| assert_eq!(s.ui.window_of_space(&space), MAIN));
}

#[gpui::test]
fn an_agent_dropped_on_a_tab_still_joins_that_space(cx: &mut TestAppContext) {
    let (view, space, events, cx) = tab_drag_setup(cx);
    view.read_with(cx, |s, _| {
        assert!(!s.ui.space(&space).unwrap().layout.agents().contains(&"docs".to_string()))
    });
    let row = drawn_at(cx, "row-body-docs".into());
    let tab = drawn_at(cx, format!("tab-name-{space}"));
    drag(cx, row.center(), &[row.center() + gpui::point(px(6.), px(0.)), tab.center()]);
    release(cx, tab.center());
    view.read_with(cx, |s, _| {
        assert!(s.ui.space(&space).unwrap().layout.agents().contains(&"docs".to_string()));
    });
    assert!(events
        .borrow()
        .iter()
        .all(|e| !matches!(e, ScreenEvent::MoveSpaceToWindow(..))));
}

#[test]
fn a_new_git_identity_restarts_the_projects_resumable_agents() {
    let resumable = |mut a: AgentView| {
        a.terminal = false;
        a.caps.restart = true;
        a.caps.resume = true;
        a
    };
    let mut in_terminal = agent("claude-in-term", "/work/alpha", "idle", 0, true);
    in_terminal.agent_in_terminal = true;
    in_terminal.caps.restart = true;
    in_terminal.caps.resume = true;
    let mut engineer = resumable(agent("engineer", "/work/alpha", "idle", 0, true));
    engineer.engineer = true;
    let mut stopped = resumable(agent("stopped", "/work/alpha", "idle", 0, true));
    stopped.running = false;
    let agents = vec![
        resumable(agent("claude", "/work/alpha", "working", 0, true)),
        in_terminal,
        agent("shell", "/work/alpha", "idle", 0, true),
        engineer,
        stopped,
        resumable(agent("elsewhere", "/work/beta", "idle", 0, true)),
    ];
    let (restart, keep) = git_restarts(&agents, "/work/alpha");
    assert_eq!(restart, ["claude", "claude-in-term"]);
    assert_eq!(keep, ["shell"], "a plain terminal takes it when the user restarts it");
}

#[gpui::test]
fn renaming_an_agent_shows_a_bad_name_in_place_and_esc_cancels(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    view.update_in(cx, |s, window, cx| s.start_agent_rename("api", window, cx));
    let input = view.read_with(cx, |s, cx| {
        let r = s.agent_rename.as_ref().expect("editing");
        assert_eq!(r.id, "api");
        assert_eq!(r.input.read(cx).text(), "api", "starts from its name");
        r.input.clone()
    });
    input.update(cx, |i, cx| i.set_text("   ", cx));
    view.update_in(cx, |s, window, cx| s.finish_agent_rename(true, window, cx));
    view.read_with(cx, |s, _| {
        let r = s.agent_rename.as_ref().expect("still editing");
        assert_eq!(r.error.as_deref(), Some("a name can't be empty"));
    });
    input.update(cx, |i, cx| i.replace_text("billing", cx));
    cx.run_until_parked();
    view.read_with(cx, |s, _| assert_eq!(s.agent_rename.as_ref().unwrap().error, None, "typing clears it"));
    cx.dispatch_action(Dismiss);
    view.read_with(cx, |s, cx| {
        assert!(s.agent_rename.is_none());
        assert_eq!(s.store.read(cx).agents.iter().find(|a| a.id == "api").unwrap().name, "api", "nothing changed");
    });
}

#[gpui::test]
fn a_renamed_agent_can_go_back_to_its_session_title(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    let labels = |view: &Entity<MainScreen>, cx: &mut VisualTestContext, a: AgentView| {
        view.update_in(cx, |s, window, cx| s.demo_agent_menu(&a, None, window, cx));
        view.read_with(cx, |s, _| s.menu_labels())
    };
    let mut a = agent("api", "/work/alpha", "idle", 1, true);
    a.session_title = Some("Fix the login page".into());
    a.name_auto = true;
    assert!(!labels(&view, cx, a.clone()).iter().any(|l| l == "Use session title"), "already follows it");
    a.name_auto = false;
    assert!(labels(&view, cx, a.clone()).iter().any(|l| l == "Use session title"), "named by the user");
    a.session_title = None;
    assert!(!labels(&view, cx, a).iter().any(|l| l == "Use session title"), "no title to use");

    // Saving the name unchanged pins nothing: the field just closes.
    cx.dispatch_action(Dismiss);
    view.update_in(cx, |s, window, cx| s.start_agent_rename("api", window, cx));
    view.update_in(cx, |s, window, cx| s.finish_agent_rename(true, window, cx));
    view.read_with(cx, |s, _| assert!(s.agent_rename.is_none(), "closed without asking the engine"));
}

#[gpui::test]
fn sidebar_rows_keep_the_order_they_are_dragged_in(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    let names = |view: &Entity<MainScreen>, cx: &mut VisualTestContext| {
        view.read_with(cx, |s, cx| s.ordered(cx).into_iter().map(|a| a.id).collect::<Vec<_>>())
    };
    assert_eq!(names(&view, cx), ["api", "web", "docs", "ops"]);
    let dropped = view.update_in(cx, |s, window, cx| s.drop_agent_on_row("ops", "docs", true, window, cx));
    assert!(dropped);
    assert_eq!(names(&view, cx), ["api", "web", "ops", "docs"]);
    view.read_with(cx, |s, _| assert_eq!(s.ui.sidebar_order, ["ops", "docs"], "kept in ui.json"));
    let dropped = view.update_in(cx, |s, window, cx| s.drop_agent_on_row("api", "web", false, window, cx));
    assert!(dropped);
    assert_eq!(names(&view, cx), ["web", "api", "ops", "docs"]);
    view.update_in(cx, |s, window, cx| s.select_index(2, window, cx));
    view.read_with(cx, |s, _| assert_eq!(s.focused_agent_id().as_deref(), Some("ops"), "⌘3 follows it"));
    let refused = view.update_in(cx, |s, window, cx| s.drop_agent_on_row("api", "docs", true, window, cx));
    assert!(!refused, "not into another project");
    assert_eq!(names(&view, cx), ["web", "api", "ops", "docs"]);
}

#[gpui::test]
fn with_profiles_the_sidebar_groups_by_profile_and_a_drop_gives_one(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    let t = pitwall_core::testing::TempDir::new("app-side-profiles");
    let paths = pitwall_core::paths::Paths::new(t.path().join("pw"));
    let work = cx.update(|_, cx| {
        let m = crate::accounts::model(cx);
        m.update(cx, |m, cx| {
            m.set_paths(paths);
            let w = m.add_profile("Work", cx).unwrap();
            m.add_profile("Personal", cx).unwrap();
            m.set_project_profile("/work/alpha", Some(&w.id), cx).unwrap();
            w.id
        })
    });
    view.update(cx, |s, cx| s.set_sidebar_group(workspace::SidebarGroup::Account, cx));
    cx.run_until_parked();
    let sections = |view: &Entity<MainScreen>, cx: &mut VisualTestContext| {
        view.read_with(cx, |s, cx| {
            s.machine_sections(s.agents(cx), cx)[0].accounts.iter().map(|a| (a.account.clone(), a.groups.iter().map(|g| g.project.clone()).collect::<Vec<_>>())).collect::<Vec<_>>()
        })
    };
    let personal = cx.update(|_, cx| crate::accounts::list(cx).unwrap().profiles[1].id.clone());
    let (ws, ps) = (format!("profile:{work}"), format!("profile:{personal}"));
    assert_eq!(
        sections(&view, cx),
        [(ws.clone(), vec!["/work/alpha".to_string()]), (ps.clone(), vec![]), ("profile:none".into(), vec!["/work/beta".to_string()])]
    );
    // Onto Work: beta is given it.
    assert!(view.update_in(cx, |s, window, cx| s.drop_project_on_section("/work/beta", &ws, window, cx)));
    cx.run_until_parked();
    let of = |cx: &mut VisualTestContext, p: &str| cx.update(|_, cx| crate::accounts::list(cx).unwrap().profile_of(p).map(|p| p.name.clone()));
    assert_eq!(of(cx, "/work/beta").as_deref(), Some("Work"));
    assert_eq!(sections(&view, cx), [(ws.clone(), vec!["/work/alpha".to_string(), "/work/beta".to_string()]), (ps.clone(), vec![])], "no \"No profile\" when empty");
    // A pick of its own: it stays under Work, marked.
    cx.update(|_, cx| crate::accounts::model(cx).update(cx, |m, cx| m.set_project_pick("/work/beta", pitwall_core::accounts::GIT, Some("default"), cx))).ok();
    assert_eq!(sections(&view, cx)[0].1.len(), 2);
    // Onto Personal, then back to none: none can only be dropped on
    // while it shows, so beta leaves Work for Personal first.
    assert!(view.update_in(cx, |s, window, cx| s.drop_project_on_section("/work/beta", &ps, window, cx)));
    cx.run_until_parked();
    assert_eq!(of(cx, "/work/beta").as_deref(), Some("Personal"));
    cx.update(|_, cx| crate::accounts::model(cx).update(cx, |m, cx| m.set_project_profile("/work/alpha", None, cx))).unwrap();
    assert!(view.update_in(cx, |s, window, cx| s.drop_project_on_section("/work/beta", "profile:none", window, cx)));
    cx.run_until_parked();
    assert_eq!(of(cx, "/work/beta"), None, "\"No profile\" clears it");
    // The project menu says profile.
    view.update_in(cx, |s, window, cx| s.demo_project_menu("/work/alpha", window, cx));
    view.read_with(cx, |s, _| {
        let labels = s.menu_labels();
        assert!(labels.iter().any(|l| l == "Profile…") && labels.iter().any(|l| l == "Group by profile"), "{labels:?}");
    });
}

#[gpui::test]
fn this_mac_alone_has_no_machine_headings(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    for group in [workspace::SidebarGroup::Project, workspace::SidebarGroup::Account] {
        view.update(cx, |s, cx| s.set_sidebar_group(group, cx));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(cx.debug_bounds("row-body-api").is_some(), "drawn");
        assert!(cx.debug_bounds("machine-head-machine:this-mac").is_none(), "{group:?}: no machine level");
    }
}

/// Liquid Glass with its regions: scrolling the sidebar redraws the
/// sidebar part alone (the top bar replays its paint), each part keeps its
/// own glass piece, and a part that leaves the screen hides its piece.
#[gpui::test]
fn under_liquid_glass_the_sidebar_scrolls_alone_and_parts_keep_their_glass(cx: &mut TestAppContext) {
    use crate::theme::{Appearance, GlassOffer, Inputs, Look, OsPrefs};
    cx.update(|cx| {
        let ap = Appearance::resolve(
            Inputs { look: Look::Glass, ..Inputs::default() },
            gpui::WindowAppearance::Dark,
            GlassOffer::Liquid,
            OsPrefs::default(),
        );
        cx.set_global(ap.theme());
        cx.set_global(ap);
        init(cx);
        assert!(crate::theme::glass_regions(cx));
    });
    let store = cx.new(|_| {
        AgentStore::new((0..60).map(|i| agent(&format!("a{i}"), &format!("/work/p{}", i % 6), "idle", i, true)).collect())
    });
    let s = store.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(1500.), gpui::px(700.)));
    cx.run_until_parked();
    let wid = cx.update(|window, _| window.window_handle().window_id());
    let pieces = || crate::platform::glass::regions::of(wid).0;
    let renders = |cx: &mut VisualTestContext| {
        view.read_with(cx, |s, cx| {
            let p = s.parts.as_ref().expect("parts under Liquid Glass too");
            (p.sidebar.read(cx).renders, p.topbar.read(cx).renders)
        })
    };
    assert!(pieces().contains(&"sidebar") && pieces().contains(&"topbar"), "{:?}", pieces());
    let before = renders(cx);

    let at = gpui::point(gpui::px(120.), gpui::px(400.));
    cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    for _ in 0..3 {
        cx.simulate_event(gpui::ScrollEvent {
            position: at,
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-80.))),
            ..Default::default()
        });
        cx.run_until_parked();
    }
    let after = renders(cx);
    assert!(after.0 > before.0, "the sidebar scrolled: {before:?} -> {after:?}");
    assert_eq!(after.1, before.1, "scrolling the sidebar re-rendered the top bar");
    assert!(pieces().contains(&"sidebar") && pieces().contains(&"topbar"), "replayed parts keep their glass: {:?}", pieces());

    // The sidebar collapsed: its part isn't drawn, its glass goes.
    view.update(cx, |s, cx| {
        s.sidebar_collapsed = true;
        cx.notify();
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(!pieces().contains(&"sidebar"), "{:?}", pieces());
    assert!(pieces().contains(&"topbar"));
}

/// A long sidebar: a scroll step builds only the rows on screen (not all
/// 200), and showing an agent far down scrolls its row into view.
#[gpui::test]
fn a_long_sidebar_builds_only_the_rows_on_screen(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        init(cx);
    });
    let store = cx.new(|_| {
        AgentStore::new((0..200).map(|i| agent(&format!("a{i}"), &format!("/work/p{}", i % 4), "idle", i, true)).collect())
    });
    let s = store.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    cx.simulate_resize(gpui::size(px(1500.), px(900.)));
    cx.run_until_parked();
    let built = |cx: &mut VisualTestContext| view.read_with(cx, |s, _| s.side.agent_rows);
    let first = built(cx);
    assert!(first >= 200, "each row is measured once at first: {first}");

    let at = gpui::point(px(120.), px(400.));
    cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    let top = drawn_at(cx, "row-body-a28".into());
    for step in 0..3 {
        let before = built(cx);
        cx.simulate_event(gpui::ScrollEvent {
            position: at,
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-80.))),
            ..Default::default()
        });
        cx.run_until_parked();
        let n = built(cx) - before;
        assert!(n > 0 && n <= 30, "step {step}: {n} agent rows built");
    }
    let moved = drawn_at(cx, "row-body-a28".into());
    assert_eq!(moved.top(), top.top() - px(240.), "scrolled by three steps");

    // An agent off screen needs you: its row alone is measured again.
    let before = built(cx);
    store.update(cx, |s, cx| {
        let a = s.agents.iter_mut().find(|a| a.id == "a199").unwrap();
        a.status = pitwall_proto::Status::Blocked;
        a.status_detail = Some("Allow Bash?".into());
        cx.notify();
    });
    cx.run_until_parked();
    let n = built(cx) - before;
    assert!(n > 0 && n <= 31, "a store change built {n} agent rows");

    // The last agent, far below: shown, its row comes into view.
    let last = view.read_with(cx, |s, cx| s.ordered(cx).last().unwrap().id.clone());
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(cx.debug_bounds(Box::leak(format!("row-body-{last}").into_boxed_str())).is_none(), "off screen");
    view.update_in(cx, |s, window, cx| s.show_agent(&last, window, cx));
    cx.run_until_parked();
    let row = drawn_at(cx, format!("row-body-{last}"));
    let h = cx.update(|window, _| window.viewport_size().height);
    assert!(row.top() >= TOPBAR_HEIGHT && row.bottom() <= h, "{row:?} in view");

    // ↑ from there walks back up; the cursor's row stays in view.
    view.update_in(cx, |s, window, cx| window.focus(&s.sidebar_focus, cx));
    for _ in 0..30 {
        cx.dispatch_action(SelectPrev);
    }
    cx.run_until_parked();
    let cur = view.read_with(cx, |s, _| s.cursor.clone().unwrap());
    let row = drawn_at(cx, format!("row-body-{cur}"));
    assert!(row.top() >= TOPBAR_HEIGHT && row.bottom() <= h, "{cur}: {row:?} in view");
}

/// Under Liquid Glass with pop-ups a dialog's box is one, over its
/// backdrop: the window's own focus stays, Esc in it closes the dialog,
/// and a picker in the dialog is a pop-up of the dialog's.
#[gpui::test]
fn under_liquid_glass_a_dialog_is_a_pop_up_over_its_backdrop(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    liquid_glass(cx);
    cx.run_until_parked();
    let parent = cx.update(|window, _| window.window_handle().window_id());
    let before = cx.update(|window, cx| window.focused(cx));
    let esc = |pop: gpui::AnyWindowHandle, cx: &mut VisualTestContext| {
        cx.update_window(pop, |_, window, cx| window.dispatch_action(Box::new(Dismiss), cx)).unwrap();
        cx.run_until_parked();
    };

    // One drawn from values ("Remove agent").
    let a = view.read_with(cx, |s, cx| s.agents(cx)[0].clone());
    view.update_in(cx, |s, _, cx| s.open_remove(&a.id, cx));
    cx.run_until_parked();
    let open = pops(cx);
    assert_eq!(open.len(), 1, "the dialog's pop-up");
    assert!(view.read_with(cx, |s, _| matches!(s.modal, Some(Modal::Remove(_)))));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), before, "the window's own focus stays where it was");
    // A dialog: in the middle of the window.
    let (size, at) = (cx.update(|window, _| window.viewport_size()), crate::kit::pop::open_pops()[0].2.unwrap());
    assert!(
        (at.center().x - size.width / 2.).abs() <= px(1.) && (at.center().y - size.height / 2.).abs() <= px(1.),
        "{at:?} in {size:?}"
    );
    esc(open[0], cx);
    assert!(view.read_with(cx, |s, _| s.modal.is_none()));
    assert!(pops(cx).is_empty());

    // One that is a view ("New terminal"): its pop-up alone draws it,
    // and the screen's window is still its own.
    view.update_in(cx, |s, window, cx| s.open_terminal_dialog(window, cx));
    cx.run_until_parked();
    let open = pops(cx);
    assert_eq!(open.len(), 1);
    assert_eq!(view.update_in(cx, |_, window, _| window.window_handle().window_id()), parent);
    let dialog = view.read_with(cx, |s, _| match &s.modal {
        Some(Modal::Terminal(d)) => d.clone(),
        _ => panic!("the terminal dialog"),
    });
    assert_eq!(
        dialog.update_in(cx, |_, window, _| window.window_handle().window_id()),
        open[0].window_id(),
        "drawn in its pop-up"
    );
    esc(open[0], cx);
    assert!(view.read_with(cx, |s, _| s.modal.is_none()));
    assert!(pops(cx).is_empty());

    // "New agent" with its project picker open: a pop-up of the dialog's.
    view.update_in(cx, |s, window, cx| s.demo_open("new-agent-picker", window, cx));
    cx.run_until_parked();
    let open = pops(cx);
    assert_eq!(open.len(), 1);
    let all = cx.windows();
    let inner: Vec<_> = crate::kit::pop::open_pops()
        .into_iter()
        .filter(|(_, p, _)| p.window_id() == open[0].window_id())
        .filter_map(|(id, _, _)| all.iter().find(|w| w.window_id() == id).copied())
        .collect();
    assert_eq!(inner.len(), 1, "the picker, in the dialog");
    // Esc: the picker only, then the dialog.
    esc(inner[0], cx);
    assert_eq!(crate::kit::pop::open_pops().len(), 1);
    assert!(view.read_with(cx, |s, _| matches!(s.modal, Some(Modal::NewAgent(_)))));
    esc(open[0], cx);
    assert!(view.read_with(cx, |s, _| s.modal.is_none()));
    assert!(crate::kit::pop::open_pops().is_empty());
}

/// A click on `input` where it was drawn in `window`, then `text` typed
/// there: GPUI's key dispatch first, then the field's text input, as
/// AppKit's key handling does it.
fn click_and_type(window: gpui::AnyWindowHandle, input: &Entity<crate::kit::TextInput>, text: &str, cx: &mut VisualTestContext) {
    // A pop-up's window takes its content's size (AppKit's part, not in
    // tests): a press outside its panel would close it.
    let placed = crate::kit::pop::open_pops().into_iter().find(|(id, _, _)| *id == window.window_id()).and_then(|p| p.2);
    if let Some(placed) = placed {
        cx.simulate_window_resize(window, placed.size);
        cx.run_until_parked();
    }
    let at = input.read_with(cx, |i, _| i.drawn_bounds()).expect("the field is drawn").center();
    cx.update_window(window, |_, window, cx| {
        // Somewhere else had the keyboard: the click is what gives it.
        window.blur(cx);
        let (button, modifiers) = (gpui::MouseButton::Left, gpui::Modifiers::default());
        let down = gpui::MouseDownEvent { position: at, button, modifiers, click_count: 1, first_mouse: false };
        window.dispatch_event(gpui::PlatformInput::MouseDown(down), cx);
        let up = gpui::MouseUpEvent { position: at, button, modifiers, click_count: 1 };
        window.dispatch_event(gpui::PlatformInput::MouseUp(up), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        assert!(input.read(cx).is_focused(window), "the click focused the field");
        for c in text.chars() {
            let k = gpui::Keystroke { modifiers: Default::default(), key: c.to_string(), key_char: Some(c.to_string()) };
            window.dispatch_keystroke(k, cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
}

/// New agent's name takes what is typed, in the window (Flat) and in the
/// dialog's glass pop-up (Liquid Glass), and so does the project search in
/// the picker's pop-up of its own. (That the pop-up's native window hands
/// key presses to GPUI at all is AppKit's part: `platform::macos`
/// `keys_to_view`, which tests don't reach.)
#[gpui::test]
fn new_agent_takes_a_typed_name_flat_and_on_glass(cx: &mut TestAppContext) {
    let (view, _, cx) = screen(cx);
    cx.run_until_parked();
    let dialog = |view: &Entity<MainScreen>, cx: &mut VisualTestContext| {
        view.read_with(cx, |s, _| match &s.modal {
            Some(Modal::NewAgent(d)) => d.clone(),
            _ => panic!("New agent"),
        })
    };
    let main = cx.update(|window, _| window.window_handle());

    // Flat: in the window.
    view.update_in(cx, |s, window, cx| s.open_new_agent(None, window, cx));
    cx.run_until_parked();
    let name = dialog(&view, cx).read_with(cx, |d, _| d.name_input());
    click_and_type(main, &name, "scout", cx);
    assert_eq!(name.read_with(cx, |i, _| i.text().to_string()), "scout");
    view.update_in(cx, |s, window, cx| s.close_modal(window, cx));
    cx.run_until_parked();

    // Liquid Glass: in the dialog's pop-up.
    liquid_glass(cx);
    cx.run_until_parked();
    view.update_in(cx, |s, window, cx| s.open_new_agent(None, window, cx));
    cx.run_until_parked();
    let open = pops(cx);
    assert_eq!(open.len(), 1, "the dialog's pop-up");
    let d = dialog(&view, cx);
    let name = d.read_with(cx, |d, _| d.name_input());
    click_and_type(open[0], &name, "pilot", cx);
    assert_eq!(name.read_with(cx, |i, _| i.text().to_string()), "pilot");

    // The project search, in the picker's pop-up in the dialog's.
    d.update_in(cx, |d, window, cx| d.demo_open_picker(window, cx));
    cx.run_until_parked();
    let all = cx.windows();
    let inner: Vec<_> = crate::kit::pop::open_pops()
        .into_iter()
        .filter(|(_, p, _)| p.window_id() == open[0].window_id())
        .filter_map(|(id, _, _)| all.iter().find(|w| w.window_id() == id).copied())
        .collect();
    assert_eq!(inner.len(), 1, "the picker");
    let query = d.read_with(cx, |d, _| d.query_input());
    click_and_type(inner[0], &query, "api", cx);
    assert_eq!(query.read_with(cx, |i, _| i.text().to_string()), "api");
}

/// ⌘↵ (Ctrl+Shift+↵ elsewhere) maximises the focused tile, but a text
/// field with the keys keeps its own ⌘↵ (send: the inbox's reply, a
/// review comment, a commit message…), with the
/// bindings registered in the app's order (the fields' first).
#[test]
fn command_enter_sends_in_a_field_and_maximises_elsewhere() {
    use gpui::Action as _;
    let mut all = crate::kit::input::bindings();
    all.extend(bindings());
    let keymap = gpui::Keymap::new(all);
    let os = if cfg!(target_os = "macos") { gpui::OperatingSystem::Mac } else { gpui::OperatingSystem::Linux };
    let key = gpui::Keystroke::parse_with_os(if cfg!(target_os = "macos") { "cmd-enter" } else { "ctrl-shift-enter" }, os).unwrap();
    let ctx = |names: &[&str]| names.iter().map(|n| gpui::KeyContext::parse(n).unwrap()).collect::<Vec<_>>();
    let first = |stack: &[gpui::KeyContext]| keymap.bindings_for_input(std::slice::from_ref(&key), stack).0.first().map(|b| b.action().name());
    // In a field of the main screen (a tile's).
    // (Off macOS fields send with Ctrl+↵, a key of their own.)
    let in_field = if cfg!(target_os = "macos") { crate::kit::input::Submit.name() } else { ToggleMaximize.name() };
    assert_eq!(first(&ctx(&["MainView", "MainScreen", "InboxTile", "PwTextInput"])), Some(in_field));
    // Anywhere else in it: a terminal, a tile, the screen itself.
    assert_eq!(first(&ctx(&["MainView", "MainScreen", "Terminal"])), Some(ToggleMaximize.name()));
    assert_eq!(first(&ctx(&["MainView", "MainScreen"])), Some(ToggleMaximize.name()));
}
