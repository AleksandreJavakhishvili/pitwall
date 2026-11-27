//! Screenshots of the rest of agents' tile tools, headless (GPUI's test
//! platform, no window shown): an agent's `act` call of hello's
//! `send_message` as a card in the floating control, and the palette's
//! form for that command, dark and light. Run by hand:
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app agent_tools_finish_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{px, size, AnyWindowHandle, AppContext, HeadlessAppContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::tile_tools::TileRun;
use pitwall_proto::{ApprovalView, Requester, RequesterKind, Risk};

use crate::agents::AgentStore;
use crate::extensions::catalog;
use crate::tiles::form::Given;

#[test]
#[ignore]
fn agent_tools_finish_shots() {
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("agent-tools-finish-shots");
    let root = t.path().join("data");
    super::tests::copy_fixture(&root.join("extensions/hello"));

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
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true));
        let store = cx.new(|_| AgentStore::new(vec![crate::agents::tests::agent("api", "/work/alpha", "working", 1, true)]));
        crate::approvals::init(&store, pitwall_daemon::Approvals::new(Duration::from_secs(120)), cx);
        // The user is away from Pitwall: the floating control comes up.
        cx.set_global(crate::float::InFront(false));
        crate::float::init(&store, Arc::new(|_, _, _| Ok(())), Arc::new(|_, _| Ok(())), Arc::new(|_, _| {}), Arc::new(|_, _, _| {}), cx);
        store
    });
    let shot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str| {
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            for _ in 0..6 {
                let _ = cx.update_window(w, |_, window, _| window.refresh());
                cx.run_until_parked();
                std::thread::sleep(Duration::from_millis(20));
            }
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("agent-tools-{name}-{theme}.png"))).expect("saved");
        }
    };
    crate::kit::motion::set_on(false);

    // api's act, as `run_tile` asks it, in the store as the bridge puts it.
    let req = TileRun { agent_id: "api".into(), agent_name: "api".into(), project: Some("~/work/alpha".into()), extension: "hello".into(), tile: "main".into(), tool: "send_message".into(), kind: "act".into(), input: r#"{"text":"Release 0.9 is tagged and the notes are up"}"#.into(), ..Default::default() };
    let a = cx.update(|cx| super::agents::check_run(&req, cx)).unwrap().expect("asks");
    let now = crate::approvals::text::now_ms();
    let view = ApprovalView {
        id: "send".into(),
        action: pitwall_proto::tile_tools::approval_action("act", "hello"),
        summary: a.summary,
        details: a.details,
        requester: Requester { kind: RequesterKind::Agent, agent_id: Some("api".into()), name: "api".into(), pid: None, process: None },
        risk: Risk::High,
        rememberable: false,
        created_at: now,
        expires_at: now + 118_000,
        always: a.always,
    };
    cx.update(|cx| {
        store.update(cx, |s, cx| {
            s.approvals = vec![view];
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|cx| crate::float::bench("expand", cx));
    let preview = cx.open_window(size(px(580.), px(330.)), |window, cx| cx.new(|cx| crate::float::Preview::new(window, cx))).unwrap();
    shot(&mut cx, preview.into(), "float");
    let _ = cx.update_window(preview.into(), |_, window, _| window.remove_window());
    cx.update(|cx| {
        store.update(cx, |s, cx| {
            s.approvals.clear();
            cx.notify();
        })
    });

    // The palette's form for "Hello: Send a message", the text given and
    // a number that isn't one.
    let tool = cx.update(|cx| super::agents::tools(cx)).into_iter().find(|t| t.tool.as_deref() == Some("send_message")).expect("hello's command");
    let fields = super::form::fields(tool.input.as_ref().unwrap()).unwrap();
    let run = TileRun { by_user: true, extension: "hello".into(), tile: "main".into(), tool: "send_message".into(), kind: "act".into(), input: "{}".into(), ..Default::default() };
    let fv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let fv2 = fv.clone();
    let win = cx
        .open_window(size(px(620.), px(420.)), move |window, cx| {
            let f = cx.new(|cx| crate::main_screen::tool_form::ToolForm::new(run, "Hello: Send a message".into(), fields, window, cx));
            *fv2.borrow_mut() = Some(f.clone());
            cx.new(|_| super::tests::Shell(f.into(), false))
        })
        .unwrap();
    let form: gpui::Entity<crate::main_screen::tool_form::ToolForm> = fv.borrow().clone().unwrap();
    cx.update(|cx| {
        form.update(cx, |f, cx| {
            f.set(0, Given::Text("Release 0.9 is tagged".into()), cx);
            f.set(1, Given::Text("soon".into()), cx);
            f.run(cx);
        })
    });
    shot(&mut cx, win.into(), "form");
}
