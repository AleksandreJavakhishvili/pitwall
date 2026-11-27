//! The rest of agents' tile tools, headless (GPUI's test platform): the
//! palette's form for hello's `send_message` (it needs input) made from
//! its schema, checked, and run as the user's own; and hello's skill for
//! agents, given to a project, kept in step with the extension and its
//! tile's agent switch.

use std::time::{Duration, Instant};

use gpui::{AppContext, Entity, Global, TestAppContext, WindowHandle};
use pitwall_core::testing::{record, FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};

use crate::agents::tests::agent;
use crate::agents::AgentStore;
use crate::main_screen::tree::Surface;
use crate::tiles::form::Given;
use crate::ui::{Content, MainView};

const WAIT: Duration = Duration::from_secs(60);

struct Main(Option<WindowHandle<MainView>>, Entity<AgentStore>);

impl Global for Main {}

fn show_main(cx: &mut gpui::App) -> WindowHandle<MainView> {
    if let Some(h) = cx.global::<Main>().0 {
        if cx.windows().iter().any(|w| w.window_id() == h.window_id()) {
            return h;
        }
    }
    let store = cx.global::<Main>().1.clone();
    let h = cx.open_window(Default::default(), |window, cx| cx.new(|cx| MainView::new(Content::Live { store }, None, window, cx))).unwrap();
    cx.global_mut::<Main>().0 = Some(h);
    h
}

fn until(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let end = Instant::now() + WAIT;
    loop {
        cx.update(|cx| {
            for w in cx.windows() {
                let _ = w.update(cx, |_, window, _| window.refresh());
            }
        });
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < end, "no {what} within {WAIT:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Pitwall with the hello fixture on (its folder at `root`) and a main
/// window; `skill`: an `agents/SKILL.md` for it.
fn setup(t: &TempDir, skill: Option<&str>, cx: &mut TestAppContext) -> WindowHandle<MainView> {
    let root = t.path().join("data");
    super::tests::copy_fixture(&root.join("extensions/hello"));
    if let Some(s) = skill {
        std::fs::create_dir_all(root.join("extensions/hello/agents")).unwrap();
        std::fs::write(root.join("extensions/hello/agents/SKILL.md"), s).unwrap();
    }
    cx.executor().allow_parking();
    let engine = pitwall_core::Engine::open(pitwall_core::Deps {
        paths: pitwall_core::paths::Paths::new(t.path().join("engine")),
        events: RecordingSink::new(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![record("api", "/work/alpha")]),
        providers: vec![FakeProvider::named("local", "this-mac", std::sync::Arc::new(pitwall_core::exec::LocalExec))],
    });
    let main = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::main_screen::init(cx);
        crate::ui_state::init(None, cx);
        crate::preview::fake::install(cx);
        crate::extensions::init(Some(root.clone()), cx);
        crate::extensions::catalog::set_bundle(None, cx);
        crate::extensions::set_engine(engine, cx);
        crate::extensions::follow(cx);
        cx.set_global(super::DataDir(root.clone()));
        let store = cx.new(|_| AgentStore::new(vec![agent("api", "/work/alpha", "idle", 1, true)]));
        cx.set_global(Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: Content::Live { store: store.clone() }, root: None, main: None });
        crate::windows::init(Content::Live { store }, None, show_main, cx);
        let main = show_main(cx);
        crate::windows::restore(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true));
        main
    });
    cx.run_until_parked();
    main
}

fn shown(cx: &mut TestAppContext) -> Vec<String> {
    let key = Surface::View("hello".into(), "main".into()).key();
    cx.update(|cx| crate::main_screen::view_tile::get(&key, cx).and_then(|v| v.read(cx).tile().cloned()).map(|t| t.read(cx).texts()).unwrap_or_default())
}

#[gpui::test]
fn the_palette_asks_for_a_commands_input_in_a_form_and_runs_it_as_the_user(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-tool-form");
    let main = setup(&t, None, cx);
    let screen = cx.update(|cx| main.read(cx).unwrap().screen()).expect("a main screen");
    // Hello on screen, to see what was sent.
    cx.update(|cx| crate::windows::show_extension_view("hello", "main", cx));
    until(cx, "hello", |cx| shown(cx).iter().any(|s| s.contains("Clicked 0 times")));

    // Its palette entry says it asks for more ("…").
    let rows = cx.update(|cx| {
        let state = screen.read(cx).command_state(cx);
        let pc = crate::palette::PaletteContext { query: "", state: &state, screen: Some(screen.downgrade()), explorer: None };
        crate::palette::builtin::commands(&pc, cx)
    });
    let row = rows.iter().find(|c| c.id.as_ref() == "tile-tool-hello-send_message").expect("hello's command in the palette");
    assert_eq!(row.plain(), "Hello: Send a message…");
    let crate::palette::Run::Do(run) = row.run.clone() else { panic!("runs") };
    let _ = main.update(cx, |_, window, cx| run(window, cx));
    cx.run_until_parked();
    let form = cx.update(|cx| screen.read(cx).tool_form()).expect("the form opened");

    // Run with nothing given: the needed field says so, nothing runs.
    cx.update(|cx| form.update(cx, |f, cx| f.run(cx)));
    assert!(cx.update(|cx| screen.read(cx).tool_form()).is_some(), "still open");
    // A number that isn't one, then fixed.
    cx.update(|cx| {
        form.update(cx, |f, cx| {
            f.set(0, Given::Text("hi from the palette".into()), cx);
            f.set(1, Given::Text("soon".into()), cx);
            f.run(cx);
        })
    });
    assert!(cx.update(|cx| screen.read(cx).tool_form()).is_some(), "a whole number is asked for");
    cx.update(|cx| {
        form.update(cx, |f, cx| {
            f.set(1, Given::Text("0".into()), cx);
            f.run(cx);
        })
    });
    cx.run_until_parked();
    assert!(cx.update(|cx| screen.read(cx).tool_form()).is_none(), "run: the form closes");
    until(cx, "what was sent", |cx| shown(cx).iter().any(|s| s.contains("Sent: hi from the palette;")));
    let log = cx.update(|cx| crate::extensions::calls::activity("hello", cx).lock().unwrap().entries());
    assert!(log.iter().any(|e| e.detail.as_deref().is_some_and(|d| d.starts_with("You (act)") && d.contains("\"wait_ms\":0"))), "{log:?}");
}

#[gpui::test]
fn an_extensions_skill_reaches_the_projects_it_was_given_while_agents_may_use_it(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-skill");
    let _main = setup(&t, Some("---\nname: hello\ndescription: Show messages in Pitwall's Hello tile.\n---\nCall `hello__show_message` with a short message.\n"), cx);
    let file = t.path().join("engine/run/ext-skills/hello/SKILL.md");
    assert!(cx.update(|cx| super::agents::skills(cx)).is_empty(), "given to no project yet");
    cx.update(|cx| super::agents::set_skill_in("hello", "/work/alpha", true, cx));
    cx.run_until_parked();
    let skills = cx.update(|cx| super::agents::skills(cx));
    assert_eq!(skills.len(), 1);
    assert_eq!((skills[0].id.as_str(), skills[0].projects.clone()), ("hello", vec!["/work/alpha".to_string()]));
    let text = std::fs::read_to_string(&file).expect("waiting for agents");
    assert!(text.starts_with("---\nname: hello\n") && text.contains("This skill comes from the Hello tile installed in Pitwall") && text.contains("hello__show_message"), "{text}");
    let dirs = pitwall_core::rules::Dirs { base: t.path().join("engine") };
    assert_eq!(pitwall_core::rules::ext_skills::for_project(&dirs, "/work/alpha").len(), 1);
    assert!(pitwall_core::rules::ext_skills::for_project(&dirs, "/work/beta").is_empty());

    // The tile's agent switch off: agents don't get it.
    cx.update(|cx| super::agents::set_tile_on("hello", "main", false, cx));
    cx.run_until_parked();
    assert!(!file.exists(), "taken away with its tools");
    cx.update(|cx| super::agents::set_tile_on("hello", "main", true, cx));
    cx.run_until_parked();
    assert!(file.exists());
    // The extension off: gone too.
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", false)));
    cx.run_until_parked();
    assert!(!file.exists(), "taken away with the extension");
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true)));
    cx.run_until_parked();
    assert!(file.exists());
    // Taken off the project.
    cx.update(|cx| super::agents::set_skill_in("hello", "/work/alpha", false, cx));
    cx.run_until_parked();
    assert!(!file.exists());
}
