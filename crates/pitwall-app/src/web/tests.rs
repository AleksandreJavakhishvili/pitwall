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

// ── Browser profiles ─────────────────────────────────────────────────────

/// Profiles kept in a temp folder (no engine), as macOS 14+ when
/// `separate`. Returns the folder and profile Work's id.
fn with_profiles(cx: &mut TestAppContext, separate: bool) -> (pitwall_core::testing::TempDir, String) {
    let t = pitwall_core::testing::TempDir::new("web-profiles-app");
    let paths = pitwall_core::paths::Paths::new(t.path().join("pw"));
    let work = cx.update(|cx| {
        cx.set_global(super::profiles::Separate(separate));
        let m = crate::accounts::model(cx);
        m.update(cx, |m, cx| {
            m.set_paths(paths);
            m.add_web("Work", cx).unwrap().id
        })
    });
    (t, work)
}

fn set_project_profile(project: &str, id: Option<&str>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        let m = crate::accounts::model(cx);
        m.update(cx, |m, cx| m.set_project_default(project, pitwall_core::accounts::WEB, id, cx)).unwrap();
    });
}

/// The folder (store) of the view made last; checks that a profile's has a
/// store of its own and Default's the one it always had.
fn last_dir(log: &fake::Shared) -> String {
    let l = log.borrow();
    let dir = l.dirs.last().map(|d| d.file_name().unwrap().to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(own) = l.own.last() {
        assert_eq!(*own, dir != "profile-default", "{dir}: own store {own}");
    }
    dir
}

/// An agent's page opens in its project's profile, and the answer says
/// which; the project's profile changed, its page stays until the agent
/// shows it again, which moves it there (loaded again) and says so.
#[gpui::test]
fn an_agents_page_opens_in_its_projects_profile(cx: &mut TestAppContext) {
    let (_, log) = app(cx, None);
    let (_t, work) = with_profiles(cx, true);
    // No profile for the project: Default, as before profiles.
    let r = show(&ask("api", Some("example.org"), None, None), cx).expect("shown");
    assert_eq!(r.profile, "Default");
    assert_eq!(last_dir(&log), "profile-default");
    let first = cx.update(|cx| page_of_app("api", cx));
    assert_eq!(cx.update(|cx| crate::ui_state::get(cx).web_tiles[&first].profile.clone()), None);
    cx.update(|cx| super::pages::close(&first, cx));
    frames(cx);

    set_project_profile("/work/alpha", Some(&work), cx);
    let r = show(&ask("api", Some("example.com"), None, None), cx).expect("shown");
    assert_eq!(r.profile, "Work");
    let id = page_of("api", cx);
    assert_eq!(last_dir(&log), "profile-web-work", "its view in Work's store");
    cx.update(|cx| assert_eq!(crate::ui_state::get(cx).web_tiles[&id].profile.as_deref(), Some(work.as_str())));
    Log::take(&log);

    // The project goes back to Default: its open page is left as it is.
    set_project_profile("/work/alpha", None, cx);
    frames(cx);
    assert!(!Log::take(&log).iter().any(|c| c.starts_with("create") || c == "drop"), "nothing reloads by itself");
    // The agent shows it again: into Default, loaded again there, said.
    let r = show(&ask("api", None, None, None), cx).expect("again");
    assert!(r.reused);
    assert_eq!(r.profile, "Default");
    let note = r.note.unwrap_or_default();
    assert!(note.contains("was in the Work browser profile; it is in Default now"), "{note}");
    let c = Log::take(&log);
    let drop = c.iter().position(|c| c == "drop").expect("its view let go");
    let made = c.iter().position(|c| c == "create https://example.com").expect("made again");
    assert!(drop < made, "{c:?}");
    assert_eq!(last_dir(&log), "profile-default");
    cx.update(|cx| assert_eq!(crate::ui_state::get(cx).web_tiles[&id].profile, None));
}

fn page_of_app(agent: &str, cx: &gpui::App) -> String {
    crate::ui_state::get(cx).web_by(agent).expect("its page")
}

/// The bar's profile menu switches a page: its view made again in the
/// other store on the address it shows; a profile can't be removed while a
/// page is in it, and once removed its data is erased and its projects
/// forget it.
#[gpui::test]
fn a_page_switches_profile_and_a_profile_in_use_isnt_removed(cx: &mut TestAppContext) {
    let (_, log) = app(cx, None);
    let (_t, work) = with_profiles(cx, true);
    show(&ask("api", Some("example.com"), None, None), cx).expect("shown");
    let id = page_of("api", cx);
    let page = cx.update(|cx| super::pages::get(&id, cx)).unwrap();
    Log::emit(&log, crate::webview::WebEvent::Url("https://example.com/inbox".into()));
    cx.run_until_parked();
    Log::take(&log);
    page.update(cx, |p, cx| p.set_profile(Some(work.clone()), cx));
    frames(cx);
    let c = Log::take(&log);
    assert_eq!(c.iter().filter(|c| c.starts_with("create")).collect::<Vec<_>>(), ["create https://example.com/inbox"], "{c:?}");
    assert!(c.iter().position(|c| c == "drop") < c.iter().position(|c| c.starts_with("create")));
    assert_eq!(last_dir(&log), "profile-web-work");
    cx.update(|cx| assert_eq!(crate::ui_state::get(cx).web_tiles[&id].profile.as_deref(), Some(work.as_str())));
    // The same one again: nothing.
    page.update(cx, |p, cx| p.set_profile(Some(work.clone()), cx));
    frames(cx);
    assert!(Log::take(&log).is_empty());

    // In use: refused, nothing erased.
    set_project_profile("/work/beta", Some(&work), cx);
    let e = cx.update(|cx| super::profiles::remove(&work, cx)).expect_err("in use");
    assert!(e.contains("A page is open in Work"), "{e}");
    assert!(cx.update(|cx| cx.try_global::<super::profiles::Erased>().is_none_or(|e| e.0.is_empty())));
    // Switched back to Default: removed, its data erased, projects forget it.
    page.update(cx, |p, cx| p.set_profile(None, cx));
    frames(cx);
    cx.update(|cx| super::profiles::remove(&work, cx)).expect("removed");
    cx.update(|cx| {
        let erased = &cx.global::<super::profiles::Erased>().0;
        assert_eq!(erased.len(), 1);
        assert!(erased[0].ends_with("webview/profile-web-work"), "{erased:?}");
        assert!(super::profiles::all(cx).is_empty());
        assert_eq!(super::profiles::for_project("/work/beta", cx), None);
    });
}

/// A page comes back in its profile after a restart; one whose profile is
/// gone comes back in Default and says so once (ui.json forgets it).
#[gpui::test]
fn restored_pages_keep_their_profile_or_fall_back_once(cx: &mut TestAppContext) {
    let (_, log) = app(cx, None);
    let (_t, work) = with_profiles(cx, true);
    show(&ask("api", Some("example.com"), Some(ViewerPlace::Window), None), cx).expect("window");
    let id = page_of("api", cx);
    let restart = |profile: Option<&str>, cx: &mut TestAppContext| {
        let p = profile.map(String::from);
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
            crate::ui_state::update(cx, |s| *s = s.clone().set_web_profile(&id, p.as_deref()));
        });
        Log::take(&log);
        cx.update(super::window::restore);
        frames(cx);
    };
    restart(Some(&work), cx);
    assert_eq!(last_dir(&log), "profile-web-work", "back in its profile");
    cx.update(|cx| assert_eq!(super::pages::get(&id, cx).unwrap().read(cx).profile_note(), None));
    restart(Some("web-gone"), cx);
    assert_eq!(last_dir(&log), "profile-default");
    cx.update(|cx| {
        let page = super::pages::get(&id, cx).unwrap();
        assert!(page.read(cx).profile_note().unwrap_or_default().contains("was removed, so it opened in Default"));
        assert_eq!(crate::ui_state::get(cx).web_tiles[&id].profile, None, "said once");
    });
}

/// Before macOS 14 (one store for every view): only Default. A project's
/// profile isn't used, and no view is ever made for another profile there.
#[gpui::test]
fn without_separate_stores_only_default_is_used(cx: &mut TestAppContext) {
    let (_, log) = app(cx, None);
    let (_t, work) = with_profiles(cx, false);
    set_project_profile("/work/alpha", Some(&work), cx);
    let r = show(&ask("api", Some("example.com"), None, None), cx).expect("shown");
    assert_eq!(r.profile, "Default");
    assert_eq!(last_dir(&log), "profile-default");
    let id = page_of("api", cx);
    let page = cx.update(|cx| super::pages::get(&id, cx)).unwrap();
    Log::take(&log);
    // Forced into Work anyway (the menu doesn't offer it): no view, why.
    page.update(cx, |p, cx| p.set_profile(Some(work.clone()), cx));
    frames(cx);
    assert!(!Log::take(&log).iter().any(|c| c.starts_with("create")), "never in a shared store");
    let e = cx.update(|cx| page.read(cx).error()).unwrap_or_default();
    assert!(e.contains("macOS 14"), "{e}");
}

/// A page the user opens (⌘L, "Open web page…") is in the profile of the
/// project of the agent whose tile has the focus; with none, Default.
#[gpui::test]
fn a_page_you_open_takes_the_focused_agents_project_profile(cx: &mut TestAppContext) {
    let (space, _log) = app(cx, None);
    let (_t, work) = with_profiles(cx, true);
    set_project_profile("/work/alpha", Some(&work), cx);
    let open = |focus: Option<&str>, cx: &mut TestAppContext| {
        let sid = space.clone();
        let focus = focus.map(String::from);
        cx.update(|cx| {
            // (A page with the keys would take ⌘L itself.)
            for (id, _) in super::pages::all(cx) {
                super::pages::close(&id, cx);
            }
            crate::ui_state::update(cx, |s| {
                let pane = focus.as_deref().and_then(|a| s.space(&sid).unwrap().layout.find_by_agent(a)).map(|p| p.id);
                if let Some(sp) = s.spaces.iter_mut().find(|sp| sp.id == sid) {
                    sp.focused_pane_id = pane;
                }
            });
        });
        cx.run_until_parked();
        let before: Vec<String> = cx.update(|cx| crate::ui_state::get(cx).web_tiles.keys().cloned().collect());
        cx.update(|cx| {
            let h = crate::windows::main_window(MAIN, cx).unwrap();
            h.update(cx, |v, window, cx| v.screen().unwrap().update(cx, |s, cx| s.cmd_open_web_page(window, cx))).unwrap();
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let ui = crate::ui_state::get(cx);
            let (_, t) = ui.web_tiles.iter().find(|(k, _)| !before.contains(k)).unwrap_or_else(|| panic!("a new page: {before:?} {:?}", ui.web_tiles));
            t.profile.clone()
        })
    };
    assert_eq!(open(Some("api"), cx).as_deref(), Some(work.as_str()), "the focused agent's project's");
    assert_eq!(open(None, cx), None, "no focused agent: Default");
}
