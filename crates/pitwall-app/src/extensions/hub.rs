//! Every extension's adapters and what they said (docs/spec/extensions.md,
//! profiles.md): one [`Hub`] for the app, which windows and Settings
//! observe. An extension with settings runs one adapter per configured
//! connection (`pitwall_core::accounts::picks`), at most [`MAX_RUNNING`];
//! one without settings (or with no connection made yet) runs one. The hub
//! starts an adapter when its extension is on and its connection's
//! required settings are set, starts it again when they change, stops it
//! when the extension is turned off, the connection is removed, or Pitwall
//! quits, and keeps each adapter's state (connection, threads, open
//! threads, how requests went).
//!
//! What views read is one state per extension ([`Hub::get`]): all its
//! connections' threads in one list, their ids `<connection id>:<id>`
//! (as the adapter's own for an extension without connections), each
//! labelled with its connection's name ([`ExtState::label`]); counts added
//! up; a connection that stopped or can't reach its service shows as a
//! note ([`ExtState::conn`]) without hiding the others' threads. Requests
//! on a thread go to its connection's adapter.
//!
//! An adapter that stops, crashes or floods stays stopped with its reason
//! shown ([`Run::Stopped`]) until the user restarts it from Settings or its
//! inbox tile. A thread with an ask it didn't have is told to watchers
//! ([`HubEvent::NewAsk`]: the notification an agent that needs you gets).

use std::collections::{BTreeMap, HashMap, HashSet};
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

/// How many of an extension's connections run an adapter at once.
pub const MAX_RUNNING: usize = 8;

/// Where an adapter (or, merged, an extension's adapters) is.
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

/// One connection of an extension, as its adapter is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnView {
    /// The connection's id (empty: the extension's only adapter, without
    /// connections).
    pub id: String,
    pub name: String,
    pub run: Run,
    pub conn: Option<(ConnState, String)>,
}

impl ConnView {
    /// It can't show its threads: stopped, signed out or unreachable.
    pub fn offline(&self) -> bool {
        matches!(self.run, Run::Stopped(_)) || matches!(self.conn, Some((ConnState::SignedOut | ConnState::Error, _)))
    }
}

/// An adapter's state, or (from [`Hub::get`]) an extension's, merged.
#[derive(Default)]
pub struct ExtState {
    pub run: Run,
    /// What its adapter last said about its connection (merged: the
    /// offline connections, named, when it has several).
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
    /// Merged: each connection and how its adapter is.
    pub connections: Vec<ConnView>,
    /// Merged: thread id → its connection's name.
    labels: HashMap<String, String>,
    link: Option<Link>,
    /// Which start this state is of (events of older ones are dropped).
    gen: u64,
    fingerprint: Option<u64>,
    /// A list arrived since it started (the first one is no news).
    listed: bool,
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

    /// What there is to draw, copied out.
    pub fn snapshot(&self) -> ExtState {
        ExtState {
            run: self.run.clone(),
            conn: self.conn.clone(),
            threads: self.threads.clone(),
            totals: self.totals,
            details: self.details.clone(),
            notes: self.notes.clone(),
            busy: self.busy.clone(),
            connections: self.connections.clone(),
            labels: self.labels.clone(),
            listed: self.listed,
            done: self.done.clone(),
            ..Default::default()
        }
    }

    /// A list arrived since it started (else it is still loading).
    pub fn listed(&self) -> bool {
        self.listed
    }

    /// How request `req` went (`None`: not yet, or long ago).
    pub fn result(&self, req: u64) -> Option<bool> {
        self.done.iter().find(|(r, _)| *r == req).map(|(_, ok)| *ok)
    }

    /// The name of the connection thread `id` is from (merged state; none
    /// for an extension without connections).
    pub fn label(&self, id: &str) -> Option<&str> {
        self.labels.get(id).map(String::as_str)
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

/// What the hub tells its watchers.
#[derive(Debug, Clone, PartialEq)]
pub enum HubEvent {
    /// A thread has an ask it didn't have (not on the first list); its id
    /// as views know it.
    NewAsk { ext: String, thread: Thread },
}

impl gpui::EventEmitter<HubEvent> for Hub {}

/// The threads of `next` with more open asks than in `prev`.
pub fn new_asks(prev: &[Thread], next: &[Thread]) -> Vec<Thread> {
    next.iter()
        .filter(|t| t.open_asks > prev.iter().find(|p| p.id == t.id).map_or(0, |p| p.open_asks))
        .cloned()
        .collect()
}

/// One adapter: (extension id, connection id; empty without connections).
type Key = (String, String);

/// Thread `id` of connection `conn` as views know it.
pub fn thread_id(conn: &str, id: &str) -> String {
    if conn.is_empty() {
        id.to_string()
    } else {
        format!("{conn}:{id}")
    }
}

/// A view's thread id of extension `m`: (connection, the adapter's id).
/// Connections' ids start with `ext-` and have no `:`.
pub fn split_thread<'a>(m: &Manifest, id: &'a str) -> (&'a str, &'a str) {
    if m.settings.is_empty() {
        return ("", id);
    }
    match id.split_once(':') {
        Some((c, rest)) if c.starts_with("ext-") => (c, rest),
        _ => ("", id),
    }
}

/// What one adapter runs with.
struct Target {
    conn: String,
    name: String,
    values: BTreeMap<String, String>,
    secrets_set: Vec<String>,
    secrets_name: String,
}

pub struct Hub {
    runs: HashMap<Key, ExtState>,
    /// Per extension: what views read.
    merged: HashMap<String, ExtState>,
    /// Per extension: the adapters it runs now, in order.
    targets: HashMap<String, Vec<(String, String)>>,
    /// Per extension without any adapter wanted: why (off, setup).
    idle: HashMap<String, Run>,
    next_req: u64,
    next_gen: u64,
    /// Bumped when a connection's secret is changed (it starts again).
    secrets_gen: HashMap<Key, u64>,
    /// Tests: the other ends of the adapters' links, by extension id
    /// (`<ext>/<connection>` for a connection).
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
        runs: HashMap::new(),
        merged: HashMap::new(),
        targets: HashMap::new(),
        idle: HashMap::new(),
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

/// What extension `m` needs with plain settings `values` and secrets
/// `secrets_set`: `Err(what to do)` when a required setting isn't set.
pub fn missing(m: &Manifest, values: &BTreeMap<String, String>, secrets_set: &[String]) -> Result<(), String> {
    let gone: Vec<&str> = m
        .settings
        .iter()
        .filter(|s| s.required)
        .filter(|s| match s.kind {
            SettingKind::Url => values.get(&s.key).is_none_or(|v| v.trim().is_empty()),
            SettingKind::Secret => !secrets_set.contains(&s.key),
        })
        .map(|s| s.label.as_str())
        .collect();
    if gone.is_empty() {
        return Ok(());
    }
    Err(format!("Set {} in Settings → Extensions.", gone.join(" and ")))
}

#[cfg(test)]
fn end_key(k: &Key) -> String {
    if k.1.is_empty() {
        k.0.clone()
    } else {
        format!("{}/{}", k.0, k.1)
    }
}

impl Hub {
    /// Extension `id`'s state: all its connections merged.
    pub fn get(&self, id: &str) -> Option<&ExtState> {
        self.merged.get(id)
    }

    /// One adapter's own state (connection `conn`; empty without
    /// connections).
    pub fn connection(&self, ext: &str, conn: &str) -> Option<&ExtState> {
        self.runs.get(&(ext.to_string(), conn.to_string()))
    }

    /// The adapters each enabled extension of `found` wants, with the
    /// accounts in `list`.
    fn targets_of(m: &Manifest, list: &pitwall_core::accounts::Accounts) -> Vec<Target> {
        let conns: Vec<_> = if m.settings.is_empty() { vec![] } else { list.connections(&m.id).collect() };
        if conns.is_empty() {
            return vec![Target { conn: String::new(), name: m.name.clone(), values: BTreeMap::new(), secrets_set: vec![], secrets_name: m.id.clone() }];
        }
        conns
            .into_iter()
            .map(|c| Target { conn: c.id.clone(), name: c.name.clone(), values: c.values.clone(), secrets_set: c.secrets_set.clone(), secrets_name: c.secrets_name().to_string() })
            .collect()
    }

    /// Start, stop or start again each adapter to match its extension's
    /// switch, its connections and their settings.
    pub fn reconcile(&mut self, cx: &mut Context<Self>) {
        let found = super::found(cx);
        let ui = crate::ui_state::get(cx);
        let dir = super::dir(cx);
        let list = crate::accounts::model(cx).read(cx).list.clone();
        let mut wanted: HashSet<Key> = HashSet::new();
        self.targets.clear();
        self.idle.clear();
        for m in found.iter().filter_map(|f| f.manifest.as_ref().ok()) {
            if m.adapter.is_none() || !ui.extension_on(&m.id) {
                self.idle.insert(m.id.clone(), Run::Off);
                continue;
            }
            let mut order = Vec::new();
            let mut running = 0;
            for t in Self::targets_of(m, &list) {
                let key: Key = (m.id.clone(), t.conn.clone());
                order.push((t.conn.clone(), t.name.clone()));
                wanted.insert(key.clone());
                let gen_now = self.secrets_gen.get(&key).copied();
                let st = self.runs.entry(key.clone()).or_default();
                match missing(m, &t.values, &t.secrets_set) {
                    Err(why) => {
                        Self::stop(st);
                        st.run = Run::Setup(why);
                        st.fingerprint = None;
                    }
                    Ok(()) if running >= MAX_RUNNING => {
                        Self::stop(st);
                        st.run = Run::Setup(format!("At most {MAX_RUNNING} connections of an extension run at once."));
                        st.fingerprint = None;
                    }
                    Ok(()) => {
                        running += 1;
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        (m.adapter.as_ref().map(|a| &a.command), &m.settings.iter().map(|s| (&s.key, t.values.get(&s.key))).collect::<Vec<_>>(), &t.secrets_set, &t.secrets_name, gen_now).hash(&mut h);
                        let fp = h.finish();
                        if st.fingerprint != Some(fp) {
                            st.fingerprint = Some(fp);
                            self.start(m, &key, &t, dir.clone(), cx);
                        }
                    }
                }
            }
            self.targets.insert(m.id.clone(), order);
        }
        for (key, st) in self.runs.iter_mut() {
            if !wanted.contains(key) {
                Self::stop(st);
                st.run = Run::Off;
                st.fingerprint = None;
            }
        }
        // A connection that is gone keeps nothing (an extension turned off
        // keeps its open threads for when it is on again).
        let targets = &self.targets;
        self.runs.retain(|k, _| k.1.is_empty() || !targets.contains_key(&k.0) || wanted.contains(k));
        let exts: HashSet<String> = self.runs.keys().map(|k| k.0.clone()).chain(self.idle.keys().cloned()).chain(self.targets.keys().cloned()).collect();
        for ext in exts {
            self.merge(&ext);
        }
        cx.notify();
    }

    /// Rebuild what views read for extension `ext`.
    fn merge(&mut self, ext: &str) {
        let order = self.targets.get(ext).cloned().unwrap_or_default();
        let mut out = ExtState::default();
        if order.is_empty() {
            out.run = self.idle.get(ext).cloned().unwrap_or_default();
            self.merged.insert(ext.to_string(), out);
            return;
        }
        let several = order.len() > 1;
        let mut sum = Counts::default();
        let mut any_totals = false;
        for (c, name) in &order {
            let Some(st) = self.runs.get(&(ext.to_string(), c.clone())) else { continue };
            out.connections.push(ConnView { id: c.clone(), name: name.clone(), run: st.run.clone(), conn: st.conn.clone() });
            let counts = st.counts();
            sum.unread += counts.unread;
            sum.open_asks += counts.open_asks;
            sum.blocking += counts.blocking;
            any_totals |= st.totals.is_some();
            for t in &st.threads {
                let id = thread_id(c, &t.id);
                if !c.is_empty() {
                    out.labels.insert(id.clone(), name.clone());
                }
                out.threads.push(Thread { id, ..t.clone() });
            }
            for (t, d) in &st.details {
                let id = thread_id(c, t);
                out.details.insert(id.clone(), ThreadDetail { id, ..d.clone() });
            }
            out.notes.extend(st.notes.iter().map(|(t, n)| (thread_id(c, t), n.clone())));
            out.busy.extend(st.busy.iter().map(|t| thread_id(c, t)));
            out.done.extend(st.done.iter().copied());
            out.listed |= st.listed;
        }
        let views = &out.connections;
        out.totals = any_totals.then_some(sum);
        out.listed |= !views.iter().any(|v| matches!(v.run, Run::Running | Run::Starting));
        // How it is: the best any connection is; the offline ones named.
        let pick = |want: fn(&Run) -> bool| views.iter().find(|v| want(&v.run));
        let named = |v: &ConnView, why: &str| if several { format!("{}: {why}", v.name) } else { why.to_string() };
        out.run = if pick(|r| *r == Run::Running).is_some() {
            Run::Running
        } else if pick(|r| *r == Run::Starting).is_some() {
            Run::Starting
        } else if let Some(v) = pick(|r| matches!(r, Run::Stopped(_))) {
            let Run::Stopped(why) = &v.run else { unreachable!() };
            Run::Stopped(named(v, why))
        } else if let Some(v) = pick(|r| matches!(r, Run::Setup(_))) {
            let Run::Setup(why) = &v.run else { unreachable!() };
            Run::Setup(named(v, why))
        } else {
            Run::Off
        };
        out.conn = if !several {
            views.first().and_then(|v| v.conn.clone())
        } else {
            let off: Vec<&ConnView> = views.iter().filter(|v| v.offline()).collect();
            if off.is_empty() {
                views.iter().find(|v| v.run == Run::Running).and_then(|v| v.conn.clone())
            } else {
                let state = if off.iter().all(|v| matches!(v.conn, Some((ConnState::SignedOut, _)))) { ConnState::SignedOut } else { ConnState::Error };
                let text = off
                    .iter()
                    .map(|v| match (&v.run, &v.conn) {
                        (Run::Stopped(why), _) => format!("{} stopped: {why}", v.name),
                        (_, Some((ConnState::SignedOut, m))) if !m.is_empty() => format!("{} is signed out: {m}", v.name),
                        (_, Some((ConnState::SignedOut, _))) => format!("{} is signed out.", v.name),
                        (_, Some((_, m))) if !m.is_empty() => format!("{} can't connect: {m}", v.name),
                        _ => format!("{} can't connect.", v.name),
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                Some((state, text))
            }
        };
        self.merged.insert(ext.to_string(), out);
    }

    /// Start extension `id`'s adapters again (after they stopped).
    pub fn restart(&mut self, id: &str, cx: &mut Context<Self>) {
        for (k, st) in self.runs.iter_mut() {
            if k.0 == id {
                Self::stop(st);
                st.fingerprint = None;
            }
        }
        self.reconcile(cx);
    }

    /// Connection `conn`'s secrets changed (empty: an extension without
    /// connections): its adapter starts again.
    pub fn secrets_changed(&mut self, ext: &str, conn: &str, cx: &mut Context<Self>) {
        self.bump_secrets(ext, conn);
        self.reconcile(cx);
    }

    /// Connection `conn`'s secrets changed: its adapter starts again at
    /// the next [`Hub::reconcile`] (a change to the accounts brings one).
    pub fn bump_secrets(&mut self, ext: &str, conn: &str) {
        *self.secrets_gen.entry((ext.to_string(), conn.to_string())).or_default() += 1;
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
        for st in self.runs.values_mut() {
            if let Some(l) = st.link.take() {
                l.stop_now();
            }
        }
    }

    fn start(&mut self, m: &Manifest, key: &Key, t: &Target, dir: Option<std::path::PathBuf>, cx: &mut Context<Self>) {
        let gen = self.next_gen;
        self.next_gen += 1;
        let st = self.runs.entry(key.clone()).or_default();
        Self::stop(st);
        st.run = Run::Starting;
        st.gen = gen;
        st.threads.clear();
        st.listed = false;
        st.totals = None;
        st.details.clear();
        st.notes.clear();
        let mut settings = Map::new();
        for s in m.settings.iter().filter(|s| s.kind == SettingKind::Url) {
            if let Some(v) = t.values.get(&s.key).filter(|v| !v.trim().is_empty()) {
                settings.insert(s.key.clone(), Value::String(v.clone()));
            }
        }
        let secret_keys: Vec<String> = m.settings.iter().filter(|s| s.kind == SettingKind::Secret && t.secrets_set.contains(&s.key)).map(|s| s.key.clone()).collect();
        let store = super::secret_store(cx);
        let accounts: Vec<(String, String)> = secret_keys.iter().map(|k| (k.clone(), super::account(&t.secrets_name, k, cx))).collect();
        let command = m.adapter.as_ref().map(|a| a.command.clone()).unwrap_or_default();
        let folder = dir.map(|d| d.join(&m.id));
        let key = key.clone();
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
            let rx = this.update(cx, |h, cx| h.attach(&key, gen, res, cx)).ok().flatten();
            let Some(mut rx) = rx else { return };
            let timer_key = key.clone();
            let t = this.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(HELLO_WITHIN).await;
                let _ = t.update(cx, |h, cx| h.hello_late(&timer_key, gen, cx));
            })
            .detach();
            while let Some(ev) = rx.next().await {
                if this.update(cx, |h, cx| h.event(&key, gen, ev, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// The adapter started (or not): keep its link; its events' channel.
    fn attach(&mut self, key: &Key, gen: u64, res: Result<Opened, String>, cx: &mut Context<Self>) -> Option<UnboundedReceiver<Event>> {
        let st = self.runs.get_mut(key)?;
        let opened = match res {
            Ok(o) => o,
            Err(why) => {
                if st.gen == gen {
                    st.run = Run::Stopped(why);
                    st.gen = 0;
                }
                self.merge(&key.0);
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
            self.ends.insert(end_key(key), end);
        }
        cx.notify();
        Some(opened.events)
    }

    fn hello_late(&mut self, key: &Key, gen: u64, cx: &mut Context<Self>) {
        let Some(st) = self.runs.get_mut(key) else { return };
        if st.gen == gen && !st.hello {
            Self::stop(st);
            st.run = Run::Stopped(format!("It didn't answer within {} seconds.", HELLO_WITHIN.as_secs()));
            self.merge(&key.0);
            cx.notify();
        }
    }

    fn event(&mut self, key: &Key, gen: u64, ev: Event, cx: &mut Context<Self>) {
        let Some(st) = self.runs.get_mut(key) else { return };
        if st.gen != gen {
            return;
        }
        let mut news = Vec::new();
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
            Event::Msg(FromAdapter::Threads(list)) => {
                if st.listed {
                    news = new_asks(&st.threads, &list);
                }
                st.listed = true;
                st.threads = list;
            }
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
        self.merge(&key.0);
        for t in news {
            cx.emit(HubEvent::NewAsk { ext: key.0.clone(), thread: Thread { id: thread_id(&key.1, &t.id), ..t } });
        }
        cx.notify();
    }

    /// The adapter a view's thread id is of, and the adapter's own id.
    fn route(&self, ext: &str, thread: &str) -> (Key, String) {
        if let Some((c, rest)) = thread.split_once(':') {
            let key = (ext.to_string(), c.to_string());
            if !c.is_empty() && self.runs.contains_key(&key) {
                return (key, rest.to_string());
            }
        }
        ((ext.to_string(), String::new()), thread.to_string())
    }

    /// A view of `thread` opened: the adapter sends it and keeps it current.
    pub fn open(&mut self, id: &str, thread: &str, cx: &mut Context<Self>) {
        let (key, inner) = self.route(id, thread);
        let st = self.runs.entry(key.clone()).or_default();
        let n = st.open.entry(inner.clone()).or_default();
        *n += 1;
        if *n == 1 && st.hello {
            if let Some(l) = &st.link {
                l.send(&ToAdapter::Open { thread: inner });
            }
        }
        self.merge(&key.0);
        cx.notify();
    }

    /// A view of `thread` closed.
    pub fn close(&mut self, id: &str, thread: &str) {
        let (key, inner) = self.route(id, thread);
        let Some(st) = self.runs.get_mut(&key) else { return };
        let Some(n) = st.open.get_mut(&inner) else { return };
        *n -= 1;
        if *n == 0 {
            st.open.remove(&inner);
            st.details.remove(&inner);
            if let (true, Some(l)) = (st.hello, &st.link) {
                l.send(&ToAdapter::Close { thread: inner });
            }
        }
        self.merge(&key.0);
    }

    /// Send `r` on `thread`: its request number, or why not.
    pub fn request(&mut self, id: &str, thread: &str, r: Request, cx: &mut Context<Self>) -> Result<u64, String> {
        let (key, inner) = self.route(id, thread);
        let st = self.runs.get_mut(&key).ok_or("The extension isn't running.")?;
        let link = st.link.as_ref().filter(|_| st.hello).ok_or("The extension isn't running.")?;
        let req = self.next_req;
        let thread_s = inner.clone();
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
        st.pending.insert(req, inner.clone());
        st.busy.insert(inner.clone());
        st.notes.remove(&inner);
        self.merge(&key.0);
        cx.notify();
        Ok(req)
    }

    /// Why something on `thread` didn't go (shown in it until the next
    /// request on it).
    pub fn set_note(&mut self, id: &str, thread: &str, why: String, cx: &mut Context<Self>) {
        let (key, inner) = self.route(id, thread);
        if let Some(st) = self.runs.get_mut(&key) {
            st.notes.insert(inner, why);
            self.merge(&key.0);
            cx.notify();
        }
    }

    /// Ask every adapter of extension `id` for its list again.
    pub fn refresh(&mut self, id: &str) {
        for (k, st) in &self.runs {
            if k.0 == id && st.hello {
                if let Some(l) = &st.link {
                    l.send(&ToAdapter::Refresh);
                }
            }
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
mod tests;
