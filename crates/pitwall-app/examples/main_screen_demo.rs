//! The main screen with the React browser mock's made-up agents and no
//! engine, for looking at it next to the web UI (`pnpm dev`, `?onboarded`).
//!
//! `cargo run -p pitwall-app --example main_screen_demo`
//!
//! - `DEMO_SIZE=1600x1000` window size (logical px)
//! - `DEMO_THEME=light` (default: the system's)
//! - `DEMO_SHOW=api-fix` show that agent (default: the first, as on a first run)
//! - `DEMO_OPEN=new-agent|remove|diff|menu|terminal` open a dialog or menu

use gpui::{
    px, size, App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};

use pitwall_app::agents::AgentStore;
use pitwall_app::main_screen::{self, demo, MainScreen};
use pitwall_app::theme::{Mode, Theme};

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
            main_screen::init(cx);
            let store = cx.new(|_| AgentStore::new(demo::agents()));
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
                    if let Ok(what) = std::env::var("DEMO_OPEN") {
                        s.demo_open(&what, window, cx);
                    }
                });
                screen
            })
            .expect("open the demo window");
            cx.activate(true);
        });
}
