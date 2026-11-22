//! Every extension's adapter and what it said (docs/spec/extensions.md):
//! one [`Hub`] for the app, which windows and Settings observe. It starts
//! an extension's adapter when the extension is on and its required
//! settings are set, starts it again when they change, stops it when it is
//! turned off or Pitwall quits, and keeps the adapter's state (connection,
//! threads, open threads, how requests went) for the native UI to draw.
//!
//! An adapter that stops, crashes or floods stays stopped with its reason
//! shown ([`Run::Stopped`]) until the user restarts it from Settings or its
//! sidebar section.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::time::Duration;

use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt;
use gpui::{App, AppContext, Context, Entity, Global};
use pitwall_core::extensions::protocol::{ConnState, Counts, FromAdapter, Thread, ThreadDetail, ToAdapter, MAX_REPLY, PROTOCOL};
use pitwall_core::extensions::{Manifest, SettingKind};
use serde_json::{Map, Value};

use super::adapter::{Event, Link};

/// How long an adapter has to answer `hello`.
pub const HELLO_WITHIN: Duration = Duration::from_secs(10);

/// Where an extension's adapter is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Run {
    /// Off, or it has no adapter.
    #[default]
    Off,
    /// On, but settings it needs aren't set (what to do).
    Setup(String),
    Starting,
    Running,
    /// It stopped by itself or was stopped for misbehaving (why).
    Stopped(String),
}

/// An extension's state.
#[derive(Default)]
pub struct ExtState {
    pub run: Run,
    /// What its adapter last said about its connection.
    pub conn: Option<(ConnState, String)>,
    pub threads: Vec<Thread>,
    /// The totals it last sent (none: its list is counted).
    pub totals: Option<Counts>,
    /// Threads opened, as last sent.
    pub details: HashMap<String, ThreadDetail>,
    /// Why the last request on a thread failed.
    pub notes: HashMap<String, String>,
    /// Threads with a request in flight.
    pub busy: HashSet<String>,
    link: Option<Link>,
    /// Which start this state is of (events of older ones are dropped).
    gen: u64,
    fingerprint: Option<u64>,
    hello: bool,
    /// Views open per thread.
    open: HashMap<String, usize>,
    /// Requests in flight: their thread.
    pending: HashMap<u64, String>,
    /// How finished requests went (the latest few).
    done: Vec<(u64, bool)>,
}

impl ExtState {
    /// Unread threads, open asks and blocking asks: the adapter's totals
    /// when it sent some, else its list counted.
    pub fn counts(&self) -> Counts {
        if let Some(c) = self.totals {
            return c;
        }
        Counts {
            unread: self.threads.iter().filter(|t| t.unread > 0).count() as u32,
            open_asks: self.threads.iter().map(|t| t.open_asks).sum(),
            blocking: self.threads.iter().filter(|t| t.blocking).count() as u32,
        }
    }

    /// How request `req` went (`None`: not yet, or long ago).
    pub fn result(&self, req: u64) -> Option<bool> {
        self.done.iter().find(|(r, _)| *r == req).map(|(_, ok)| *ok)
    }
}

/// A request on a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Answer { ask: u32, picks: Vec<u32>, text: Option<String>, declined: bool },
    Message { text: String },
    MarkRead,
    Archive,
}

pub struct Hub {
    exts: HashMap<String, ExtState>,
    next_req: u64,
    next_gen: u64,
    /// Bumped when an extension's secret is changed (it starts again).
    secrets_gen: HashMap<String, u64>,
    /// Tests: the other ends of the adapters' links.
    #[cfg(test)]
    pub ends: HashMap<String, super::adapter::TestEnd>,
}

#[derive(Clone)]
struct HubHandle(Entity<Hub>);

impl Global for HubHandle {}

/// The app's hub (made when there is none).
pub fn hub(cx: &mut App) -> Entity<Hub> {
    if let Some(h) = cx.try_global::<HubHandle>() {
        return h.0.clone();
    }
    let h = cx.new(|_| Hub {
        exts: HashMap::new(),
        next_req: 1,
        next_gen: 1,
        secrets_gen: HashMap::new(),
        #[cfg(test)]
        ends: HashMap::new(),
    });
    cx.set_global(HubHandle(h.clone()));
    h
}

/// The hub, if there is one (drawing code).
pub fn try_hub(cx: &App) -> Option<Entity<Hub>> {
    cx.try_global::<HubHandle>().map(|h| h.0.clone())
}

/// What extension `m` needs with settings `prefs`: `Err(what to do)` when
/// a required setting isn't set.
pub fn missing(m: &Manifest, prefs: &crate::main_screen::workspace::ExtensionPrefs) -> Result<(), String> {
    let gone: Vec<&str> = m
        .settings
        .iter()
        .filter(|s| s.required)
        .filter(|s| match s.kind {
            SettingKind::Url => prefs.values.get(&s.key).is_none_or(|v| v.trim().is_empty()),
            SettingKind::Secret => !prefs.secrets_set.contains(&s.key),
        })
        .map(|s| s.label.as_str())
        .collect();
    if gone.is_empty() {
        return Ok(());
    }
    Err(format!("Set {} in Settings → General → Extensions.", gone.join(" and ")))
}

impl Hub {
    pub fn get(&self, id: &str) -> Option<&ExtState> {
        self.exts.get(id)
    }

    /// Start, stop or start again each extension's adapter to match its
    /// switch and settings.
    pub fn reconcile(&mut self, cx: &mut Context<Self>) {
        let found = super::found(cx);
        let ui = crate::ui_state::get(cx);
        let dir = super::dir(cx);
        let mut seen = HashSet::new();
        for m in found.iter().filter_map(|f| f.manifest.as_ref().ok()) {
            seen.insert(m.id.clone());
            let prefs = ui.extension(&m.id);
            let st = self.exts.entry(m.id.clone()).or_default();
            let want = match (&m.adapter, prefs.enabled) {
                (Some(_), true) => match missing(m, &prefs) {
                    Ok(()) => {
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        (m.adapter.as_ref().map(|a| &a.command), &m.settings.iter().map(|s| (&s.key, prefs.values.get(&s.key))).collect::<Vec<_>>(), &prefs.secrets_set, self.secrets_gen.get(&m.id)).hash(&mut h);
                        Some(Ok(h.finish()))
                    }
                    Err(why) => Some(Err(why)),
                },
                _ => None,
            };
            match want {
                None => {
                    Self::stop(st);
                    st.run = Run::Off;
                    st.fingerprint = None;
                }
                Some(Err(why)) => {
                    Self::stop(st);
                    st.run = Run::Setup(why);
                    st.fingerprint = None;
                }
                Some(Ok(fp)) => {
                    if st.fingerprint != Some(fp) {
                        st.fingerprint = Some(fp);
                        let (m, dir) = (m.clone(), dir.clone());
                        self.start(&m, dir, cx);
                    }
                }
            }
        }
        for (id, st) in self.exts.iter_mut() {
            if !seen.contains(id) {
                Self::stop(st);
                st.run = Run::Off;
                st.fingerprint = None;
            }
        }
        cx.notify();
    }

    /// Start extension `id`'s adapter again (after it stopped).
    pub fn restart(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(st) = self.exts.get_mut(id) {
            Self::stop(st);
            st.fingerprint = None;
        }
        self.reconcile(cx);
    }

    /// Extension `id`'s secrets changed: its adapter starts again.
    pub fn secrets_changed(&mut self, id: &str, cx: &mut Context<Self>) {
        *self.secrets_gen.entry(id.to_string()).or_default() += 1;
        self.reconcile(cx);
    }

    fn stop(st: &mut ExtState) {
        if let Some(l) = st.link.take() {
            l.stop();
        }
        st.gen = 0;
        st.hello = false;
        st.conn = None;
        st.busy.clear();
        st.pending.clear();
        if matches!(st.run, Run::Running | Run::Starting) {
            st.run = Run::Off;
        }
    }

    /// Kill every adapter now (Pitwall quits).
    pub fn quit(&mut self) {
        for st in self.exts.values_mut() {
            if let Some(l) = st.link.take() {
                l.stop_now();
            }
        }
    }

    fn start(&mut self, m: &Manifest, dir: Option<std::path::PathBuf>, cx: &mut Context<Self>) {
        let gen = self.next_gen;
        self.next_gen += 1;
        let st = self.exts.entry(m.id.clone()).or_default();
        Self::stop(st);
        st.run = Run::Starting;
        st.gen = gen;
        st.threads.clear();
        st.totals = None;
        st.details.clear();
        st.notes.clear();
        let ui = crate::ui_state::get(cx);
        let prefs = ui.extension(&m.id);
        let mut settings = Map::new();
        for s in m.settings.iter().filter(|s| s.kind == SettingKind::Url) {
            if let Some(v) = prefs.values.get(&s.key).filter(|v| !v.trim().is_empty()) {
                settings.insert(s.key.clone(), Value::String(v.clone()));
            }
        }
        let secret_keys: Vec<String> = m.settings.iter().filter(|s| s.kind == SettingKind::Secret && prefs.secrets_set.contains(&s.key)).map(|s| s.key.clone()).collect();
        let store = super::secret_store(cx);
        let accounts: Vec<(String, String)> = secret_keys.iter().map(|k| (k.clone(), super::account(&m.id, k, cx))).collect();
        let command = m.adapter.as_ref().map(|a| a.command.clone()).unwrap_or_default();
        let folder = dir.map(|d| d.join(&m.id));
        let id = m.id.clone();
        let started = cx.background_executor().spawn(async move {
            let mut secrets = Map::new();
            for (key, account) in accounts {
                match store.get(&account) {
                    Ok(Some(s)) => {
                        secrets.insert(key, Value::String(s));
                    }
                    Ok(None) => return Err("A secret it needs is gone from the system's secure store: set it again in Settings.".to_string()),
                    Err(e) => return Err(e),
                }
            }
            let hello = ToAdapter::Hello { pitwall: env!("CARGO_PKG_VERSION").to_string(), settings, secrets };
            open_link(folder, &command, &hello)
        });
        cx.spawn(async move |this, cx| {
            let res = started.await;
            let rx = this.update(cx, |h, cx| h.attach(&id, gen, res, cx)).ok().flatten();
            let Some(mut rx) = rx else { return };
            let timer_id = id.clone();
            let t = this.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(HELLO_WITHIN).await;
                let _ = t.update(cx, |h, cx| h.hello_late(&timer_id, gen, cx));
            })
            .detach();
            while let Some(ev) = rx.next().await {
                if this.update(cx, |h, cx| h.event(&id, gen, ev, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// The adapter started (or not): keep its link; its events' channel.
    fn attach(&mut self, id: &str, gen: u64, res: Result<Opened, String>, cx: &mut Context<Self>) -> Option<UnboundedReceiver<Event>> {
        let st = self.exts.get_mut(id)?;
        let opened = match res {
            Ok(o) => o,
            Err(why) => {
                if st.gen == gen {
                    st.run = Run::Stopped(why);
                    st.gen = 0;
                }
                cx.notify();
                return None;
            }
        };
        if st.gen != gen {
            opened.link.stop();
            return None;
        }
        st.link = Some(opened.link);
        #[cfg(test)]
        if let Some(end) = opened.end {
            self.ends.insert(id.to_string(), end);
        }
        cx.notify();
        Some(opened.events)
    }

    fn hello_late(&mut self, id: &str, gen: u64, cx: &mut Context<Self>) {
        let Some(st) = self.exts.get_mut(id) else { return };
        if st.gen == gen && !st.hello {
            Self::stop(st);
            st.run = Run::Stopped(format!("It didn't answer within {} seconds.", HELLO_WITHIN.as_secs()));
            cx.notify();
        }
    }

    fn event(&mut self, id: &str, gen: u64, ev: Event, cx: &mut Context<Self>) {
        let Some(st) = self.exts.get_mut(id) else { return };
        if st.gen != gen {
            return;
        }
        match ev {
            Event::Stopped(why) => {
                Self::stop(st);
                st.run = Run::Stopped(why);
            }
            Event::Msg(FromAdapter::Hello { protocol }) => {
                if protocol != PROTOCOL {
                    Self::stop(st);
                    st.run = Run::Stopped(format!("It speaks protocol {protocol}; this Pitwall speaks {PROTOCOL}. Update the extension or Pitwall."));
                } else if !st.hello {
                    st.hello = true;
                    st.run = Run::Running;
                    // Threads open in tiles from before (a restart).
                    for thread in st.open.keys() {
                        if let Some(l) = &st.link {
                            l.send(&ToAdapter::Open { thread: thread.clone() });
                        }
                    }
                }
            }
            // Nothing before its hello counts.
            Event::Msg(_) if !st.hello => {}
            Event::Msg(FromAdapter::Status { state, message }) => st.conn = Some((state, message)),
            Event::Msg(FromAdapter::Threads(list)) => st.threads = list,
            Event::Msg(FromAdapter::Counts(c)) => st.totals = Some(c),
            Event::Msg(FromAdapter::Thread(d)) => {
                if st.open.contains_key(&d.id) {
                    st.details.insert(d.id.clone(), d);
                }
            }
            Event::Msg(FromAdapter::Result { req, ok, message }) => {
                if let Some(thread) = st.pending.remove(&req) {
                    st.busy.remove(&thread);
                    if ok {
                        st.notes.remove(&thread);
                    } else {
                        st.notes.insert(thread, if message.is_empty() { "That didn't work.".into() } else { message });
                    }
                }
                st.done.push((req, ok));
                if st.done.len() > 32 {
                    st.done.remove(0);
                }
            }
            Event::Msg(FromAdapter::Unknown) => {}
        }
        cx.notify();
    }

    /// A view of `thread` opened: the adapter sends it and keeps it current.
    pub fn open(&mut self, id: &str, thread: &str, cx: &mut Context<Self>) {
        let st = self.exts.entry(id.to_string()).or_default();
        let n = st.open.entry(thread.to_string()).or_default();
        *n += 1;
        if *n == 1 && st.hello {
            if let Some(l) = &st.link {
                l.send(&ToAdapter::Open { thread: thread.to_string() });
            }
        }
        cx.notify();
    }

    /// A view of `thread` closed.
    pub fn close(&mut self, id: &str, thread: &str) {
        let Some(st) = self.exts.get_mut(id) else { return };
        let Some(n) = st.open.get_mut(thread) else { return };
        *n -= 1;
        if *n == 0 {
            st.open.remove(thread);
            st.details.remove(thread);
            if let (true, Some(l)) = (st.hello, &st.link) {
                l.send(&ToAdapter::Close { thread: thread.to_string() });
            }
        }
    }

    /// Send `r` on `thread`: its request number, or why not.
    pub fn request(&mut self, id: &str, thread: &str, r: Request, cx: &mut Context<Self>) -> Result<u64, String> {
        let st = self.exts.get_mut(id).ok_or("The extension isn't running.")?;
        let link = st.link.as_ref().filter(|_| st.hello).ok_or("The extension isn't running.")?;
        let req = self.next_req;
        let thread_s = thread.to_string();
        let msg = match r {
            Request::Answer { ask, picks, text, declined } => {
                let text = text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
                if text.as_ref().is_some_and(|t| t.chars().count() > MAX_REPLY) {
                    return Err(format!("Keep it under {MAX_REPLY} characters."));
                }
                if !declined && picks.is_empty() && text.is_none() {
                    return Err("Pick an answer or write one.".into());
                }
                ToAdapter::Answer { req, thread: thread_s, ask, picks: if declined { vec![] } else { picks }, text: if declined { None } else { text }, declined }
            }
            Request::Message { text } => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Err("Write something first.".into());
                }
                if text.chars().count() > MAX_REPLY {
                    return Err(format!("Keep it under {MAX_REPLY} characters."));
                }
                ToAdapter::Message { req, thread: thread_s, text }
            }
            Request::MarkRead => ToAdapter::MarkRead { req, thread: thread_s },
            Request::Archive => ToAdapter::Archive { req, thread: thread_s },
        };
        if !link.send(&msg) {
            return Err("The extension isn't running.".into());
        }
        self.next_req += 1;
        st.pending.insert(req, thread.to_string());
        st.busy.insert(thread.to_string());
        st.notes.remove(thread);
        cx.notify();
        Ok(req)
    }

    /// Ask for the list again.
    pub fn refresh(&mut self, id: &str) {
        if let Some(l) = self.exts.get(id).filter(|s| s.hello).and_then(|s| s.link.as_ref()) {
            l.send(&ToAdapter::Refresh);
        }
    }
}

/// A started adapter.
struct Opened {
    link: Link,
    events: UnboundedReceiver<Event>,
    #[cfg(test)]
    end: Option<super::adapter::TestEnd>,
}

#[cfg(not(test))]
fn open_link(folder: Option<std::path::PathBuf>, command: &[String], hello: &ToAdapter) -> Result<Opened, String> {
    let folder = folder.ok_or("Extensions need Pitwall's data folder.")?;
    if command.is_empty() {
        return Err("It has no adapter.".into());
    }
    let (link, events) = super::adapter::spawn(&folder, command, hello)?;
    Ok(Opened { link, events })
}

/// Tests: no process; the test drives the other end ([`Hub::ends`]).
#[cfg(test)]
fn open_link(_: Option<std::path::PathBuf>, _: &[String], hello: &ToAdapter) -> Result<Opened, String> {
    let (link, events, end) = super::adapter::pair(hello);
    Ok(Opened { link, events, end: Some(end) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use pitwall_core::extensions::{Adapter, Found, SettingDef};
    use serde_json::json;

    fn manifest() -> Manifest {
        let setting = |key: &str, kind| SettingDef { key: key.into(), kind, label: key.to_uppercase(), hint: None, placeholder: None, required: true };
        Manifest {
            settings: vec![setting("server", SettingKind::Url), setting("apiKey", SettingKind::Secret)],
            adapter: Some(Adapter { command: vec!["node".into(), "a.mjs".into()] }),
            ..Manifest::named("inbox", "Inbox")
        }
    }

    fn setup(cx: &mut TestAppContext) -> Entity<Hub> {
        cx.update(|cx| {
            crate::extensions::set_found(vec![Found { folder: "inbox".into(), manifest: Ok(manifest()) }], cx);
            crate::extensions::follow(cx);
            hub(cx)
        })
    }

    fn run(h: &Entity<Hub>, cx: &mut TestAppContext) -> Run {
        cx.run_until_parked();
        h.read_with(cx, |h, _| h.get("inbox").map(|s| s.run.clone()).unwrap_or_default())
    }

    fn sent(h: &Entity<Hub>, cx: &mut TestAppContext) -> Vec<serde_json::Value> {
        h.read_with(cx, |h, _| h.ends.get("inbox").map(|e| e.sent()).unwrap_or_default())
    }

    fn say(h: &Entity<Hub>, line: serde_json::Value, cx: &mut TestAppContext) {
        h.read_with(cx, |h, _| h.ends["inbox"].say(line));
        cx.run_until_parked();
    }

    /// Off until on; on but unset asks for its settings; set, it starts
    /// with them (the secret from the secure store, only in hello); its
    /// list, counts and an answer's round trip; a stop shows why, and
    /// Restart starts it again.
    #[gpui::test]
    fn an_adapter_follows_its_switch_and_settings(cx: &mut TestAppContext) {
        let h = setup(cx);
        assert_eq!(run(&h, cx), Run::Off);
        cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("inbox", true)));
        assert!(matches!(run(&h, cx), Run::Setup(why) if why.contains("SERVER and APIKEY")));
        cx.update(|cx| {
            crate::ui_state::update(cx, |u| {
                *u = std::mem::take(u).with_extension("inbox", |e| {
                    e.values.insert("server".into(), "https://inbox.example.com".into());
                })
            });
            crate::extensions::set_secret("inbox", "apiKey", "s3cret".into(), cx, |err, _| assert!(err.is_none()));
        });
        assert_eq!(run(&h, cx), Run::Starting);
        let ui = cx.update(|cx| crate::ui_state::get(cx));
        assert_eq!(ui.extension("inbox").secrets_set, ["apiKey"]);
        assert!(!serde_json::to_string(&ui).unwrap().contains("s3cret"), "the secret is never in ui.json");
        let hello = sent(&h, cx);
        assert_eq!(hello[0]["settings"], json!({"server": "https://inbox.example.com"}));
        assert_eq!(hello[0]["secrets"], json!({"apiKey": "s3cret"}));

        // Nothing counts before its hello.
        say(&h, json!({"type": "threads", "threads": [{"id": "early"}]}), cx);
        assert!(h.read_with(cx, |h, _| h.get("inbox").unwrap().threads.is_empty()));
        say(&h, json!({"type": "hello", "protocol": 1}), cx);
        assert_eq!(run(&h, cx), Run::Running);
        say(&h, json!({"type": "threads", "threads": [{"id": "t1", "unread": 2, "openAsks": 1, "blocking": true}, {"id": "t2"}]}), cx);
        let c = h.read_with(cx, |h, _| h.get("inbox").unwrap().counts());
        assert_eq!((c.unread, c.open_asks, c.blocking), (1, 1, 1), "counted from the list");
        say(&h, json!({"type": "counts", "unread": 9, "openAsks": 4, "blocking": 2}), cx);
        assert_eq!(h.read_with(cx, |h, _| h.get("inbox").unwrap().counts().unread), 9, "its totals win");

        // Open, answer, result.
        h.update(cx, |h, cx| h.open("inbox", "t1", cx));
        assert_eq!(sent(&h, cx), [json!({"type": "open", "thread": "t1"})]);
        let bad = h.update(cx, |h, cx| h.request("inbox", "t1", Request::Answer { ask: 1, picks: vec![], text: Some("  ".into()), declined: false }, cx));
        assert!(bad.is_err(), "an empty answer isn't sent");
        let req = h.update(cx, |h, cx| h.request("inbox", "t1", Request::Answer { ask: 1, picks: vec![0], text: None, declined: false }, cx)).unwrap();
        assert_eq!(sent(&h, cx)[0], json!({"type": "answer", "req": req, "thread": "t1", "ask": 1, "picks": [0], "declined": false}));
        assert!(h.read_with(cx, |h, _| h.get("inbox").unwrap().busy.contains("t1")));
        say(&h, json!({"type": "result", "req": req, "ok": false, "message": "That question isn't open any more."}), cx);
        h.read_with(cx, |h, _| {
            let s = h.get("inbox").unwrap();
            assert!(!s.busy.contains("t1"));
            assert_eq!(s.notes["t1"], "That question isn't open any more.");
            assert_eq!(s.result(req), Some(false));
        });
        say(&h, json!({"type": "thread", "thread": {"id": "t1", "messages": [{"id": "1", "text": "hi"}]}}), cx);
        say(&h, json!({"type": "thread", "thread": {"id": "not-open", "messages": []}}), cx);
        h.read_with(cx, |h, _| {
            let s = h.get("inbox").unwrap();
            assert!(s.details.contains_key("t1") && !s.details.contains_key("not-open"), "only open threads are kept");
        });

        // It stops: why is kept until Restart.
        h.read_with(cx, |h, _| {
            let _ = h.ends["inbox"].events.unbounded_send(Event::Stopped("It exited with code 3.".into()));
        });
        assert_eq!(run(&h, cx), Run::Stopped("It exited with code 3.".into()));
        assert!(h.update(cx, |h, cx| h.request("inbox", "t1", Request::MarkRead, cx)).is_err());
        h.update(cx, |h, cx| h.restart("inbox", cx));
        assert_eq!(run(&h, cx), Run::Starting);
        say(&h, json!({"type": "hello", "protocol": 1}), cx);
        assert_eq!(sent(&h, cx).last(), Some(&json!({"type": "open", "thread": "t1"})), "open threads are opened again");

        // A wrong protocol stops it, saying what to do.
        h.update(cx, |h, cx| h.restart("inbox", cx));
        cx.run_until_parked();
        say(&h, json!({"type": "hello", "protocol": 2}), cx);
        assert!(matches!(run(&h, cx), Run::Stopped(why) if why.contains("Update the extension or Pitwall")));

        // Off: stopped.
        cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("inbox", false)));
        assert_eq!(run(&h, cx), Run::Off);
    }

    /// No hello within the time: stopped with why.
    #[gpui::test]
    fn a_silent_adapter_is_stopped(cx: &mut TestAppContext) {
        let h = setup(cx);
        cx.update(|cx| {
            let store = crate::extensions::secret_store(cx);
            let acct = crate::extensions::account("inbox", "apiKey", cx);
            store.set(&acct, "k").unwrap();
            crate::ui_state::update(cx, |u| {
                *u = std::mem::take(u).set_extension("inbox", true).with_extension("inbox", |e| {
                    e.values.insert("server".into(), "https://x.example".into());
                    e.secrets_set.push("apiKey".into());
                })
            });
        });
        assert_eq!(run(&h, cx), Run::Starting);
        cx.executor().advance_clock(HELLO_WITHIN + Duration::from_secs(1));
        assert!(matches!(run(&h, cx), Run::Stopped(why) if why.contains("didn't answer within 10 seconds")));
    }
}
