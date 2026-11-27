//! What a tile's store holds, and the host side of each imported interface
//! (runtime.md §3.8): `ui`, `log`, `tasks`, `storage`, `settings`. All of
//! it runs on the tile's worker thread, inside the guest's calls; nothing
//! here waits on the UI thread. Requests for the UI are queued and handed
//! to the app after the call ([`crate::UiRequest`]).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use pitwall_core::extensions::{SettingDef, SettingKind};
use serde_json::Value;
use wasmtime::component::ResourceTable;
use wasmtime::StoreLimits;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::bindings::pitwall::tile::{log, settings, storage, tasks, types, ui};
use crate::data::{KvError, KvStore};
use crate::limits::{HOST_CALLS_PER_SECOND, MAX_LOG_LINE, MAX_TIMERS, MAX_TOAST, MIN_TIMER, TOASTS_PER_MINUTE};
use crate::stdio::LogPipe;
use crate::ui::{HostError, Tone};
use crate::{Grants, LogLevel, LogLine, Secrets, UiRequest};

/// A running timer.
pub(crate) struct Timer {
    pub due: Instant,
    pub every: Option<Duration>,
}

/// Something for the app, queued during a call.
pub(crate) enum Pending {
    Log(LogLine),
    Ui(UiRequest),
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
            window: Instant::now(),
            dropped_lines: 0,
            toasts: VecDeque::new(),
            stdout: p.stdout,
            stderr: p.stderr,
        }
    }

    /// Count a host call; false once over the limit for this second.
    fn count_call(&mut self) -> bool {
        let now = Instant::now();
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
        self.pending.push(Pending::Log(LogLine { level, message, at: SystemTime::now(), from_host: true }));
    }

    fn guest_log(&mut self, level: LogLevel, message: String) {
        if !self.count_call() {
            self.dropped_lines += 1;
            return;
        }
        self.pending.push(Pending::Log(LogLine { level, message: cut(&message, MAX_LOG_LINE), at: SystemTime::now(), from_host: false }));
    }

    /// Stdout and stderr lines written so far, into the Log.
    pub fn drain_stdio(&mut self, all: bool) {
        for line in self.stdout.take_lines(all) {
            self.pending.push(Pending::Log(LogLine { level: LogLevel::Info, message: line, at: SystemTime::now(), from_host: false }));
        }
        for line in self.stderr.take_lines(all) {
            self.pending.push(Pending::Log(LogLine { level: LogLevel::Warn, message: line, at: SystemTime::now(), from_host: false }));
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
        let now = Instant::now();
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
}

impl tasks::Host for HostState {
    fn timer(&mut self, ms: u32, repeat: bool) -> Result<u32, HostError> {
        if !self.count_call() || self.timers.len() >= MAX_TIMERS {
            return Err(HostError::Busy);
        }
        let every = Duration::from_millis(u64::from(ms)).max(MIN_TIMER);
        let id = self.next_timer;
        self.next_timer = self.next_timer.wrapping_add(1).max(1);
        self.timers.insert(id, Timer { due: Instant::now() + every, every: repeat.then_some(every) });
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
