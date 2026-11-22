use super::*;
use gpui::TestAppContext;
use pitwall_core::accounts::Accounts;
use pitwall_core::extensions::{Adapter, Found, MatchAgents, SettingDef};
use pitwall_core::paths::Paths;
use pitwall_core::testing::TempDir;
use serde_json::json;

fn manifest() -> Manifest {
    let setting = |key: &str, kind| SettingDef { key: key.into(), kind, label: key.to_uppercase(), hint: None, placeholder: None, required: true };
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
    say(&h, &end, json!({"type": "hello", "protocol": 2}), cx);
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
    let today = json!({"v": 1, "extensions": {"inbox": {"enabled": true, "values": {"server": "https://old.example"}, "secretsSet": ["apiKey"], "selected": "t9", "actionsDone": ["setup", "tidy"]}}});
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
