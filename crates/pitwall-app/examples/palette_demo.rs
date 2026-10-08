//! The command palette over the main screen with the React browser mock's
//! made-up agents and no engine, for looking at it next to the web UI
//! (`pnpm dev`, `?onboarded`, then ⌘K).
//!
//! `cargo run -p pitwall-app --example palette_demo`
//!
//! - `DEMO_SIZE=1400x900` window size (logical px)
//! - `DEMO_THEME=light|dark` (default: the system's)
//! - `DEMO_SHOW=api-fix` show that agent first
//! - `DEMO_QUERY=density` type this into the palette
//! - `DEMO_DOWN=3` move the selection down this many rows

use gpui::{
    div, prelude::*, px, size, App, AppContext, Application, Bounds, Context, Entity, IntoElement,
    Render, TitlebarOptions, Window, WindowBounds, WindowOptions,
};

use pitwall_app::agents::AgentStore;
use pitwall_app::main_screen::{self, demo, MainScreen};
use pitwall_app::palette::{self, PaletteHost};
use pitwall_app::theme::{Mode, Theme};

struct Root {
    screen: Entity<MainScreen>,
    host: Entity<PaletteHost>,
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        palette::on_actions(div(), &self.host)
            .relative()
            .size_full()
            .flex()
            .child(self.screen.clone())
            .child(self.host.clone())
    }
}

fn main() {
    let (w, h) = std::env::var("DEMO_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((1400.0_f32, 900.0_f32));
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
            pitwall_app::menu::register(cx);
            main_screen::init(cx);
            palette::init(cx);
            let store = cx.new(|_| AgentStore::new(demo::agents()));
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(w), px(h)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Pitwall palette demo".into()),
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
                });
                let host = cx.new(|cx| PaletteHost::new(Some(screen.clone()), None, window, cx));
                host.update(cx, |h, cx| {
                    h.open(window, cx);
                    if let Some(p) = h.palette().cloned() {
                        p.update(cx, |p, cx| {
                            if let Ok(q) = std::env::var("DEMO_QUERY") {
                                p.set_query(&q, cx);
                            }
                            let down = std::env::var("DEMO_DOWN")
                                .ok()
                                .and_then(|d| d.parse().ok())
                                .unwrap_or(0);
                            p.select(down, cx);
                        });
                    }
                });
                cx.new(|_| Root { screen, host })
            })
            .expect("open the demo window");
            cx.activate(true);
        });
}
