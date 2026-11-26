use super::*;
use gpui::TestAppContext;
use pitwall_core::accounts::Accounts;
use pitwall_core::extensions::{Adapter, Found, MatchAgents, SettingDef};
use pitwall_core::paths::Paths;
use pitwall_core::testing::TempDir;
use serde_json::json;

fn manifest() -> Manifest {
    let setting = |key: &str, kind| SettingDef { required: true, ..SettingDef::new(key, kind, &key.to_uppercase()) };
    Manifest {
        settings: vec![setting("server", SettingKind::Url), setting("apiKey", SettingKind::Secret)],
        adapter: Some(Adapter { command: vec!["node".into(), "a.mjs".into()] }),
        match_agents: Some(MatchAgents { agent: "{project}".into(), session: "{agent_id}".into() }),
        ..Manifest::named("inbox", "Inbox")
    }
}

/// The hub with extension "inbox" found, accounts kept in `t`.
fn setup(cx: &mut TestAppContext, t: &TempDir) -> Entity<Hub> {
    let paths = Paths::new(t.path().join("pw"));
    cx.update(|cx| {
        crate::accounts::model(cx).update(cx, |m, _| m.set_paths(paths));
        crate::extensions::set_found(vec![Found { folder: "inbox".into(), manifest: Ok(manifest()) }], cx);
        crate::extensions::follow(cx);
        hub(cx)
    })
}

fn run(h: &Entity<Hub>, cx: &mut TestAppContext) -> Run {
    cx.run_until_parked();
    h.read_with(cx, |h, _| h.get("inbox").map(|s| s.run.clone()).unwrap_or_default())
}

fn sent(h: &Entity<Hub>, end: &str, cx: &mut TestAppContext) -> Vec<serde_json::Value> {
    h.read_with(cx, |h, _| h.ends.get(end).map(|e| e.sent()).unwrap_or_default())
}

fn say(h: &Entity<Hub>, end: &str, line: serde_json::Value, cx: &mut TestAppContext) {
    h.read_with(cx, |h, _| h.ends[end].say(line));
    cx.run_until_parked();
}

fn turn_on(cx: &mut TestAppContext) {
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("inbox", true)));
}

/// A connection called `name` with its server set and its key in the
/// secure store.
fn connection(name: &str, server: &str, cx: &mut TestAppContext) -> String {
    cx.update(|cx| {
        let m = crate::accounts::model(cx);
        let c = m.update(cx, |m, cx| m.add_connection("inbox", name, cx)).unwrap();
        m.update(cx, |m, cx| m.set_connection_value(&c.id, "server", Some(server), cx)).unwrap();
        let store = crate::extensions::secret_store(cx);
        store.set(&crate::extensions::account(&c.id, "apiKey", cx), &format!("key-{name}")).unwrap();
        m.update(cx, |m, cx| m.set_connection_secret(&c.id, "apiKey", true, cx)).unwrap();
        c.id
    })
}

/// Off until on; on but unset asks for its settings; set (through
/// Settings' connection), it starts with them (the secret from the secure
/// store, only in hello); its list, counts and an answer's round trip; a
/// stop shows why, and Restart starts it again.
#[gpui::test]
fn an_adapter_follows_its_switch_and_settings(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-switch");
    let h = setup(cx, &t);
    assert_eq!(run(&h, cx), Run::Off);
    turn_on(cx);
    assert!(matches!(run(&h, cx), Run::Setup(why) if why.contains("SERVER and APIKEY")));
    cx.update(|cx| {
        let m = manifest();
        crate::extensions::connections::set_setting_value(&m.id, "server", Some("https://inbox.example.com".into()), cx).unwrap();
        crate::extensions::set_secret("inbox", "apiKey", "s3cret".into(), cx, |err, _| assert!(err.is_none()));
    });
    assert_eq!(run(&h, cx), Run::Starting);
    let (conn, ui, list) = cx.update(|cx| (crate::extensions::connections::settings_target("inbox", cx).unwrap(), crate::ui_state::get(cx), crate::accounts::list(cx).cloned().unwrap()));
    assert_eq!((conn.name.as_str(), conn.secrets_set.as_slice()), ("Inbox", ["apiKey".to_string()].as_slice()), "one named after it");
    assert_eq!(list.default_connection("inbox").map(|c| c.id.clone()), Some(conn.id.clone()));
    assert!(!serde_json::to_string(&ui).unwrap().contains("s3cret"), "the secret is never in ui.json");
    assert!(!std::fs::read_to_string(t.path().join("pw/accounts.json")).unwrap().contains("s3cret"), "nor in accounts.json");
    let end = format!("inbox/{}", conn.id);
    let hello = sent(&h, &end, cx);
    assert_eq!(hello[0]["settings"], json!({"server": "https://inbox.example.com"}));
    assert_eq!(hello[0]["secrets"], json!({"apiKey": "s3cret"}));

    // Nothing counts before its hello.
    say(&h, &end, json!({"type": "threads", "threads": [{"id": "early"}]}), cx);
    assert!(h.read_with(cx, |h, _| h.get("inbox").unwrap().threads.is_empty()));
    say(&h, &end, json!({"type": "hello", "protocol": 1}), cx);
    assert_eq!(run(&h, cx), Run::Running);
    say(&h, &end, json!({"type": "threads", "threads": [{"id": "t1", "unread": 2, "openAsks": 1, "blocking": true}, {"id": "t2"}]}), cx);
    let t1 = format!("{}:t1", conn.id);
    h.read_with(cx, |h, _| {
        let s = h.get("inbox").unwrap();
        assert_eq!(s.threads.iter().map(|t| t.id.clone()).collect::<Vec<_>>(), [t1.clone(), format!("{}:t2", conn.id)], "ids carry their connection");
        assert_eq!(s.label(&t1), Some("Inbox"));
        let c = s.counts();
        assert_eq!((c.unread, c.open_asks, c.blocking), (1, 1, 1), "counted from the list");
    });
    say(&h, &end, json!({"type": "counts", "unread": 9, "openAsks": 4, "blocking": 2}), cx);
    assert_eq!(h.read_with(cx, |h, _| h.get("inbox").unwrap().counts().unread), 9, "its totals win");

    // Open, answer, result: the adapter sees its own ids.
    h.update(cx, |h, cx| h.open("inbox", &t1, cx));
    assert_eq!(sent(&h, &end, cx), [json!({"type": "open", "thread": "t1"})]);
    let bad = h.update(cx, |h, cx| h.request("inbox", &t1, Request::Answer { ask: 1, picks: vec![], text: Some("  ".into()), declined: false }, cx));
    assert!(bad.is_err(), "an empty answer isn't sent");
    let req = h.update(cx, |h, cx| h.request("inbox", &t1, Request::Answer { ask: 1, picks: vec![0], text: None, declined: false }, cx)).unwrap();
    assert_eq!(sent(&h, &end, cx)[0], json!({"type": "answer", "req": req, "thread": "t1", "ask": 1, "picks": [0], "declined": false}));
    assert!(h.read_with(cx, |h, _| h.get("inbox").unwrap().busy.contains(&t1)));
    say(&h, &end, json!({"type": "result", "req": req, "ok": false, "message": "That question isn't open any more."}), cx);
    h.read_with(cx, |h, _| {
        let s = h.get("inbox").unwrap();
        assert!(!s.busy.contains(&t1));
        assert_eq!(s.notes[&t1], "That question isn't open any more.");
        assert_eq!(s.result(req), Some(false));
    });
    say(&h, &end, json!({"type": "thread", "thread": {"id": "t1", "messages": [{"id": "1", "text": "hi"}]}}), cx);
    say(&h, &end, json!({"type": "thread", "thread": {"id": "not-open", "messages": []}}), cx);
    h.read_with(cx, |h, _| {
        let s = h.get("inbox").unwrap();
        assert_eq!(s.details.get(&t1).map(|d| d.id.clone()), Some(t1.clone()));
        assert!(!s.details.keys().any(|k| k.ends_with("not-open")), "only open threads are kept");
    });

    // It stops: why is kept until Restart.
    h.read_with(cx, |h, _| {
        let _ = h.ends[&end].events.unbounded_send(Event::Stopped("It exited with code 3.".into()));
    });
    assert_eq!(run(&h, cx), Run::Stopped("It exited with code 3.".into()));
    assert!(h.update(cx, |h, cx| h.request("inbox", &t1, Request::MarkRead, cx)).is_err());
    h.update(cx, |h, cx| h.restart("inbox", cx));
    assert_eq!(run(&h, cx), Run::Starting);
    say(&h, &end, json!({"type": "hello", "protocol": 1}), cx);
    assert_eq!(sent(&h, &end, cx).last(), Some(&json!({"type": "open", "thread": "t1"})), "open threads are opened again");

    // A wrong protocol stops it, saying what to do.
    h.update(cx, |h, cx| h.restart("inbox", cx));
    cx.run_until_parked();
    say(&h, &end, json!({"type": "hello", "protocol": 3}), cx);
    assert!(matches!(run(&h, cx), Run::Stopped(why) if why.contains("Update the extension or Pitwall")));

    // Off: stopped.
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("inbox", false)));
    assert_eq!(run(&h, cx), Run::Off);
}

/// Two connections: two adapters, each with its own settings and secret;
/// one inbox with both lists, labelled; counts added up; requests go to
/// the right adapter; one offline shows as a note without hiding the
/// other's threads; a removed connection's adapter stops.
#[gpui::test]
fn each_connection_runs_its_own_adapter_into_one_inbox(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-two");
    let h = setup(cx, &t);
    turn_on(cx);
    let work = connection("Work", "https://work.example", cx);
    let home = connection("Home", "https://home.example", cx);
    cx.run_until_parked();
    let (ew, eh) = (format!("inbox/{work}"), format!("inbox/{home}"));
    let (hw, hh) = (sent(&h, &ew, cx), sent(&h, &eh, cx));
    assert_eq!(hw.len(), 1, "started once");
    assert_eq!(hw[0]["settings"], json!({"server": "https://work.example"}));
    assert_eq!(hw[0]["secrets"], json!({"apiKey": "key-Work"}));
    assert_eq!(hh[0]["settings"], json!({"server": "https://home.example"}));
    assert_eq!(hh[0]["secrets"], json!({"apiKey": "key-Home"}));
    for e in [&ew, &eh] {
        say(&h, e, json!({"type": "hello", "protocol": 1}), cx);
    }
    // The same adapter id in both: two threads.
    say(&h, &ew, json!({"type": "threads", "threads": [{"id": "t1", "openAsks": 1, "unread": 1}]}), cx);
    say(&h, &eh, json!({"type": "threads", "threads": [{"id": "t1", "openAsks": 2, "blocking": true}]}), cx);
    let (tw, th) = (format!("{work}:t1"), format!("{home}:t1"));
    h.read_with(cx, |h, _| {
        let s = h.get("inbox").unwrap();
        assert_eq!(s.run, Run::Running);
        assert_eq!(s.threads.len(), 2);
        assert_eq!((s.label(&tw), s.label(&th)), (Some("Work"), Some("Home")));
        let c = s.counts();
        assert_eq!((c.unread, c.open_asks, c.blocking), (1, 3, 1), "added up");
        assert_eq!(s.connections.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Work", "Home"]);
        // What a per-connection filter needs: each thread's connection,
        // and counts per connection as well as in total.
        assert_eq!(s.connection_of(&th), Some((home.as_str(), "Home")));
        assert_eq!(s.threads_of(Some(&work)).map(|t| t.id.clone()).collect::<Vec<_>>(), std::slice::from_ref(&tw));
        assert_eq!(s.threads_of(None).count(), 2);
        assert_eq!((s.counts_of(Some(&home)).open_asks, s.counts_of(Some(&work)).open_asks, s.counts_of(None).open_asks), (2, 1, 3));
    });
    // Its totals and the other's list add up too.
    say(&h, &ew, json!({"type": "counts", "unread": 5, "openAsks": 7, "blocking": 0}), cx);
    assert_eq!(h.read_with(cx, |h, _| h.get("inbox").unwrap().counts().open_asks), 9);

    // Requests go to the thread's own adapter.
    let req = h.update(cx, |h, cx| h.request("inbox", &th, Request::MarkRead, cx)).unwrap();
    assert_eq!(sent(&h, &eh, cx), [json!({"type": "markRead", "req": req, "thread": "t1"})]);
    assert!(sent(&h, &ew, cx).is_empty());
    say(&h, &eh, json!({"type": "result", "req": req, "ok": true}), cx);
    assert_eq!(h.read_with(cx, |h, _| h.get("inbox").unwrap().result(req)), Some(true));

    // Home can't reach its server; then it stops: Work's threads stay, a
    // note names Home.
    say(&h, &eh, json!({"type": "status", "state": "error", "message": "No route to the server."}), cx);
    h.read_with(cx, |h, _| {
        let s = h.get("inbox").unwrap();
        assert_eq!(s.run, Run::Running);
        assert_eq!(s.conn, Some((ConnState::Error, "Home can't connect: No route to the server.".into())));
        assert!(s.connections[1].offline() && !s.connections[0].offline());
    });
    h.read_with(cx, |h, _| {
        let _ = h.ends[&eh].events.unbounded_send(Event::Stopped("It exited with code 1.".into()));
    });
    cx.run_until_parked();
    h.read_with(cx, |h, _| {
        let s = h.get("inbox").unwrap();
        assert_eq!(s.run, Run::Running, "Work still runs");
        assert!(s.threads.iter().any(|t| t.id == tw));
        assert_eq!(s.conn, Some((ConnState::Error, "Home stopped: It exited with code 1.".into())));
    });

    // Home removed: its adapter is gone, Work's goes on untouched.
    let before = sent(&h, &ew, cx).len();
    cx.update(|cx| crate::extensions::connections::remove(&home, cx, |e, _| assert!(e.is_none())));
    cx.run_until_parked();
    h.read_with(cx, |h, _| {
        assert!(h.connection("inbox", &home).is_none());
        let s = h.get("inbox").unwrap();
        assert_eq!(s.connections.len(), 1);
        assert_eq!(s.conn, None);
    });
    assert_eq!(sent(&h, &ew, cx).len(), before, "not started again");
    let store = cx.update(crate::extensions::secret_store);
    let gone = cx.update(|cx| crate::extensions::account(&home, "apiKey", cx));
    assert_eq!(store.get(&gone).unwrap(), None, "its secret is deleted");
}

/// At most [`MAX_RUNNING`] adapters per extension; the rest say why.
#[gpui::test]
fn the_adapters_of_an_extension_are_bounded(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-bound");
    let h = setup(cx, &t);
    turn_on(cx);
    let ids: Vec<String> = (0..MAX_RUNNING + 1).map(|i| connection(&format!("C{i}"), "https://x.example", cx)).collect();
    cx.run_until_parked();
    h.read_with(cx, |h, _| {
        let running = ids.iter().filter(|id| h.connection("inbox", id).is_some_and(|s| s.run == Run::Starting)).count();
        assert_eq!(running, MAX_RUNNING);
        let last = h.connection("inbox", ids.last().unwrap()).unwrap();
        assert!(matches!(&last.run, Run::Setup(why) if why.contains("At most 8")));
    });
}

/// Threads match only agents whose project's connection they are from;
/// a project that picks none never matches.
#[gpui::test]
fn threads_match_agents_of_their_connection(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-match");
    let _h = setup(cx, &t);
    let work = connection("Work", "https://work.example", cx);
    let home = connection("Home", "https://home.example", cx);
    cx.update(|cx| {
        let m = crate::accounts::model(cx);
        m.update(cx, |m, cx| m.set_project_pick("/work/side", "ext:inbox", Some(&home), cx)).unwrap();
        m.update(cx, |m, cx| m.set_project_pick("/work/none", "ext:inbox", Some("none"), cx)).unwrap();
    });
    let m = manifest();
    let alpha = crate::agents::tests::agent("web", "/work/alpha", "idle", 1, true);
    let side = crate::agents::tests::agent("api", "/work/side", "idle", 1, true);
    let none = crate::agents::tests::agent("doc", "/work/none", "idle", 1, true);
    let from = |a: &pitwall_proto::AgentView| pitwall_core::extensions::protocol::From {
        agent: pitwall_core::extensions::name_form(a.project.rsplit('/').next().unwrap()),
        session: pitwall_core::extensions::name_form(&a.id),
    };
    cx.update(|cx| {
        let list: Option<&Accounts> = crate::accounts::list(cx);
        let is = |a, conn: &str| crate::extensions::is_agents(&m, a, &thread_id(conn, "t"), &from(a), list);
        assert!(is(&alpha, &work), "the default connection");
        assert!(!is(&alpha, &home));
        assert!(is(&side, &home) && !is(&side, &work), "its own pick");
        assert!(!is(&none, &work) && !is(&none, &home), "none: never");
    });
}

/// An extension set up before connections, as `ui.json` holds it today
/// (on, a server address, its key in the keychain under the extension's
/// id, the thread shown, two setup actions done: one using a setting, one
/// not), becomes one connection on start: the default, found for every
/// project, its secret read from the same keychain item, both setup steps
/// still ticked. `ui.json` keeps a copy for an older Pitwall; what such a
/// Pitwall changes there after a downgrade comes back on the next start,
/// and nothing is lost either way.
#[gpui::test]
fn an_extension_set_up_before_connections_carries_over(cx: &mut TestAppContext) {
    use pitwall_core::extensions::{Action, Step as RunStep};
    let t = TempDir::new("hub-migrate");
    let h = setup(cx, &t);
    let action = |id: &str, run: &[&str]| Action { id: id.into(), label: id.into(), description: String::new(), setup: true, steps: vec![RunStep { run: run.iter().map(|x| x.to_string()).collect(), for_kind: None, optional: false }] };
    let m = Manifest { actions: vec![action("setup", &["tool", "add", "{setting:server}"]), action("tidy", &["tool", "tidy"])], ..manifest() };
    // Exactly what main writes in ui.json today.
    let today = json!({"v": 1, "extensions": {"inbox": {"enabled": true, "values": {"server": "https://old.example"}, "secretsSet": ["apiKey"], "selected": "t9", "actionsDone": ["setup", "tidy"], "window": true}}});
    let item = cx.update(|cx| crate::extensions::account("inbox", "apiKey", cx));
    cx.update(|cx| {
        crate::extensions::set_found(vec![Found { folder: "inbox".into(), manifest: Ok(m.clone()) }], cx);
        crate::extensions::secret_store(cx).set(&item, "old-key").unwrap();
        let ui = crate::main_screen::workspace::UiState::sanitize(today.clone());
        crate::ui_state::update(cx, |u| *u = ui);
        crate::extensions::connections::migrate(cx);
    });
    cx.run_until_parked();
    let (list, ui) = cx.update(|cx| (crate::accounts::list(cx).cloned().unwrap(), crate::ui_state::get(cx)));
    let c = list.default_connection("inbox").cloned().unwrap();
    assert_eq!((c.name.as_str(), c.secrets_name(), c.secrets_set.as_slice()), ("Inbox", "inbox", ["apiKey".to_string()].as_slice()));
    assert_eq!(c.values["server"], "https://old.example");
    assert_eq!(c.actions_done, ["setup"], "the step that used its server is the connection's");
    assert_eq!(list.connection_for("inbox", "/any/project", "/any/project").map(|x| x.id.clone()), Some(c.id.clone()), "found for every project");
    // Its adapter starts with the old keychain item, read as it was.
    let hello = sent(&h, &format!("inbox/{}", c.id), cx);
    assert_eq!(hello[0]["secrets"], json!({"apiKey": "old-key"}));
    assert_eq!(hello[0]["settings"], json!({"server": "https://old.example"}));
    assert_eq!(cx.update(|cx| crate::extensions::secret_store(cx).get(&item)).unwrap().as_deref(), Some("old-key"), "the keychain item is left as it is");
    // Settings shows both setup steps ticked.
    let prefs = cx.update(|cx| crate::extensions::connections::prefs_for(&m, Some(&c), cx));
    assert!(prefs.actions_done.contains(&"setup".to_string()) && prefs.actions_done.contains(&"tidy".to_string()), "{:?}", prefs.actions_done);
    // ui.json: the same keys an older Pitwall reads, with the same values.
    let e = ui.extension("inbox");
    assert!(e.enabled);
    assert_eq!((e.values.get("server").map(String::as_str), e.secrets_set.as_slice()), (Some("https://old.example"), ["apiKey".to_string()].as_slice()), "a copy for an older Pitwall");
    let mut done = e.actions_done.clone();
    done.sort();
    assert_eq!(done, ["setup", "tidy"]);
    assert_eq!(e.selected, Some(format!("{}:t9", c.id)));
    assert!(e.window, "its inbox's own window is the extension's, kept as it was");
    // Again: nothing more.
    cx.update(crate::extensions::connections::migrate);
    assert_eq!(cx.update(|cx| crate::accounts::list(cx).unwrap().connections("inbox").count()), 1);

    // A downgrade: the older Pitwall changes its server, its setup step
    // fails, it shows another thread (all in ui.json, as it writes it).
    // Back on this one, the connection takes all of it.
    cx.update(|cx| {
        crate::ui_state::update(cx, |u| {
            *u = std::mem::take(u).with_extension("inbox", |e| {
                e.values.insert("server".into(), "https://moved.example".into());
                e.actions_done.retain(|a| a != "setup");
                e.selected = Some("t5".into());
            })
        });
        crate::extensions::connections::migrate(cx);
    });
    cx.run_until_parked();
    let (list, ui) = cx.update(|cx| (crate::accounts::list(cx).cloned().unwrap(), crate::ui_state::get(cx)));
    let c2 = list.default_connection("inbox").cloned().unwrap();
    assert_eq!(c2.id, c.id, "the same connection");
    assert_eq!(c2.values["server"], "https://moved.example");
    assert!(c2.actions_done.is_empty());
    assert_eq!(ui.extension("inbox").selected, Some(format!("{}:t5", c.id)));
    assert_eq!(ui.extension("inbox").actions_done, ["tidy"], "the other step stays ticked");

    // Changed here: the copy follows, so an older Pitwall sees it too.
    cx.update(|cx| crate::extensions::connections::set_value("inbox", Some(&c.id), "server", Some("https://here.example".into()), cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| crate::ui_state::get(cx).extension("inbox").values.get("server").cloned()).as_deref(), Some("https://here.example"));
    // A second connection never touches the copy.
    connection("Home", "https://home.example", cx);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| crate::ui_state::get(cx).extension("inbox").values.get("server").cloned()).as_deref(), Some("https://here.example"));
    // The carried-over one removed: the copy goes, so it isn't carried
    // over again on the next start.
    cx.update(|cx| crate::extensions::connections::remove(&c.id, cx, |e, _| assert!(e.is_none())));
    cx.run_until_parked();
    let e = cx.update(|cx| crate::ui_state::get(cx).extension("inbox"));
    assert!(e.values.is_empty() && e.secrets_set.is_empty() && e.actions_done == ["tidy"], "{e:?}");
    cx.update(crate::extensions::connections::migrate);
    assert_eq!(cx.update(|cx| crate::accounts::list(cx).unwrap().connections("inbox").count()), 1, "only Home");
}

/// A thread with more open asks than before is news; the same or fewer
/// isn't (the first list never is: `listed`).
#[test]
fn new_asks_are_those_not_there_before() {
    let t = |id: &str, n: u32| Thread { id: id.into(), title: String::new(), from: None, unread: 0, open_asks: n, blocking: false, last_activity: String::new() };
    let prev = vec![t("a", 1), t("b", 0)];
    let next = vec![t("a", 1), t("b", 2), t("c", 1), t("d", 0)];
    let ids: Vec<String> = new_asks(&prev, &next).into_iter().map(|t| t.id).collect();
    assert_eq!(ids, ["b", "c"]);
}

#[test]
fn thread_ids_carry_their_connection() {
    let m = manifest();
    assert_eq!(thread_id("ext-inbox-work", "a:b"), "ext-inbox-work:a:b");
    assert_eq!(split_thread(&m, "ext-inbox-work:a:b"), ("ext-inbox-work", "a:b"));
    assert_eq!(split_thread(&m, "plain"), ("", "plain"));
    assert_eq!(split_thread(&Manifest::named("demo", "Demo"), "ext-x:1"), ("", "ext-x:1"), "without settings: as the adapter said");
}

/// No hello within the time: stopped with why.
#[gpui::test]
fn a_silent_adapter_is_stopped(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-silent");
    let h = setup(cx, &t);
    turn_on(cx);
    connection("Work", "https://x.example", cx);
    assert_eq!(run(&h, cx), Run::Starting);
    cx.executor().advance_clock(HELLO_WITHIN + Duration::from_secs(1));
    assert!(matches!(run(&h, cx), Run::Stopped(why) if why.contains("didn't answer within 10 seconds")));
}

/// Extension "inbox" in format 2, asking for `agents.read` and
/// `projects.read`; the hub with a daemon server answering its calls.
fn setup_2(cx: &mut TestAppContext, t: &TempDir) -> Entity<Hub> {
    use pitwall_core::extensions::PermissionAsk;
    let m = Manifest {
        format: 2,
        permissions: vec![PermissionAsk { name: "agents.read".into(), reason: "To match".into() }, PermissionAsk { name: "projects.read".into(), reason: "To list".into() }],
        ..manifest()
    };
    setup_m(cx, t, m)
}

/// Extension `m` (its id "inbox"); the hub with a daemon server answering
/// its calls.
fn setup_m(cx: &mut TestAppContext, t: &TempDir, m: Manifest) -> Entity<Hub> {
    use pitwall_core::testing::{FakeProvider, ManualClock, MemStore, RecordingSink};
    let events = pitwall_daemon::Hub::new(RecordingSink::new());
    let engine = pitwall_core::Engine::open(pitwall_core::Deps {
        paths: Paths::new(t.path().join("engine")),
        events: events.clone(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![]),
        providers: vec![FakeProvider::named("local", "this-mac", Arc::new(pitwall_core::exec::LocalExec))],
    });
    let reg = pitwall_daemon::Registry::new();
    let identify = pitwall_daemon::ProcessIdentity::with_extensions(engine.clone(), reg.clone());
    let cfg = pitwall_daemon::Config { socket: t.path().join("unused.sock"), version: "test".into(), settings: None, workspace: None, viewer: None, events: Some(events), extensions: Some(reg), ext_backend: None };
    let server = pitwall_daemon::build(engine, pitwall_daemon::Approvals::new(Duration::from_secs(5)), identify, cfg);
    let paths = Paths::new(t.path().join("pw"));
    cx.update(|cx| {
        crate::extensions::calls::set_api(server, cx);
        crate::accounts::model(cx).update(cx, |a, _| a.set_paths(paths));
        crate::extensions::set_found(vec![Found { folder: "inbox".into(), manifest: Ok(m) }], cx);
        crate::extensions::follow(cx);
        hub(cx)
    })
}

/// Turned on with `agents.read` granted and `projects.read` refused; its
/// adapter's end.
fn on_with_grants(cx: &mut TestAppContext) -> String {
    use pitwall_core::extensions::permissions::Decision;
    cx.update(|cx| {
        crate::ui_state::update(cx, |u| {
            *u = std::mem::take(u).with_extension("inbox", |e| {
                e.enabled = true;
                e.grants.insert("agents.read".into(), Decision { granted: true, at: 1 });
                e.grants.insert("projects.read".into(), Decision { granted: false, at: 1 });
            })
        })
    });
    let id = connection("Work", "https://x.example", cx);
    cx.run_until_parked();
    format!("inbox/{id}")
}

fn reply(h: &Entity<Hub>, end: &str, id: &str, cx: &mut TestAppContext) -> serde_json::Value {
    cx.run_until_parked();
    sent(h, end, cx).into_iter().rev().find(|l| l["type"] == "reply" && l["id"] == id).unwrap_or_else(|| panic!("no reply to {id}: {:?}", sent(h, end, cx)))
}

/// Hello offers both protocols with its grants; an adapter that answers
/// 2 calls what it is granted, is refused the rest and every action, a
/// revoke applies to its next call, and each call is in its activity.
#[gpui::test]
fn a_protocol_2_adapter_calls_what_it_is_granted(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-calls");
    let h = setup_2(cx, &t);
    let end = on_with_grants(cx);
    let hello = sent(&h, &end, cx);
    assert_eq!(hello[0]["protocols"], json!([1, 2]));
    assert_eq!(hello[0]["grants"], json!(["agents.read"]), "only what was granted");
    assert_eq!(hello[0]["offers"], json!(["inbox", "views", "commands", "badges", "notify", "panels", "webViews", "call", "events"]));
    assert_eq!(hello[0]["connection"]["name"], json!("Work"));
    say(&h, &end, json!({"type": "hello", "protocol": 2, "uses": ["call"]}), cx);
    assert_eq!(run(&h, cx), Run::Running);

    say(&h, &end, json!({"type": "call", "id": "c1", "method": "agent.list"}), cx);
    assert!(reply(&h, &end, "c1", cx).get("result").is_some(), "granted");
    say(&h, &end, json!({"type": "call", "id": "c2", "method": "project.list"}), cx);
    assert_eq!(reply(&h, &end, "c2", cx)["error"]["code"], json!("denied"), "refused by the user");
    say(&h, &end, json!({"type": "call", "id": "c3", "method": "agent.create", "params": {}}), cx);
    let no = reply(&h, &end, "c3", cx);
    assert_eq!(no["error"]["code"], json!("denied"), "{no}");
    assert!(no["error"]["message"].as_str().unwrap().contains("isn't granted agents.start"), "an action without its grant is refused, saying which: {no}");
    say(&h, &end, json!({"type": "call", "id": "c3b", "method": "queue.send", "params": {"agentId": "a1"}}), cx);
    assert_eq!(reply(&h, &end, "c3b", cx)["error"]["code"], json!("unsupported"), "sending is never an extension's");
    say(&h, &end, json!({"type": "call", "id": "c4", "method": "agent.kill"}), cx);
    assert!(reply(&h, &end, "c4", cx).get("error").is_some());

    // Revoked: its next call is refused, and it is told its grants.
    cx.update(|cx| crate::extensions::calls::decide("inbox", "agents.read", false, cx));
    cx.run_until_parked();
    assert_eq!(sent(&h, &end, cx).last(), Some(&json!({"type": "grants", "grants": []})));
    say(&h, &end, json!({"type": "call", "id": "c5", "method": "agent.list"}), cx);
    assert_eq!(reply(&h, &end, "c5", cx)["error"]["code"], json!("denied"));
    assert_eq!(run(&h, cx), Run::Running, "a refusal doesn't stop it");

    let log = cx.update(|cx| crate::extensions::calls::entries("inbox", cx));
    let methods: Vec<(&str, bool)> = log.iter().map(|e| (e.method.as_str(), e.refused())).collect();
    assert_eq!(methods, [("agent.list", false), ("project.list", true), ("agent.create", true), ("queue.send", true), ("agent.kill", true), ("agent.list", true)]);
}

/// An adapter that answers protocol 1 runs as before: calls are refused
/// (it never makes them), its inbox works.
#[gpui::test]
fn a_protocol_1_adapter_is_unchanged(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-calls-v1");
    let h = setup_2(cx, &t);
    let end = on_with_grants(cx);
    say(&h, &end, json!({"type": "hello", "protocol": 1}), cx);
    assert_eq!(run(&h, cx), Run::Running);
    say(&h, &end, json!({"type": "threads", "threads": [{"id": "t1"}]}), cx);
    assert_eq!(h.read_with(cx, |h, _| h.get("inbox").unwrap().threads.len()), 1);
    say(&h, &end, json!({"type": "call", "id": "c1", "method": "agent.list"}), cx);
    assert_eq!(reply(&h, &end, "c1", cx)["error"]["code"], json!("unsupported"));
    // Grants aren't sent to it.
    cx.update(|cx| crate::extensions::calls::decide("inbox", "projects.read", true, cx));
    cx.run_until_parked();
    assert!(!sent(&h, &end, cx).iter().any(|l| l["type"] == "grants"));
}

/// Past its limits: calls waiting for answers are capped, and one that
/// keeps calling past them is stopped, saying why.
#[gpui::test]
fn an_adapter_past_its_limits_is_stopped(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-calls-limits");
    let h = setup_2(cx, &t);
    let end = on_with_grants(cx);
    say(&h, &end, json!({"type": "hello", "protocol": 2}), cx);
    // Calls sent without the hub getting a turn: past the in-flight cap.
    h.read_with(cx, |h, _| {
        for i in 0..(calls::MAX_IN_FLIGHT + 5) {
            h.ends[&end].say(json!({"type": "call", "id": format!("c{i}"), "method": "agent.list"}));
        }
    });
    cx.run_until_parked();
    let busy = sent(&h, &end, cx).iter().filter(|l| l["error"]["code"] == "busy").count();
    assert!(busy >= 1, "some refused as busy");
    // Keeps at it: stopped.
    h.read_with(cx, |h, _| {
        for i in 0..(calls::MAX_STRIKES as usize * 2) {
            h.ends[&end].say(json!({"type": "call", "id": format!("d{i}"), "method": "agent.list"}));
        }
    });
    cx.run_until_parked();
    assert!(matches!(run(&h, cx), Run::Stopped(why) if why.contains("limits")), "{:?}", run(&h, cx));
}

/// Its log, for its author: what Pitwall sent (the secret taken out of
/// hello), kept across a restart, forgotten when it is turned off.
#[gpui::test]
fn an_adapter_s_log_is_kept_while_it_is_on(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-log");
    let h = setup(cx, &t);
    turn_on(cx);
    cx.update(|cx| {
        crate::extensions::connections::set_setting_value("inbox", "server", Some("https://inbox.example.com".into()), cx).unwrap();
        crate::extensions::set_secret("inbox", "apiKey", "s3cret".into(), cx, |err, _| assert!(err.is_none()));
    });
    assert_eq!(run(&h, cx), Run::Starting);
    let conn = cx.update(|cx| crate::extensions::connections::settings_target("inbox", cx).unwrap());
    let end = format!("inbox/{}", conn.id);
    say(&h, &end, json!({"type": "hello", "protocol": 1}), cx);
    h.update(cx, |h, cx| h.open("inbox", &format!("{}:t1", conn.id), cx));
    let texts = |h: &Entity<Hub>, cx: &mut TestAppContext| h.read_with(cx, |h, _| h.log("inbox").into_iter().map(|(_, l)| l.text).collect::<Vec<_>>());
    let log = texts(&h, cx);
    assert!(log[0].contains("\"type\":\"hello\"") && log[0].contains("[secret]"), "{log:?}");
    assert!(!log.iter().any(|l| l.contains("s3cret")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("\"type\":\"open\"") && l.contains("\"t1\"")), "{log:?}");
    h.update(cx, |h, cx| h.restart("inbox", cx));
    cx.run_until_parked();
    assert!(texts(&h, cx).len() > log.len(), "kept across a restart");
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("inbox", false)));
    assert_eq!(run(&h, cx), Run::Off);
    assert!(texts(&h, cx).is_empty(), "forgotten when turned off");
}

/// Extension "inbox" in format 2 contributing a view "runs", a command,
/// badges and notifications (granted `agents.read` and `notify`).
fn setup_views(cx: &mut TestAppContext, t: &TempDir) -> (Entity<Hub>, String) {
    use pitwall_core::extensions::permissions::Decision;
    use pitwall_core::extensions::{CommandDef, Contributes, PermissionAsk, ViewDef};
    let m = Manifest {
        format: 2,
        permissions: vec![PermissionAsk { name: "agents.read".into(), reason: "To match".into() }, PermissionAsk { name: "notify".into(), reason: "To tell".into() }],
        contributes: Contributes {
            views: vec![ViewDef { id: "runs".into(), title: "Runs".into(), icon: None }],
            commands: vec![CommandDef { id: "rerun".into(), title: "Re-run".into(), context: Default::default() }],
            badges: true,
            notifications: true,
            ..Default::default()
        },
        ..manifest()
    };
    let h = setup_m(cx, t, m);
    cx.update(|cx| {
        crate::ui_state::update(cx, |u| {
            *u = std::mem::take(u).with_extension("inbox", |e| {
                e.enabled = true;
                e.grants.insert("agents.read".into(), Decision { granted: true, at: 1 });
                e.grants.insert("notify".into(), Decision { granted: true, at: 1 });
            })
        })
    });
    let id = connection("Work", "https://x.example", cx);
    cx.run_until_parked();
    let end = format!("inbox/{id}");
    say(&h, &end, json!({"type": "hello", "protocol": 2}), cx);
    (h, end)
}

fn view_of(h: &Entity<Hub>, cx: &mut TestAppContext) -> Option<views::ViewData> {
    cx.run_until_parked();
    h.read_with(cx, |h, _| h.get("inbox").and_then(|s| s.views.get("runs").cloned()))
}

/// A view is kept only while a tile shows it: shown, its adapter is told
/// (again after a restart); what it sends is drawn, its rows' ids with
/// their connection; a selection, an action and its answer go both ways;
/// a view it doesn't contribute is refused; hidden, it is told and it goes.
#[gpui::test]
fn a_view_goes_both_ways(cx: &mut TestAppContext) {
    use pitwall_core::extensions::view::Root;
    let t = TempDir::new("hub-views");
    let (h, end) = setup_views(cx, &t);
    let conn = end.trim_start_matches("inbox/").to_string();
    let list = json!({"type": "view", "view": "runs", "root": {"kind": "list", "sections": [{"items": [{"id": "r1", "title": "Build"}]}], "detail": {"kind": "markdown", "text": "hi"}}});
    say(&h, &end, list.clone(), cx);
    assert_eq!(view_of(&h, cx), None, "not shown: not kept");
    h.update(cx, |h, cx| h.show_view("inbox", "runs", cx));
    cx.run_until_parked();
    assert_eq!(sent(&h, &end, cx).last(), Some(&json!({"type": "show", "view": "runs"})));
    say(&h, &end, list, cx);
    let Some(Ok(Root::List { list, detail })) = view_of(&h, cx) else { panic!("drawn") };
    let item = format!("{conn}:r1");
    assert_eq!(list.items().map(|i| i.id.clone()).collect::<Vec<_>>(), std::slice::from_ref(&item));
    assert!(detail.is_some());
    h.update(cx, |h, _| h.select("inbox", "runs", &item));
    assert_eq!(sent(&h, &end, cx).last(), Some(&json!({"type": "select", "view": "runs", "item": "r1"})));
    let req = h.update(cx, |h, cx| h.view_action("inbox", "runs", Some(&item), "retry", cx)).unwrap();
    assert_eq!(sent(&h, &end, cx).last(), Some(&json!({"type": "action", "req": req, "view": "runs", "item": "r1", "action": "retry"})));
    assert!(h.read_with(cx, |h, _| h.get("inbox").unwrap().view_busy.contains(&("runs".to_string(), item.clone()))));
    say(&h, &end, json!({"type": "result", "req": req, "ok": false, "message": "No runner free"}), cx);
    assert_eq!(h.read_with(cx, |h, _| h.get("inbox").unwrap().view_notes.get("runs").cloned()), Some("No runner free".into()));
    // Too big or a form: refused, said in the tile.
    say(&h, &end, json!({"type": "view", "view": "runs", "root": {"kind": "form"}}), cx);
    assert!(matches!(view_of(&h, cx), Some(Err(why)) if why.contains("form")));
    // Not its own: refused, logged.
    say(&h, &end, json!({"type": "view", "view": "other", "root": {"kind": "markdown", "text": "x"}}), cx);
    let log = cx.update(|cx| crate::extensions::calls::entries("inbox", cx));
    assert!(log.iter().any(|e| e.method == "view" && e.refused()), "{log:?}");
    assert!(log.iter().any(|e| e.method.starts_with("view action: runs retry")));
    // Restarted: told again.
    h.update(cx, |h, cx| h.restart("inbox", cx));
    cx.run_until_parked();
    say(&h, &end, json!({"type": "hello", "protocol": 2}), cx);
    assert!(sent(&h, &end, cx).iter().any(|l| *l == json!({"type": "show", "view": "runs"})));
    h.update(cx, |h, _| h.hide_view("inbox", "runs"));
    assert_eq!(sent(&h, &end, cx).last(), Some(&json!({"type": "hide", "view": "runs"})));
    assert_eq!(view_of(&h, cx), None);
}

/// A command goes to the adapter and its answer comes back as an event.
#[gpui::test]
fn a_command_is_sent_and_answered(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-commands");
    let (h, end) = setup_views(cx, &t);
    let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let g = got.clone();
    cx.update(|cx| cx.subscribe(&h, move |_, e: &HubEvent, _| g.borrow_mut().push(e.clone())).detach());
    let req = h.update(cx, |h, cx| h.command("inbox", "rerun", "Re-run", Some("a1".into()), cx)).unwrap();
    assert_eq!(sent(&h, &end, cx).last(), Some(&json!({"type": "command", "req": req, "command": "rerun", "agentId": "a1"})));
    say(&h, &end, json!({"type": "result", "req": req, "ok": true, "message": "Queued"}), cx);
    assert!(got.borrow().contains(&HubEvent::CommandDone { ext: "inbox".into(), title: "Re-run".into(), ok: true, message: "Queued".into() }));
}

/// Badges: kept when the extension may name the row, at most a set every
/// quarter second (the newest wins), gone when it stops.
#[gpui::test]
fn badges_follow_permissions_limits_and_the_adapter(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-badges");
    let (h, end) = setup_views(cx, &t);
    let badges = |h: &Entity<Hub>, cx: &mut TestAppContext| h.read_with(cx, |h, _| h.get("inbox").unwrap().badges.iter().map(|(_, b)| b.note.clone()).collect::<Vec<_>>());
    say(&h, &end, json!({"type": "badges", "badges": [{"agent": {"agentId": "a1"}, "note": "one"}, {"project": {"path": "/p"}, "count": 2}]}), cx);
    assert_eq!(badges(&h, cx), ["one"], "a project needs projects.read");
    let log = cx.update(|cx| crate::extensions::calls::entries("inbox", cx));
    assert!(log.iter().any(|e| e.method == "badges" && e.refused()));
    say(&h, &end, json!({"type": "badges", "badges": [{"agent": {"agentId": "a1"}, "note": "two"}]}), cx);
    say(&h, &end, json!({"type": "badges", "badges": [{"agent": {"agentId": "a1"}, "note": "three"}]}), cx);
    assert_eq!(badges(&h, cx), ["one"], "too soon: waits");
    cx.executor().advance_clock(views::BADGES_EVERY);
    cx.run_until_parked();
    assert_eq!(badges(&h, cx), ["three"], "the newest");
    h.update(cx, |h, cx| h.restart("inbox", cx));
    cx.run_until_parked();
    assert!(badges(&h, cx).is_empty(), "gone with it");
}

/// Notifications: granted, a few a minute, the same one once.
#[gpui::test]
fn notifications_are_limited(cx: &mut TestAppContext) {
    let t = TempDir::new("hub-notify");
    let (h, end) = setup_views(cx, &t);
    let got = std::rc::Rc::new(std::cell::RefCell::new(0));
    let g = got.clone();
    cx.update(|cx| {
        cx.subscribe(&h, move |_, e: &HubEvent, _| {
            if matches!(e, HubEvent::Notify { .. }) {
                *g.borrow_mut() += 1
            }
        })
        .detach()
    });
    say(&h, &end, json!({"type": "notify", "id": "n1", "title": "Build failed"}), cx);
    say(&h, &end, json!({"type": "notify", "id": "n1", "title": "Build failed"}), cx);
    assert_eq!(*got.borrow(), 1, "the same one once");
    for i in 2..6 {
        say(&h, &end, json!({"type": "notify", "id": format!("n{i}"), "title": "More"}), cx);
    }
    assert_eq!(*got.borrow(), 3, "three a minute");
    let log = cx.update(|cx| crate::extensions::calls::entries("inbox", cx));
    assert_eq!(log.iter().filter(|e| e.method == "notify" && !e.refused()).count(), 3);
    assert!(log.iter().any(|e| e.method == "notify" && e.refused()));
    // Revoked: refused.
    cx.update(|cx| crate::extensions::calls::decide("inbox", "notify", false, cx));
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(61));
    say(&h, &end, json!({"type": "notify", "id": "n9", "title": "Late"}), cx);
    assert_eq!(*got.borrow(), 3);
}

/// A view's buttons: one Pitwall runs itself goes through the method
/// table as the extension (from a view), so without its grant it is
/// refused, said in the view and in the activity; what an adapter sends
/// for an agent (a right-panel section, which Pitwall no longer draws) is
/// refused, not kept; a web view's address from the adapter is kept, one
/// that isn't https or local refused.
#[gpui::test]
fn a_views_buttons_go_through_the_grants(cx: &mut TestAppContext) {
    use pitwall_core::extensions::permissions::Decision;
    use pitwall_core::extensions::view::{Action, Run as Does};
    use pitwall_core::extensions::{Contributes, PermissionAsk, ViewDef, WebViewDef};
    let t = TempDir::new("hub-views");
    let m = Manifest {
        format: 2,
        permissions: vec![PermissionAsk { name: "agents.read".into(), reason: "To show".into() }, PermissionAsk { name: "agents.focus".into(), reason: "To jump".into() }],
        contributes: Contributes {
            views: vec![ViewDef { id: "about".into(), title: "About".into(), icon: None }],
            web_views: vec![WebViewDef { id: "site".into(), title: "Site".into(), icon: None, url: None }],
            ..Default::default()
        },
        ..manifest()
    };
    let h = setup_m(cx, &t, m);
    cx.update(|cx| {
        crate::ui_state::update(cx, |u| {
            *u = std::mem::take(u).with_extension("inbox", |e| {
                e.enabled = true;
                e.grants.insert("agents.read".into(), Decision { granted: true, at: 1 });
                // Asked for, not granted.
                e.grants.insert("agents.focus".into(), Decision { granted: false, at: 1 });
            })
        })
    });
    let id = connection("Work", "https://x.example", cx);
    cx.run_until_parked();
    let end = format!("inbox/{id}");
    say(&h, &end, json!({"type": "hello", "protocol": 2}), cx);
    let key = "about";
    h.update(cx, |h, cx| h.show_view("inbox", key, cx));
    cx.run_until_parked();
    assert_eq!(sent(&h, &end, cx).last(), Some(&json!({"type": "show", "view": "about"})));
    say(&h, &end, json!({"type": "view", "view": "about", "agentId": "a1", "root": {"kind": "markdown", "text": "for a1"}}), cx);
    assert!(!h.read_with(cx, |h, _| h.get("inbox").unwrap().views.contains_key(key)), "a section for an agent isn't kept");
    let log = cx.update(|cx| crate::extensions::calls::entries("inbox", cx));
    assert!(log.iter().any(|e| e.method == "view" && e.refused()), "{log:?}");
    say(&h, &end, json!({"type": "view", "view": "about", "root": {"kind": "markdown", "text": "hi"}}), cx);
    assert!(h.read_with(cx, |h, _| h.get("inbox").unwrap().views.get(key).is_some_and(|v| v.is_ok())), "kept");
    // Pressed without agents.focus: refused by the daemon, not sent on.
    let before = sent(&h, &end, cx).len();
    let focus = Action { id: "jump".into(), label: "Jump".into(), primary: false, run: Some(Does::Focus("a1".into())), needs: None };
    h.update(cx, |h, cx| h.press("inbox", key, None, Some("a1"), &focus, cx)).unwrap();
    cx.run_until_parked();
    let note = h.read_with(cx, |h, _| h.get("inbox").unwrap().view_notes.get(key).cloned());
    assert!(note.as_deref().is_some_and(|n| n.contains("agents.focus")), "{note:?}");
    assert_eq!(sent(&h, &end, cx).len(), before, "the adapter isn't told");
    let log = cx.update(|cx| crate::extensions::calls::entries("inbox", cx));
    assert!(log.iter().any(|e| e.method == "agent.focus" && e.via == pitwall_core::extensions::activity::Via::View && e.refused()), "{log:?}");
    // One its adapter acts on, needing what it isn't granted: not sent.
    let needs = Action { id: "go".into(), label: "Go".into(), primary: false, run: None, needs: Some("agents.start".into()) };
    assert!(h.update(cx, |h, cx| h.press("inbox", key, None, Some("a1"), &needs, cx)).is_err());
    assert_eq!(sent(&h, &end, cx).len(), before);
    // Web view addresses.
    say(&h, &end, json!({"type": "webView", "view": "site", "url": "https://x.example/app"}), cx);
    assert_eq!(h.read_with(cx, |h, _| h.get("inbox").unwrap().web_urls.get("site").cloned()), Some(Ok("https://x.example/app".into())));
    say(&h, &end, json!({"type": "webView", "view": "site", "url": "http://x.example/app"}), cx);
    assert!(h.read_with(cx, |h, _| h.get("inbox").unwrap().web_urls.get("site").cloned()).is_some_and(|r| r.is_err()));
    h.update(cx, |h, _| h.hide_view("inbox", key));
    assert_eq!(sent(&h, &end, cx).last(), Some(&json!({"type": "hide", "view": "about"})));
}
