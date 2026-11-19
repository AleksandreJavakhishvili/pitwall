//! Web pages across windows in a test app, with fake web views
//! (`crate::preview::fake`): an agent's `show_web` for every `where`, its
//! page reused; a page moved into a window of its own and docked back as
//! the same page (its view moved, not made again); restored after a
//! restart; refused when off or not a web address.

use gpui::{AppContext, Entity, Global, TestAppContext, WindowHandle};

use pitwall_proto::{code, ViewerPlace, WebShow};

use crate::agents::tests::agent;
use crate::agents::AgentStore;
use crate::main_screen::tree::Side;
use crate::main_screen::workspace::Whereabouts;
use crate::preview::fake::{self, Log};
use crate::ui::{Content, MainView};
use crate::windows::MAIN;

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

/// An app with agents api and web side by side in a space of main, fake
/// web views. The space's id and the views' log.
fn app(cx: &mut TestAppContext, home: Option<std::path::PathBuf>) -> (String, fake::Shared) {
    let log = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::main_screen::init(cx);
        crate::ui_state::init(home.clone(), cx);
        let log = fake::install(cx);
        let store = cx.new(|_| AgentStore::new(vec![agent("api", "/work/alpha", "idle", 1, true), agent("web", "/work/alpha", "idle", 2, true)]));
        cx.set_global(Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: Content::Live { store: store.clone() }, root: None, main: None });
        crate::windows::init(Content::Live { store }, home, show_main, cx);
        show_main(cx);
        crate::windows::restore(cx);
        log
    });
    cx.run_until_parked();
    let space = cx.update(|cx| {
        let (ui, id) = crate::ui_state::get(cx).create_custom_space(MAIN);
        let ui = ui.place_agent(&id, "api", None, None).place_agent(&id, "web", None, Some(Side::Right));
        crate::ui_state::update(cx, |s| *s = ui);
        let h = crate::windows::main_window(MAIN, cx).unwrap();
        let sid = id.clone();
        h.update(cx, |v, _, cx| v.screen().unwrap().update(cx, |s, cx| s.show_space(&sid, cx))).unwrap();
        id
    });
    frames(cx);
    Log::take(&log);
    (space, log)
}

/// Every window draws a frame (the views are placed after it), settled.
fn frames(cx: &mut TestAppContext) {
    for _ in 0..2 {
        cx.update(|cx| {
            for w in cx.windows() {
                let _ = w.update(cx, |_, window, _| window.refresh());
            }
        });
        cx.run_until_parked();
    }
}

fn ask(agent: &str, url: Option<&str>, place: Option<ViewerPlace>, of: Option<&str>) -> WebShow {
    WebShow { agent_id: agent.into(), url: url.map(String::from), place, of: of.map(String::from) }
}

fn show(req: &WebShow, cx: &mut TestAppContext) -> Result<pitwall_proto::WebShown, pitwall_proto::ErrorBody> {
    let r = cx.update(|cx| super::show::show(req, cx));
    frames(cx);
    r
}

fn page_of(agent: &str, cx: &mut TestAppContext) -> String {
    cx.update(|cx| crate::ui_state::get(cx).web_by(agent)).expect("its page")
}

fn whereabouts(id: &str, cx: &mut TestAppContext) -> Option<Whereabouts> {
    cx.update(|cx| super::place::whereabouts_of(id, cx))
}

#[gpui::test]
fn an_agents_page_goes_where_it_asks_and_is_reused(cx: &mut TestAppContext) {
    let (space, log) = app(cx, None);
    // No place: right of its own terminal, in the window showing it.
    let r = show(&ask("api", Some("example.com"), None, None), cx).expect("shown");
    let id = page_of("api", cx);
    assert!(!r.reused);
    assert_eq!(r.window, MAIN);
    assert_eq!(r.place, ViewerPlace::Right);
    assert!(r.at.contains("right of api's terminal"), "{}", r.at);
    assert_eq!(whereabouts(&id, cx), Some(Whereabouts::Next(Side::Right, "api".into())));
    let c = Log::take(&log);
    assert_eq!(c.first().map(String::as_str), Some("create https://example.com"), "{c:?}");
    assert!(c.contains(&"show".to_string()), "{c:?}");
    cx.update(|cx| {
        let ui = crate::ui_state::get(cx);
        let t = &ui.web_tiles[&id];
        assert_eq!((t.by.as_deref(), t.agent.as_deref(), t.window), (Some("api"), None, false), "a free page, remembered as api's");
        assert_eq!(ui.web_tile(&id).map(|(s, _)| s), Some(space.clone()));
    });
    // The same page again, somewhere else on it: navigated, not moved.
    let r = show(&ask("api", Some("https://example.com/docs"), None, None), cx).expect("again");
    assert!(r.reused);
    assert_eq!(page_of("api", cx), id);
    assert_eq!(Log::take(&log), ["navigate https://example.com/docs"]);
    assert_eq!(cx.update(|cx| crate::ui_state::get(cx).web_tiles.len()), 1, "one page per agent");
    // Each side of its terminal.
    for (place, side) in [(ViewerPlace::Left, Side::Left), (ViewerPlace::Above, Side::Top), (ViewerPlace::Below, Side::Bottom), (ViewerPlace::Right, Side::Right)] {
        let r = show(&ask("api", None, Some(place), None), cx).expect("moved");
        assert!(r.reused && r.place == place);
        assert_eq!(whereabouts(&id, cx), Some(Whereabouts::Next(side, "api".into())), "{place:?}");
    }
    let c = Log::take(&log);
    assert!(!c.iter().any(|c| c.starts_with("create") || c == "drop"), "the same view throughout: {c:?}");
    // Next to another agent's terminal (`of`).
    let r = show(&ask("api", None, Some(ViewerPlace::Below), Some("web")), cx).expect("of web");
    assert!(r.at.contains("below web's terminal"), "{}", r.at);
    // In web's terminal's place: web keeps running, out of the layout.
    let r = show(&ask("api", None, Some(ViewerPlace::Replace), Some("web")), cx).expect("replace");
    assert!(r.at.contains("in place of web's terminal"), "{}", r.at);
    cx.update(|cx| assert!(crate::ui_state::get(cx).space(&space).unwrap().layout.find_by_agent("web").is_none()));
}

#[gpui::test]
fn a_page_moves_to_a_window_and_back_as_the_same_page(cx: &mut TestAppContext) {
    let (space, log) = app(cx, None);
    show(&ask("api", Some("example.com"), None, None), cx).expect("shown");
    let id = page_of("api", cx);
    let page = cx.update(|cx| super::pages::get(&id, cx)).expect("its page");
    Log::take(&log);
    // `where: window`: out of the layout into a window of its own.
    let r = show(&ask("api", None, Some(ViewerPlace::Window), None), cx).expect("window");
    assert_eq!(r.place, ViewerPlace::Window);
    assert_eq!(r.at, "a window of its own");
    assert!(r.window.ends_with("— Web"), "{}", r.window);
    cx.update(|cx| {
        assert!(crate::windows::web_window(&id, cx).is_some());
        let ui = crate::ui_state::get(cx);
        assert!(ui.web_tile(&id).is_none() && ui.web_tiles[&id].window);
        assert_eq!(ui.space(&space).unwrap().layout.web_ids().len(), 0);
    });
    let c = Log::take(&log);
    assert!(c.contains(&"reparent".to_string()), "its view moved: {c:?}");
    assert!(!c.iter().any(|c| c.starts_with("create") || c == "drop"), "not made again: {c:?}");
    assert!(c.contains(&"show".to_string()), "placed in its window: {c:?}");
    assert_eq!(cx.update(|cx| super::pages::get(&id, cx)).map(|p| p.entity_id()), Some(page.entity_id()), "the same page");
    // Asked again with no place: it stays in its window.
    let r = show(&ask("api", Some("https://example.com/b"), None, None), cx).expect("again");
    assert!(r.reused && r.place == ViewerPlace::Window);
    assert_eq!(Log::take(&log), ["navigate https://example.com/b"]);
    // Docked back from its menu: right of api's terminal, the window gone.
    cx.update(|cx| super::place::place(&id, crate::explorer::dock::FilesPlace::Next(Side::Right), Some("api"), cx)).expect("docked");
    frames(cx);
    cx.update(|cx| {
        assert!(crate::windows::web_window(&id, cx).is_none(), "its window closed");
        assert!(!crate::ui_state::get(cx).web_tiles[&id].window);
        assert!(crate::ui_state::get(cx).web_tiles.contains_key(&id), "closing the window for the move kept the page");
    });
    assert_eq!(whereabouts(&id, cx), Some(Whereabouts::Next(Side::Right, "api".into())));
    let c = Log::take(&log);
    assert!(c.contains(&"reparent".to_string()) && !c.iter().any(|c| c.starts_with("create") || c == "drop"), "{c:?}");
    // The bar's "Open in a window of its own", then its window closed by
    // the user: the page closes.
    drop(page);
    cx.update(|cx| super::place::pop_out(&id, None, true, cx)).expect("popped out");
    frames(cx);
    cx.update(|cx| super::pages::close(&id, cx));
    frames(cx);
    cx.update(|cx| {
        assert!(crate::windows::web_window(&id, cx).is_none());
        assert!(crate::ui_state::get(cx).web_tiles.is_empty());
    });
    assert!(Log::take(&log).contains(&"drop".to_string()));
}

/// Where a view can't move between windows (Windows, Linux): made again
/// in the new one, the page loading again.
#[gpui::test]
fn without_reparenting_the_page_loads_again_in_its_window(cx: &mut TestAppContext) {
    let (_, log) = app(cx, None);
    log.borrow_mut().no_reparent = true;
    show(&ask("api", Some("example.com"), None, None), cx).expect("shown");
    Log::take(&log);
    show(&ask("api", None, Some(ViewerPlace::Window), None), cx).expect("window");
    let c = Log::take(&log);
    let refused = c.iter().position(|c| c == "reparent refused").expect("asked");
    let drop = c.iter().position(|c| c == "drop").expect("let go");
    let made = c.iter().position(|c| c == "create https://example.com").expect("made again");
    assert!(refused < drop && drop < made, "{c:?}");
}

/// A page in a window of its own opens in one again at the next start
/// (`ui.json`), behind the rest; its place and size are kept.
#[gpui::test]
fn a_page_in_a_window_comes_back_after_a_restart(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("pw-web-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (_, log) = app(cx, Some(dir.clone()));
    show(&ask("api", Some("example.com"), Some(ViewerPlace::Window), None), cx).expect("window");
    let id = page_of("api", cx);
    cx.executor().advance_clock(std::time::Duration::from_millis(800));
    cx.run_until_parked();
    let kept: crate::main_screen::workspace::UiState = serde_json::from_str(&std::fs::read_to_string(dir.join("ui.json")).unwrap()).unwrap();
    assert!(kept.web_tiles[&id].window && kept.web_tiles[&id].url == "https://example.com");
    // "Restarted": the page and its window gone from memory, ui.json kept.
    cx.update(|cx| {
        if let Some(h) = crate::windows::web_window(&id, cx) {
            let _ = h.update(cx, |w, window, _| {
                w.closing();
                window.remove_window();
            });
        }
    });
    cx.run_until_parked();
    cx.update(|cx| {
        cx.remove_global::<super::pages::Pages>();
        crate::ui_state::update(cx, |s| *s = kept.clone());
    });
    Log::take(&log);
    cx.update(super::window::restore);
    frames(cx);
    cx.update(|cx| assert!(crate::windows::web_window(&id, cx).is_some(), "open again"));
    let c = Log::take(&log);
    assert_eq!(c.first().map(String::as_str), Some("create https://example.com"), "{c:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn what_isnt_shown_is_refused_or_said(cx: &mut TestAppContext) {
    let (_, log) = app(cx, None);
    // Not a web address: refused, nothing made.
    for bad in ["javascript:alert(1)", "file:///etc/passwd", "how to center a div"] {
        let e = show(&ask("api", Some(bad), None, None), cx).expect_err(bad);
        assert_eq!(e.code, code::BAD_PARAMS, "{bad}");
    }
    assert!(cx.update(|cx| crate::ui_state::get(cx).web_tiles.is_empty()));
    assert!(Log::take(&log).is_empty());
    // No url: an empty page, its address field waiting, the keys left
    // where they were.
    let r = show(&ask("web", None, None, None), cx).expect("empty");
    assert!(r.at.contains("right of web's terminal"), "{}", r.at);
    let id = page_of("web", cx);
    let field_focused = cx.update(|cx| {
        let page = super::pages::ensure(&id, &[], cx).unwrap();
        let field = page.read(cx).field().clone();
        crate::windows::main_window(MAIN, cx).unwrap().update(cx, |_, w, cx| field.read(cx).is_focused(w)).unwrap()
    });
    assert!(!field_focused, "the keyboard stayed where it was");
    assert!(Log::take(&log).is_empty(), "no view without an address");
    // Next to a terminal no window shows: a window of its own, and said so.
    let r = show(&ask("api", Some("example.com"), Some(ViewerPlace::Left), Some("ghost")), cx).expect("shown");
    assert_eq!(r.place, ViewerPlace::Window);
    assert!(r.note.as_deref().unwrap_or_default().contains("isn't shown in any open Pitwall window"), "{:?}", r.note);
    // Turned off (agents.showWeb, or web pages): refused.
    for setting in [super::show::SETTING, super::page::SETTING] {
        cx.update(|cx| {
            let def = pitwall_proto::settings::find(setting).unwrap();
            crate::ui_state::update(cx, |s| *s = crate::settings::data::with(s, def, &serde_json::Value::Bool(false)));
        });
        let e = show(&ask("web", Some("example.com"), None, None), cx).expect_err(setting);
        assert_eq!(e.code, code::OFF);
        cx.update(|cx| {
            let def = pitwall_proto::settings::find(setting).unwrap();
            crate::ui_state::update(cx, |s| *s = crate::settings::data::with(s, def, &serde_json::Value::Bool(true)));
        });
    }
}
