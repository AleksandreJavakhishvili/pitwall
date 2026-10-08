//! The Wall with the React app's demo data (`src/mock.ts`), no engine: for
//! side-by-side screenshots against the React Wall.
//!
//! `cargo run -p pitwall-app --example wall_preview -- [--width 1132]
//! [--height 790] [--app-width 1400] [--dark]`
//!
//! The fixture holds the demo agents and the screen frame the React Wall
//! got for each (`watch_screen`); a watch here sends that frame.

use std::sync::Arc;

use gpui::{px, size, App, AppContext, Application, Bounds, WindowBounds, WindowOptions};
use serde::Deserialize;

use pitwall_app::agents::AgentStore;
use pitwall_app::theme::Theme;
use pitwall_app::wall::source::{ScreenSource, Screens};
use pitwall_app::wall::{self, WallView};
use pitwall_core::term::FrameSink;
use pitwall_proto::{AgentView, ScreenFrame};

#[derive(Deserialize)]
struct Fixture {
    agents: Vec<AgentView>,
    frames: std::collections::HashMap<String, ScreenFrame>,
}

struct Demo(std::collections::HashMap<String, ScreenFrame>);

impl ScreenSource for Demo {
    fn watch(&self, agent_id: &str, mut sink: FrameSink) -> Result<u64, String> {
        let f = self.0.get(agent_id).ok_or("agent is not running")?;
        sink(f);
        Ok(1)
    }
    fn unwatch(&self, _: &str, _: u64) {}
}

fn arg(name: &str) -> Option<f32> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1))
        .and_then(|v| v.parse().ok())
}

fn main() {
    let fx: Fixture = serde_json::from_str(include_str!("fixtures/wall_demo.json"))
        .expect("the demo fixture parses");
    let frames = fx.frames;
    let (w, h) = (
        arg("--width").unwrap_or(1132.),
        arg("--height").unwrap_or(790.),
    );
    let app_width = arg("--app-width");
    let dark = std::env::args().any(|a| a == "--dark");
    Application::new().run(move |cx: &mut App| {
        cx.set_global(if dark { Theme::dark() } else { Theme::light() });
        cx.set_global(Screens(Arc::new(Demo(frames))));
        cx.bind_keys(wall::bindings());
        let store = cx.new(|_| AgentStore::new(fx.agents));
        let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let mut wall = WallView::new(store, window, cx);
                    wall.set_app_width(app_width, cx);
                    wall
                })
            },
        )
        .expect("open the preview window");
        cx.activate(true);
    });
}
