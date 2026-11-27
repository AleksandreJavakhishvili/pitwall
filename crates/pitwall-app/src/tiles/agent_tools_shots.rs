//! Screenshots of agents' `read` and `act` calls, headless (GPUI's test
//! platform, no window shown): the approval for hello's `send_message`
//! and `current_messages` with "Always for api", the hello tile with Undo
//! beside its mark, and the card's "Allowed without asking", dark and
//! light. Run by hand:
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app agent_tools_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{div, prelude::*, px, size, AnyWindowHandle, AppContext, Context, Entity, HeadlessAppContext, Render, Window};
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;
use pitwall_proto::tile_tools::{TileRun, TileShow};
use pitwall_proto::{ApprovalView, Requester, RequesterKind, Risk};

use crate::agents::AgentStore;
use crate::extensions::catalog;
use crate::settings::view::SettingsView;

const WAIT: Duration = Duration::from_secs(120);

/// A window that draws the approval overlay over a plain surface.
struct Over;
impl Render for Over {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).clone();
        div().size_full().bg(t.bg).children(crate::approvals::overlay(window, cx))
    }
}

#[test]
#[ignore]
fn agent_tools_shots() {
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("agent-tools-shots");
    let root = t.path().join("data");
    super::tests::copy_fixture(&root.join("extensions/hello"));
    let _ = Path::new(env!("CARGO_MANIFEST_DIR"));

    let text: Arc<dyn gpui::PlatformTextSystem> = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(crate::kit::Assets), gpui_platform::current_headless_renderer);
    cx.allow_parking();
    let r2 = root.clone();
    let store = cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(r2), cx);
        catalog::set_bundle(None, cx);
        cx.set_global(super::DataDir(root.clone()));
        crate::settings::data::set_page(cx, Page::Extensions);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true));
        let store = cx.new(|_| AgentStore::new(vec![]));
        crate::approvals::init(&store, pitwall_daemon::Approvals::new(Duration::from_secs(120)), cx);
        store
    });
    let settle = |cx: &mut HeadlessAppContext, what: &str, done: &mut dyn FnMut(&mut gpui::App) -> bool| {
        let end = Instant::now() + WAIT;
        loop {
            cx.run_until_parked();
            if cx.update(|cx| done(cx)) {
                return;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let shot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str| {
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            cx.run_until_parked();
            let _ = cx.update_window(w, |_, window, _| window.refresh());
            cx.run_until_parked();
            // Past the dialog's fade-in.
            for _ in 0..25 {
                let _ = cx.update_window(w, |_, window, _| window.refresh());
                cx.run_until_parked();
                std::thread::sleep(Duration::from_millis(20));
            }
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("agent-tools-{name}-{theme}.png"))).expect("saved");
        }
    };

    // The approvals, as `run_tile` asks them.
    let ask = |cx: &mut HeadlessAppContext, store: &Entity<AgentStore>, tool: &str, kind: &str, input: &str| {
        let req = TileRun { agent_id: "api".into(), agent_name: "api".into(), project: Some("~/work/alpha".into()), extension: "hello".into(), tile: "main".into(), tool: tool.into(), kind: kind.into(), input: input.into(), ..Default::default() };
        let a = cx.update(|cx| super::agents::check_run(&req, cx)).unwrap().expect("asks");
        let view = ApprovalView {
            id: tool.into(),
            action: pitwall_proto::tile_tools::approval_action(kind, "hello"),
            summary: a.summary,
            details: a.details,
            requester: Requester { kind: RequesterKind::Agent, agent_id: Some("api".into()), name: "api".into(), pid: None, process: None },
            risk: if kind == "read" { Risk::Low } else { Risk::High },
            rememberable: false,
            created_at: crate::approvals::text::now_ms(),
            expires_at: crate::approvals::text::now_ms() + 118_000,
            always: a.always,
        };
        cx.update(|cx| store.update(cx, |s, cx| {
            s.approvals = vec![view];
            cx.notify();
        }));
    };
    let win = cx.open_window(size(px(640.), px(460.)), |_, cx| cx.new(|_| Over)).unwrap();
    ask(&mut cx, &store, "send_message", "act", r#"{"text":"Release 0.9 is tagged and the notes are up"}"#);
    shot(&mut cx, win.into(), "approval-act");
    ask(&mut cx, &store, "current_messages", "read", "{}");
    shot(&mut cx, win.into(), "approval-read");
    cx.update(|cx| store.update(cx, |s, cx| {
        s.approvals.clear();
        cx.notify();
    }));
    let _ = cx.update_window(win.into(), |_, window, _| window.remove_window());

    // Hello with api's presented message: the mark and Undo.
    let vt = std::rc::Rc::new(std::cell::RefCell::new(None));
    let vt2 = vt.clone();
    let win = cx
        .open_window(size(px(520.), px(380.)), move |_, cx| {
            let v = crate::main_screen::view_tile::view("hello/main", cx);
            *vt2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), true))
        })
        .unwrap();
    let vtile: Entity<crate::main_screen::view_tile::ViewTile> = vt.borrow().clone().unwrap();
    settle(&mut cx, "the tile", &mut |cx| vtile.read(cx).tile().is_some());
    let tile = cx.update(|cx| vtile.read(cx).tile().cloned()).unwrap();
    settle(&mut cx, "hello", &mut |cx| tile.read(cx).texts().iter().any(|t| t.contains("Clicked 0 times")));
    let req = TileShow { agent_id: "api".into(), agent_name: "api".into(), extension: "hello".into(), tile: "main".into(), tool: Some("show_message".into()), item: Some("release".into()), input: Some(r#"{"message":"Release 0.9 is tagged"}"#.into()), ..Default::default() };
    let placed = cx.update(|cx| super::agents::show(&req, cx)).unwrap().unwrap_or_else(|_| panic!("placed"));
    let (ext, call) = (placed.extension.clone(), placed.call);
    let end = Instant::now() + WAIT;
    while placed.answer.try_recv().is_err() {
        assert!(Instant::now() < end, "no answer");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
    cx.update(|cx| super::agents::finished(&ext, "main", call, &req, &Ok(()), cx));
    settle(&mut cx, "the message", &mut |cx| tile.read(cx).texts().iter().any(|t| t.contains("api: Release 0.9")));
    shot(&mut cx, win.into(), "undo");
    let _ = cx.update_window(win.into(), |_, window, _| window.remove_window());

    // The card, with a read allowed always for api.
    cx.update(|cx| super::agents::grant("hello", "api", "api", "current_messages", cx));
    let sv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sv2 = sv.clone();
    let shell = cx
        .open_window(size(px(900.), px(1100.)), move |_, cx| {
            let v = cx.new(|cx| SettingsView::new(true, cx));
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), false))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    cx.run_until_parked();
    let dy = std::env::var("PITWALL_SHOTS_CARD_DY").ok().and_then(|v| v.parse().ok()).unwrap_or(640.);
    cx.update(|cx| settings.read(cx).scroll_by(px(dy)));
    let _ = cx.update_window(shell.into(), |_, window, _| window.refresh());
    cx.run_until_parked();
    shot(&mut cx, shell.into(), "card");
}
