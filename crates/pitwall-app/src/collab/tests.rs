//! The collaboration screens over the daemon's service on a stand-in
//! backend (no network, no sessions): People → Ask, the Share dialog's
//! steps and summary, Save's diff with widening lines, the sidebar's
//! Shared group.

use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use serde_json::json;

use pitwall_daemon::collab::memory::{AskClient, GuestRunner, MemoryCollab};
use pitwall_daemon::collab::Me;
use pitwall_daemon::{Approvals, CollabBackend, CollabService};
use pitwall_proto::api::collab::{AskSend, CollabSettings};
use pitwall_proto::collab::{
    AskFrom, AskStatus, AskView, Capability, DirectoryEntry, Front, SessionCaller, SessionState, SessionUsage, ShareSettings, ShareState,
    SharedSessionView,
};
use pitwall_proto::{AgentView, ErrorBody};

use super::share_dialog::{ShareDialog, Step, STEPS};
use crate::agents::AgentStore;
use crate::main_screen::Route;
use crate::ui::{Content, MainView};

struct Echo;
impl AskClient for Echo {
    fn send(&self, _: &DirectoryEntry, req: &AskSend, _: Option<&str>, _: Option<&str>) -> Result<AskView, ErrorBody> {
        Ok(AskView {
            turn: "c_1/1".into(),
            conversation: "c_1".into(),
            share: req.share.clone(),
            text: req.message.clone(),
            from: AskFrom::Me,
            status: AskStatus::Done,
            status_message: None,
            reply: Some("In auth/refresh.rs.".into()),
            reply_source: None,
            changes: None,
            at: "2026-10-16T09:00:00Z".into(),
            done_at: None,
        })
    }
}

struct NoLeak;
impl GuestRunner for NoLeak {
    fn run(&self, _: &ShareSettings, _: &str) -> Result<(Option<String>, Vec<String>), ErrorBody> {
        Ok((Some("I can't do that here.".into()), vec![]))
    }
}

/// A coding agent on this Mac in `project`.
fn coder(id: &str, project: &str) -> AgentView {
    let mut a = crate::agents::tests::agent(id, project, "idle", 1, true);
    a.kind = "claude".into();
    a.kind_name = "Claude Code".into();
    a.terminal = false;
    a.name = "api".into();
    a
}

struct World {
    mem: Arc<MemoryCollab>,
    project: std::path::PathBuf,
    agents: Vec<AgentView>,
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.project);
    }
}

/// The service over a stand-in backend, with coding agents `ids` in one
/// project (a `.env` in it), on a Fake network this machine is reachable on.
fn world(cx: &mut TestAppContext, ids: &[&str]) -> World {
    world_with(cx, ids, None)
}

fn world_with(cx: &mut TestAppContext, ids: &[&str], live: Option<pitwall_daemon::collab::memory::LiveRule>) -> World {
    // One folder per world: tests with the same agents run in parallel, and
    // each removes its folder when it ends.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let project = std::env::temp_dir().join(format!("pw-app-collab-{}-{n}-{}", std::process::id(), ids.join("-")));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(project.join(".env"), "SECRET=1\n").unwrap();
    let agents: Vec<AgentView> = ids.iter().map(|id| coder(id, &project.to_string_lossy())).collect();
    let net = pitwall_core::net::fake::FakeNet::new("example.ts.net");
    let studio: Arc<dyn pitwall_core::net::Network> = Arc::new(net.device("alice@example.com", "studio"));
    let mem = Arc::new(
        MemoryCollab::new(Me { login: "alice@example.com".into(), name: "alice".into() }, None)
            .with_networks(vec![studio])
            .with_ask_client(Arc::new(Echo))
            .with_guest_runner(Arc::new(NoLeak)),
    );
    let mem = match live {
        Some(rule) => Arc::new(Arc::into_inner(mem).unwrap().with_live_rule(rule)),
        None => mem,
    };
    let mut s = CollabSettings { use_teams: true, ..Default::default() };
    s.reachable.insert("fake:example.ts.net".into(), true);
    mem.set_settings(s).unwrap();
    mem.add_directory_entry(DirectoryEntry {
        card_url: "http://x/a2a/docs".into(),
        card: json!({"name": "Docs", "description": "Questions about the handbook"}),
        owner: "carol@example.com".into(),
        slug: "docs".into(),
        capability: Capability::Read,
        kind: "claude".into(),
        left: Some(17),
        online: true,
        last_seen: None,
        via: vec!["direct".into()],
    });
    let backend: Arc<dyn CollabBackend> = mem.clone();
    let approvals = Approvals::new(Duration::from_millis(50));
    let list = agents.clone();
    let svc = CollabService::new(backend, approvals, move || list.clone());
    cx.update(|cx| {
        crate::kit::init(cx);
        crate::settings::init(cx, None, None);
        crate::palette::init(cx);
        crate::main_screen::init(cx);
        crate::ui_state::init(None, cx);
        super::init(Some(svc), cx);
    });
    World { mem, project, agents }
}

fn window(cx: &mut TestAppContext, agents: Vec<AgentView>) -> (Entity<MainView>, &mut VisualTestContext) {
    let store = cx.new(|_| AgentStore::new(agents));
    let (view, vcx) = cx.add_window_view(move |window, cx| MainView::new(Content::Live { store }, None, window, cx));
    vcx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(900.)));
    vcx.run_until_parked();
    (view, vcx)
}

fn redraw(view: &Entity<MainView>, vcx: &mut VisualTestContext) {
    let screen = view.read_with(vcx, |v, _| v.screen.clone().unwrap());
    screen.update(vcx, |_, cx| cx.notify());
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

#[gpui::test]
fn the_directory_opens_the_ask_panel_on_that_agent(cx: &mut TestAppContext) {
    let w = world(cx, &["a1"]);
    let (view, vcx) = window(cx, w.agents.clone());
    let screen = view.read_with(vcx, |v, _| v.screen.clone().unwrap());
    screen.update(vcx, |s, cx| s.set_route(Route::People, cx));
    vcx.run_until_parked();
    redraw(&view, vcx);
    let card = vcx.debug_bounds("dir-0").expect("carol's agent is listed");
    vcx.simulate_click(card.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(screen.read_with(vcx, |s, _| s.route()), Route::Ask);
    assert_eq!(vcx.update(|_, cx| super::model(cx).unwrap().read(cx).ask_target.clone()), Some("carol/docs".into()));
    // ⌘⇧P toggles People (on macOS).
    if cfg!(target_os = "macos") {
        vcx.simulate_keystrokes("cmd-shift-p");
        vcx.run_until_parked();
        assert_eq!(screen.read_with(vcx, |s, _| s.route()), Route::People);
        vcx.simulate_keystrokes("cmd-shift-a");
        vcx.run_until_parked();
        assert_eq!(screen.read_with(vcx, |s, _| s.route()), Route::Ask);
    }
}

#[gpui::test]
fn the_share_dialog_walks_seven_steps_to_a_draft(cx: &mut TestAppContext) {
    let w = world(cx, &["a1"]);
    let a = w.agents[0].clone();
    let (_view, vcx) = window(cx, w.agents.clone());
    let dialog = vcx.update(|_, cx| cx.new(|cx| ShareDialog::new(a.clone(), None, cx)));
    dialog.update(vcx, |d, cx| d.rescan(cx));
    vcx.run_until_parked();
    dialog.read_with(vcx, |d, _| {
        assert_eq!(d.sensitive.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), [".env"]);
        assert_eq!(d.draft.scope.exclude, [".env"], "found and left out by default");
        assert_eq!(d.step, Step::Who);
    });
    // Enforced on every step but the instructions.
    let soft: Vec<Step> = STEPS.into_iter().filter(|s| s.enforced() == Some(false)).collect();
    assert_eq!(soft, [Step::OffLimits]);
    dialog.update(vcx, |d, cx| {
        d.inputs.person.update(cx, |i, cx| i.set_text("bob@example.com", cx));
    });
    vcx.update(|_, cx| dialog.update(cx, |d, cx| d.inputs.off_limits.update(cx, |i, cx| i.set_text("Don't discuss pricing", cx))));
    for _ in 0..STEPS.len() {
        dialog.update(vcx, |d, cx| d.next(cx));
        vcx.run_until_parked();
    }
    let (step, summary) = dialog.read_with(vcx, |d, _| (d.step, d.summary.clone()));
    assert_eq!(step, Step::Summary);
    let text = summary.expect("the summary").text;
    assert!(text.starts_with("Nobody yet can send messages"), "{text}");
    assert!(text.contains("without 1 sensitive file (`.env`)"), "{text}");
    assert!(text.ends_with("Not guaranteed: \"Don't discuss pricing\"."), "{text}");
    assert!(w.mem.shares().is_empty(), "nothing saved before Save or Try as guest");
    // Try as guest: saved as a draft, then a probe through the pipeline.
    dialog.update(vcx, |d, cx| d.try_as_guest(cx));
    vcx.run_until_parked();
    assert_eq!(dialog.read_with(vcx, |d, _| d.step), Step::Guest);
    assert_eq!(w.mem.shares()[0].state, ShareState::Draft);
    dialog.update(vcx, |d, cx| d.probe("env-file", None, cx));
    vcx.run_until_parked();
    let probes = dialog.read_with(vcx, |d, _| d.probes.clone());
    assert_eq!((probes[0].probe.as_str(), probes[0].reply.as_deref()), ("env-file", Some("I can't do that here.")));
    // Go live from the window itself: no approval; it needs an audience.
    dialog.update(vcx, |d, cx| d.go_live(cx));
    vcx.run_until_parked();
    assert!(dialog.read_with(vcx, |d, _| d.error.clone()).unwrap().contains("add people or a team"));
    dialog.update(vcx, |d, _| d.draft.audience.people.push(pitwall_proto::collab::Identity::new("fake", "bob@example.com")));
    dialog.update(vcx, |d, cx| {
        let _ = cx;
        d.error = None;
    });
    dialog.update(vcx, |d, cx| d.go_live(cx));
    vcx.run_until_parked();
    assert_eq!(w.mem.shares()[0].state, ShareState::Live);
}

#[gpui::test]
fn saving_a_live_share_shows_the_diff_with_widening_lines(cx: &mut TestAppContext) {
    let w = world(cx, &["a1"]);
    let a = w.agents[0].clone();
    let mut live = super::share_dialog::defaults(&a, vec![".env".into()]);
    live.state = ShareState::Live;
    live.audience.people.push(pitwall_proto::collab::Identity::new("fake", "bob@example.com"));
    w.mem.save_share(&live, None).unwrap();
    let (view, vcx) = window(cx, w.agents.clone());
    let existing = vcx.update(|_, cx| super::share_of("a1", cx)).expect("read by the poll");
    let dialog = vcx.update(|window, cx| {
        let d = cx.new(|cx| ShareDialog::new(a.clone(), Some(existing), cx));
        super::show_dialog(d.clone().into(), window, cx);
        d
    });
    dialog.update(vcx, |d, cx| {
        d.draft.audience.people.push(pitwall_proto::collab::Identity::new("fake", "dave@example.com"));
        d.inputs.sessions.update(cx, |i, cx| i.set_text("1", cx));
        d.review_changes(cx);
    });
    vcx.run_until_parked();
    let diff = dialog.read_with(vcx, |d, _| (d.step, d.diff.clone().unwrap()));
    assert_eq!(diff.0, Step::Diff);
    let fields: Vec<(String, bool)> = diff.1 .0.iter().map(|l| (l.field.clone(), l.widens)).collect();
    assert_eq!(fields, [("audience.people".to_string(), true), ("limits.sessionsRunning".to_string(), false)]);
    redraw(&view, vcx);
    assert!(vcx.debug_bounds("diff-line-0-widens").is_some(), "+ dave: amber");
    assert!(vcx.debug_bounds("diff-line-1-narrows").is_some(), "fewer sessions: not amber");
    assert_eq!(w.mem.shares()[0].audience.people.len(), 1, "Save shows the diff first; nothing changed");
    // Apply: Pitwall's own window applies it directly, and the dialog closes.
    dialog.update(vcx, |d, cx| d.save(cx, |_, cx| super::close_dialog(cx)));
    vcx.run_until_parked();
    assert_eq!(w.mem.shares()[0].audience.people.len(), 2);
    assert_eq!(w.mem.shares()[0].limits.sessions_running, 1);
    assert!(vcx.update(|_, cx| super::model(cx).unwrap().read(cx).dialog.is_none()));
}

#[gpui::test]
fn the_sidebar_lists_shared_agents_and_their_callers(cx: &mut TestAppContext) {
    let w = world(cx, &["a1"]);
    let a = w.agents[0].clone();
    let mut live = super::share_dialog::defaults(&a, vec![]);
    live.state = ShareState::Live;
    w.mem.save_share(&live, None).unwrap();
    let session = |agent: &str, login: &str, state: SessionState, held: u32| SharedSessionView {
        conversation: format!("c_{agent}"),
        share: "api".into(),
        caller: SessionCaller { login: login.into(), agent: None, client: Some("Claude Code".into()), front: Front::Mcp, guest: false, pitwall: false },
        state,
        agent_id: agent.into(),
        turns: 3,
        results: 0,
        held,
        usage: SessionUsage { minutes_today: 4, messages_today: 3, tokens: None },
        last_at: None,
        flagged: false,
    };
    w.mem.add_session("api", session("s1", "erin@example.com", SessionState::Working, 0));
    w.mem.add_session("api", session("s2", "bob@example.com", SessionState::Blocked, 0));
    let (view, vcx) = window(cx, w.agents.clone());
    redraw(&view, vcx);
    assert!(vcx.debug_bounds("share-head-0").is_some(), "the shared agent's heading");
    assert!(vcx.debug_bounds("shared-row-0-0").is_some() && vcx.debug_bounds("shared-row-0-1").is_some(), "a row per caller");
    let snap = vcx.update(|_, cx| super::snapshot(cx));
    let rows = &snap.sessions["api"];
    assert_eq!(super::sidebar::caller_name(&rows[0]), "erin · Claude Code (MCP)");
    assert!(!super::sidebar::needs_you(&rows[0]) && super::sidebar::needs_you(&rows[1]));
    // The shared session's own agent is never offered Share….
    let s1 = coder("s1", "/work/api");
    assert!(vcx.update(|_, cx| !super::can_share(&s1, cx)));
}

#[gpui::test]
fn going_live_needs_a_separate_account_and_the_dialog_says_why(cx: &mut TestAppContext) {
    let rule: pitwall_daemon::collab::memory::LiveRule = Arc::new(|s: &ShareSettings| pitwall_daemon::collab::LiveCheck {
        blocks: match s.access.account.as_deref() {
            Some("claude-sharing") => vec![],
            _ => vec!["api would run on your own Claude Code login (~/.claude). …".into()],
        },
        login: Default::default(),
    });
    let w = world_with(cx, &["a1"], Some(rule));
    let a = w.agents[0].clone();
    let (view, vcx) = window(cx, w.agents.clone());
    let dialog = vcx.update(|window, cx| {
        let d = cx.new(|cx| ShareDialog::new(a.clone(), None, cx));
        super::show_dialog(d.clone().into(), window, cx);
        d
    });
    dialog.update(vcx, |d, cx| {
        d.draft.audience.people.push(pitwall_proto::collab::Identity::new("fake", "bob@example.com"));
        d.go(Step::Access, cx);
    });
    redraw(&view, vcx);
    assert!(vcx.debug_bounds("login-own-account").is_some(), "amber: guest tries only on the own login");
    // The summary lists what keeps it from going live, and Go live is refused.
    dialog.update(vcx, |d, cx| d.go(Step::Summary, cx));
    vcx.run_until_parked();
    redraw(&view, vcx);
    assert_eq!(dialog.read_with(vcx, |d, _| d.live_blocks.len()), 1);
    assert!(vcx.debug_bounds("live-block-0").is_some());
    // A guest try works on it anyway.
    dialog.update(vcx, |d, cx| d.try_as_guest(cx));
    vcx.run_until_parked();
    dialog.update(vcx, |d, cx| d.probe("env-file", None, cx));
    vcx.run_until_parked();
    assert_eq!(dialog.read_with(vcx, |d, _| d.probes.len()), 1);
    dialog.update(vcx, |d, cx| d.go_live(cx));
    vcx.run_until_parked();
    let e = dialog.read_with(vcx, |d, _| d.error.clone()).unwrap_or_default();
    assert!(e.contains("your own Claude Code login"), "{e}");
    assert_eq!(w.mem.shares()[0].state, ShareState::Draft);
    // A separate account: the warning goes, and it goes live.
    dialog.update(vcx, |d, cx| {
        d.error = None;
        d.draft.access.account = Some("claude-sharing".into());
        d.check_live(cx);
    });
    vcx.run_until_parked();
    assert!(dialog.read_with(vcx, |d, _| d.live_blocks.is_empty()));
    dialog.update(vcx, |d, cx| d.go_live(cx));
    vcx.run_until_parked();
    assert_eq!(w.mem.shares()[0].state, ShareState::Live);
}

#[test]
fn words() {
    let mut a = AskView {
        turn: "c/1".into(),
        conversation: "c".into(),
        share: "alice/api".into(),
        text: "x".into(),
        from: AskFrom::Me,
        status: AskStatus::WaitingForOwner,
        status_message: None,
        reply: None,
        reply_source: None,
        changes: None,
        at: String::new(),
        done_at: None,
    };
    assert_eq!(super::ask::status_line(&a), "waiting for alice");
    a.status_message = Some("2nd in line".into());
    assert_eq!(super::ask::status_line(&a), "2nd in line");
    assert_eq!(super::short("bob@example.com"), "bob");
    assert_eq!(super::sidebar::ago("not a time"), "");
}

#[gpui::test]
fn the_owner_sees_turns_audit_and_flags_of_shared_sessions(cx: &mut TestAppContext) {
    use pitwall_proto::api::collab::{AuditEntryView, NoticeKind, OwnerNotice, OwnerTurnView};
    let w = world(cx, &["a1"]);
    let a = w.agents[0].clone();
    let mut live = super::share_dialog::defaults(&a, vec![]);
    live.state = ShareState::Live;
    w.mem.save_share(&live, None).unwrap();
    let session = SharedSessionView {
        conversation: "c_1".into(),
        share: "api".into(),
        caller: SessionCaller { login: "bob@example.com".into(), agent: None, client: None, front: Front::A2a, guest: false, pitwall: true },
        state: SessionState::Ready,
        agent_id: "s1".into(),
        turns: 2,
        results: 0,
        held: 0,
        usage: SessionUsage { minutes_today: 1, messages_today: 2, tokens: None },
        last_at: Some("2026-10-16T09:00:05Z".into()),
        flagged: true,
    };
    w.mem.add_session("api", session);
    let turn = |n: u32, reply: Option<&str>, status: AskStatus| OwnerTurnView {
        turn: format!("c_1/{n}"),
        text: format!("question {n}"),
        delivered: Some(format!("From bob@example.com:\nquestion {n}")),
        internal: false,
        status,
        status_message: (status == AskStatus::Failed).then(|| "the agent's reply couldn't be read".into()),
        reply: reply.map(String::from),
        reply_source: reply.map(|_| pitwall_proto::collab::ReplySource::NonInteractive),
        at: "2026-10-16T09:00:00Z".into(),
        done_at: None,
    };
    w.mem.set_owner_turns("api", "c_1", vec![turn(1, Some("In docs/."), AskStatus::Done), turn(2, None, AskStatus::Failed)]);
    w.mem.add_audit(AuditEntryView {
        at: "2026-10-16T09:00:00Z".into(),
        method: "turn".into(),
        outcome: "failed:no_answer".into(),
        share: Some("api".into()),
        who: "tailscale:bob@example.com".into(),
        network: "tailscale:example.ts.net".into(),
        front: Some("a2a".into()),
        turn: Some("c_1/2".into()),
        detail: None,
    });
    w.mem.add_notice(OwnerNotice {
        id: 7,
        at: "2026-10-16T09:00:06Z".into(),
        kind: NoticeKind::NoAnswer,
        share: "api".into(),
        conversation: Some("c_1".into()),
        caller: Some("bob@example.com".into()),
        agent_id: Some("s1".into()),
        text: "api: the reply in bob@example.com's conversation couldn't be read".into(),
    });
    let (view, vcx) = window(cx, w.agents.clone());
    redraw(&view, vcx);
    // Flagged: amber and in the badge's count, like a blocked agent.
    let snap = vcx.update(|_, cx| super::snapshot(cx));
    assert!(super::sidebar::needs_you(&snap.sessions["api"][0]));
    assert_eq!(vcx.update(|_, cx| super::extra_attention(cx)), 1);
    assert_eq!(snap.notices.len(), 1);
    assert_eq!(vcx.update(|_, cx| super::model(cx).unwrap().read(cx).notified), Some(7), "seen; not told again");
    // The session pane's Messages: each turn, its reply and where it came from.
    let s1 = coder("s1", "/work/api");
    let panel = vcx.update(|_, cx| cx.new(super::messages::SessionPanel::new));
    panel.update(vcx, |p, cx| p.set_agent(&s1, cx));
    vcx.run_until_parked();
    let turns = panel.read_with(vcx, |p, _| p.turns.clone());
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].reply.as_deref(), Some("In docs/."));
    assert_eq!(super::messages::source_words(turns[0].reply_source.unwrap()), "from its non-interactive run");
    // Usage → the audit log, for this share.
    let share = snap.shares[0].clone();
    let usage = vcx.update(|window, cx| {
        let d = cx.new(|cx| super::share_dialog::UsageDialog::new(share, cx));
        super::show_dialog(d.clone().into(), window, cx);
        d
    });
    vcx.run_until_parked();
    assert_eq!(usage.read_with(vcx, |d, _| d.audit.iter().map(|a| a.outcome.clone()).collect::<Vec<_>>()), ["failed:no_answer"]);
    redraw(&view, vcx);
    assert!(vcx.debug_bounds("audit-0").is_some());
}
