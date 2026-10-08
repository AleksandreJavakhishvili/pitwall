//! The main screen with the React browser mock's made-up agents and no
//! engine, for looking at it next to the web UI (`pnpm dev`, `?onboarded`).
//!
//! `cargo run -p pitwall-app --example main_screen_demo`
//!
//! - `DEMO_SIZE=1600x1000` window size (logical px)
//! - `DEMO_THEME=light` (default: the system's)
//! - `DEMO_SHOW=api-fix` show that agent (default: the first, as on a first run)
//! - `DEMO_OPEN=new-agent|remove|diff|menu|terminal|remove-worktree` open a dialog or menu
//! - `DEMO_WT=open` expand every worktree list (the mock's worktrees show either way)
//! - `DEMO_FILES=1` the right panel on its Files tab
//! - `DEMO_RULES=1` the browser mock's rules, the first agent's rules stale
//!   and the second's failed (debug builds)
//! - `DEMO_OPEN=toasts|bring` also: the attention toasts, Bring in
//! - `DEMO_LOOK=glass` Glass (with `PITWALL_GLASS=lite|liquid` for the tier)
//! - `DEMO_MOTION=0` Reduce motion

use gpui::{
    px, size, App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};

use pitwall_app::agents::AgentStore;
use pitwall_app::main_screen::{self, demo, MainScreen};
use pitwall_app::theme::{self, Mode, Theme};

fn main() {
    let (w, h) = std::env::var("DEMO_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((1600.0_f32, 1000.0_f32));
    Application::new()
        .with_assets(pitwall_app::kit::Assets)
        .run(move |cx: &mut App| {
            let mode = match std::env::var("DEMO_THEME").as_deref() {
                Ok("light") => Mode::Light,
                Ok("dark") => Mode::Dark,
                _ => Mode::for_appearance(cx.window_appearance()),
            };
            cx.set_global(Theme::for_mode(mode));
            pitwall_app::kit::init(cx);
            let look = if std::env::var("DEMO_LOOK").as_deref() == Ok("glass") {
                theme::Look::Glass
            } else {
                theme::Look::Flat
            };
            theme::apply(
                theme::Inputs {
                    pref: if mode == Mode::Light {
                        theme::ThemePref::Light
                    } else {
                        theme::ThemePref::Dark
                    },
                    look,
                    reduce_motion: std::env::var("DEMO_MOTION").as_deref() == Ok("0"),
                    ..Default::default()
                },
                cx,
            );
            main_screen::init(cx);
            pitwall_app::code_view::register(cx);
            pitwall_app::next_up::register(cx);
            if std::env::var("DEMO_FILES").is_ok() {
                pitwall_app::ui_state::update(cx, |s| s.right_files = true);
            }
            let store = cx.new(|_| AgentStore::new(demo::agents()));
            #[cfg(debug_assertions)]
            if std::env::var_os("DEMO_RULES").is_some() {
                let rules = std::sync::Arc::new(pitwall_app::rules::mock::MockRules::new());
                let agents = demo::agents();
                if let Some(a) = agents.first() {
                    rules.set_agent(&a.id, true, None);
                }
                if let Some(a) = agents.get(1) {
                    rules.set_agent(&a.id, false, Some("rulesync exited with status 1".into()));
                }
                pitwall_app::rules::install(cx, rules);
            }
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(w), px(h)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Pitwall demo".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            cx.open_window(options, |window, cx| {
                theme::apply_window(window, cx);
                let screen = cx.new(|cx| MainScreen::new(store.clone(), window, cx));
                screen.update(cx, |s, cx| {
                    if let Ok(name) = std::env::var("DEMO_SHOW") {
                        let id = s
                            .store()
                            .read(cx)
                            .agents
                            .iter()
                            .find(|a| a.name == name)
                            .map(|a| a.id.clone());
                        if let Some(id) = id {
                            s.show_agent(&id, window, cx);
                        }
                    }
                    s.demo_changes(cx);
                    s.demo_elsewhere(cx);
                    s.demo_worktrees(std::env::var("DEMO_WT").is_ok(), cx);
                    match std::env::var("DEMO_OPEN").as_deref() {
                        Ok("remove-worktree") => s.demo_remove_worktree(cx),
                        Ok(what) => s.demo_open(what, window, cx),
                        Err(_) => {}
                    }
                });
                screen
            })
            .expect("open the demo window");
            cx.activate(true);
        });
}
