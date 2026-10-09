//! A look at the OS-integration views with made-up data: the approval
//! dialog over the sessions-on-other-machines panel, and the Dock badge.
//! No engine, no data folder, no real agw: everything is invented here.
//!
//! `cargo run -p pitwall-app --example os_preview [light] [menu]`
//! (quits by itself after `PREVIEW_SECS`, default 20).

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, size, App, Application, Bounds, Context, Entity, IntoElement,
    Render, TitlebarOptions, Window, WindowBounds, WindowOptions,
};

use pitwall_app::agents::AgentStore;
use pitwall_app::approvals;
use pitwall_app::bridge::{AppEvent, Bridge};
use pitwall_app::platform::{self, app_menu::AppMenu};
use pitwall_app::remote::{MachineSessions, Source};
use pitwall_app::theme::{self, Mode, Theme};
use pitwall_daemon::approvals::Ask;
use pitwall_daemon::Approvals;
use pitwall_proto::{Requester, RequesterKind, Risk, ScannedMachine, ScannedPlace, ScannedSession};

fn session(
    native: &str,
    kind: &str,
    status: &str,
    user: Option<&str>,
    in_pitwall: bool,
) -> ScannedSession {
    ScannedSession {
        provider: "agw".into(),
        machine: "vm-1".into(),
        native: native.into(),
        name: native.into(),
        kind: kind.to_lowercase(),
        kind_name: kind.into(),
        program: kind.to_lowercase(),
        workspace: Some("pit-lane".into()),
        user: user.map(Into::into),
        cwd: None,
        status: status.into(),
        in_pitwall,
    }
}

fn places() -> Vec<ScannedPlace> {
    vec![ScannedPlace {
        provider: "agw".into(),
        label: "agw".into(),
        version: Some("1.4".into()),
        machines: Some(vec![
            ScannedMachine {
                id: "vm-1".into(),
                label: "vm-1".into(),
                detail: Some("site-a".into()),
                sessions: vec![
                    session("telemetry", "Claude", "running", None, false),
                    session("strategy", "Codex", "running", Some("builder"), true),
                    session("tyre-model", "Claude", "stopped", None, false),
                ],
            },
            ScannedMachine {
                id: "vm-2".into(),
                label: "vm-2".into(),
                detail: None,
                sessions: vec![],
            },
        ]),
    }]
}

struct Preview {
    panel: Entity<MachineSessions>,
    menu: Option<Entity<AppMenu>>,
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            // The app's body text (`MainView`'s root).
            .font_family(pitwall_app::kit::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(pitwall_app::kit::BODY_LINE_HEIGHT))
            .child(
                div()
                    .h(px(36.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(t.line)
                    .children(self.menu.clone())
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.text_3)
                            .child("Sessions on other machines"),
                    ),
            )
            .child(div().p_4().w(px(620.)).child(self.panel.clone()))
            .children(approvals::overlay(window, cx))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let light = args.iter().any(|a| a == "light");
    let with_menu = args.iter().any(|a| a == "menu");
    let secs: u64 = std::env::var("PREVIEW_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let no_dialog = args.iter().any(|a| a == "panel");
    Application::new()
        .with_assets(pitwall_app::kit::Assets)
        .run(move |cx: &mut App| {
        cx.set_global(Theme::for_mode(if light { Mode::Light } else { Mode::Dark }));
        pitwall_app::kit::init(cx);
        pitwall_app::menu::register(cx);
        platform::init(cx);
        cx.on_action(|_: &pitwall_app::menu::Quit, cx| cx.quit());

        let store = cx.new(|_| AgentStore::new(vec![]));
        let (bridge, rx) = Bridge::new();
        AgentStore::listen(&store, rx, cx);
        let pending = Approvals::new(Duration::from_secs(120));
        pending.on_change(move |l| bridge.send(AppEvent::Approvals(l)));
        approvals::init(&store, pending.clone(), cx);
        platform::follow(&store, cx);
        platform::set_badge(3, cx);

        // The React mock's sample request (`?approval`), and with `two` a
        // second one waiting behind it.
        let two = args.iter().any(|a| a == "two");
        let asks = [
            ("Race Engineer", "start the session \"work\" on vm-1 (agw)"),
            ("Strategist", "start the session \"tyre-model\" on vm-1 (agw)"),
        ];
        let n = if no_dialog { 0 } else if two { 2 } else { 1 };
        for (name, summary) in asks.into_iter().take(n) {
            let asking = pending.clone();
            std::thread::spawn(move || {
                asking.ask(Ask {
                    action: "session.add".into(),
                    summary: summary.into(),
                    details: vec![
                        "It is stopped there. Starting it runs it on that machine, where it keeps running after Pitwall quits.".into(),
                        "It has already been added to Pitwall (that needs no approval).".into(),
                    ],
                    requester: Requester {
                        kind: RequesterKind::Agent,
                        agent_id: Some("a1".into()),
                        name: name.into(),
                        pid: Some(4242),
                        process: Some("pitwall".into()),
                    },
                    caller_key: name.into(),
                    risk: Risk::Low,
                })
            });
            std::thread::sleep(Duration::from_millis(30));
        }

        let source = Source {
            places: Arc::new(places),
            add: Arc::new(|_| Ok(())),
        };
        let bounds = Bounds::new(point(px(80.), px(80.)), size(px(900.), px(620.)));
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Pitwall preview".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| {
                let panel = cx.new(|cx| MachineSessions::new(source, cx));
                let menu = with_menu.then(|| cx.new(|_| AppMenu::new()));
                cx.new(|_| Preview { panel, menu })
            },
        )
        .expect("open the preview window");
        cx.activate(true);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(secs)).await;
            let _ = cx.update(|cx| {
                platform::set_badge(0, cx);
                cx.quit()
            });
        })
        .detach();
    });
}
