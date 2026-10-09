//! The palette: matching, the queue syntax, the built-in rows and their
//! order, the registry, and the palette in a test window (open, filter,
//! ↑/↓/↵, Esc, ⌘K toggles), with made-up agents and no engine.

use gpui::{
    div, prelude::*, AppContext, Context, Entity, IntoElement, Render, TestAppContext,
    VisualTestContext, Window,
};

use super::matching::{looks_like_path, matches, parse_queue, visible};
use super::registry::{collect, rank, Command, PaletteContext};
use super::*;
use crate::agents::tests::agent;
use crate::agents::AgentStore;
use crate::main_screen::{self, CommandState, MainScreen, OpenPalette, Route};

fn state() -> CommandState {
    CommandState {
        agents: vec![
            agent("web", "/work/alpha", "blocked", 2, true),
            agent("api", "/work/alpha", "idle", 1, true),
        ],
        selected: Some("api".into()),
        projects: vec![("/work/alpha".into(), "alpha".into())],
        presets: vec![
            main_screen::tree::Preset::One,
            main_screen::tree::Preset::Two,
        ],
    }
}

fn rows(cx: &gpui::App, q: &str, st: &CommandState) -> Vec<Command> {
    let pc = PaletteContext {
        query: q,
        state: st,
        screen: None,
        explorer: None,
    };
    visible(collect(&pc, cx), q)
}

fn ids(rows: &[Command]) -> Vec<String> {
    rows.iter().map(|c| c.id.to_string()).collect()
}

#[test]
fn every_word_must_match() {
    assert!(matches("go jump api-fix working orders-api", "api WORK"));
    assert!(matches("anything", "  "));
    assert!(!matches("go jump api-fix working", "api blocked"));
}

#[test]
fn typed_paths_and_queue_syntax() {
    assert!(looks_like_path(" ~ ") && looks_like_path("/tmp") && looks_like_path("~/code"));
    assert!(!looks_like_path("code/x"));
    let st = state();
    let (a, text) = parse_queue("api: run the tests\nthen lint", &st.agents).unwrap();
    assert_eq!(
        (a.id.as_str(), text.as_str()),
        ("api", "run the tests\nthen lint")
    );
    let (a, text) = parse_queue("queue for web:  hi", &st.agents).unwrap();
    assert_eq!((a.id.as_str(), text.as_str()), ("web", " hi"));
    assert!(parse_queue("Queue web: x", &st.agents).is_some());
    assert!(parse_queue("nobody: x", &st.agents).is_none());
    assert!(parse_queue("api:   ", &st.agents).is_none());
    assert!(parse_queue("api", &st.agents).is_none());
}

#[gpui::test]
fn empty_query_lists_the_react_rows_in_order(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        let st = state();
        let r = rows(cx, "", &st);
        assert_eq!(
            ids(&r),
            [
                "go-web",
                "go-api",
                "remove-web",
                "remove-api",
                "next-blocked",
                "new",
                "term-here",
                "term-choose",
                "wall",
                "review",
                "window",
                "sidebar",
                "right",
                "settings",
                "quit",
                "quit-stop",
            ]
        );
        assert_eq!(r[1].plain(), "apiidle · alpha");
        assert_eq!(
            r[1].hint,
            Some(Hint::Sub("current".into())),
            "the focused agent"
        );
        assert_eq!(
            r[0].lead,
            Some(Lead::Status(pitwall_proto::Status::Blocked))
        );
        assert_eq!(r[5].hint, Some(Hint::Keys("⌘N")));

        // Nothing blocked: no "Jump to next blocked".
        let mut calm = state();
        calm.agents.retain(|a| a.id == "api");
        assert!(!ids(&rows(cx, "", &calm)).contains(&"next-blocked".to_string()));
    });
}

#[gpui::test]
fn typing_filters_and_reveals_the_query_only_rows(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        let st = state();
        assert_eq!(
            ids(&rows(cx, "density space", &st)),
            [
                "density-space-comfortable",
                "density-space-compact",
                "density-space-dense",
                "density-space-default",
            ]
        );
        let theme = rows(cx, "theme", &st);
        assert_eq!(
            theme.iter().map(Command::plain).collect::<Vec<_>>(),
            ["Theme: System", "Theme: Dark", "Theme: Light"]
        );
        assert_eq!(
            rows(cx, "density dense", &st)[0].plain(),
            "Density: Dense (40×8, default for all spaces)"
        );
        assert_eq!(ids(&rows(cx, "layout", &st)), ["preset-1", "preset-2"]);
        assert_eq!(ids(&rows(cx, "queue api", &st)), ["q-api"]);
        assert_eq!(
            ids(&rows(cx, "terminal in alpha", &st)),
            ["term-in-/work/alpha"]
        );
        // A path: "Terminal at" first, whatever else matches after it.
        let r = rows(cx, "/work/alpha", &st);
        assert_eq!(r[0].id.as_ref(), "term-at");
        assert_eq!(r[0].plain(), "Terminal at /work/alpha");
        assert!(rows(cx, "zzz qqq", &st).is_empty());
        // No explorer: no file rows.
        assert!(rows(cx, "file", &st).iter().all(|c| !c.id.contains("file")));
    });
}

#[gpui::test]
fn other_modules_add_rows_by_rank(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        register(cx, |pc, _| {
            if pc.selected().is_none() {
                return vec![];
            }
            vec![
                Command::new("extra", "Race Engineer", "race engineer", |_, _| {})
                    .glyph("◎")
                    .rank(rank::NEW + 50),
            ]
        });
        let st = state();
        let r = ids(&rows(cx, "", &st));
        let at = |id: &str| r.iter().position(|x| x == id).unwrap();
        assert_eq!(at("extra"), at("new") + 1);
        let mut none = state();
        none.selected = None;
        assert!(!ids(&rows(cx, "", &none)).contains(&"extra".to_string()));
    });
}

/// The main screen with the palette over it, as `ui::MainView` has them.
struct Root {
    screen: Entity<MainScreen>,
    host: Entity<PaletteHost>,
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        on_actions(div(), &self.host)
            .size_full()
            .child(self.screen.clone())
            .child(self.host.clone())
    }
}

fn window(cx: &mut TestAppContext) -> (Entity<Root>, &mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::menu::register(cx);
        main_screen::init(cx);
        init(cx);
    });
    let store = cx.new(|_| {
        AgentStore::new(vec![
            agent("api", "/work/alpha", "idle", 1, true),
            agent("web", "/work/alpha", "blocked", 2, true),
            agent("docs", "/work/beta", "done", 3, true),
        ])
    });
    cx.add_window_view(move |window, cx| {
        let screen = cx.new(|cx| MainScreen::new(store.clone(), window, cx));
        let host = cx.new(|cx| PaletteHost::new(Some(screen.clone()), None, window, cx));
        Root { screen, host }
    })
}

fn palette(root: &Entity<Root>, cx: &mut VisualTestContext) -> Option<Entity<Palette>> {
    root.read_with(cx, |r, cx| r.host.read(cx).palette().cloned())
}

#[gpui::test]
fn opens_filters_runs_and_closes(cx: &mut TestAppContext) {
    let (root, cx) = window(cx);
    cx.run_until_parked();
    cx.dispatch_action(OpenPalette);
    cx.run_until_parked();
    let p = palette(&root, cx).expect("⌘K opens it");
    p.update(cx, |p, cx| p.set_query("docs", cx));
    cx.run_until_parked();
    p.read_with(cx, |p, _| {
        assert_eq!(ids(p.rows()), ["go-docs", "remove-docs", "q-docs"]);
        assert_eq!(p.active(), 0);
    });
    cx.simulate_keystrokes("down");
    p.read_with(cx, |p, _| assert_eq!(p.active(), 1));
    cx.simulate_keystrokes("up up");
    p.read_with(cx, |p, _| assert_eq!(p.active(), 0, "stops at the top"));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(palette(&root, cx).is_none(), "running a row closes it");
    root.read_with(cx, |r, cx| {
        assert_eq!(
            r.screen.read(cx).focused_agent_id().as_deref(),
            Some("docs")
        );
    });

    // Esc closes; ⌘K toggles.
    cx.dispatch_action(OpenPalette);
    cx.run_until_parked();
    assert!(palette(&root, cx).is_some());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(palette(&root, cx).is_none(), "Esc closes it");
    cx.dispatch_action(OpenPalette);
    cx.run_until_parked();
    cx.dispatch_action(OpenPalette);
    cx.run_until_parked();
    assert!(palette(&root, cx).is_none(), "⌘K again closes it");
}

#[gpui::test]
fn queue_for_fills_the_input_and_stays_open(cx: &mut TestAppContext) {
    let (root, cx) = window(cx);
    cx.dispatch_action(OpenPalette);
    cx.run_until_parked();
    let p = palette(&root, cx).unwrap();
    p.update(cx, |p, cx| p.set_query("queue web", cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let p = palette(&root, cx).expect("still open");
    p.read_with(cx, |p, cx| {
        assert_eq!(p.query(cx), "web: ");
        assert_eq!(ids(p.rows()).len(), 0, "\"web: \" alone has no prompt yet");
    });
    cx.simulate_input("fix it");
    cx.run_until_parked();
    p.read_with(cx, |p, cx| {
        assert_eq!(p.query(cx), "web: fix it");
        assert_eq!(ids(p.rows()), ["queue"]);
        assert_eq!(p.rows()[0].plain(), "Queue for web: fix it");
    });
}

#[gpui::test]
fn wall_row_switches_the_route(cx: &mut TestAppContext) {
    let (root, cx) = window(cx);
    cx.dispatch_action(OpenPalette);
    cx.run_until_parked();
    let p = palette(&root, cx).unwrap();
    p.update(cx, |p, cx| p.set_query("wall", cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    root.read_with(cx, |r, cx| {
        assert_eq!(r.screen.read(cx).route(), Route::Wall)
    });
}

/// The hover pass over the open palette: moving over rows selects them and
/// nothing moves.
#[gpui::test]
fn hovering_rows_moves_nothing(cx: &mut TestAppContext) {
    let (_root, cx) = window(cx);
    cx.run_until_parked();
    // No opening animation under the pointer.
    crate::kit::motion::set_on(false);
    cx.dispatch_action(OpenPalette);
    cx.run_until_parked();
    let n = crate::kit::hover::probe::assert_hover_keeps_layout(cx, "palette");
    assert!(n >= 10, "only {n} rows hovered");
}
