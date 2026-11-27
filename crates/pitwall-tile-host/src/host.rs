//! What a tile's store holds, and the host side of each imported interface
//! (runtime.md §3.8): `ui`, `log`, `tasks`, `storage`, `settings` (the
//! agents' questions group is in [`crate::group`]). All of
//! it runs on the tile's worker thread, inside the guest's calls; nothing
//! here waits on the UI thread. Requests for the UI are queued and handed
//! to the app after the call ([`crate::UiRequest`]).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pitwall_core::extensions::{SettingDef, SettingKind};
use serde_json::Value;
use wasmtime::component::ResourceTable;
use wasmtime::StoreLimits;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::bindings::pitwall::tile::{agent_tools, log, settings, storage, tasks, types, ui};
use crate::clock::Clock;
use crate::data::{KvError, KvStore};
use crate::limits::{HOST_CALLS_PER_SECOND, MAX_GET_MANY, MAX_LOG_LINE, MAX_TIMERS, MAX_TOAST, MIN_TIMER, TOASTS_PER_MINUTE};
use crate::stdio::LogPipe;
use crate::ui::{Font, HostError, Tone, Weight};
use crate::net::Fetcher;
use crate::services::no_service;
use crate::sql::Db;
use crate::worker::Msg;
use crate::{Grants, LogLevel, LogLine, Secrets, UiRequest};
use pitwall_core::extensions::toolbar::ToolbarDef;

/// A running timer.
pub(crate) struct Timer {
    pub due: Instant,
    pub every: Option<Duration>,
}

/// Something for the app, queued during a call.
pub(crate) enum Pending {
    Log(LogLine),
    Ui(UiRequest),
    /// `agent-tools.user-edited`.
    Edited(u64),
    /// How an agent's call ended.
    Answer(u64, crate::agent::Answer),
}

/// A tile's store data.
pub(crate) struct HostState {
    pub wasi: WasiCtx,
    pub table: ResourceTable,
    pub limits: StoreLimits,
    /// When the running call must end (wall clock, runtime.md §2.5).
    pub deadline: Instant,
    /// Set by `TileHandle::stop`: the running call ends at the next tick.
    pub stop: Arc<AtomicBool>,
    pub extension: String,
    /// The views the manifest declares (`open-view` may name only these).
    pub known_views: Vec<String>,
    pub setting_defs: Vec<SettingDef>,
    /// The stored settings, re-read on `settings_changed`.
    pub settings: BTreeMap<String, String>,
    pub secrets: Arc<dyn Secrets>,
    pub grants: Grants,
    pub kv: KvStore,
    pub timers: BTreeMap<u32, Timer>,
    next_timer: u32,
    pub pending: Vec<Pending>,
    /// Views `ui.request-render` asked for.
    pub render_requests: BTreeSet<String>,
    calls: u32,
    window: Instant,
    dropped_lines: u32,
    toasts: VecDeque<Instant>,
    pub stdout: LogPipe,
    pub stderr: LogPipe,
    /// The time timers and rate limits go by.
    pub clock: Clock,
    /// The agents' questions group (`agents`, `badges`, `asks`).
    pub group: crate::group::Group,
    /// Images put with `ui.put-image`: their sizes by name.
    pub images: BTreeMap<String, usize>,
    /// `net`: who makes the requests, the ones waiting, and where their
    /// answers go (the worker's own channel).
    pub fetcher: Arc<dyn Fetcher>,
    pub fetches: BTreeSet<u32>,
    pub next_fetch: u32,
    pub done: std::sync::mpsc::Sender<Msg>,
    /// `sql`: its database, opened on first use.
    pub db: Db,
    /// `notify`: when each of the last ones was sent, and what.
    pub notified: VecDeque<(Instant, String)>,
    /// `toolbar`: the manifest's item, and when the tile last showed or
    /// hid it.
    pub toolbar: Option<ToolbarDef>,
    pub shown_at: Option<Instant>,
    /// `service`: whether the manifest declares one, and its state.
    pub has_service: bool,
    pub service: crate::ui::ServiceInfo,
    /// Pitwall has said the state once (the first, sent after start,
    /// isn't a change).
    pub service_known: bool,
    /// The agent's call being delivered now (`content.call` of the event
    /// `handle` runs for), and the tile's reply to it, if any.
    pub awaiting: Option<u64>,
    pub reply: Option<agent_tools::ToolOutput>,
    /// `read` and `act` calls the `tool` export left to answer later.
    pub open_calls: BTreeSet<u64>,
    /// The kind of agent call running without the tile's acting
    /// permissions (`show`, `present` or `read`), while it runs.
    pub gated: Option<&'static str>,
    /// The grants put aside while a gated call runs.
    pub ungated: Option<Grants>,
    /// Large inputs of calls (`from_file`), shared with the host.
    pub large: crate::agent::LargeInputs,
    /// `files`: each view's project, the watches.
    pub files: crate::files::Files,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

/// What a store is made from (the worker fills it in).
pub(crate) struct StateParts {
    pub wasi: WasiCtx,
    pub limits: StoreLimits,
    pub stop: Arc<AtomicBool>,
    pub extension: String,
    pub known_views: Vec<String>,
    pub setting_defs: Vec<SettingDef>,
    pub settings: BTreeMap<String, String>,
    pub secrets: Arc<dyn Secrets>,
    pub grants: Grants,
    pub kv: KvStore,
    pub stdout: LogPipe,
    pub stderr: LogPipe,
    pub clock: Clock,
    pub group: crate::group::Group,
    pub fetcher: Arc<dyn Fetcher>,
    pub done: std::sync::mpsc::Sender<Msg>,
    pub db: Db,
    pub toolbar: Option<ToolbarDef>,
    pub has_service: bool,
    pub large: crate::agent::LargeInputs,
    pub files: crate::files::Files,
}

impl HostState {
    pub fn new(p: StateParts) -> HostState {
        HostState {
            wasi: p.wasi,
            table: ResourceTable::new(),
            limits: p.limits,
            deadline: Instant::now() + crate::limits::INIT_DEADLINE,
            stop: p.stop,
            extension: p.extension,
            known_views: p.known_views,
            setting_defs: p.setting_defs,
            settings: p.settings,
            secrets: p.secrets,
            grants: p.grants,
            kv: p.kv,
            timers: BTreeMap::new(),
            next_timer: 1,
            pending: Vec::new(),
            render_requests: BTreeSet::new(),
            calls: 0,
            window: p.clock.now(),
            dropped_lines: 0,
            toasts: VecDeque::new(),
            stdout: p.stdout,
            stderr: p.stderr,
            clock: p.clock,
            group: p.group,
            images: BTreeMap::new(),
            fetcher: p.fetcher,
            fetches: BTreeSet::new(),
            next_fetch: 1,
            done: p.done,
            db: p.db,
            notified: VecDeque::new(),
            toolbar: p.toolbar,
            shown_at: None,
            has_service: p.has_service,
            service: no_service(p.has_service),
            service_known: false,
            awaiting: None,
            reply: None,
            open_calls: BTreeSet::new(),
            gated: None,
            ungated: None,
            large: p.large,
            files: p.files,
        }
    }

    /// After each call into the tile: a database transaction it left open
    /// is rolled back, with a line in the Log.
    pub fn end_call(&mut self) {
        if self.db.end_call() {
            self.host_log(LogLevel::Warn, "a database transaction was still open when the call returned: it was rolled back (commit it in the same call)".into());
        }
    }

    /// An agent's `kind` call starts (`show`, `present` or `read`): the
    /// tile's acting permissions are off until [`HostState::ungate`]
    /// (agent-tools.md §11.2).
    pub fn gate(&mut self, kind: &'static str) {
        if self.gated.is_none() {
            let acting = self.grants.without_acting();
            self.ungated = Some(std::mem::replace(&mut self.grants, acting));
        }
        self.gated = Some(kind);
    }

    /// The gated call ended: the tile's grants are back.
    pub fn ungate(&mut self) {
        self.gated = None;
        if let Some(g) = self.ungated.take() {
            self.grants = g;
        }
    }

    /// Count a host call; false once over the limit for this second.
    pub(crate) fn count_call(&mut self) -> bool {
        let now = self.clock.now();
        if now.duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.calls = 0;
            if self.dropped_lines > 0 {
                let n = std::mem::take(&mut self.dropped_lines);
                self.host_log(LogLevel::Warn, format!("{n} log lines were dropped: more than {HOST_CALLS_PER_SECOND} host calls a second"));
            }
        }
        self.calls += 1;
        self.calls <= HOST_CALLS_PER_SECOND
    }

    /// A line in the tile's Log from the host itself.
    pub fn host_log(&mut self, level: LogLevel, message: String) {
        self.pending.push(Pending::Log(LogLine { level, message, at: self.clock.system_now(), from_host: true }));
    }

    fn guest_log(&mut self, level: LogLevel, message: String) {
        if !self.count_call() {
            self.dropped_lines += 1;
            return;
        }
        self.pending.push(Pending::Log(LogLine { level, message: cut(&message, MAX_LOG_LINE), at: self.clock.system_now(), from_host: false }));
    }

    /// Stdout and stderr lines written so far, into the Log.
    pub fn drain_stdio(&mut self, all: bool) {
        let at = self.clock.system_now();
        for line in self.stdout.take_lines(all) {
            self.pending.push(Pending::Log(LogLine { level: LogLevel::Info, message: line, at, from_host: false }));
        }
        for line in self.stderr.take_lines(all) {
            self.pending.push(Pending::Log(LogLine { level: LogLevel::Warn, message: line, at, from_host: false }));
        }
    }

    /// Timers due at `now`: their ids, rescheduled or removed.
    pub fn due_timers(&mut self, now: Instant) -> Vec<u32> {
        let due: Vec<u32> = self.timers.iter().filter(|(_, t)| t.due <= now).map(|(id, _)| *id).collect();
        for id in &due {
            let every = self.timers.get(id).and_then(|t| t.every);
            match every {
                Some(every) => {
                    if let Some(t) = self.timers.get_mut(id) {
                        t.due = now + every;
                    }
                }
                None => {
                    self.timers.remove(id);
                }
            }
        }
        due
    }

    /// When the next timer is due.
    pub fn next_due(&self) -> Option<Instant> {
        self.timers.values().map(|t| t.due).min()
    }
}

/// `s` cut to at most `max` bytes, on a character boundary, with "…" when
/// cut.
pub(crate) fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max.saturating_sub('…'.len_utf8());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

impl types::Host for HostState {}

impl log::Host for HostState {
    fn debug(&mut self, message: String) {
        self.guest_log(LogLevel::Debug, message);
    }

    fn info(&mut self, message: String) {
        self.guest_log(LogLevel::Info, message);
    }

    fn warn(&mut self, message: String) {
        self.guest_log(LogLevel::Warn, message);
    }

    fn error(&mut self, message: String) {
        self.guest_log(LogLevel::Error, message);
    }
}

impl ui::Host for HostState {
    fn request_render(&mut self, view: String) {
        if self.count_call() {
            self.render_requests.insert(view);
        }
    }

    fn scroll_to(&mut self, view: String, key_path: String) {
        if self.count_call() {
            self.pending.push(Pending::Ui(UiRequest::ScrollTo { view, key_path }));
        }
    }

    fn toast(&mut self, text: String, tone: Tone) {
        if !self.count_call() {
            return;
        }
        let now = self.clock.now();
        while self.toasts.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60)) {
            self.toasts.pop_front();
        }
        if self.toasts.len() >= TOASTS_PER_MINUTE {
            self.host_log(LogLevel::Warn, format!("a toast was dropped: at most {TOASTS_PER_MINUTE} a minute (“{}”)", cut(&text, 80)));
            return;
        }
        self.toasts.push_back(now);
        self.pending.push(Pending::Ui(UiRequest::Toast { text: cut(&text, MAX_TOAST), tone }));
    }

    fn open_view(&mut self, view: String) {
        if !self.count_call() {
            return;
        }
        if self.known_views.contains(&view) {
            self.pending.push(Pending::Ui(UiRequest::OpenView(view)));
        } else {
            self.host_log(LogLevel::Warn, format!("open-view: the manifest declares no tile “{}”", cut(&view, 80)));
        }
    }

    fn put_image(&mut self, name: String, bytes: Vec<u8>) -> Result<(), HostError> {
        use crate::limits::{MAX_IMAGES_BYTES, MAX_IMAGE_BYTES, MAX_KEY_BYTES};
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        if name.is_empty() || name.len() > MAX_KEY_BYTES {
            return Err(HostError::Invalid("an image's name is 1 to 128 bytes".into()));
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(HostError::TooLarge);
        }
        let Some(format) = crate::components::image_format(&bytes) else {
            return Err(HostError::Invalid("not a PNG, JPEG, GIF, WebP or SVG image".into()));
        };
        let others: usize = self.images.iter().filter(|(n, _)| **n != name).map(|(_, b)| *b).sum();
        if others + bytes.len() > MAX_IMAGES_BYTES {
            return Err(HostError::TooLarge);
        }
        self.images.insert(name.clone(), bytes.len());
        self.pending.push(Pending::Ui(UiRequest::PutImage { name, format, bytes: bytes.into() }));
        Ok(())
    }

    fn drop_image(&mut self, name: String) {
        if self.count_call() && self.images.remove(&name).is_some() {
            self.pending.push(Pending::Ui(UiRequest::DropImage(name)));
        }
    }

    fn measure_text(&mut self, texts: Vec<String>, font: Font, size: f32, weight: Weight) -> Vec<f32> {
        if !self.count_call() {
            return Vec::new();
        }
        crate::measure::widths(&texts, font, size, weight).unwrap_or_else(|why| {
            self.host_log(LogLevel::Warn, why);
            Vec::new()
        })
    }
}

impl tasks::Host for HostState {
    fn timer(&mut self, ms: u32, repeat: bool) -> Result<u32, HostError> {
        if !self.count_call() || self.timers.len() >= MAX_TIMERS {
            return Err(HostError::Busy);
        }
        let every = Duration::from_millis(u64::from(ms)).max(MIN_TIMER);
        let id = self.next_timer;
        self.next_timer = self.next_timer.wrapping_add(1).max(1);
        self.timers.insert(id, Timer { due: self.clock.now() + every, every: repeat.then_some(every) });
        Ok(id)
    }

    fn cancel(&mut self, id: u32) {
        self.timers.remove(&id);
    }
}

fn kv_error(e: KvError) -> HostError {
    match e {
        KvError::BadKey(why) => HostError::Invalid(why),
        KvError::Full => HostError::TooLarge,
    }
}

impl storage::Host for HostState {
    fn get(&mut self, key: String) -> Result<Option<Vec<u8>>, HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        self.kv.get(&key).map_err(kv_error)
    }

    fn get_many(&mut self, keys: Vec<String>) -> Result<Vec<Option<Vec<u8>>>, HostError> {
        // One call, however many keys: loading a tile's data at start
        // shouldn't spend its budget of calls.
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        if keys.len() > MAX_GET_MANY {
            return Err(HostError::TooLarge);
        }
        keys.iter().map(|k| self.kv.get(k).map_err(kv_error)).collect()
    }

    fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        self.kv.set(key, value).map_err(kv_error)
    }

    fn delete(&mut self, key: String) -> Result<(), HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        self.kv.delete(&key).map_err(kv_error)
    }

    fn list(&mut self, prefix: String) -> Result<Vec<String>, HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        Ok(self.kv.list(&prefix))
    }

    fn usage(&mut self) -> u64 {
        self.kv.usage()
    }
}

impl settings::Host for HostState {
    fn get(&mut self, key: String) -> Option<settings::SettingValue> {
        let def = self.setting_defs.iter().find(|d| d.key == key)?;
        match def.json(self.settings.get(&key).map(String::as_str))? {
            Value::Bool(b) => Some(settings::SettingValue::Flag(b)),
            Value::Number(n) => n.as_f64().map(settings::SettingValue::Number),
            Value::String(s) => Some(settings::SettingValue::Text(s)),
            _ => None,
        }
    }

    fn secret(&mut self, key: String) -> Result<Option<String>, HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        match self.setting_defs.iter().find(|d| d.key == key) {
            Some(d) if d.kind == SettingKind::Secret => Ok(self.secrets.secret(&self.extension, &key)),
            _ => Err(HostError::Invalid(format!("the manifest has no secret setting “{}”", cut(&key, 80)))),
        }
    }

    fn open(&mut self) {
        if self.count_call() {
            self.pending.push(Pending::Ui(UiRequest::OpenSettings));
        }
    }
}

impl agent_tools::Host for HostState {
    fn reply(&mut self, call: u64, out: agent_tools::ToolOutput) {
        if !self.count_call() {
            return;
        }
        if self.open_calls.remove(&call) {
            self.pending.push(Pending::Answer(call, crate::agent::Answer::Reply(crate::agent::capped_data(out))));
            return;
        }
        if self.awaiting != Some(call) {
            self.host_log(LogLevel::Warn, format!("reply to call {call} was dropped: Pitwall answers a show call only while the event that delivered it is handled, and a read or act call until its time limit"));
            return;
        }
        if self.reply.is_some() {
            return;
        }
        self.reply = Some(crate::agent::capped(out));
    }

    fn user_edited(&mut self, call: u64) {
        if self.count_call() {
            self.pending.push(Pending::Edited(call));
        }
    }

    fn input_part(&mut self, call: u64, offset: u64, len: u32) -> Result<String, String> {
        if !self.count_call() {
            return Err("busy: too many calls this second".into());
        }
        if self.awaiting != Some(call) {
            return Err(format!("call {call}'s input can be read only while the event that delivered it is handled"));
        }
        let text = self.large.get(&self.extension, call).ok_or_else(|| format!("call {call} named no file"))?;
        Ok(crate::agent::part(&text, offset, len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_keeps_whole_characters() {
        assert_eq!(cut("hello", 10), "hello");
        assert_eq!(cut("héllo wörld", 6), "hé…");
        assert!(cut(&"é".repeat(100), 9).len() <= 9);
    }
}
