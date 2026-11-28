//! Runs extensions' tile components (docs/spec/tile-sdk/runtime.md §2):
//! Wasmtime, the WIT world `pitwall:tile@0.1.0` (`wit/`), one worker
//! thread per running component, the limits of §2.5, Pitwall's own
//! compile cache (§2.6) and the host side of `ui`, `log`, `tasks`,
//! `storage` and `settings`. No GPUI: the app draws the frames this crate
//! hands it. The API is described in this crate's README.md.
//!
//! - [`TileHost`]: one per app. The engine, the linker, the epoch ticker
//!   and Pitwall's data folder.
//! - [`Extension`]: an extension folder with tiles, checked
//!   ([`TileHost::load`]).
//! - [`TileHost::start`]: compiles (or loads from the cache) and runs the
//!   extension's component on its own thread, `tile:<id>`, and returns a
//!   [`TileHandle`] at once. Everything the tile produces arrives through
//!   the sink as [`TileOutput`]s: frames with their diff, Log lines, UI
//!   requests, and why it stopped.
//! - [`ui`]: the element tree and event types of the WIT world.
//! - [`ExtensionData`]: an extension's settings and stored data on disk.

pub mod agent;
mod bindings;
pub mod blocks;
mod cache;
mod clock;
pub mod components;
mod data;
pub mod files;
pub mod frame;
mod group;
mod host;
mod ignore;
/// Key bindings and the keys Pitwall keeps (shared with the manifest's
/// shortcuts, in `pitwall-core`).
pub use pitwall_core::extensions::keys;
pub mod limits;
pub mod measure;
pub mod net;
mod platform;
mod services;
pub mod sql;
mod stdio;
pub mod widgets;
mod worker;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use pitwall_core::extensions::{self, Manifest, TileDef};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine};

pub use clock::{Clock, ManualClock};
pub use data::ExtensionData;
pub use frame::Diff;
pub use group::{MethodError, Methods, NoMethods};
pub use worker::TileHandle;

/// The element tree, events and other types of `pitwall:tile@0.1.0`, as
/// Rust types (generated from `wit/`; each item's WIT doc comment says
/// what it means).
pub mod ui {
    pub use crate::bindings::pitwall::tile::agents::{Agent, AgentState};
    pub use crate::bindings::pitwall::tile::asks::{Ask, Suggestion};
    pub use crate::bindings::pitwall::tile::badges::{Badge, Row};
    /// A row of a virtual list, table or tree (`Row` is a badge's row).
    pub use crate::bindings::pitwall::tile::types::Row as ListRow;
    pub use crate::bindings::pitwall::tile::settings::SettingValue;
    pub use crate::bindings::pitwall::tile::service::{ServiceInfo, State as ServiceState};
    pub use crate::bindings::pitwall::tile::toolbar::{Badge as ToolbarBadge, BadgeTone};
    /// The project a view is open in (`files.project`).
    pub use crate::bindings::pitwall::tile::files::ProjectInfo as Project;
    pub use crate::bindings::pitwall::tile::types::*;

    /// The appearance a tile starts with when the app gives none: dark,
    /// compact, motion on.
    pub fn default_theme() -> ThemeInfo {
        ThemeInfo { mode: ThemeMode::Dark, density: Density::Compact, reduce_motion: false }
    }
}

/// The permission for linear memory above the default
/// ([`Grants::memory_limit`]).
pub const MEMORY_PERMISSION: &str = "memory";
/// The permission for `net`, with the manifest's hosts.
pub const NET_PERMISSION: &str = "net";
/// The permission for a database above the default size.
pub const SQL_PERMISSION: &str = "sql";
/// The permission for `notify`.
pub const NOTIFY_PERMISSION: &str = "notify";
/// The permission for `show`.
pub const SHOW_PERMISSION: &str = "show";
/// The permission for reading a project's files (`files`).
pub const FILES_READ_PERMISSION: &str = "files.read";
/// The permission for changing a project's files (`files`).
pub const FILES_WRITE_PERMISSION: &str = "files.write";
/// The view id of the toolbar popover (ui.md §3.9): a view of the tile
/// the manifest's popover action names. Tile ids can't hold a `:`.
pub const POPOVER_VIEW: &str = "toolbar:popover";

/// The WIT worlds this host serves, newest first.
pub const WORLDS: [&str; 1] = ["pitwall:tile@0.1.0"];

/// Where the host keeps files, and what it needs from the app.
#[derive(Clone)]
pub struct HostConfig {
    /// Pitwall's data folder (the caller's; this crate never guesses
    /// one). Compiled components go in `<data>/cache/tiles/<id>/`, each
    /// extension's storage and settings in `<data>/extension-data/<id>/`.
    pub data_dir: PathBuf,
    /// Reads `secret` settings from the system's secure store.
    pub secrets: Arc<dyn Secrets>,
    /// Answers `agents.list` and `agents.queue-add` (the app's method
    /// table, as adapters reach it).
    pub methods: Arc<dyn Methods>,
}

impl HostConfig {
    /// A config with no secret store (every secret reads as unset) and no
    /// method table (every agents call fails).
    pub fn new(data_dir: impl Into<PathBuf>) -> HostConfig {
        HostConfig { data_dir: data_dir.into(), secrets: Arc::new(NoSecrets), methods: Arc::new(NoMethods) }
    }
}

/// The app's secure store, for `settings.secret` (called on the tile's
/// worker thread, so it must not need the UI thread).
pub trait Secrets: Send + Sync {
    /// The value of extension `extension`'s secret setting `key`, if set.
    fn secret(&self, extension: &str, key: &str) -> Option<String>;
}

/// No secret store: every secret reads as unset.
pub struct NoSecrets;

impl Secrets for NoSecrets {
    fn secret(&self, _extension: &str, _key: &str) -> Option<String> {
        None
    }
}

/// The permissions the user granted an extension (permissions.md §5),
/// which the host enforces at each call: `agents.read` and
/// `agents.queue` (`agents`), `projects.read` (badges on project rows,
/// a toolbar's project badge), `net` (with the manifest's hosts),
/// `notify` and `show`. An interface whose grant is missing returns
/// `denied`, so the tile loads and degrades instead of failing
/// (permissions.md §5.2). `memory` and `sql` raise limits to the sizes
/// the manifest names (linear memory instead of 64 MiB, the database
/// instead of 100 MiB). Besides, its own server's address
/// ([`Grants::with_own_server`]), reachable without `net` (services.md
/// §13.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants {
    granted: BTreeSet<String>,
    hosts: Vec<pitwall_core::extensions::net::HostPattern>,
    sql_mib: Option<u32>,
    own_server: Option<std::net::SocketAddr>,
}

impl Grants {
    /// These permissions, by name (no `net` hosts: see
    /// [`Grants::for_extension`]).
    pub fn new<I: IntoIterator<Item = S>, S: Into<String>>(granted: I) -> Grants {
        Grants { granted: granted.into_iter().map(Into::into).collect(), ..Grants::default() }
    }

    /// These permissions of extension `m`, with what its manifest names
    /// for them: `net`'s hosts and `sql`'s size.
    pub fn for_extension<I: IntoIterator<Item = S>, S: Into<String>>(m: &Manifest, granted: I) -> Grants {
        Grants { hosts: m.net_hosts.clone(), sql_mib: m.sql_mib, ..Grants::new(granted) }
    }

    /// With the user's plain settings `values` (by key) of `m`: `net`
    /// also reaches the host of each `{setting:KEY}` address set
    /// ([`pitwall_core::extensions::permissions::net_hosts_with`]). Give
    /// the grants again when those settings change.
    pub fn with_settings(self, m: &Manifest, values: &std::collections::BTreeMap<String, String>) -> Grants {
        Grants { hosts: pitwall_core::extensions::permissions::net_hosts_with(m, values), ..self }
    }

    /// Its own server listens on `addr` (its `ready.listen`, with the
    /// user's settings): its tiles may reach it over http without `net`.
    pub fn with_own_server(self, addr: Option<std::net::SocketAddr>) -> Grants {
        Grants { own_server: addr.filter(|a| a.ip().is_loopback()), ..self }
    }

    /// Where its tiles' requests may go.
    pub fn reach(&self) -> net::Reach {
        let hosts = if self.has(NET_PERMISSION) { self.hosts.clone() } else { vec![] };
        net::Reach { hosts, own_server: self.own_server }
    }

    /// The most its database may take, in bytes.
    pub fn sql_quota(&self) -> u64 {
        match self.sql_mib {
            Some(mib) if self.has(SQL_PERMISSION) => u64::from(mib) << 20,
            _ => limits::SQL_MIB << 20,
        }
    }

    /// These grants without the acting ones (`net` and the tile's own
    /// server, `show`, `notify`, `agents.queue`, `agents.start`,
    /// `agents.focus`, `files.write`): what an agent's `show`, `present` or `read` call
    /// runs with (agent-tools.md §11.2).
    pub fn without_acting(&self) -> Grants {
        use pitwall_core::extensions::permissions as p;
        let acting = [p::NET, p::SHOW, p::NOTIFY, p::AGENTS_QUEUE, p::AGENTS_START, p::AGENTS_FOCUS, p::FILES_WRITE];
        Grants { granted: self.granted.iter().filter(|g| !acting.contains(&g.as_str()) && !g.starts_with("net.")).cloned().collect(), own_server: None, ..self.clone() }
    }

    /// Whether `permission` is granted.
    pub fn has(&self, permission: &str) -> bool {
        self.granted.contains(permission)
    }

    /// The granted permissions, by name.
    pub fn names(&self) -> Vec<String> {
        self.granted.iter().cloned().collect()
    }

    /// The linear memory a tile of `manifest` may use with these grants.
    pub fn memory_limit(&self, manifest: &Manifest) -> usize {
        match manifest.memory_mib {
            Some(mib) if self.has(MEMORY_PERMISSION) => (mib as usize) << 20,
            _ => limits::MEMORY,
        }
    }
}

/// Why an extension can't be loaded or started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Its manifest is missing or wrong; the text says how.
    Manifest(String),
    /// It contributes no tiles.
    NoTiles,
    /// Its tiles name different components; one extension has one
    /// component (extension-model.md §15).
    SeveralComponents,
    /// A tile was built for a world this Pitwall doesn't serve.
    World { tile: String, world: String },
    /// Its component file is missing, unreadable, a link or too large.
    Wasm(String),
    /// The file isn't a component this Pitwall can compile.
    Compile(String),
    /// The component imports something this Pitwall doesn't provide.
    Imports(String),
    /// No tile with this id.
    UnknownTile(String),
    /// The engine or a thread couldn't be made.
    Host(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Manifest(why) => write!(f, "{why}"),
            LoadError::NoTiles => write!(f, "it contributes no tiles (\"contributes.tiles\")"),
            LoadError::SeveralComponents => write!(f, "its tiles name different .wasm files: an extension has one component"),
            LoadError::World { tile, world } => write!(f, "tile “{tile}” was built for {world}, which this Pitwall doesn't run (it runs {}); it may need a newer Pitwall", WORLDS.join(", ")),
            LoadError::Wasm(why) => write!(f, "{why}"),
            LoadError::Compile(why) => write!(f, "{why}"),
            LoadError::Imports(what) => write!(f, "it needs {what}, which this Pitwall doesn't provide; it may need a newer Pitwall"),
            LoadError::UnknownTile(id) => write!(f, "it has no tile “{id}”"),
            LoadError::Host(why) => write!(f, "the tile runtime couldn't start: {why}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// An extension folder with tiles, checked: its manifest, its component
/// file (present, a plain file, at most 20 MiB) and its world.
#[derive(Debug, Clone)]
pub struct Extension {
    dir: PathBuf,
    manifest: Manifest,
    wasm: PathBuf,
}

impl Extension {
    /// Read and check the extension in `dir` (installed, or a development
    /// folder used in place). Reads only the manifest and the file's
    /// metadata; nothing is compiled.
    pub fn load(dir: &Path) -> Result<Extension, LoadError> {
        let manifest = extensions::read_in_place(dir).map_err(LoadError::Manifest)?;
        let tiles = &manifest.contributes.tiles;
        let first = tiles.first().ok_or(LoadError::NoTiles)?;
        if tiles.iter().any(|t| t.wasm != first.wasm) {
            return Err(LoadError::SeveralComponents);
        }
        for t in tiles {
            if let Some(w) = &t.world {
                if !WORLDS.contains(&w.as_str()) {
                    return Err(LoadError::World { tile: t.id.clone(), world: w.clone() });
                }
            }
        }
        for t in tiles {
            if let Some(schema) = &t.input {
                if !dir.join(schema).is_file() {
                    return Err(LoadError::Manifest(format!("tile “{}”: its input schema {schema} isn't there", t.id)));
                }
            }
        }
        let wasm = dir.join(&first.wasm);
        let meta = std::fs::symlink_metadata(&wasm).map_err(|_| LoadError::Wasm(format!("its component {} isn't there", first.wasm)))?;
        if !meta.is_file() {
            return Err(LoadError::Wasm(format!("its component {} must be a plain file (not a link or a folder)", first.wasm)));
        }
        if meta.len() > limits::MAX_WASM_BYTES {
            return Err(LoadError::Wasm(format!("its component {} is larger than {} MiB", first.wasm, limits::MAX_WASM_BYTES >> 20)));
        }
        Ok(Extension { dir: dir.to_path_buf(), manifest, wasm })
    }

    /// Its id.
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    /// Its manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Its folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Its tiles (the views its component renders).
    pub fn tiles(&self) -> &[TileDef] {
        &self.manifest.contributes.tiles
    }

    /// Its Setup view's id (`contributes.setup.view`), when it has one:
    /// open it with [`ViewSpec::tile`] of this id, on the extension's
    /// page (ui.md §3.12).
    pub fn setup_view(&self) -> Option<&str> {
        self.manifest.contributes.setup.as_ref().map(|s| s.view.as_str())
    }

    /// Its component file.
    pub fn wasm_path(&self) -> &Path {
        &self.wasm
    }

    fn read_wasm(&self) -> Result<Vec<u8>, LoadError> {
        let bytes = std::fs::read(&self.wasm).map_err(|e| LoadError::Wasm(format!("its component can't be read: {e}")))?;
        if bytes.len() as u64 > limits::MAX_WASM_BYTES {
            return Err(LoadError::Wasm(format!("its component is larger than {} MiB", limits::MAX_WASM_BYTES >> 20)));
        }
        Ok(bytes)
    }
}

/// What [`TileHost::compile`] did: the component is now in the compile
/// cache, so the next [`TileHost::start`] loads it at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compiled {
    /// It came from the compile cache.
    pub cached: bool,
    /// How long loading or compiling took.
    pub took: Duration,
}

/// How a tile starts.
#[derive(Debug, Clone)]
pub struct StartOptions {
    /// The views open at start; each gets `view-opened` after `init`, is
    /// rendered, and its frames arrive with its view id. More with
    /// [`TileHandle::open_view`].
    pub views: Vec<ViewSpec>,
    /// What the user granted.
    pub grants: Grants,
    /// The user's appearance.
    pub theme: ui::ThemeInfo,
    /// Shown at start (false: it starts suspended, renders once shown).
    pub visible: bool,
    /// What `save-state` returned last time ([`TileOutput::StateSaved`]).
    pub state: Option<Vec<u8>>,
    /// The time it lives by: the wall clock (the app), or a virtual one a
    /// test moves with [`TileHandle::advance`].
    pub clock: Clock,
}

impl StartOptions {
    /// Visible, with tile `tile` open once (view id = tile id), nothing
    /// granted, the default theme, the wall clock.
    pub fn view(tile: &str) -> StartOptions {
        StartOptions { views: vec![ViewSpec::tile(tile)], grants: Grants::default(), theme: ui::default_theme(), visible: true, state: None, clock: Clock::Wall }
    }
}

/// A view to open: one place in the layout showing one of the
/// extension's tiles. A view is addressed by (extension id, tile, view
/// id); the app chooses the view id and keeps it while the view is open,
/// so later content for it (an agent's call with the same content id)
/// goes to [`TileHandle::set_content`] instead of a new view.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewSpec {
    /// The view's id, unique among the extension's open views.
    pub id: String,
    /// Which of the manifest's tiles it shows.
    pub tile: String,
    /// What it opens with (checked by the app against the tile's
    /// `input` schema); at most 8 MiB of JSON and bytes.
    pub content: Option<ui::Content>,
    /// The project it is open in (its space's, or the agent's it opened
    /// next to): the only project folder `files` reaches for it. Later
    /// moves go to [`TileHandle::set_project`].
    pub project: Option<ui::Project>,
}

impl ViewSpec {
    /// Tile `tile` opened once, with no content: its view id is the tile's
    /// id. Also the Setup view, by its id ([`Extension::setup_view`]).
    pub fn tile(tile: &str) -> ViewSpec {
        ViewSpec { id: tile.to_string(), tile: tile.to_string(), content: None, project: None }
    }

    /// This view, open in `project`.
    pub fn in_project(self, project: Option<ui::Project>) -> ViewSpec {
        ViewSpec { project, ..self }
    }
}

/// The most content a view is given at once (JSON and bytes together).
pub const MAX_CONTENT_BYTES: usize = 8 << 20;

/// A Log line's level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// A line for the tile's Log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub level: LogLevel,
    pub message: String,
    pub at: SystemTime,
    /// Written by the host about the tile (a slow call, a dropped toast),
    /// not by the tile.
    pub from_host: bool,
}

/// Something the tile asked of the UI; the app applies it later (with
/// `cx.defer`, runtime.md §2.4).
#[derive(Debug, Clone, PartialEq)]
pub enum UiRequest {
    /// Show a toast, labelled with the extension's name.
    Toast { text: String, tone: ui::Tone },
    /// Open one of its tiles (a tile id from the manifest), or focus a
    /// view of it that is open.
    OpenView(String),
    /// Scroll the node at `key_path` into sight after `view`'s next frame.
    ScrollTo { view: String, key_path: String },
    /// Open the extension's card in Settings.
    OpenSettings,
    /// Its badges on sidebar rows now (`badges.set`): the whole set,
    /// checked against the limits and its grants.
    Badges(Vec<ui::Badge>),
    /// Its open questions now (`asks.set`): the whole set, checked.
    Asks(Vec<ui::Ask>),
    /// Its reply to an answer (`asks.reply`): taken, or why not.
    AskResult { id: String, refused: Option<String> },
    /// Its connection's status (`status.set`), as an adapter reports it.
    Status { state: pitwall_core::extensions::protocol::ConnState, message: String },
    /// `ui.put-image`: keep these bytes (sniffed: one of the
    /// [`components::ImageFormat`]s, within the limits) for `image` nodes
    /// with source `data(name)`. Sent before the frames rendered after it.
    PutImage { name: String, format: components::ImageFormat, bytes: Arc<[u8]> },
    /// `ui.drop-image`: forget the image put under this name.
    DropImage(String),
    /// A system notification (`notify.send`, granted `notify`, within its
    /// rate): labelled with the extension's name; a click opens its tile.
    Notify { title: String, body: String },
    /// Open `path` in the editor at `line` (`show.file`, granted `show`).
    OpenFile { path: String, line: Option<u32> },
    /// Open `url` in a new web tile (`show.web`, granted `show`).
    OpenWeb { url: String },
    /// A change to its toolbar button (ui.md §3.9).
    Toolbar(ToolbarRequest),
    /// Start its own server again (`service.restart`).
    RestartService,
    /// Show `path` in the tile's file viewer at `line` (`show.view`,
    /// granted `show`).
    ViewFile { path: String, line: Option<u32> },
    /// A line for the extension's Activity: a call that changed a
    /// project's files (`files.write`, `files.rename`…; `detail` names
    /// the project and the path), or one that was refused and why.
    Activity { method: String, detail: String, refused: Option<String> },
}

/// Rows of a virtual list, table or tree, as the tile returned them for
/// [`TileHandle::rows`].
#[derive(Debug, Clone)]
pub struct RowsUpdate {
    pub view: String,
    /// The list's key path.
    pub list: String,
    /// The list's revision in the newest frame the worker rendered when it
    /// asked: the rows are as the tile's state was then.
    pub revision: u64,
    /// The first row's index.
    pub start: u32,
    /// The rows, checked ([`components::check_rows`]); fewer than asked at
    /// the end of the list.
    pub rows: Arc<Vec<ui::ListRow>>,
    /// How long `guest.rows` took.
    pub took: Duration,
}

/// A change a tile asks for on its toolbar button. The app keeps the
/// newest of each kind, and draws badge changes at most
/// [`limits::TOOLBAR_BADGES_PER_SECOND`] times a second.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolbarRequest {
    /// The badge: everywhere (`project: None`), or in project `project`'s
    /// space for a per-project item.
    Badge { project: Option<String>, badge: ui::ToolbarBadge, tone: ui::BadgeTone },
    /// The tooltip (at most 80 characters).
    Tooltip(String),
    /// Shown or hidden by the tile (never over the user's Hide).
    Shown(bool),
    /// Close the popover.
    ClosePopover,
}

/// A new frame of a view.
#[derive(Debug, Clone)]
pub struct FrameUpdate {
    /// Which view.
    pub view: String,
    /// The tile it shows.
    pub tile: String,
    /// The whole tree, checked against the limits.
    pub frame: Arc<ui::Frame>,
    /// How it differs from the view's previous frame.
    pub diff: Diff,
    /// The worker's time from taking the events to the lifted frame.
    pub took: Duration,
}

/// Why a tile stopped. After any of these the worker is gone; the app
/// shows the reason and offers Restart (a new [`TileHost::start`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// [`TileHandle::stop`], or the handle was dropped.
    Stopped,
    /// A call ran past its deadline.
    Deadline { call: &'static str, limit: Duration },
    /// It trapped (a panic, a memory limit, a bug); the text is Wasmtime's.
    Crashed(String),
    /// It sent a frame over the limits.
    FrameRefused(String),
    /// `init` returned an error; the text is the tile's.
    InitFailed(String),
    /// It couldn't be loaded, compiled or instantiated.
    LoadFailed(String),
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopReason::Stopped => write!(f, "it was stopped"),
            StopReason::Deadline { call, limit } => write!(f, "it didn't answer in time ({call} took over {} ms)", limit.as_millis()),
            StopReason::Crashed(why) => write!(f, "it crashed: {why}"),
            StopReason::FrameRefused(why) => write!(f, "its view was too large or malformed: {why}"),
            StopReason::InitFailed(why) => write!(f, "it couldn't start: {why}"),
            StopReason::LoadFailed(why) => write!(f, "it couldn't be loaded: {why}"),
        }
    }
}

/// What a running tile produces, in order, through the sink given to
/// [`TileHost::start`]. Called on the tile's worker thread: the sink
/// should only forward (a channel send, a foreground-task wakeup).
#[derive(Debug, Clone)]
pub enum TileOutput {
    /// Not in the compile cache: compiling now ("Preparing…").
    Preparing,
    /// Instantiated and `init` returned ok.
    Started { cached: bool, took: Duration },
    /// A view's new frame.
    Frame(FrameUpdate),
    /// A line for its Log.
    Log(LogLine),
    /// A UI request.
    Ui(UiRequest),
    /// What `save-state` returned ([`TileHandle::save_state`]).
    StateSaved(Vec<u8>),
    /// An agent's call (content with `call`, [`agent::ShowCalls::begin`])
    /// was handled, answered or not delivered: answer the agent now.
    Answered { call: u64, answer: agent::Answer },
    /// The tile said the user changed what call `call` showed
    /// (`agent-tools.user-edited`): refuse that agent's later calls with
    /// the same item ([`agent::ShowCalls::user_edited`]).
    UserEdited(u64),
    /// It stopped; nothing follows.
    Stopped(StopReason),
    /// Rows asked for with [`TileHandle::rows`].
    Rows(RowsUpdate),
}

/// The tile runtime: one per app, cheap to clone. Holds the engine, the
/// linker and the epoch ticker; dropped with the last clone and the last
/// running tile.
#[derive(Clone)]
pub struct TileHost {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub engine: Engine,
    pub linker: Linker<host::HostState>,
    pub config: HostConfig,
    /// Makes `net` requests ([`TileHost::set_fetcher`]).
    pub fetcher: std::sync::RwLock<Arc<dyn net::Fetcher>>,
    /// Large inputs of agents' calls ([`TileHost::large_inputs`]).
    pub large: agent::LargeInputs,
    /// Follows changes for `files.watch` ([`TileHost::set_watcher`]).
    pub watcher: std::sync::RwLock<Arc<dyn files::Watcher>>,
    /// Where `files.delete` puts a project's files ([`TileHost::set_trash`]).
    pub trash: std::sync::RwLock<Arc<dyn files::Trash>>,
    _ticker: EpochTicker,
}

impl TileHost {
    /// Make the engine (Cranelift, epoch interruption) and start its
    /// epoch ticker thread (`tile-epoch`).
    pub fn new(config: HostConfig) -> Result<TileHost, LoadError> {
        let mut c = Config::new();
        c.epoch_interruption(true);
        c.parallel_compilation(true);
        let engine = Engine::new(&c).map_err(|e| LoadError::Host(format!("{e:#}")))?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|e| LoadError::Host(format!("{e:#}")))?;
        bindings::Tile::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s).map_err(|e| LoadError::Host(format!("{e:#}")))?;
        let ticker = EpochTicker::start(&engine).map_err(|e| LoadError::Host(e.to_string()))?;
        Ok(TileHost { inner: Arc::new(Inner { engine, linker, config, fetcher: std::sync::RwLock::new(Arc::new(net::HttpFetcher)), large: agent::LargeInputs::default(), watcher: std::sync::RwLock::new(Arc::new(files::FsWatcher)), trash: std::sync::RwLock::new(Arc::new(files::SystemTrash)), _ticker: ticker }) })
    }

    /// [`Extension::load`].
    pub fn load(&self, dir: &Path) -> Result<Extension, LoadError> {
        Extension::load(dir)
    }

    /// Pitwall's data folder, as configured.
    pub fn data_dir(&self) -> &Path {
        &self.inner.config.data_dir
    }

    /// Extension `id`'s settings and stored data.
    pub fn extension_data(&self, id: &str) -> ExtensionData {
        ExtensionData::new(&self.inner.config.data_dir, id)
    }

    /// Whether `ext`'s component is in the compile cache (so starting it
    /// is quick and shows no "Preparing…").
    pub fn is_compiled(&self, ext: &Extension) -> bool {
        ext.read_wasm().is_ok_and(|w| cache::cache_file(&self.inner.config.data_dir, ext.id(), &w).is_file())
    }

    /// Load `ext`'s component from the compile cache, or compile it and
    /// keep it there, and check its imports. Blocks (tens to hundreds of
    /// milliseconds on a miss): call it off the UI thread, as `pitwall
    /// extension check` does. [`TileHost::start`] does this on the tile's
    /// worker itself.
    pub fn compile(&self, ext: &Extension) -> Result<Compiled, LoadError> {
        let wasm = ext.read_wasm()?;
        let t = Instant::now();
        let (component, cached) = cache::load_or_compile(&self.inner.engine, &self.inner.config.data_dir, ext.id(), &wasm).map_err(LoadError::Compile)?;
        self.check_imports(&component)?;
        Ok(Compiled { cached, took: t.elapsed() })
    }

    /// Every import must be in a world this host serves, or WASI 0.2.
    fn check_imports(&self, component: &Component) -> Result<(), LoadError> {
        let ty = component.component_type();
        let unknown: Vec<String> = ty
            .imports(&self.inner.engine)
            .map(|(name, _)| name.to_string())
            .filter(|name| {
                let pitwall = WORLDS.iter().any(|w| {
                    let (pkg, ver) = w.split_once('@').unwrap_or((w, ""));
                    name.strip_prefix(pkg).and_then(|r| r.strip_prefix('/')).and_then(|r| r.split_once('@')).is_some_and(|(_, v)| v == ver)
                });
                let wasi = name.starts_with("wasi:") && name.contains("@0.2.");
                !(pitwall || wasi)
            })
            .collect();
        if unknown.is_empty() {
            Ok(())
        } else {
            Err(LoadError::Imports(unknown.join(", ")))
        }
    }

    /// Start `ext`'s component on a new worker thread, `tile:<id>`, and
    /// return its handle at once. The worker compiles or loads it
    /// ([`TileOutput::Preparing`] on a cache miss), instantiates it with
    /// the limits and grants, calls `init`, renders the open views, then
    /// handles events. Everything arrives through `sink`, on the worker
    /// thread. Errors only for views the manifest doesn't declare and a
    /// thread that can't be made; a component that fails to load arrives
    /// as `Stopped(LoadFailed)`.
    pub fn start(&self, ext: &Extension, options: StartOptions, sink: impl FnMut(TileOutput) + Send + 'static) -> Result<TileHandle, LoadError> {
        if let Some(v) = options.views.iter().find(|v| ext.manifest.contributes.tile(&v.tile).is_none() && ext.setup_view() != Some(v.tile.as_str())) {
            return Err(LoadError::UnknownTile(v.tile.clone()));
        }
        worker::spawn(self.clone(), ext.clone(), options, Box::new(sink))
    }

    /// Make tiles' `net` requests with `fetcher` from now on (tiles started
    /// after this): the test harness's fake answers ([`net::FakeFetcher`]).
    /// Pitwall's own, [`net::HttpFetcher`], is the default.
    pub fn set_fetcher(&self, fetcher: Arc<dyn net::Fetcher>) {
        if let Ok(mut f) = self.inner.fetcher.write() {
            *f = fetcher;
        }
    }

    /// Follow `files.watch` changes with `watcher` from now on (tiles
    /// started after this): the harness's [`files::FakeWatcher`]. The
    /// system's events ([`files::FsWatcher`]) are the default.
    pub fn set_watcher(&self, watcher: Arc<dyn files::Watcher>) {
        if let Ok(mut w) = self.inner.watcher.write() {
            *w = watcher;
        }
    }

    /// Put what `files.delete` removes from a project in `trash` from now
    /// on (tiles started after this): tests' [`files::FolderTrash`], so
    /// they never fill the user's Trash. The system's is the default.
    pub fn set_trash(&self, trash: Arc<dyn files::Trash>) {
        if let Ok(mut t) = self.inner.trash.write() {
            *t = trash;
        }
    }

    pub(crate) fn watcher(&self) -> Arc<dyn files::Watcher> {
        self.inner.watcher.read().map(|w| w.clone()).unwrap_or_else(|_| Arc::new(files::FsWatcher))
    }

    pub(crate) fn trash(&self) -> Arc<dyn files::Trash> {
        self.inner.trash.read().map(|t| t.clone()).unwrap_or_else(|_| Arc::new(files::SystemTrash))
    }

    pub(crate) fn fetcher(&self) -> Arc<dyn net::Fetcher> {
        self.inner.fetcher.read().map(|f| f.clone()).unwrap_or_else(|_| Arc::new(net::HttpFetcher))
    }

    /// Where the text of the file an agent's `present` call named
    /// (`from_file`) waits for the tile's `input-part` while the call is
    /// delivered: put it before, forget it when the call ends.
    pub fn large_inputs(&self) -> &agent::LargeInputs {
        &self.inner.large
    }

    pub(crate) fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }
}

/// Bumps the engine's epoch every [`limits::EPOCH_TICK`] until dropped.
/// Each tick lets a running call check its wall-clock deadline. It sleeps
/// to absolute tick times, so a late wake-up doesn't push later ticks back.
struct EpochTicker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn start(engine: &Engine) -> std::io::Result<EpochTicker> {
        let stop = Arc::new(AtomicBool::new(false));
        let engine = engine.clone();
        let flag = stop.clone();
        let thread = std::thread::Builder::new().name("tile-epoch".into()).spawn(move || {
            let mut next = Instant::now();
            while !flag.load(Ordering::Relaxed) {
                next += limits::EPOCH_TICK;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now;
                }
                engine.increment_epoch();
            }
        })?;
        Ok(EpochTicker { stop, thread: Some(thread) })
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod grants_tests {
    use super::*;

    /// `{setting:KEY}` in `net`'s hosts: the host of the address the user
    /// set, reached once `net` is granted; nothing until it is set.
    #[test]
    fn a_host_from_a_setting_is_reached_once_set() {
        let m = extensions::parse(r#"{"v": 2, "id": "x", "name": "X", "settings": [{"key": "server", "type": "url", "label": "Server"}], "permissions": {"net": {"reason": "Its API", "hosts": ["{setting:server}"]}}, "contributes": {"tiles": [{"id": "m", "title": "M", "wasm": "t.wasm"}]}}"#).unwrap();
        let url = |u: &str| url::Url::parse(u).unwrap();
        let mut values = std::collections::BTreeMap::new();
        let g = Grants::for_extension(&m, ["net"]).with_settings(&m, &values);
        assert!(g.reach().check(&url("https://inbox.example.com/api")).is_err(), "not set yet");
        values.insert("server".to_string(), "https://inbox.example.com".to_string());
        let g = Grants::for_extension(&m, ["net"]).with_settings(&m, &values);
        assert!(g.reach().check(&url("https://inbox.example.com/api")).is_ok());
        assert!(g.reach().check(&url("https://other.example.com/")).is_err());
        let refused = Grants::for_extension(&m, Vec::<String>::new()).with_settings(&m, &values);
        assert!(refused.reach().check(&url("https://inbox.example.com/api")).is_err(), "net not granted");
    }
}
