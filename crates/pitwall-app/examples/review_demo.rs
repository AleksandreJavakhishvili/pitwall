//! The app with Review open, for looking at it on made-up agents:
//!
//! ```sh
//! PITWALL_HOME=/tmp/pw-demo cargo run -p pitwall-app --example review_demo [-- light]
//! ```
//!
//! It hosts the engine on `PITWALL_HOME` (never the default folder), opens
//! the main window and toggles Review after `PW_REVIEW_DELAY` seconds
//! (default 3). `PW_FONTS_DIR` registers the `.ttf` files of a folder
//! first (the app's own font set arrives with phase 1).

use std::time::Duration;

use gpui::{
    point, px, size, App, AppContext, Application, Bounds, SharedString, WindowBounds,
    WindowOptions,
};

use pitwall_app::agents::AgentStore;
use pitwall_app::bridge::Bridge;
use pitwall_app::host::Host;
use pitwall_app::review::{self, ReviewEngine, ToggleReview};
use pitwall_app::theme::{Mode, Theme};
use pitwall_app::ui::{Content, MainView};
use pitwall_core::paths::Paths;

fn main() {
    let home = std::env::var("PITWALL_HOME").expect("set PITWALL_HOME to a made-up folder");
    let light = std::env::args().any(|a| a == "light");
    let delay: u64 = std::env::var("PW_REVIEW_DELAY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let paths = Paths::new(&home);
    Application::new().run(move |cx: &mut App| {
        if let Ok(dir) = std::env::var("PW_FONTS_DIR") {
            let fonts: Vec<_> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "ttf"))
                .filter_map(|e| std::fs::read(e.path()).ok())
                .map(std::borrow::Cow::Owned)
                .collect();
            let _ = cx.text_system().add_fonts(fonts);
        }
        cx.set_global(Theme::for_mode(if light {
            Mode::Light
        } else {
            Mode::Dark
        }));
        pitwall_app::menu::register(cx);
        review::register(cx);
        let (bridge, rx) = Bridge::new();
        let host = Host::start(paths.clone(), bridge);
        cx.set_global(ReviewEngine {
            engine: host.engine.clone(),
            root: Some(paths.root().into()),
        });
        let store = cx.new(|_| AgentStore::new(host.engine.views()));
        AgentStore::listen(&store, rx, cx);
        let host = Box::leak(Box::new(host));
        cx.on_app_quit(move |_| {
            host.shutdown();
            async {}
        })
        .detach();
        let bounds = Bounds::new(point(px(60.), px(60.)), size(px(1440.), px(860.)));
        let content = Content::Live {
            store,
            data_dir: SharedString::from(home.clone()),
        };
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| MainView::new(content, None, window, cx)),
            )
            .expect("window");
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_secs(delay))
                .await;
            let _ = window.update(cx, |_, window, cx| {
                window.dispatch_action(Box::new(ToggleReview), cx);
                // Drawn only while visible: bring it to the front once.
                window.activate_window();
            });
        })
        .detach();
        cx.activate(true);
    });
}
