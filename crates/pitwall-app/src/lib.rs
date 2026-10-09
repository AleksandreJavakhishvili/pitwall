//! Pitwall's native app, drawn with GPUI (docs/spec/gpui/README.md). It
//! hosts `pitwall-core`'s Engine in-process exactly as the Tauri app does
//! (`host`), hears it through a channel (`bridge`) into GPUI entities
//! (`agents`), and draws windows from them (`ui`). Phase 0: the shell — one
//! window that remembers its bounds, the app menu, theme tokens and a live
//! sidebar of agents; terminals come from `pitwall-term-view`.

pub mod agents;
pub mod bench;
pub mod approvals;
pub mod bridge;
pub mod cli_spaces;
pub mod explorer;
pub mod frame_stats;
pub mod code_view;
pub mod engineer;
pub mod home;
pub mod kit;
pub mod host;
pub mod main_screen;
pub mod menu;
pub mod palette;
pub mod next_up;
pub mod settings;
pub mod platform;
pub mod remote;
pub mod packaging;
pub mod terminal;
pub mod review;
pub mod rules;
pub mod theme;
pub mod ui;
pub mod ui_state;
pub mod wall;
pub mod window_state;
pub mod windows;

use std::path::PathBuf;

use gpui::{
    px, size, App, AppContext, Application, Bounds, Global, TitlebarOptions,
    WindowBounds, WindowHandle, WindowOptions,
};

use pitwall_core::paths::Paths;

use crate::agents::AgentStore;
use crate::bridge::Bridge;
use crate::host::Host;
use crate::menu::{OpenSettings, Quit, QuitAndStopAgents, ShowMain};
use crate::theme::{Mode, Theme};
use crate::ui::{Content, MainView};

/// The app's own state: the hosted engine (none when refused), what windows
/// show, and the main window.
struct AppState {
    host: Option<Host>,
    content: Content,
    /// The data folder (`None` when refused: nothing is written anywhere).
    root: Option<PathBuf>,
    main: Option<WindowHandle<MainView>>,
}

impl Global for AppState {}

pub fn run() {
    let started = std::time::Instant::now();
    // This thread draws the UI: debug builds fail loudly when a program is
    // started on it (pitwall_core::exec::assert_off_ui).
    pitwall_core::exec::mark_ui_thread();
    let chosen = home::choose(&home::Env::current(), Paths::default_root(), home::answers);
    // A second launch on the same folder brings the running app forward.
    if let Err(home::Refusal::InUse { root }) = &chosen {
        if platform::single_instance::focus_running(root) {
            return;
        }
    }
    // PATH as the user's terminal has it, for everything agents start (an app
    // opened from the Dock gets a minimal one). Tauri: platform::prepare_env.
    let _probe =
        pitwall_core::shell::adopt_login_path(chosen.as_ref().ok().map(Paths::login_path_file));

    let app = Application::new().with_assets(kit::Assets);
    // macOS: closing the window keeps the app (and the Dock icon); the Dock
    // brings the window back. Elsewhere the last window quits (the Windows
    // tray arrives in phase 7).
    app.on_reopen(|cx| {
        show_main(cx);
    });
    app.run(move |cx: &mut App| {
        cx.set_global(Theme::for_mode(Mode::for_appearance(
            cx.window_appearance(),
        )));
        kit::init(cx);
        menu::register(cx);
        main_screen::init(cx);
        palette::init(cx);
        platform::init(cx);
        explorer::register(cx);
        review::register(cx);
        next_up::register(cx);

        let state = match chosen {
            Ok(paths) => {
                let (bridge, rx) = Bridge::new();
                let host = Host::start(paths.clone(), bridge);
                cx.set_global(explorer::ExplorerSource::engine(host.engine.clone()));
                cx.set_global(review::ReviewEngine { engine: host.engine.clone(), root: Some(paths.root().into()) });
                wall::register(cx, Some(host.engine.clone()));
                let store = cx.new(|_| AgentStore::new(host.engine.views()));
                cx.set_global(agents::StoreHandle(store.clone()));
                cx.set_global(main_screen::EngineHandle(host.engine.clone()));
                AgentStore::listen(&store, rx, cx);
                platform::follow(&store, cx);
                platform::single_instance::serve(paths.root(), cx);
                approvals::init(&store, host.approvals.clone(), cx);
                terminal::init(&store, cx);
                AppState {
                    host: Some(host),
                    content: Content::Live { store },
                    root: Some(paths.root().into()),
                    main: None,
                }
            }
            Err(refusal) => {
                eprintln!("pitwall: {}", refusal.title());
                for line in refusal.explanation() {
                    eprintln!("  {line}");
                }
                AppState {
                    host: None,
                    content: Content::Refused(refusal),
                    root: None,
                    main: None,
                }
            }
        };
        let engine = state.host.as_ref().map(|h| h.engine.clone());
        let root = state.root.clone();
        cx.set_global(state);
        ui_state::init(root.clone(), cx);
        windows::init(cx.global::<AppState>().content.clone(), root.clone(), show_main, cx);
        settings::init(cx, engine.clone(), root);
        if let Some(b) = cx.global::<AppState>().host.as_ref().map(|h| h.settings.clone()) {
            settings::attach_backend(b, cx);
        }
        if let Some(w) = cx.global::<AppState>().host.as_ref().map(|h| h.workspace.clone()) {
            cli_spaces::attach(w, cx);
        }
        engineer::register(cx, engine.as_ref());
        rules::init(cx, engine);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(quit_and_stop_agents);
        // These run deferred: dispatched from inside the window (a button,
        // a key), the window is being updated and can't be reached yet.
        cx.on_action(|_: &ShowMain, cx| {
            cx.defer(|cx| {
                show_main(cx);
            })
        });
        cx.on_action(|_: &OpenSettings, cx| {
            cx.defer(|cx| {
                let Some(target) = windows::active_or_main(cx) else { return };
                let _ = target.update(cx, |view, _, cx| view.open_settings(cx));
            })
        });
        // macOS: the standard About panel, like the Tauri app's default
        // menu. Elsewhere: the in-app About box (settings::about).
        cx.on_action(|_: &menu::About, cx| {
            if cfg!(target_os = "macos") {
                platform::about(cx);
            } else {
                cx.defer(|cx| {
                    let Some(target) = windows::active_or_main(cx) else { return };
                    let _ = target.update(cx, |view, _, cx| view.open_about(cx));
                })
            }
        });
        // Quitting leaves agents running in their holders (Tauri: RunEvent::Exit).
        cx.on_app_quit(|cx| {
            window_state::flush();
            if let Some(host) = cx.global_mut::<AppState>().host.as_mut() {
                host.shutdown();
            }
            async {}
        })
        .detach();
        cx.on_window_closed(|cx| {
            // Only the main window's closing forgets it (a secondary one
            // closing leaves it open).
            let main = cx.global::<AppState>().main;
            let open = cx.windows();
            if main.is_some_and(|m| open.iter().all(|w| w.window_id() != m.window_id())) {
                cx.global_mut::<AppState>().main = None;
                // Linux and Windows without a tray: closing main quits (agents
                // keep running in their holders), as the Tauri app does.
                if !cfg!(target_os = "macos") {
                    cx.quit();
                }
            }
            if !cfg!(target_os = "macos") && open.is_empty() {
                cx.quit();
            }
        })
        .detach();

        bench::prepare();
        let main = show_main(cx);
        windows::restore(cx);
        windows::debug_hook(cx);
        if bench::enabled() {
            // Bench: never take the user's focus (`bench.rs`).
            if let Some(root) = cx.global::<AppState>().root.clone() {
                bench::start(main, root, started, cx);
            }
        } else {
            cx.activate(true);
        }
    });
}

/// The main window, opened where it last was if it isn't open.
fn show_main(cx: &mut App) -> WindowHandle<MainView> {
    if let Some(main) = cx.global::<AppState>().main {
        if main
            .update(cx, |_, window, _| {
                platform::unhide(window);
                window.activate_window()
            })
            .is_ok()
            // Busy (being updated right now) but open: still the one window.
            || cx.windows().iter().any(|w| w.window_id() == main.window_id())
        {
            return main;
        }
    }
    let state = cx.global::<AppState>();
    let (content, root) = (state.content.clone(), state.root.clone());
    let displays: Vec<_> = cx.displays().iter().map(|d| d.bounds()).collect();
    let bounds = root
        .as_deref()
        .and_then(window_state::load)
        .filter(|s| s.visible_on(&displays))
        .map(window_state::Saved::to_bounds)
        .or_else(|| windows::main_bounds(cx))
        .unwrap_or_else(|| {
            let (w, h) = window_state::DEFAULT_SIZE;
            WindowBounds::Windowed(Bounds::centered(None, size(px(w), px(h)), cx))
        });
    let (min_w, min_h) = window_state::MIN_SIZE;
    let options = WindowOptions {
        window_bounds: Some(bounds),
        titlebar: Some(TitlebarOptions {
            title: Some("Pitwall".into()),
            ..Default::default()
        }),
        window_min_size: Some(size(px(min_w), px(min_h))),
        window_decorations: platform::decorations::request(),
        app_id: Some(platform::WINDOW_APP_ID.into()),
        ..Default::default()
    };
    // Debug builds and benches: `PITWALL_DEBUG_FLOAT=1` floats the window
    // above the others without taking focus, so animations keep running
    // (they pause while the window is covered) during a CPU measurement.
    let options = if bench::enabled() && !bench::visible() {
        // Bench: shown and drawn, but off-screen (`bench.rs`).
        let (x, y) = bench::OFF_SCREEN;
        let mut b = options.window_bounds.map(|b| b.get_bounds()).unwrap_or_default();
        b.origin = gpui::point(px(x), px(y));
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(b)),
            focus: false,
            ..options
        }
    } else if std::env::var_os("PITWALL_DEBUG_FLOAT").is_some() || bench::visible() {
        WindowOptions {
            kind: gpui::WindowKind::PopUp,
            focus: false,
            ..options
        }
    } else {
        options
    };
    let main = cx
        .open_window(options, |window, cx| {
            platform::lean_renderer(window);
            platform::keep_on_close(window, cx);
            cx.new(|cx| MainView::new(content, root, window, cx))
        })
        .expect("open the main window");
    cx.global_mut::<AppState>().main = Some(main);
    main
}

/// End every agent (off the main thread: stopping waits on holders), then quit.
fn quit_and_stop_agents(_: &QuitAndStopAgents, cx: &mut App) {
    let Some(engine) = cx
        .global::<AppState>()
        .host
        .as_ref()
        .map(|h| h.engine.clone())
    else {
        cx.quit();
        return;
    };
    let stopping = cx
        .background_executor()
        .spawn(async move { pitwall_core::engine::lifecycle::stop_all(&engine) });
    cx.spawn(async move |cx| {
        stopping.await;
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}
