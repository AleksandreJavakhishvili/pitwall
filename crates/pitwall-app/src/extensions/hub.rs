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
//!
//! Each adapter keeps a log for its author ([`Hub::log`]): the last 500
//! lines both ways and its stderr, secrets taken out, across its restarts
//! until its extension is turned off.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt;
use gpui::{App, AppContext, Context, Entity, Global};
use pitwall_core::extensions::log::{self, Line};
use pitwall_core::extensions::protocol::{negotiate, Call, CallError, ConnState, Connection, Counts, FromAdapter, FromInbox, Hello, Thread, ThreadDetail, ToAdapter, ToInbox, MAX_REPLY, PROTOCOL_2};
use pitwall_core::extensions::view::{Notify, Root, RowBadge};
use pitwall_core::extensions::{Manifest, SettingKind};
use serde_json::{Map, Value};

use super::adapter::{Event, Link};
use super::calls;

pub mod views;

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
    /// Its own unread threads, open asks and blocking asks (its totals,
    /// else its list counted): for a per-connection filter.
    pub counts: Counts,
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
    /// Merged: thread id → its connection (id, name).
    labels: HashMap<String, (String, String)>,
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
    /// The protocol its hello answered with (0: none yet).
    protocol: u64,
    /// Its calls waiting for their answer.
    in_flight: usize,
    /// Its calls refused for its limits in a row.
    strikes: u32,
    /// Its events (protocol 2, after `events.subscribe`).
    events: Option<Arc<pitwall_daemon::events::Queue>>,
    /// The permissions its adapter was last told it has.
    told: Vec<String>,
    /// Its process, as the socket's callers are told by.
    pid: Option<u32>,
    reg: Option<Arc<pitwall_daemon::Registry>>,
    /// Merged: each view the extension's adapters sent ([`views`]).
    pub views: HashMap<String, views::ViewData>,
    /// Merged: its sidebar badges and notes, each with its connection.
    pub badges: Vec<(String, RowBadge)>,
    /// Why the last action in a view failed, by view.
    pub view_notes: HashMap<String, String>,
    /// Actions in flight: (view, item as views know it, or "").
    pub view_busy: HashSet<(String, String)>,
    /// An adapter's own views, as last sent.
    raw_views: HashMap<String, Result<Root, String>>,
    /// Actions in flight: their view and item.
    view_pending: HashMap<u64, (String, String)>,
    /// Commands in flight: their title.
    commands: HashMap<u64, String>,
    /// When its badges were last applied, and a newer set waiting.
    badges_at: Option<Instant>,
    badges_wait: Option<Vec<RowBadge>>,
    /// What its activity last said about badges it may not draw.
    badges_note: String,
    /// Merged: the address each web view's adapter last gave (or why it
    /// can't be opened).
    pub web_urls: HashMap<String, Result<String, String>>,
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
            views: self.views.clone(),
            badges: self.badges.clone(),
            view_notes: self.view_notes.clone(),
            view_busy: self.view_busy.clone(),
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
        self.labels.get(id).map(|(_, n)| n.as_str())
    }

    /// The connection thread `id` is from: (its id, its name) (merged
    /// state; none for an extension without connections).
    pub fn connection_of(&self, id: &str) -> Option<(&str, &str)> {
        self.labels.get(id).map(|(c, n)| (c.as_str(), n.as_str()))
    }

    /// The threads of connection `conn` (`None`: all).
    pub fn threads_of<'a>(&'a self, conn: Option<&'a str>) -> impl Iterator<Item = &'a Thread> + 'a {
        self.threads.iter().filter(move |t| conn.is_none_or(|c| self.connection_of(&t.id).is_some_and(|(id, _)| id == c)))
    }

    /// Connection `conn`'s counts (`None`: the whole inbox's).
    pub fn counts_of(&self, conn: Option<&str>) -> Counts {
        match conn {
            None => self.counts(),
            Some(c) => self.connections.iter().find(|v| v.id == c).map(|v| v.counts).unwrap_or_default(),
        }
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
    /// An adapter asks for a notification, within its limits and granted
    /// (`open`'s ids as views know them).
    Notify { ext: String, notify: Notify },
    /// A command picked in the palette was answered (`message`: the
    /// adapter's words).
    CommandDone { ext: String, title: String, ok: bool, message: String },
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
    /// Each adapter's log, kept while its extension is on.
    logs: HashMap<Key, log::Shared>,
    /// Per extension: when its last notifications were shown (the last
    /// minute's), and each id's last one (when, and what it said).
    notified: HashMap<String, VecDeque<Instant>>,
    notified_ids: HashMap<(String, String), (Instant, String)>,
    /// Per extension: its views shown in tiles, and in how many (each
    /// adapter is sent `show` after its hello).
    shown: HashMap<String, HashMap<String, usize>>,
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
        logs: HashMap::new(),
        notified: HashMap::new(),
        notified_ids: HashMap::new(),
        shown: HashMap::new(),
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
        .filter(|s| !s.is_set(values.get(&s.key).map(String::as_str), secrets_set.contains(&s.key)))
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

    /// Extension `ext`'s log: each of its adapters' lines with the name of
    /// its connection (empty without connections), oldest first.
    pub fn log(&self, ext: &str) -> Vec<(String, Line)> {
        let names: HashMap<&str, &str> = self.targets.get(ext).map(|t| t.iter().map(|(c, n)| (c.as_str(), n.as_str())).collect()).unwrap_or_default();
        let several = names.len() > 1;
        let mut out: Vec<(String, Line)> = Vec::new();
        for (k, l) in self.logs.iter().filter(|(k, _)| k.0 == ext) {
            let name = if several { names.get(k.1.as_str()).copied().unwrap_or("").to_string() } else { String::new() };
            out.extend(l.lock().unwrap_or_else(|e| e.into_inner()).lines().into_iter().map(|line| (name.clone(), line)));
        }
        out.sort_by_key(|(_, l)| l.at);
        out
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
        let list = crate::accounts::model(cx).read(cx).list.clone();
        let mut wanted: HashSet<Key> = HashSet::new();
        self.targets.clear();
        self.idle.clear();
        for m in found.iter().filter_map(|f| f.manifest.as_ref().ok()) {
            if m.adapter.is_none() || !ui.extension_on(&m.id) {
                self.idle.insert(m.id.clone(), Run::Off);
                // Off: its log is forgotten.
                self.logs.retain(|k, _| k.0 != m.id);
                continue;
            }
            // Where it runs: its development folder, else its installed one.
            let folder = super::folder_of(&m.id, cx);
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
                        (m.adapter.as_ref().map(|a| &a.command), &folder, &m.settings.iter().map(|s| (&s.key, t.values.get(&s.key))).collect::<Vec<_>>(), &t.secrets_set, &t.secrets_name, gen_now).hash(&mut h);
                        let fp = h.finish();
                        if st.fingerprint != Some(fp) {
                            st.fingerprint = Some(fp);
                            self.start(m, &key, &t, folder.clone(), cx);
                        }
                    }
                }
            }
            self.targets.insert(m.id.clone(), order);
            // A grant or a revoke: its protocol 2 adapters are told.
            let grants = calls::effective(m, &ui.extension(&m.id).grants);
            for st in self.runs.iter_mut().filter(|(k, _)| k.0 == m.id).map(|(_, st)| st) {
                if st.hello && st.protocol >= PROTOCOL_2 && st.told != grants {
                    if let Some(l) = &st.link {
                        l.send(&ToAdapter::Grants(grants.clone()));
                    }
                    st.told = grants.clone();
                }
            }
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
            let counts = st.counts();
            out.connections.push(ConnView { id: c.clone(), name: name.clone(), run: st.run.clone(), conn: st.conn.clone(), counts });
            sum.unread += counts.unread;
            sum.open_asks += counts.open_asks;
            sum.blocking += counts.blocking;
            any_totals |= st.totals.is_some();
            for t in &st.threads {
                let id = thread_id(c, &t.id);
                if !c.is_empty() {
                    out.labels.insert(id.clone(), (c.clone(), name.clone()));
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
            out.badges.extend(st.badges.iter().cloned());
            for (v, n) in &st.view_notes {
                out.view_notes.entry(v.clone()).or_insert_with(|| n.clone());
            }
            out.view_busy.extend(st.view_busy.iter().map(|(v, i)| (v.clone(), if i.is_empty() { String::new() } else { thread_id(c, i) })));
            for (w, u) in &st.web_urls {
                out.web_urls.entry(w.clone()).or_insert_with(|| u.clone());
            }
        }
        let parts: Vec<(&str, &str, &HashMap<String, views::ViewData>)> = order.iter().filter_map(|(c, name)| self.runs.get(&(ext.to_string(), c.clone())).filter(|st| st.run == Run::Running).map(|st| (c.as_str(), name.as_str(), &st.raw_views))).collect();
        out.views = views::merge(&parts);
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
        if let Some(q) = st.events.take() {
            q.close();
        }
        if let (Some(pid), Some(reg)) = (st.pid.take(), &st.reg) {
            reg.adapter_stopped(pid);
        }
        st.gen = 0;
        st.hello = false;
        st.protocol = 0;
        st.in_flight = 0;
        st.strikes = 0;
        st.told.clear();
        st.conn = None;
        st.busy.clear();
        st.pending.clear();
        // What it drew goes with it: its views, badges and notes.
        st.raw_views.clear();
        st.badges.clear();
        st.badges_wait = None;
        st.badges_at = None;
        st.view_pending.clear();
        st.view_busy.clear();
        st.view_notes.clear();
        st.commands.clear();
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

    fn start(&mut self, m: &Manifest, key: &Key, t: &Target, folder: Option<std::path::PathBuf>, cx: &mut Context<Self>) {
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
        for s in m.settings.iter().filter(|s| s.kind != SettingKind::Secret) {
            if let Some(v) = s.json(t.values.get(&s.key).map(String::as_str)) {
                settings.insert(s.key.clone(), v);
            }
        }
        let grants = calls::effective(m, &crate::ui_state::get(cx).extension(&m.id).grants);
        let connection = (!t.conn.is_empty()).then(|| Connection { id: t.conn.clone(), name: t.name.clone() });
        let reg = calls::api(cx).registry;
        st.reg = Some(reg);
        st.told = grants.clone();
        let secret_keys: Vec<String> = m.settings.iter().filter(|s| s.kind == SettingKind::Secret && t.secrets_set.contains(&s.key)).map(|s| s.key.clone()).collect();
        let store = super::secret_store(cx);
        let accounts: Vec<(String, String)> = secret_keys.iter().map(|k| (k.clone(), super::account(&t.secrets_name, k, cx))).collect();
        let command = m.adapter.as_ref().map(|a| a.command.clone()).unwrap_or_default();
        let log = self.logs.entry(key.clone()).or_default().clone();
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
            // Its log takes out these values wherever they show.
            log.lock().unwrap_or_else(|e| e.into_inner()).set_secrets(secrets.values().filter_map(|v| v.as_str().map(str::to_string)).collect());
            let hello = ToAdapter::Hello(Hello { pitwall: pitwall_core::extensions::PITWALL.to_string(), settings, secrets, connection, grants });
            open_link(folder, &command, &hello, log)
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
                    if let Some(l) = self.logs.get(key) {
                        l.lock().unwrap_or_else(|e| e.into_inner()).push(log::Dir::Note, &why);
                    }
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
        // Processes it starts call the socket as the extension.
        st.pid = opened.link.pid();
        if let (Some(pid), Some(reg)) = (st.pid, &st.reg) {
            reg.adapter_started(&key.0, pid);
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
        let mut notify = None;
        let mut done_command = None;
        match ev {
            Event::Stopped(why) => {
                Self::stop(st);
                st.run = Run::Stopped(why);
            }
            Event::Msg(FromAdapter::Hello { protocol, .. }) => match negotiate(protocol) {
                Err(why) => {
                    Self::stop(st);
                    st.run = Run::Stopped(why);
                }
                Ok(p) if !st.hello => {
                    st.hello = true;
                    st.protocol = p;
                    st.run = Run::Running;
                    // Threads and views open in tiles from before (a restart).
                    for thread in st.open.keys() {
                        if let Some(l) = &st.link {
                            l.send(&ToAdapter::Inbox(ToInbox::Open { thread: thread.clone() }));
                        }
                    }
                    for view in self.shown.get(&key.0).into_iter().flat_map(|s| s.keys()) {
                        if let Some(l) = &st.link {
                            l.send(&ToAdapter::Show { view: view.clone() });
                        }
                    }
                }
                Ok(_) => {}
            },
            // Nothing before its hello counts.
            Event::Msg(_) if !st.hello => {}
            Event::Msg(FromAdapter::Status { state, message }) => st.conn = Some((state, message)),
            Event::Msg(FromAdapter::Inbox(FromInbox::Threads(list))) => {
                if st.listed {
                    news = new_asks(&st.threads, &list);
                }
                st.listed = true;
                st.threads = list;
            }
            Event::Msg(FromAdapter::Inbox(FromInbox::Counts(c))) => st.totals = Some(c),
            Event::Msg(FromAdapter::Inbox(FromInbox::Thread(d))) => {
                if st.open.contains_key(&d.id) {
                    st.details.insert(d.id.clone(), d);
                }
            }
            Event::Msg(FromAdapter::Call(c)) => {
                if let Some(why) = self.call(key, gen, c, cx) {
                    let st = self.runs.get_mut(key).expect("it is there");
                    Self::stop(st);
                    st.run = Run::Stopped(why);
                }
            }
            Event::Msg(FromAdapter::BadCall { id, why }) => {
                let e = CallError::new("bad_params", why);
                if let Some(reg) = &st.reg {
                    calls::refused(reg, &key.0, "(a bad call)", &e);
                }
                if let Some(l) = &st.link {
                    l.send(&ToAdapter::Reply { id, result: Err(e) });
                }
            }
            Event::Msg(FromAdapter::Result { req, ok, message }) => {
                if let Some((view, item)) = st.view_pending.remove(&req) {
                    st.view_busy.remove(&(view.clone(), item));
                    if ok {
                        st.view_notes.remove(&view);
                    } else {
                        st.view_notes.insert(view, if message.is_empty() { "That didn't work.".into() } else { message.clone() });
                    }
                }
                if let Some(title) = st.commands.remove(&req) {
                    done_command = Some((title, ok, message.clone()));
                }
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
            Event::Msg(FromAdapter::View(v)) => {
                let m = super::manifest_of(&key.0, cx);
                // A view is never sent for an agent: that was a right-panel
                // section, which Pitwall no longer draws.
                let declared = v.agent_id.is_none() && m.as_ref().is_some_and(|m| m.contributes.view(&v.id).is_some());
                let shown = self.shown.get(&key.0).is_some_and(|s| s.contains_key(&v.id));
                let st = self.runs.get_mut(key).expect("it is there");
                if !declared {
                    let e = match v.agent_id {
                        Some(_) => CallError::new("unknown", format!("It sent a section “{}” for an agent: Pitwall no longer has a right panel to draw it in.", v.id)),
                        None => CallError::new("unknown", format!("It sent view “{}”, which its manifest doesn't contribute.", v.id)),
                    };
                    if let Some(reg) = &st.reg {
                        calls::refused(reg, &key.0, "view", &e);
                    }
                } else if shown {
                    // Only views in a tile are kept; a hidden one is sent
                    // again when shown.
                    st.raw_views.insert(v.id, v.root);
                }
            }
            Event::Msg(FromAdapter::WebView { view, url }) => {
                let declared = super::manifest_of(&key.0, cx).is_some_and(|m| m.contributes.web_view(&view).is_some());
                let st = self.runs.get_mut(key).expect("it is there");
                if !declared {
                    let e = CallError::new("unknown", format!("It sent web view “{view}”, which its manifest doesn't contribute."));
                    if let Some(reg) = &st.reg {
                        calls::refused(reg, &key.0, "webView", &e);
                    }
                } else {
                    if let (Err(why), Some(reg)) = (&url, &st.reg) {
                        calls::refused(reg, &key.0, "webView", &CallError::new("bad_params", why.clone()));
                    }
                    st.web_urls.insert(view, url);
                }
            }
            Event::Msg(FromAdapter::Badges(list)) => self.badges(key, list, cx),
            Event::Msg(FromAdapter::Notify(n)) => notify = Some(n),
            Event::Msg(FromAdapter::Unknown) => {}
        }
        if let Some(n) = notify {
            self.notify(key, n, cx);
        }
        self.merge(&key.0);
        for t in news {
            cx.emit(HubEvent::NewAsk { ext: key.0.clone(), thread: Thread { id: thread_id(&key.1, &t.id), ..t } });
        }
        if let Some((title, ok, message)) = done_command {
            cx.emit(HubEvent::CommandDone { ext: key.0.clone(), title, ok, message });
        }
        cx.notify();
    }

    /// Adapter `key` sent its badges: those it may draw are kept, at most
    /// one set every [`views::BADGES_EVERY`] (a newer one waits).
    fn badges(&mut self, key: &Key, list: Vec<RowBadge>, cx: &mut Context<Self>) {
        let Some(st) = self.runs.get_mut(key) else { return };
        let now = Instant::now();
        if let Some(at) = st.badges_at.filter(|at| now.duration_since(*at) < views::BADGES_EVERY) {
            let waiting = st.badges_wait.replace(list).is_some();
            if !waiting {
                let (key, gen, wait) = (key.clone(), st.gen, views::BADGES_EVERY - now.duration_since(at));
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = this.update(cx, |h, cx| {
                        let Some(st) = h.runs.get_mut(&key).filter(|st| st.gen == gen) else { return };
                        let Some(list) = st.badges_wait.take() else { return };
                        st.badges_at = Some(Instant::now());
                        h.apply_badges(&key, list, cx);
                        h.merge(&key.0);
                        cx.notify();
                    });
                })
                .detach();
            }
            return;
        }
        st.badges_at = Some(now);
        self.apply_badges(key, list, cx);
    }

    /// Adapter `key`'s badges kept: those its extension may draw.
    fn apply_badges(&mut self, key: &Key, list: Vec<RowBadge>, cx: &mut Context<Self>) {
        let m = super::manifest_of(&key.0, cx);
        let grants = m.as_ref().map(|m| calls::effective(m, &crate::ui_state::get(cx).extension(&m.id).grants)).unwrap_or_default();
        let st = self.runs.get_mut(key).expect("it is there");
        let Some(m) = m.filter(|m| m.contributes.badges) else {
            if !list.is_empty() && st.badges_note.is_empty() {
                st.badges_note = "badges".into();
                if let Some(reg) = &st.reg {
                    calls::refused(reg, &key.0, "badges", &CallError::new("unknown", "It sent badges, but its manifest doesn't contribute \"badges\"."));
                }
            }
            st.badges.clear();
            return;
        };
        let mut kept = Vec::new();
        let mut why = None;
        for mut b in list {
            match views::allowed(&m, &grants, &b) {
                Ok(()) => {
                    b.open = b.open.map(|o| views::open_as_seen(&key.1, o));
                    kept.push((key.1.clone(), b));
                }
                Err(e) => why = Some(e),
            }
        }
        // Each refusal once, until it changes.
        let note = why.clone().unwrap_or_default();
        if st.badges_note != note {
            if let (Some(why), Some(reg)) = (why, &st.reg) {
                calls::refused(reg, &key.0, "badges", &CallError::new("denied", format!("Some of its badges weren't shown: {why}.")));
            }
            st.badges_note = note;
        }
        st.badges = kept;
    }

    /// Adapter `key` asks for notification `n`: shown when its extension
    /// contributes notifications and is granted `notify`, at most
    /// [`views::NOTIFY_PER_MINUTE`] a minute, the same one not again.
    fn notify(&mut self, key: &Key, n: Notify, cx: &mut Context<Self>) {
        use pitwall_core::extensions::activity::{Entry, Outcome, Via};
        let Some(reg) = self.runs.get(key).and_then(|st| st.reg.clone()) else { return };
        let ext = key.0.clone();
        let refuse = |code: &str, why: String| calls::refused(&reg, &ext, "notify", &CallError::new(code, why));
        let Some(m) = super::manifest_of(&ext, cx) else { return };
        if !m.contributes.notifications {
            return refuse("unknown", "Its manifest doesn't contribute \"notifications\".".into());
        }
        let grants = calls::effective(&m, &crate::ui_state::get(cx).extension(&m.id).grants);
        if !grants.iter().any(|g| g == pitwall_core::extensions::permissions::NOTIFY) {
            return refuse("denied", "It isn't granted \"notify\".".into());
        }
        let now = Instant::now();
        let said = format!("{}\n{}", n.title, n.body);
        let id = (ext.clone(), thread_id(&key.1, &n.id));
        if self.notified_ids.get(&id).is_some_and(|(at, s)| *s == said && now.duration_since(*at) < views::SAME_WITHIN) {
            return;
        }
        if !views::may_notify(self.notified.entry(ext.clone()).or_default(), now) {
            return refuse("busy", format!("More than {} notifications a minute: this one wasn't shown.", views::NOTIFY_PER_MINUTE));
        }
        self.notified_ids.retain(|_, (at, _)| now.duration_since(*at) < views::SAME_WITHIN);
        self.notified_ids.insert(id, (now, said));
        reg.record(&ext, Entry::now(Via::Adapter, "notify", Outcome::Ok));
        let open = n.open.clone().map(|o| views::open_as_seen(&key.1, o));
        cx.emit(HubEvent::Notify { ext, notify: Notify { open, ..n } });
    }

    /// A tile shows view `view` of extension `ext`: its adapters send it
    /// and keep it current.
    pub fn show_view(&mut self, ext: &str, view: &str, cx: &mut Context<Self>) {
        let n = self.shown.entry(ext.to_string()).or_default().entry(view.to_string()).or_default();
        *n += 1;
        if *n == 1 {
            for (_, st) in self.runs.iter().filter(|(k, st)| k.0 == ext && st.hello) {
                if let Some(l) = &st.link {
                    l.send(&ToAdapter::Show { view: view.to_string() });
                }
            }
        }
        cx.notify();
    }

    /// A tile of view `view` closed.
    pub fn hide_view(&mut self, ext: &str, view: &str) {
        let Some(shown) = self.shown.get_mut(ext) else { return };
        let Some(n) = shown.get_mut(view) else { return };
        *n -= 1;
        if *n > 0 {
            return;
        }
        shown.remove(view);
        for (_, st) in self.runs.iter_mut().filter(|(k, _)| k.0 == ext) {
            st.raw_views.remove(view);
            st.view_notes.remove(view);
            if let (true, Some(l)) = (st.hello, &st.link) {
                l.send(&ToAdapter::Hide { view: view.to_string() });
            }
        }
        self.merge(ext);
    }

    /// The adapter an item of a view is of (its id as views know it), and
    /// the item's own id; no item: the first running adapter.
    fn route_item(&self, ext: &str, item: Option<&str>) -> Option<(Key, Option<String>)> {
        if let Some(item) = item {
            let (key, inner) = self.route(ext, item);
            return Some((key, Some(inner)));
        }
        let order = self.targets.get(ext)?;
        order.iter().map(|(c, _)| (ext.to_string(), c.clone())).find(|k| self.runs.get(k).is_some_and(|st| st.hello)).map(|k| (k, None))
    }

    /// Item `item` of view `view` was selected (its detail is drawn
    /// beside the list).
    pub fn select(&mut self, ext: &str, view: &str, item: &str) {
        let Some((key, Some(inner))) = self.route_item(ext, Some(item)) else { return };
        if let Some(l) = self.runs.get(&key).filter(|st| st.hello).and_then(|st| st.link.as_ref()) {
            l.send(&ToAdapter::Select { view: view.to_string(), item: inner });
        }
    }

    /// Button `action` was pressed in view `view` (on `item`, or the
    /// view's own): its request number, or why not.
    pub fn view_action(&mut self, ext: &str, view: &str, item: Option<&str>, action: &str, cx: &mut Context<Self>) -> Result<u64, String> {
        use pitwall_core::extensions::activity::{Entry, Outcome, Via};
        let (key, inner) = self.route_item(ext, item).ok_or("The extension isn't running.")?;
        let req = self.next_req;
        let st = self.runs.get_mut(&key).ok_or("The extension isn't running.")?;
        let link = st.link.as_ref().filter(|_| st.hello).ok_or("The extension isn't running.")?;
        if !link.send(&ToAdapter::Action { req, view: view.to_string(), item: inner.clone(), action: action.to_string() }) {
            return Err("The extension isn't running.".into());
        }
        self.next_req += 1;
        let inner = inner.unwrap_or_default();
        st.view_pending.insert(req, (view.to_string(), inner.clone()));
        st.view_busy.insert((view.to_string(), inner));
        st.view_notes.remove(view);
        if let Some(reg) = &st.reg {
            reg.record(ext, Entry::now(Via::Pitwall, format!("view action: {view} {action}"), Outcome::Ok));
        }
        self.merge(ext);
        cx.notify();
        Ok(req)
    }

    /// Button `a` was pressed in view `view` (on `item`, whose agent is
    /// `agent`): one Pitwall runs itself (open a
    /// file or a page, focus an agent) is called as the extension, through
    /// the method table, so its permission decides and it is in its
    /// activity; one that needs a permission it isn't granted isn't sent;
    /// anything else goes to the adapter ([`Hub::view_action`]).
    pub fn press(&mut self, ext: &str, view: &str, item: Option<&str>, agent: Option<&str>, a: &pitwall_core::extensions::view::Action, cx: &mut Context<Self>) -> Result<(), String> {
        use pitwall_core::extensions::view::Run as Does;
        let Some(run) = &a.run else {
            if let Some(need) = &a.needs {
                if !calls::api(cx).registry.allows(ext, need) {
                    return Err(format!("“{}” needs the {need} permission (Settings → Extensions).", a.label));
                }
            }
            return self.view_action(ext, view, item, &a.id, cx).map(|_| ());
        };
        let whose = |own: &Option<String>| own.clone().or(agent.map(String::from));
        let (method, params) = match run {
            Does::File { file, line, agent: own } => {
                let spec = match line {
                    Some(l) => format!("{file}:{l}"),
                    None => file.clone(),
                };
                (pitwall_proto::method::VIEWER_OPEN, serde_json::json!({"agentId": whose(own), "files": [spec], "proto": pitwall_proto::VIEWER_PROTO}))
            }
            Does::Url { url, agent: own } => (pitwall_proto::method::WEB_OPEN, serde_json::json!({"agentId": whose(own), "url": url})),
            Does::Focus(id) => (pitwall_proto::method::AGENT_FOCUS, serde_json::json!({"agentId": id})),
        };
        let api = calls::api(cx);
        let server = api.server.clone().ok_or("This Pitwall can't run it.")?;
        let caller = server.extension_caller(ext).ok_or("The extension is off.")?;
        let (ext, view) = (ext.to_string(), view.to_string());
        if let Some(st) = self.merged.get_mut(&ext) {
            st.view_notes.remove(&view);
        }
        // Off the UI thread: showing waits for the UI thread to open it.
        let task = cx.background_executor().spawn(async move { server.call_as(&caller, method, params, pitwall_core::extensions::activity::Via::View) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            let _ = this.update(cx, |h, cx| {
                let note = match res {
                    Err(e) => Some(e.message),
                    // Not in front: focusing did nothing, and says why.
                    Ok(v) if v.get("focused") == Some(&serde_json::Value::Bool(false)) => v.get("note").and_then(|n| n.as_str()).map(String::from),
                    Ok(_) => None,
                };
                for (_, st) in h.runs.iter_mut().filter(|(k, _)| k.0 == ext) {
                    match &note {
                        Some(n) => {
                            st.view_notes.insert(view.clone(), n.clone());
                        }
                        None => {
                            st.view_notes.remove(&view);
                        }
                    }
                }
                h.merge(&ext);
                cx.notify();
            });
        })
        .detach();
        Ok(())
    }

    /// Command `command` (titled `title`) was picked in the palette, with
    /// agent `agent` focused when it asks for one and may read agents:
    /// sent to the first running adapter.
    pub fn command(&mut self, ext: &str, command: &str, title: &str, agent: Option<String>, cx: &mut Context<Self>) -> Result<u64, String> {
        use pitwall_core::extensions::activity::{Entry, Outcome, Via};
        let (key, _) = self.route_item(ext, None).ok_or("The extension isn't running.")?;
        let req = self.next_req;
        let st = self.runs.get_mut(&key).ok_or("The extension isn't running.")?;
        let link = st.link.as_ref().ok_or("The extension isn't running.")?;
        if !link.send(&ToAdapter::Command { req, command: command.to_string(), agent_id: agent }) {
            return Err("The extension isn't running.".into());
        }
        self.next_req += 1;
        st.commands.insert(req, title.to_string());
        if let Some(reg) = &st.reg {
            reg.record(ext, Entry::now(Via::Pitwall, format!("command: {command}"), Outcome::Ok));
        }
        cx.notify();
        Ok(req)
    }

    /// Adapter `key` calls `c`: answered off the UI thread by the daemon's
    /// table, with the extension as the caller. `Some(why)` when it is to
    /// be stopped (it kept calling past its limits).
    fn call(&mut self, key: &Key, gen: u64, c: Call, cx: &mut Context<Self>) -> Option<String> {
        let api = calls::api(cx);
        let st = self.runs.get_mut(key)?;
        let link = st.link.as_ref()?;
        let refuse = |e: CallError, st: &ExtState| {
            calls::refused(&api.registry, &key.0, &c.method, &e);
            if let Some(l) = &st.link {
                l.send(&ToAdapter::Reply { id: c.id.clone(), result: Err(e) });
            }
        };
        if st.protocol < PROTOCOL_2 {
            refuse(CallError::new("unsupported", "Calls need protocol 2: answer hello with \"protocol\": 2."), st);
            return None;
        }
        if st.in_flight >= calls::MAX_IN_FLIGHT {
            st.strikes += 1;
            refuse(CallError::new("busy", format!("More than {} calls wait for their answer: wait for one.", calls::MAX_IN_FLIGHT)), st);
            return (st.strikes >= calls::MAX_STRIKES).then(|| format!("It was stopped: it kept calling past its limits ({} calls refused in a row).", st.strikes));
        }
        st.in_flight += 1;
        let ext = key.0.clone();
        let Call { id, method, params } = c;
        // Subscribing: its events go straight to its adapter, paced, on a
        // thread of their own.
        if method == pitwall_proto::method::EVENTS_SUBSCRIBE {
            let to = link.sender();
            let server = api.server.clone();
            let old = st.events.take();
            let res = (|| {
                let server = server.ok_or_else(|| CallError::new("unsupported", "This Pitwall doesn't send events."))?;
                let req: pitwall_proto::EventsSubscribe = serde_json::from_value(if params.is_null() { serde_json::json!({}) } else { params }).map_err(|e| CallError::new("bad_params", e.to_string()))?;
                let caller = server.extension_caller(&ext).ok_or_else(|| CallError::new("denied", "This extension is off."))?;
                let (q, topics) = server.subscribe_as(&caller, &req, pitwall_core::extensions::activity::Via::Adapter).map_err(|e| CallError::new(e.code, e.message))?;
                Ok((server, q, topics))
            })();
            let result = match res {
                Ok((server, q, topics)) => {
                    if let Some(old) = old {
                        old.close();
                    }
                    if let Some(to) = to {
                        calls::pump(server, ext.clone(), q.clone(), to);
                    }
                    st.events = Some(q);
                    Ok(serde_json::json!({"topics": topics}))
                }
                Err(e) => {
                    st.events = old;
                    Err(e)
                }
            };
            self.call_done(key, gen, id, result, cx);
            return None;
        }
        let server = api.server.clone();
        let task = cx.background_executor().spawn(async move { calls::answer(server.as_deref(), &ext, &method, params) });
        let key = key.clone();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |h, cx| h.call_done(&key, gen, id, result, cx));
        })
        .detach();
        None
    }

    /// Call `id` of adapter `key` was answered: the reply goes to it.
    fn call_done(&mut self, key: &Key, gen: u64, id: String, result: Result<Value, CallError>, cx: &mut Context<Self>) {
        let Some(st) = self.runs.get_mut(key) else { return };
        if st.gen != gen {
            return;
        }
        st.in_flight = st.in_flight.saturating_sub(1);
        match &result {
            Err(e) if e.code == "busy" => st.strikes += 1,
            _ => st.strikes = 0,
        }
        if let Some(l) = &st.link {
            l.send(&ToAdapter::Reply { id, result });
        }
        if st.strikes >= calls::MAX_STRIKES {
            let why = format!("It was stopped: it kept calling past its limits ({} calls refused in a row).", st.strikes);
            if let Some(l) = self.logs.get(key) {
                l.lock().unwrap_or_else(|e| e.into_inner()).push(log::Dir::Note, &why);
            }
            Self::stop(st);
            st.run = Run::Stopped(why);
            self.merge(&key.0);
            cx.notify();
        }
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
                l.send(&ToAdapter::Inbox(ToInbox::Open { thread: inner }));
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
                l.send(&ToAdapter::Inbox(ToInbox::Close { thread: inner }));
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
                ToAdapter::Inbox(ToInbox::Answer { req, thread: thread_s, ask, picks: if declined { vec![] } else { picks }, text: if declined { None } else { text }, declined })
            }
            Request::Message { text } => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Err("Write something first.".into());
                }
                if text.chars().count() > MAX_REPLY {
                    return Err(format!("Keep it under {MAX_REPLY} characters."));
                }
                ToAdapter::Inbox(ToInbox::Message { req, thread: thread_s, text })
            }
            Request::MarkRead => ToAdapter::Inbox(ToInbox::MarkRead { req, thread: thread_s }),
            Request::Archive => ToAdapter::Inbox(ToInbox::Archive { req, thread: thread_s }),
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
                    l.send(&ToAdapter::Inbox(ToInbox::Refresh));
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
fn open_link(folder: Option<std::path::PathBuf>, command: &[String], hello: &ToAdapter, log: log::Shared) -> Result<Opened, String> {
    let folder = folder.ok_or("Extensions need Pitwall's data folder.")?;
    if command.is_empty() {
        return Err("It has no adapter.".into());
    }
    let (link, events) = super::adapter::spawn(&folder, command, hello, Some(log))?;
    Ok(Opened { link, events })
}

/// Tests: no process; the test drives the other end ([`Hub::ends`]).
#[cfg(test)]
fn open_link(_: Option<std::path::PathBuf>, _: &[String], hello: &ToAdapter, log: log::Shared) -> Result<Opened, String> {
    let (link, events, end) = super::adapter::pair(hello, Some(log));
    Ok(Opened { link, events, end: Some(end) })
}

#[cfg(test)]
mod tests;
