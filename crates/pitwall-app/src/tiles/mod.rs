//! Extensions' tiles in the app (docs/spec/tile-sdk/runtime.md §2): one
//! [`TileHost`] for the app, with its files under Pitwall's data folder
//! (`<data>/cache/tiles/`, `<data>/extension-data/`), and the components
//! that run now, one per extension ([`Tiles`]).
//!
//! - A component starts when one of its tiles is shown (a [`TileView`] is
//!   made for it) and stops when its last view closes, or when the
//!   extension is turned off. Hidden views (another space, a tile
//!   maximised over it) are suspended: no renders, no timers
//!   ([`Tiles::set_visible`]).
//! - What the worker sends ([`TileOutput`]) arrives on a channel and is
//!   applied here on the UI thread: frames for the views, Log lines for
//!   the extension's Log in Settings, toasts and "open a view" applied
//!   with `cx.defer`, and why it stopped.
//! - Theme changes and settings written in Settings are forwarded.
//! - [`Tiles::reload`] restarts a component in place with the state it
//!   saved, for `pitwall extension dev` (`extension.devReload`).
//!
//! [`TileView`] draws a view's frames with the kit ([`render`]).

pub mod render;
#[cfg(test)]
mod tests;
pub mod view;

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui::{App, AppContext, Context, Entity, Global};
use pitwall_core::extensions::{permissions, Manifest};
use pitwall_tile_host::{ui, FrameUpdate, Grants, HostConfig, LogLine, StartOptions, StopReason, TileHandle, TileHost, TileOutput, UiRequest, ViewSpec};

pub use view::TileView;

/// The most Log lines kept per extension.
pub const LOG_KEEP: usize = 500;

/// How a component stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Started; `preparing`: compiling it for this computer first.
    Starting { preparing: bool },
    Running,
    /// It stopped, with why (Restart starts it again).
    Stopped(StopReason),
}

/// A running (or stopped) component.
pub struct Running {
    /// Which start this is: outputs of an earlier one are dropped.
    gen: u64,
    handle: Option<TileHandle>,
    pub state: State,
    /// Each open view's newest frame, and a counter that moves with it.
    pub frames: HashMap<String, (FrameUpdate, u64)>,
    /// The open views (view id = tile id).
    views: BTreeSet<String>,
    /// The views shown now.
    shown: BTreeSet<String>,
    pub log: VecDeque<LogLine>,
    /// A reload waits for the state the component saves.
    reloading: bool,
}

/// The components that run, and the host that runs them.
pub struct Tiles {
    host: Option<Result<TileHost, String>>,
    data_dir: Option<PathBuf>,
    tx: UnboundedSender<(String, u64, TileOutput)>,
    running: HashMap<String, Running>,
    next_gen: u64,
    secrets: Arc<AppSecrets>,
    seq: u64,
}

struct Handle(Entity<Tiles>);

impl Global for Handle {}

/// Tests: the data folder the host uses (else the one extensions are in).
pub struct DataDir(pub PathBuf);

impl Global for DataDir {}

/// The tiles' runtime (made on first use).
pub fn tiles(cx: &mut App) -> Entity<Tiles> {
    if let Some(h) = cx.try_global::<Handle>() {
        return h.0.clone();
    }
    let (tx, mut rx) = unbounded::<(String, u64, TileOutput)>();
    let store = crate::extensions::secret_store(cx);
    let e = cx.new(|cx: &mut Context<Tiles>| {
        // The user's appearance, forwarded as it changes.
        cx.observe_global::<crate::theme::Appearance>(|t: &mut Tiles, cx| {
            let theme = theme_info(cx);
            for r in t.running.values() {
                if let Some(h) = &r.handle {
                    h.set_theme(theme);
                }
            }
        })
        .detach();
        Tiles { host: None, data_dir: None, tx, running: HashMap::new(), next_gen: 1, secrets: Arc::new(AppSecrets { store, accounts: Mutex::new(HashMap::new()) }), seq: 0 }
    });
    cx.set_global(Handle(e.clone()));
    // Turned off: its component stops at once (runtime.md §2.3).
    let ui = crate::ui_state::store(cx);
    let weak = e.downgrade();
    cx.observe(&ui, move |_, cx| {
        let _ = weak.update(cx, |t, cx| {
            let ui = crate::ui_state::get(cx);
            let off: Vec<String> = t.running.keys().filter(|id| !ui.extension_on(id)).cloned().collect();
            for id in off {
                t.running.remove(&id);
                cx.notify();
            }
        });
    })
    .detach();
    let weak = e.downgrade();
    cx.spawn(async move |cx| {
        while let Some((id, gen, out)) = rx.next().await {
            if weak.update(cx, |t, cx| t.apply(&id, gen, out, cx)).is_err() {
                return;
            }
        }
    })
    .detach();
    e
}

/// The runtime, when it was made.
pub fn try_tiles(cx: &App) -> Option<Entity<Tiles>> {
    cx.try_global::<Handle>().map(|h| h.0.clone())
}

/// Whether `m` has tiles drawn by its component.
pub fn has_tiles(m: &Manifest) -> bool {
    !m.contributes.tiles.is_empty()
}

/// The user's appearance as a tile sees it.
pub fn theme_info(cx: &App) -> ui::ThemeInfo {
    let a = crate::theme::appearance(cx);
    ui::ThemeInfo {
        mode: match a.mode {
            crate::theme::Mode::Dark => ui::ThemeMode::Dark,
            crate::theme::Mode::Light => ui::ThemeMode::Light,
        },
        density: match a.density {
            crate::theme::Density::Comfortable => ui::Density::Comfortable,
            crate::theme::Density::Compact => ui::Density::Compact,
            crate::theme::Density::Dense => ui::Density::Dense,
        },
        reduce_motion: a.reduce_motion,
    }
}

/// What extension `m` is granted, as the host enforces it: the
/// permissions the user granted that tiles use (`memory`).
pub fn grants_of(m: &Manifest, cx: &App) -> Grants {
    let decisions = crate::ui_state::get(cx).extension(&m.id).grants;
    Grants::new(permissions::effective(m, &decisions))
}

/// Secrets from the app's secure store: each running extension's secret
/// settings by the account name of the connection Settings shows.
struct AppSecrets {
    store: Arc<dyn crate::platform::secrets::SecretStore>,
    accounts: Mutex<HashMap<String, HashMap<String, String>>>,
}

impl pitwall_tile_host::Secrets for AppSecrets {
    fn secret(&self, extension: &str, key: &str) -> Option<String> {
        let account = self.accounts.lock().ok()?.get(extension)?.get(key)?.clone();
        self.store.get(&account).ok().flatten()
    }
}

impl Tiles {
    /// The host, made on first use with Pitwall's data folder; why not.
    fn host(&mut self, cx: &App) -> Result<TileHost, String> {
        if let Some(h) = &self.host {
            return h.clone();
        }
        let dir = cx.try_global::<DataDir>().map(|d| d.0.clone()).or_else(|| crate::extensions::dir(cx).and_then(|d| d.parent().map(PathBuf::from)));
        let made = match dir {
            Some(dir) => {
                self.data_dir = Some(dir.clone());
                TileHost::new(HostConfig { data_dir: dir, secrets: self.secrets.clone() }).map_err(|e| e.to_string())
            }
            None => Err("Tiles need Pitwall's data folder.".into()),
        };
        self.host = Some(made.clone());
        made
    }

    /// Extension `id`'s component, if it was started.
    pub fn get(&self, id: &str) -> Option<&Running> {
        self.running.get(id)
    }

    /// Moves whenever anything a view draws changes.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// View `view` of extension `id` is open (its tile is in the layout or
    /// a window): its component started when it isn't running, the view
    /// opened when it is.
    pub fn open(&mut self, id: &str, view: &str, cx: &mut Context<Self>) {
        match self.running.get_mut(id) {
            Some(r) if r.handle.is_some() => {
                if r.views.insert(view.to_string()) {
                    if let Some(h) = &r.handle {
                        h.open_view(ViewSpec::tile(view));
                    }
                }
            }
            Some(r) => {
                // Stopped: it stays so until Restart; the view is noted.
                r.views.insert(view.to_string());
            }
            None => {
                let views = BTreeSet::from([view.to_string()]);
                self.start(id, views, None, cx);
            }
        }
        cx.notify();
    }

    /// View `view` closed: the component hears it; with no view left it
    /// stops (it starts again when a tile of it is opened).
    pub fn close(&mut self, id: &str, view: &str, cx: &mut Context<Self>) {
        let Some(r) = self.running.get_mut(id) else { return };
        r.views.remove(view);
        r.shown.remove(view);
        r.frames.remove(view);
        if r.views.is_empty() {
            self.running.remove(id);
        } else if let Some(h) = &r.handle {
            h.close_view(view);
            h.set_visible(!r.shown.is_empty());
        }
        cx.notify();
    }

    /// View `view` is shown (`true`) or hidden: the component is
    /// suspended while none of its views shows (runtime.md §2.3).
    pub fn set_visible(&mut self, id: &str, view: &str, shown: bool, _cx: &mut Context<Self>) {
        let Some(r) = self.running.get_mut(id) else { return };
        let before = !r.shown.is_empty();
        if shown {
            r.shown.insert(view.to_string());
        } else {
            r.shown.remove(view);
        }
        let now = !r.shown.is_empty();
        if before != now {
            if let Some(h) = &r.handle {
                h.set_visible(now);
            }
        }
    }

    /// Send `event` to extension `id`'s component (a click, typed text,
    /// a key…); nothing when it isn't running.
    pub fn send(&self, id: &str, event: ui::Event) -> bool {
        self.running.get(id).and_then(|r| r.handle.as_ref()).is_some_and(|h| h.send(event))
    }

    /// Restart: a stopped component (or a running one) started again
    /// with its open views.
    pub fn restart(&mut self, id: &str, cx: &mut Context<Self>) {
        let views = self.running.remove(id).map(|r| r.views).unwrap_or_default();
        if !views.is_empty() {
            self.start(id, views, None, cx);
        }
        cx.notify();
    }

    /// Reload in place (`pitwall extension dev`): the running component
    /// saves its state, then a new one starts from the folder with that
    /// state and the same views. A stopped one starts again.
    pub fn reload(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.running.get_mut(id) {
            Some(r) if r.handle.is_some() && r.state == State::Running => {
                r.reloading = true;
                if let Some(h) = &r.handle {
                    h.save_state();
                }
            }
            Some(_) => self.restart(id, cx),
            None => {}
        }
    }

    /// The settings of extension `id` changed: its component reads them
    /// again.
    pub fn settings_changed(&self, id: &str) {
        if let Some(h) = self.running.get(id).and_then(|r| r.handle.as_ref()) {
            h.settings_changed();
        }
    }

    /// A secret of extension `id` was set or forgotten: its component
    /// reads its settings again.
    pub fn secrets_changed(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(m) = crate::extensions::manifest_of(id, cx) {
            self.remember_secrets(&m, cx);
        }
        self.settings_changed(id);
    }

    /// The grants of extension `id` changed.
    pub fn grants_changed(&self, id: &str, grants: Grants) {
        if let Some(h) = self.running.get(id).and_then(|r| r.handle.as_ref()) {
            h.set_grants(grants);
        }
    }

    /// Write setting `key` of extension `id` where its component reads it
    /// (`<data>/extension-data/<id>/settings.json`) and tell it.
    pub fn write_setting(&mut self, id: &str, key: &str, value: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        let host = self.host(cx)?;
        host.extension_data(id).set_setting(key, value).map_err(|e| format!("Can't save it: {e}"))?;
        self.settings_changed(id);
        Ok(())
    }

    fn start(&mut self, id: &str, views: BTreeSet<String>, state: Option<Vec<u8>>, cx: &mut Context<Self>) {
        let gen = self.next_gen;
        self.next_gen += 1;
        let mut r = Running { gen, handle: None, state: State::Starting { preparing: false }, frames: HashMap::new(), views: views.clone(), shown: BTreeSet::new(), log: VecDeque::new(), reloading: false };
        let started = (|| -> Result<TileHandle, String> {
            let host = self.host(cx)?;
            let dir = crate::extensions::folder_of(id, cx).ok_or_else(|| "Its folder isn't found.".to_string())?;
            let ext = host.load(&dir).map_err(|e| format!("It can't be loaded: {e}."))?;
            let m = ext.manifest().clone();
            self.remember_secrets(&m, cx);
            let views: Vec<ViewSpec> = views.iter().filter(|v| m.contributes.tile(v).is_some()).map(|v| ViewSpec::tile(v)).collect();
            let options = StartOptions { views, grants: grants_of(&m, cx), theme: theme_info(cx), visible: false, state };
            let tx = self.tx.clone();
            let ext_id = id.to_string();
            host.start(&ext, options, move |out| {
                let _ = tx.unbounded_send((ext_id.clone(), gen, out));
            })
            .map_err(|e| format!("It can't be started: {e}."))
        })();
        match started {
            Ok(h) => r.handle = Some(h),
            Err(why) => r.state = State::Stopped(StopReason::LoadFailed(why)),
        }
        self.running.insert(id.to_string(), r);
        self.seq += 1;
    }

    /// The accounts extension `m`'s secrets are kept under.
    fn remember_secrets(&self, m: &Manifest, cx: &mut App) {
        let Some(conn) = crate::extensions::connections::settings_target(&m.id, cx) else { return };
        let keys: Vec<String> = m.settings.iter().filter(|s| s.kind == pitwall_core::extensions::SettingKind::Secret).map(|s| s.key.clone()).collect();
        let map: HashMap<String, String> = keys.into_iter().map(|k| (k.clone(), crate::extensions::account(conn.secrets_name(), &k, cx))).collect();
        if let Ok(mut a) = self.secrets.accounts.lock() {
            a.insert(m.id.clone(), map);
        }
    }

    fn apply(&mut self, id: &str, gen: u64, out: TileOutput, cx: &mut Context<Self>) {
        let Some(r) = self.running.get_mut(id).filter(|r| r.gen == gen) else { return };
        match out {
            TileOutput::Preparing => r.state = State::Starting { preparing: true },
            TileOutput::Started { .. } => {
                r.state = State::Running;
                // Shown views are drawn at once.
                if let Some(h) = &r.handle {
                    h.set_visible(!r.shown.is_empty());
                }
            }
            TileOutput::Frame(u) => {
                self.seq += 1;
                r.frames.insert(u.view.clone(), (u, self.seq));
            }
            TileOutput::Log(l) => {
                r.log.push_back(l);
                while r.log.len() > LOG_KEEP {
                    r.log.pop_front();
                }
                return;
            }
            TileOutput::Ui(req) => {
                let id = id.to_string();
                cx.defer(move |cx| ui_request(&id, req, cx));
                return;
            }
            TileOutput::StateSaved(bytes) => {
                if r.reloading {
                    let views = r.views.clone();
                    let shown = r.shown.clone();
                    let log = std::mem::take(&mut r.log);
                    self.running.remove(id);
                    self.start(id, views, Some(bytes), cx);
                    if let Some(n) = self.running.get_mut(id) {
                        n.shown = shown;
                        n.log = log;
                        n.log.push_back(note("Reloaded from its folder, with the state it saved."));
                    }
                }
            }
            TileOutput::Stopped(why) => {
                r.handle = None;
                r.log.push_back(note(&format!("Stopped: {why}.")));
                if r.reloading {
                    // It couldn't save its state: start it fresh.
                    let views = r.views.clone();
                    self.running.remove(id);
                    self.start(id, views, None, cx);
                } else {
                    r.state = State::Stopped(why);
                }
            }
        }
        self.seq += 1;
        cx.notify();
    }

    /// Extension `id`'s Log lines, oldest first.
    pub fn log(&self, id: &str) -> Vec<LogLine> {
        self.running.get(id).map(|r| r.log.iter().cloned().collect()).unwrap_or_default()
    }

    /// Bench and tests: which start of extension `id`'s component runs
    /// (a restart or a reload makes a new one).
    pub fn generation(&self, id: &str) -> Option<u64> {
        self.running.get(id).map(|r| r.gen)
    }

    /// Bench and tests: the open views of extension `id`.
    pub fn views(&self, id: &str) -> Vec<String> {
        self.running.get(id).map(|r| r.views.iter().cloned().collect()).unwrap_or_default()
    }
}

/// A line Pitwall writes in a tile's Log.
fn note(text: &str) -> LogLine {
    LogLine { level: pitwall_tile_host::LogLevel::Info, message: text.to_string(), at: std::time::SystemTime::now(), from_host: true }
}

/// Apply a tile's UI request, labelled with its extension's name.
fn ui_request(id: &str, req: UiRequest, cx: &mut App) {
    let name = crate::extensions::manifest_of(id, cx).map(|m| m.name).unwrap_or_else(|| id.to_string());
    match req {
        UiRequest::Toast { text, .. } => crate::windows::extension_toast(id, name, text, None, cx),
        UiRequest::OpenView(tile) => crate::windows::open_extension_view(id, &tile, cx),
        UiRequest::OpenSettings => crate::windows::open_extension_settings(id, cx),
        // Scrolling to a node: drawn views follow their frames; not yet.
        UiRequest::ScrollTo { .. } => {}
    }
}

/// Extension `id`'s Log lines as Settings shows them.
pub fn log_lines(id: &str, cx: &App) -> Vec<pitwall_core::extensions::log::Line> {
    use pitwall_core::extensions::log::{Dir, Line};
    let Some(t) = try_tiles(cx) else { return vec![] };
    t.read(cx)
        .log(id)
        .into_iter()
        .map(|l| {
            let dir = match (l.from_host, l.level) {
                (true, _) => Dir::Note,
                (_, pitwall_tile_host::LogLevel::Warn | pitwall_tile_host::LogLevel::Error) => Dir::Stderr,
                _ => Dir::FromAdapter,
            };
            let text = match l.level {
                pitwall_tile_host::LogLevel::Debug => format!("debug: {}", l.message),
                pitwall_tile_host::LogLevel::Warn => format!("warn: {}", l.message),
                pitwall_tile_host::LogLevel::Error => format!("error: {}", l.message),
                pitwall_tile_host::LogLevel::Info => l.message,
            };
            Line { dir, at: l.at, text }
        })
        .collect()
}

/// A stop reason as a sentence for the tile ("It didn't answer in time
/// (handle took over 1000 ms).").
pub fn stop_words(r: &StopReason) -> String {
    // A crash names the trap, not Wasmtime's backtrace (that is in the
    // Log).
    let s = match r {
        StopReason::Crashed(why) => {
            let trap = why.rfind("wasm trap:").map(|i| &why[i..]).unwrap_or_else(|| why.lines().next().unwrap_or(""));
            format!("it crashed: {}", trap.trim().trim_end_matches('.'))
        }
        r => r.to_string(),
    };
    let s = match s.char_indices().nth(240) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    };
    let mut c = s.chars();
    match c.next() {
        Some(f) => format!("{}{}.", f.to_uppercase(), c.as_str()),
        None => String::new(),
    }
}

/// The extensions' views each main window drew in its last render, by
/// window label: a tile not drawn anywhere is hidden (another space, a
/// tile maximised over it) and its component suspended.
#[derive(Default)]
struct Drawn(HashMap<String, BTreeSet<String>>);

impl Global for Drawn {}

/// Main window `label` starts drawing: what it drew before is checked
/// once it is done.
pub fn drawing(label: &str, cx: &mut App) {
    let before = cx.default_global::<Drawn>().0.insert(label.to_string(), BTreeSet::new()).unwrap_or_default();
    if before.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        let drawn: BTreeSet<String> = cx.default_global::<Drawn>().0.values().flatten().cloned().collect();
        for key in before.difference(&drawn) {
            // In a window of its own: drawn there.
            if crate::windows::inbox_window(key, cx).is_some() {
                continue;
            }
            let tile = crate::main_screen::view_tile::get(key, cx).and_then(|v| v.read(cx).tile().cloned());
            if let Some(tile) = tile {
                tile.update(cx, |t, cx| t.set_shown(false, cx));
            }
        }
    });
}

/// Main window `label` drew view `key` (`<extension>/<view>`).
pub fn drew(label: &str, key: &str, cx: &mut App) {
    cx.default_global::<Drawn>().0.entry(label.to_string()).or_default().insert(key.to_string());
}
