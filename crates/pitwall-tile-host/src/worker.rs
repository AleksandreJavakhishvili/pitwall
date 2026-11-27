//! A running component on its own thread (runtime.md §2.4). The store is
//! created, used and dropped on the worker; the UI thread only sends
//! messages through a [`TileHandle`] (never blocking) and receives
//! [`TileOutput`]s through the sink. All calls into the guest come from the
//! worker's one loop, so the guest is never re-entered.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wasmtime::{Store, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::WasiCtxBuilder;

use crate::bindings::Tile;
use crate::clock::Clock;
use crate::data::{extension_data_dir, ExtensionData, KvStore};
use crate::frame;
use crate::host::{HostState, Pending, StateParts};
use crate::limits::*;
use crate::stdio::LogPipe;
use crate::ui::{Event, Frame, InitContext, PointerKind, ThemeInfo};
use crate::ui::{Content, ViewContent, ViewOpened};
use crate::{cache, Extension, FrameUpdate, Grants, LoadError, LogLevel, StartOptions, StopReason, TileHost, TileOutput, ViewSpec, MAX_CONTENT_BYTES};

pub(crate) type Sink = Box<dyn FnMut(TileOutput) + Send>;

/// From the UI thread to a tile's worker.
enum Msg {
    Event(Event),
    OpenView(ViewSpec),
    SetContent(String, Content),
    CloseView(String),
    Visible(bool),
    Theme(ThemeInfo),
    Grants(Grants),
    SettingsChanged,
    SaveState,
    Advance(Duration),
    Stop,
}

/// A running tile, as the app holds it. Every method only posts a message
/// and returns at once (false once the worker has stopped). Dropping the
/// handle stops the tile.
pub struct TileHandle {
    id: String,
    tx: mpsc::Sender<Msg>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TileHandle {
    /// The extension's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    fn post(&self, msg: Msg) -> bool {
        self.tx.send(msg).is_ok()
    }

    /// Send a user event (a click, typed text, a key, a resize…).
    pub fn send(&self, event: Event) -> bool {
        self.post(Msg::Event(event))
    }

    /// Open a view; the guest gets `view-opened`, and its frames follow.
    /// A view id that is already open with content given is
    /// [`TileHandle::set_content`]; without content, nothing happens.
    pub fn open_view(&self, view: ViewSpec) -> bool {
        self.post(Msg::OpenView(view))
    }

    /// Give open view `view` new content (an agent's later call with the
    /// same content id); the guest gets `view-content`.
    pub fn set_content(&self, view: &str, content: Content) -> bool {
        self.post(Msg::SetContent(view.to_string(), content))
    }

    /// Close a view: the guest gets `view-closed`, no more frames for it.
    pub fn close_view(&self, view: &str) -> bool {
        self.post(Msg::CloseView(view.to_string()))
    }

    /// Shown or hidden (runtime.md §2.3: hidden pauses timers and renders;
    /// the guest gets `visibility`).
    pub fn set_visible(&self, visible: bool) -> bool {
        self.post(Msg::Visible(visible))
    }

    /// The user's appearance changed.
    pub fn set_theme(&self, theme: ThemeInfo) -> bool {
        self.post(Msg::Theme(theme))
    }

    /// The user granted or took back permissions; applies to the next
    /// call, and the guest gets `grants-changed`. (The memory limit is
    /// set at start: a new `memory` grant applies after a restart.)
    pub fn set_grants(&self, grants: Grants) -> bool {
        self.post(Msg::Grants(grants))
    }

    /// The extension's settings changed on disk ([`ExtensionData::set_setting`]):
    /// the worker reads them again and the guest gets `settings-changed`.
    pub fn settings_changed(&self) -> bool {
        self.post(Msg::SettingsChanged)
    }

    /// Ask for `save-state`; the bytes arrive as
    /// [`TileOutput::StateSaved`] (keep them for the next start).
    pub fn save_state(&self) -> bool {
        self.post(Msg::SaveState)
    }

    /// With a virtual clock ([`Clock::manual`]): let `time` pass. Timers
    /// due meanwhile fire in order, each at its own time (while shown),
    /// and the views render after. Ignored, with a Log line, on the wall
    /// clock.
    pub fn advance(&self, time: Duration) -> bool {
        self.post(Msg::Advance(time))
    }

    /// Stop the tile now: a call that is running ends at the next epoch
    /// tick (about 10 ms), then `Stopped(Stopped)` arrives.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Msg::Stop);
    }

    /// Whether the worker thread is still running.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Wait until the worker has ended (after [`TileHandle::stop`] or a
    /// stop of its own). Blocks: not for the UI thread.
    pub fn join(mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for TileHandle {
    fn drop(&mut self) {
        // Never joins: the UI thread mustn't wait for a call to end.
        self.stop();
    }
}

pub(crate) fn spawn(host: TileHost, ext: Extension, options: StartOptions, mut sink: Sink) -> Result<TileHandle, LoadError> {
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let id = ext.id().to_string();
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name(format!("tile:{id}"))
        .spawn(move || {
            let reason = match Worker::start(&host, &ext, options, rx, flag, &mut sink) {
                Ok(mut w) => w.run().err().unwrap_or(StopReason::Stopped),
                Err(reason) => reason,
            };
            sink(TileOutput::Stopped(reason));
        })
        .map_err(|e| LoadError::Host(e.to_string()))?;
    Ok(TileHandle { id, tx, stop, thread: Some(thread) })
}

struct Worker<'a> {
    store: Store<HostState>,
    tile: Tile,
    sink: &'a mut Sink,
    rx: mpsc::Receiver<Msg>,
    data: ExtensionData,
    /// Open views by id: the tile each shows and the last frame sent.
    views: BTreeMap<String, View>,
    dirty: BTreeSet<String>,
    visible: bool,
    /// When the current batch of messages was taken (a frame's `took`).
    batch: Instant,
    /// Reserved keys a frame bound, told in the Log once each.
    told_keys: HashSet<String>,
}

struct View {
    tile: String,
    frame: Option<Arc<Frame>>,
}

fn content_size(c: &Content) -> usize {
    c.json.as_ref().map_or(0, String::len) + c.bytes.as_ref().map_or(0, Vec::len) + c.id.as_ref().map_or(0, String::len)
}

/// Why a call into the guest failed, as a stop reason.
fn stop_reason(e: &wasmtime::Error, call: &'static str, limit: Duration, stop: &AtomicBool) -> StopReason {
    if stop.load(Ordering::Relaxed) {
        return StopReason::Stopped;
    }
    match e.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => StopReason::Deadline { call, limit },
        _ => StopReason::Crashed(format!("{e:#}")),
    }
}

impl<'a> Worker<'a> {
    fn start(host: &TileHost, ext: &Extension, options: StartOptions, rx: mpsc::Receiver<Msg>, stop: Arc<AtomicBool>, sink: &'a mut Sink) -> Result<Worker<'a>, StopReason> {
        let inner = host.inner();
        let data_dir = &inner.config.data_dir;
        let wasm = ext.read_wasm().map_err(|e| StopReason::LoadFailed(e.to_string()))?;
        if !cache::cache_file(data_dir, ext.id(), &wasm).is_file() {
            sink(TileOutput::Preparing);
        }
        let t = Instant::now();
        let (component, cached) = cache::load_or_compile(&inner.engine, data_dir, ext.id(), &wasm).map_err(StopReason::LoadFailed)?;
        host.check_imports(&component).map_err(|e| StopReason::LoadFailed(e.to_string()))?;
        drop(wasm);

        let manifest = ext.manifest();
        let data = ExtensionData::new(data_dir, ext.id());
        let (kv, warning) = KvStore::open(&extension_data_dir(data_dir, ext.id()));
        let (stdout, stderr) = (LogPipe::default(), LogPipe::default());
        // WasiCtxBuilder's defaults: no stdin, environment, arguments,
        // preopened folders or name lookup (runtime.md §3.1). Sockets are
        // refused outright, whatever the defaults are (S7).
        let mut wasi = WasiCtxBuilder::new();
        wasi.stdout(stdout.clone()).stderr(stderr.clone()).allow_tcp(false).allow_udp(false).allow_ip_name_lookup(false).socket_addr_check(|_, _| Box::pin(async { false }));
        // A virtual clock is the guest's `Instant` and `SystemTime` too.
        if let Some((wall, mono)) = options.clock.wasi() {
            wasi.wall_clock(wall).monotonic_clock(mono);
        }
        let limits = StoreLimitsBuilder::new()
            .memory_size(options.grants.memory_limit(manifest))
            .trap_on_grow_failure(true)
            .instances(INSTANCES)
            .tables(TABLES)
            .memories(MEMORIES)
            .table_elements(TABLE_ELEMENTS)
            .build();
        let state = HostState::new(StateParts {
            wasi: wasi.build(),
            limits,
            stop: stop.clone(),
            extension: ext.id().to_string(),
            known_views: ext.tiles().iter().map(|t| t.id.clone()).collect(),
            setting_defs: manifest.settings.clone(),
            settings: data.settings(),
            secrets: inner.config.secrets.clone(),
            grants: options.grants.clone(),
            kv,
            stdout,
            stderr,
            clock: options.clock.clone(),
            group: crate::group::Group::new(manifest.contributes.badges, manifest.contributes.asks, inner.config.methods.clone()),
        });
        let mut store = Store::new(&inner.engine, state);
        store.limiter(|s| &mut s.limits);
        // Wall-clock deadlines (runtime.md §2.5): every tick checks the
        // clock and the Stop flag; counting ticks alone overshoots.
        store.epoch_deadline_callback(|cx| {
            let s = cx.data();
            if s.stop.load(Ordering::Relaxed) || Instant::now() >= s.deadline {
                Ok(UpdateDeadline::Interrupt)
            } else {
                Ok(UpdateDeadline::Continue(1))
            }
        });
        if let Some(w) = warning {
            store.data_mut().host_log(LogLevel::Warn, w);
        }
        arm(&mut store, INIT_DEADLINE);
        let tile = Tile::instantiate(&mut store, &component, &inner.linker).map_err(|e| match stop_reason(&e, "instantiate", INIT_DEADLINE, &stop) {
            StopReason::Crashed(why) => StopReason::LoadFailed(why),
            other => other,
        })?;
        let mut w = Worker { store, tile, sink, rx, data, views: BTreeMap::new(), dirty: BTreeSet::new(), visible: options.visible, batch: Instant::now(), told_keys: HashSet::new() };
        let cx = InitContext {
            extension: ext.id().to_string(),
            version: manifest.version.clone().unwrap_or_default(),
            theme: options.theme,
            granted: options.grants.names(),
            state: options.state,
        };
        arm(&mut w.store, INIT_DEADLINE);
        let init = w.tile.pitwall_tile_guest().call_init(&mut w.store, &cx);
        match init {
            Ok(Ok(())) => {}
            Ok(Err(why)) => return Err(StopReason::InitFailed(crate::host::cut(&why, 1000))),
            Err(e) => return Err(stop_reason(&e, "init", INIT_DEADLINE, &stop)),
        }
        (w.sink)(TileOutput::Started { cached, took: t.elapsed() });
        for v in options.views {
            w.open_view(v)?;
        }
        w.flush();
        Ok(w)
    }

    fn stopping(&self) -> bool {
        self.store.data().stop.load(Ordering::Relaxed)
    }

    fn fail(&self, e: &wasmtime::Error, call: &'static str, limit: Duration) -> StopReason {
        stop_reason(e, call, limit, &self.store.data().stop)
    }

    /// The loop: render what changed, hand over what the calls queued,
    /// wait for messages or the next timer, handle them as one batch.
    fn run(&mut self) -> Result<(), StopReason> {
        loop {
            self.render_dirty()?;
            self.flush();
            if self.stopping() {
                return Err(StopReason::Stopped);
            }
            // A virtual clock's timers fire only when it is moved.
            let manual = self.store.data().clock.is_manual();
            let due = if self.visible && !manual { self.store.data().next_due() } else { None };
            let first = match due {
                Some(at) => match self.rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                    Ok(m) => Some(m),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Err(StopReason::Stopped),
                },
                None => match self.rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => return Err(StopReason::Stopped),
                },
            };
            self.batch = Instant::now();
            let mut batch: Vec<Msg> = first.into_iter().chain(self.rx.try_iter()).collect();
            coalesce(&mut batch);
            for msg in batch {
                self.apply(msg)?;
            }
            if self.visible {
                let now = self.store.data().clock.now();
                for id in self.store.data_mut().due_timers(now) {
                    self.handle(Event::Timer(id))?;
                }
            }
            let asked = std::mem::take(&mut self.store.data_mut().render_requests);
            self.dirty.extend(asked.into_iter().filter(|v| self.views.contains_key(v)));
        }
    }

    fn apply(&mut self, msg: Msg) -> Result<(), StopReason> {
        match msg {
            // Only a tile that may read the agents hears they changed.
            Msg::Event(Event::AgentsChanged) if !self.store.data().grants.has(pitwall_core::extensions::permissions::AGENTS_READ) => Ok(()),
            Msg::Event(e) => self.handle(e),
            Msg::OpenView(v) => self.open_view(v),
            Msg::SetContent(view, content) => self.set_content(view, content),
            Msg::CloseView(v) => {
                if self.views.remove(&v).is_none() {
                    return Ok(());
                }
                self.dirty.remove(&v);
                self.handle(Event::ViewClosed(v))
            }
            Msg::Visible(shown) if shown == self.visible => Ok(()),
            Msg::Visible(shown) => {
                if shown {
                    self.visible = true;
                    self.dirty.extend(self.views.keys().cloned());
                }
                // Hiding: the call runs while still visible; suspension
                // starts when it returns (its storage writes are flushed
                // before the worker waits).
                let r = self.handle(Event::Visibility(shown));
                self.visible = shown;
                r
            }
            Msg::Theme(t) => self.handle(Event::Theme(t)),
            Msg::Grants(g) => {
                let names = g.names();
                self.store.data_mut().grants = g;
                self.handle(Event::GrantsChanged(names))
            }
            Msg::SettingsChanged => {
                self.store.data_mut().settings = self.data.settings();
                self.handle(Event::SettingsChanged)
            }
            Msg::SaveState => {
                arm(&mut self.store, SAVE_STATE_DEADLINE);
                let r = self.tile.pitwall_tile_guest().call_save_state(&mut self.store);
                let bytes = r.map_err(|e| self.fail(&e, "save-state", SAVE_STATE_DEADLINE))?;
                if bytes.len() > MAX_STATE_BYTES {
                    self.store.data_mut().host_log(LogLevel::Warn, format!("save-state returned {} KiB, more than {} KiB; nothing was kept", bytes.len() / 1024, MAX_STATE_BYTES / 1024));
                } else {
                    (self.sink)(TileOutput::StateSaved(bytes));
                }
                Ok(())
            }
            Msg::Advance(time) => self.advance(time),
            Msg::Stop => Err(StopReason::Stopped),
        }
    }

    /// Move a virtual clock on by `time`, firing each timer due meanwhile
    /// at its own time.
    fn advance(&mut self, time: Duration) -> Result<(), StopReason> {
        let Clock::Manual(m) = self.store.data().clock.clone() else {
            self.store.data_mut().host_log(LogLevel::Warn, "advance: the tile runs on the wall clock".into());
            return Ok(());
        };
        let end = self.store.data().clock.now() + time;
        while self.visible {
            let Some(next) = self.store.data().next_due().filter(|n| *n <= end) else { break };
            m.advance_to(next);
            for id in self.store.data_mut().due_timers(next) {
                self.handle(Event::Timer(id))?;
            }
        }
        m.advance_to(end);
        Ok(())
    }

    fn open_view(&mut self, mut v: ViewSpec) -> Result<(), StopReason> {
        if !self.store.data().known_views.contains(&v.tile) {
            self.store.data_mut().host_log(LogLevel::Warn, format!("the app opened an unknown tile “{}”", v.tile));
            return Ok(());
        }
        if self.views.contains_key(&v.id) {
            return match v.content {
                Some(c) => self.set_content(v.id, c),
                None => Ok(()),
            };
        }
        if v.content.as_ref().is_some_and(|c| content_size(c) > MAX_CONTENT_BYTES) {
            self.store.data_mut().host_log(LogLevel::Warn, format!("view “{}” was opened with more than {} MiB of content; it opens without it", v.id, MAX_CONTENT_BYTES >> 20));
            v.content = None;
        }
        self.views.insert(v.id.clone(), View { tile: v.tile.clone(), frame: None });
        self.dirty.insert(v.id.clone());
        self.handle(Event::ViewOpened(ViewOpened { view: v.id, tile: v.tile, content: v.content }))
    }

    fn set_content(&mut self, view: String, content: Content) -> Result<(), StopReason> {
        if !self.views.contains_key(&view) {
            self.store.data_mut().host_log(LogLevel::Warn, format!("content for view “{view}”, which isn't open, was dropped"));
            return Ok(());
        }
        if content_size(&content) > MAX_CONTENT_BYTES {
            self.store.data_mut().host_log(LogLevel::Warn, format!("content over {} MiB for view “{view}” was dropped", MAX_CONTENT_BYTES >> 20));
            return Ok(());
        }
        self.handle(Event::ViewContent(ViewContent { view, content }))
    }

    fn handle(&mut self, event: Event) -> Result<(), StopReason> {
        arm(&mut self.store, HANDLE_DEADLINE);
        let t = Instant::now();
        let r = self.tile.pitwall_tile_guest().call_handle(&mut self.store, &event);
        let changed = r.map_err(|e| self.fail(&e, "handle", HANDLE_DEADLINE))?;
        let took = t.elapsed();
        if took > HANDLE_WARN {
            self.store.data_mut().host_log(LogLevel::Warn, format!("handling an event took {} ms (keep it under {} ms)", took.as_millis(), HANDLE_WARN.as_millis()));
        }
        if changed {
            self.dirty.extend(self.views.keys().cloned());
        }
        Ok(())
    }

    fn render_dirty(&mut self) -> Result<(), StopReason> {
        if !self.visible {
            return Ok(());
        }
        for view in std::mem::take(&mut self.dirty) {
            let Some(v) = self.views.get(&view) else { continue };
            let (prev, tile) = (v.frame.clone(), v.tile.clone());
            arm(&mut self.store, RENDER_DEADLINE);
            let t = Instant::now();
            let r = self.tile.pitwall_tile_guest().call_render(&mut self.store, &view);
            let new = r.map_err(|e| self.fail(&e, "render", RENDER_DEADLINE))?;
            let took = t.elapsed();
            if took > RENDER_WARN {
                self.store.data_mut().host_log(LogLevel::Warn, format!("rendering “{view}” took {} ms (keep it under 8 ms)", took.as_millis()));
            }
            frame::check(&new).map_err(|e| StopReason::FrameRefused(e.to_string()))?;
            let new = self.without_reserved_keys(new);
            let diff = frame::diff(prev.as_deref(), &new);
            if prev.as_deref().is_some_and(|p| diff.is_empty() && p.focus == new.focus && p.keymap == new.keymap) {
                continue;
            }
            let frame = Arc::new(new);
            if let Some(v) = self.views.get_mut(&view) {
                v.frame = Some(frame.clone());
            }
            (self.sink)(TileOutput::Frame(FrameUpdate { view, tile, frame, diff, took: self.batch.elapsed() }));
        }
        Ok(())
    }

    /// `frame` without the bindings of keys Pitwall keeps (ui.md §3.5),
    /// each told in the Log the first time.
    fn without_reserved_keys(&mut self, mut frame: Frame) -> Frame {
        if !frame.keymap.iter().any(|b| crate::keys::reserved(&b.keys)) {
            return frame;
        }
        let mut dropped = Vec::new();
        frame.keymap.retain(|b| {
            let r = crate::keys::reserved(&b.keys);
            if r {
                dropped.push(b.keys.clone());
            }
            !r
        });
        for k in dropped {
            if self.told_keys.insert(k.clone()) {
                self.store.data_mut().host_log(LogLevel::Warn, format!("the key binding “{}” was dropped: Pitwall keeps that key for itself (see the reserved keys in the docs)", crate::host::cut(&k, 40)));
            }
        }
        frame
    }

    /// After calls: stdout/stderr into the Log, the store to disk, and the
    /// queued Log lines and UI requests to the app.
    fn flush(&mut self) {
        let s = self.store.data_mut();
        s.drain_stdio(false);
        if let Err(e) = s.kv.flush() {
            s.host_log(LogLevel::Error, format!("its stored data couldn't be written: {e}"));
        }
        for p in std::mem::take(&mut s.pending) {
            (self.sink)(match p {
                Pending::Log(line) => TileOutput::Log(line),
                Pending::Ui(r) => TileOutput::Ui(r),
            });
        }
    }
}

impl Drop for Worker<'_> {
    fn drop(&mut self) {
        // The unfinished last lines of stdout and stderr, and anything
        // queued by the call that stopped.
        self.store.data_mut().drain_stdio(true);
        self.flush();
    }
}

/// Set the running call's deadline (wall clock) and arm the next tick.
fn arm(store: &mut Store<HostState>, limit: Duration) {
    store.data_mut().deadline = Instant::now() + limit;
    store.set_epoch_deadline(1);
}

/// Events of one kind for one target that piled up keep only the newest
/// (runtime.md §2.4: scroll, pointer moves, resize, typed text).
fn coalesce(batch: &mut Vec<Msg>) {
    fn key(m: &Msg) -> Option<(u8, String)> {
        match m {
            Msg::Event(e) => match e {
                Event::InputChanged(c) => Some((1, c.handler.to_string())),
                Event::InputSelected(s) => Some((2, s.handler.to_string())),
                Event::Scroll(s) => Some((3, s.handler.to_string())),
                Event::Hover(h) => Some((4, h.handler.to_string())),
                Event::Resize(r) => Some((5, r.view.clone())),
                Event::Pointer(p) if matches!(p.kind, PointerKind::Move) => Some((6, p.handler.to_string())),
                Event::AgentsChanged => Some((8, String::new())),
                _ => None,
            },
            Msg::Theme(_) => Some((7, String::new())),
            _ => None,
        }
    }
    let mut seen = HashSet::new();
    let keep: Vec<bool> = batch.iter().rev().map(|m| key(m).is_none_or(|k| seen.insert(k))).collect();
    let mut keep = keep.into_iter().rev();
    batch.retain(|_| keep.next().unwrap_or(true));
}
