//! The explorer over a real engine and a folder you name, for looking at it
//! (design checks, screenshots) without the rest of the app:
//!
//! ```sh
//! cargo run -p pitwall-app --example explorer_demo -- <folder> [panel|viewer|search|quick] [file] [line] [query]
//! ```
//!
//! The folder's agent runs on a test engine (pitwall-core's `Harness`, no
//! holders, nothing written outside a temp dir); its change letters are
//! against `PW_DEMO_BASE` (a commit) when set.

use std::sync::Arc;

use gpui::{
    div, prelude::*, px, size, App, AppContext, Application, Bounds, Context, Entity, Render,
    Window, WindowBounds, WindowOptions,
};

use pitwall_app::agents::AgentStore;
use pitwall_app::explorer::{self, Explorer, ExplorerSource};
use pitwall_app::theme::{self, Mode, Theme, RIGHT_W, SIDEBAR_W};
use pitwall_core::testing::{record, Harness};

struct Demo {
    explorer: Entity<Explorer>,
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let e = self.explorer.clone();
        let root = div()
            .relative()
            .size_full()
            .flex()
            .bg(t.bg)
            .text_color(t.text)
            .text_size(px(13.))
            .child(
                div()
                    .w(SIDEBAR_W)
                    .h_full()
                    .flex_none()
                    .bg(t.surface)
                    .border_r_1()
                    .border_color(t.line),
            );
        let root = explorer::on_actions(root, &e);
        let root = match e.read(cx).main_view() {
            Some(v) => root.child(v),
            None => {
                let panel = e.update(cx, |e, cx| e.panel(window, cx));
                root.child(div().flex_1().h_full().bg(t.bg))
                    .when_some(panel, |r, p| {
                        r.child(
                            div()
                                .w(RIGHT_W)
                                .h_full()
                                .flex_none()
                                .border_l_1()
                                .border_color(t.line)
                                .bg(t.surface)
                                .child(p),
                        )
                    })
            }
        };
        root.child(e)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let folder = args.first().cloned().expect("a folder");
    let mode = args.get(1).cloned().unwrap_or_else(|| "panel".into());
    let file = args.get(2).cloned();
    let line: u32 = args.get(3).and_then(|l| l.parse().ok()).unwrap_or(1);
    let query = args.get(4).cloned().unwrap_or_default();

    let mut rec = record("demo", &folder);
    rec.name = std::env::var("PW_DEMO_NAME").unwrap_or_else(|_| "tests".into());
    rec.base_commit = std::env::var("PW_DEMO_BASE").ok();
    let h = Harness::new(vec![rec]);
    let engine = h.engine.clone();

    Application::new()
        .with_assets(pitwall_app::kit::Assets)
        .run(move |cx: &mut App| {
        cx.set_global(Theme::for_mode(Mode::for_appearance(
            cx.window_appearance(),
        )));
        cx.set_global(ExplorerSource::engine(engine.clone()));
        pitwall_app::kit::init(cx);
        explorer::register(cx);
        let store = cx.new(|_| AgentStore::new(engine.views()));
        let bounds = Bounds::centered(None, size(px(1400.), px(820.)), cx);
        let _keep = Arc::new(h);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                let _keep = _keep.clone();
                let explorer = cx.new(|cx| Explorer::new(store.clone(), cx));
                explorer.update(cx, |e, cx| {
                    e.set_agent(Some("demo".into()), cx);
                    match mode.as_str() {
                        "viewer" => e.open_viewer(
                            "demo",
                            file.clone(),
                            Some((line, None)),
                            None,
                            window,
                            cx,
                        ),
                        "search" => {
                            if let Some(f) = file.clone() {
                                e.open_viewer(
                                    "demo",
                                    Some(f),
                                    Some((line, None)),
                                    None,
                                    window,
                                    cx,
                                );
                            }
                            e.search_for(&query, window, cx)
                        }
                        "quick" => {
                            if let Some(f) = file.clone() {
                                e.open_viewer(
                                    "demo",
                                    Some(f),
                                    Some((line, None)),
                                    None,
                                    window,
                                    cx,
                                );
                            }
                            e.go_to_file_with(&query, window, cx)
                        }
                        _ => {}
                    }
                });
                cx.new(|cx| {
                    cx.observe(&explorer, |_, _, cx| cx.notify()).detach();
                    let _ = &_keep;
                    Demo { explorer }
                })
            },
        )
        .expect("a window");
        cx.activate(true);
    });
}
