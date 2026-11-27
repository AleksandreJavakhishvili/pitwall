//! Test Pitwall tiles (docs/spec/tile-sdk/authoring.md §9.3): load a built
//! `tile.wasm` in Pitwall's own tile runtime (`pitwall-tile-host`) with a
//! fake host (a temporary data folder, settings and secrets you give, the
//! grants you name), send it what the user does, and check what it shows,
//! stores and logs.
//!
//! ```no_run
//! use pitwall_extension_test::Harness;
//!
//! let mut tile = Harness::build(env!("CARGO_MANIFEST_DIR")) // cargo build --target wasm32-wasip2
//!     .setting("greeting", "Hi")
//!     .open("main");
//! assert!(tile.find_text("Clicked 0 times").is_some());
//! tile.click_text("Click me");
//! assert!(tile.find_text("Clicked 1 times").is_some());
//! assert!(tile.stored("clicks").is_some());
//! ```
//!
//! Every action waits until the tile has handled it and rendered (a round
//! trip through its worker), so assertions after it see the new frame.
//!
//! **Time is virtual.** The tile's timers and its clocks (`Instant`,
//! `SystemTime`) stand still until the test lets time pass with
//! [`Session::advance`]: timers due meanwhile fire in order, at once, so a
//! test of a one-second save doesn't wait a second
//! ([`Harness::wall_clock`] for the real clock).
//!
//! **Focus works as in Pitwall.** A click, typing into a field or a key
//! gives the tile focus first, as the pointer and keyboard would; typing
//! into a field focuses it; [`Session::focus_key`], [`Session::blur`] and
//! [`Session::focus`] move it by hand. The tile hears `focus-changed` and
//! `focus(bool)` as it would in Pitwall: a field's blur (the SDK's
//! `on_blur`) when the focus leaves it or the field goes away. A frame's
//! `focus` request is honoured while the tile has focus. A key binding
//! without ⌘/Ctrl/Alt doesn't fire while a field has focus, and Pitwall's
//! reserved keys are dropped from the keymap, as in Pitwall.
//!
//! **Services are fakes.** `net` requests never leave the test: they are
//! answered from a table ([`Harness::respond`], [`Session::respond`]; a 404
//! otherwise) and kept ([`Session::requests`]). The `sql` database is in
//! the test's temporary data folder. Notifications, files and pages to
//! open, and toolbar changes are captured ([`Session::notifications`],
//! [`Session::ui_requests`], [`Session::toolbar`]). The extension's own
//! server is said to be up at an address you give ([`Harness::own_server`]).

mod canvas;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod agents;

pub use agents::{agent, waiting_agent, Queued};

use pitwall_tile_host::ui::{self, Event, Modifiers, MouseButton, Point};
use pitwall_tile_host::net::{FakeFetcher, FetchError, Outgoing};
use pitwall_tile_host::{Clock, Extension, ExtensionData, FrameUpdate, Grants, HostConfig, LogLevel, LogLine, Secrets, StartOptions, StopReason, TileHandle, TileHost, TileOutput, ToolbarRequest, UiRequest, ViewSpec, POPOVER_VIEW};

/// A frame's types, as the tile sent them (for checks the helpers don't
/// cover): a [`Frame`] is a list of [`Node`]s, each an [`Element`].
pub use pitwall_tile_host::ui::{Density, Element, Frame, Input, Listen, Node, ThemeInfo, ThemeMode, Tone};
/// A canvas's, a code view's and a diff view's types, as the tile sent
/// them ([`Session::canvas`], [`Session::code_view`]).
pub use pitwall_tile_host::ui::{Canvas, CodeView, DiffSide as Side, DiffView, DrawOp, HitRegion, Paint, PointerKind, Token};
/// The agents' questions group's types: an agent of `agents::list`, a
/// badge and its row, a question and its suggestions.
pub use pitwall_tile_host::ui::{Agent, AgentState, Ask, Badge, Row, Suggestion};
/// The toolbar button's and the extension's own server's types
/// ([`Session::toolbar`], [`Session::set_service`]).
pub use pitwall_tile_host::ui::{BadgeTone, ServiceInfo, ServiceState, ToolbarBadge};
/// Why a faked `net` request failed ([`Harness::respond_error`]).
pub use pitwall_tile_host::net::FetchError as NetError;
pub use pitwall_tile_host::{LogLevel as Level, StopReason as Stop};

mod components;
pub use components::ListRow;

/// The Rust target tiles are built for.
pub const WASM_TARGET: &str = "wasm32-wasip2";

/// How long one round trip may take before the test fails (a tile's own
/// deadlines are shorter: 1 s to handle, 250 ms to render).
const WAIT: Duration = Duration::from_secs(30);

/// Build the tile in `dir` (a Cargo project with an `extension.json`) with
/// `cargo build --release --target wasm32-wasip2` and copy the component
/// to where the manifest's tile entry names it (`"wasm"`, like
/// `tile.wasm`). Returns that path, or what went wrong (with the
/// compiler's output, or how to install the target). Built once per test
/// process and folder, however many tests ask.
pub fn build(dir: &Path) -> Result<PathBuf, String> {
    static BUILT: Mutex<BTreeMap<PathBuf, Result<PathBuf, String>>> = Mutex::new(BTreeMap::new());
    let key = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut built = BUILT.lock().unwrap_or_else(|e| e.into_inner());
    built.entry(key).or_insert_with(|| build_now(dir)).clone()
}

fn build_now(dir: &Path) -> Result<PathBuf, String> {
    let manifest = pitwall_core::extensions::read_in_place(dir)?;
    let wasm = manifest.contributes.tiles.first().map(|t| t.wasm.clone()).ok_or("its extension.json contributes no tiles")?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    if !target_installed(dir) {
        return Err(format!("the Rust target {WASM_TARGET} isn't installed: run `rustup target add {WASM_TARGET}`"));
    }
    let out = Command::new(&cargo)
        .args(["build", "--release", "--target", WASM_TARGET, "--message-format=json-render-diagnostics"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cargo couldn't be run: {e}"))?;
    if !out.status.success() {
        return Err(format!("the build failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let built = artifact(&String::from_utf8_lossy(&out.stdout), dir).ok_or("the build made no .wasm: is the crate's crate-type [\"cdylib\"]?")?;
    let to = dir.join(&wasm);
    // Written beside, then renamed: a reader never sees half a file.
    let tmp = to.with_extension("wasm.part");
    std::fs::copy(&built, &tmp).and_then(|_| std::fs::rename(&tmp, &to)).map_err(|e| format!("{} couldn't be copied to {}: {e}", built.display(), to.display()))?;
    Ok(to)
}

/// The `.wasm` the build of the package in `dir` made, from cargo's JSON
/// messages.
fn artifact(messages: &str, dir: &Path) -> Option<PathBuf> {
    let want = dir.join("Cargo.toml").canonicalize().ok();
    messages
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact")
        .filter(|m| m["manifest_path"].as_str().map(|p| Path::new(p).canonicalize().ok()) == Some(want.clone()))
        .flat_map(|m| m["filenames"].as_array().cloned().unwrap_or_default())
        .filter_map(|f| f.as_str().map(PathBuf::from))
        .rfind(|f| f.extension().is_some_and(|e| e == "wasm"))
}

/// Whether the toolchain used in `dir` has the `wasm32-wasip2` target.
pub fn target_installed(dir: &Path) -> bool {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    Command::new(rustc)
        .args(["--print", "target-libdir", "--target", WASM_TARGET])
        .current_dir(dir)
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .is_some_and(|p| p.is_dir())
}

/// A tile to start: its folder, and the settings, secrets, grants and
/// state it starts with.
pub struct Harness {
    dir: PathBuf,
    settings: Vec<(String, String)>,
    secrets: HashMap<String, String>,
    grants: Vec<String>,
    state: Option<Vec<u8>>,
    visible: bool,
    wall: bool,
    stored: BTreeMap<String, Vec<u8>>,
    methods: Arc<agents::FakeMethods>,
    net: Arc<FakeFetcher>,
    own_server: Option<std::net::SocketAddr>,
}

/// The toolbar button as the tile left it ([`Session::toolbar`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolbarState {
    /// The badge everywhere, and its tone.
    pub badge: Option<(ToolbarBadge, BadgeTone)>,
    /// Each project's badge, by project id.
    pub project_badges: BTreeMap<String, (ToolbarBadge, BadgeTone)>,
    /// The tooltip, when the tile changed it.
    pub tooltip: Option<String>,
    /// Shown or hidden by the tile (`None`: as the manifest says).
    pub shown: Option<bool>,
    /// The popover is open.
    pub popover_open: bool,
}

impl Harness {
    /// The extension in `dir`, already built (its component where its
    /// manifest names it).
    pub fn load(dir: impl AsRef<Path>) -> Harness {
        Harness { dir: dir.as_ref().to_path_buf(), settings: Vec::new(), secrets: HashMap::new(), grants: Vec::new(), state: None, visible: true, wall: false, stored: BTreeMap::new(), methods: agents::methods(), net: FakeFetcher::new(), own_server: None }
    }

    /// The extension in `dir`, built first with [`build`]. Panics with the
    /// compiler's output when the build fails.
    pub fn build(dir: impl AsRef<Path>) -> Harness {
        let dir = dir.as_ref();
        if let Err(e) = build(dir) {
            panic!("{} can't be built: {e}", dir.display());
        }
        Harness::load(dir)
    }

    /// Setting `key` set by the user to `value`, as Settings keeps it
    /// (`"true"`/`"false"` for a switch, digits for a number, a choice's
    /// value).
    pub fn setting(mut self, key: &str, value: &str) -> Harness {
        self.settings.push((key.to_string(), value.to_string()));
        self
    }

    /// Secret setting `key` set to `value`.
    pub fn secret(mut self, key: &str, value: &str) -> Harness {
        self.secrets.insert(key.to_string(), value.to_string());
        self
    }

    /// Permission `name` granted.
    pub fn grant(mut self, name: &str) -> Harness {
        self.grants.push(name.to_string());
        self
    }

    /// Start with `value` stored under `key`, as though the tile had
    /// stored it in an earlier run.
    pub fn stored(mut self, key: &str, value: impl Into<Vec<u8>>) -> Harness {
        self.stored.insert(key.to_string(), value.into());
        self
    }

    /// [`Harness::stored`] with `value` as JSON (as the SDK's
    /// `storage::set_json` keeps it).
    pub fn stored_json(self, key: &str, value: &impl serde::Serialize) -> Harness {
        let bytes = serde_json::to_vec(value).expect("the value is JSON");
        self.stored(key, bytes)
    }

    /// Answer `net` requests to `url` (the whole address, query included)
    /// with `status` and `body`. Requests to addresses with no answer get
    /// a 404; none leaves the test. The tile still needs the `net` grant
    /// and the host in its manifest, as in Pitwall.
    pub fn respond(self, url: &str, status: u16, body: impl Into<Vec<u8>>) -> Harness {
        self.net.respond(url, status, body);
        self
    }

    /// Answer `net` requests to `url` with an error (offline, timed out).
    pub fn respond_error(self, url: &str, error: FetchError) -> Harness {
        self.net.fail(url, error);
        self
    }

    /// Its own server (the manifest's `service`) is up at `addr`
    /// (`127.0.0.1:8790`): `service.info` says so and requests to it are
    /// allowed without `net`, as in Pitwall. Answer them with
    /// [`Harness::respond`].
    pub fn own_server(mut self, addr: &str) -> Harness {
        self.own_server = Some(addr.parse().unwrap_or_else(|_| panic!("“{addr}” isn't an address like 127.0.0.1:8790")));
        self
    }

    /// Start with this saved state (what `save_state` returned).
    pub fn state(mut self, state: Vec<u8>) -> Harness {
        self.state = Some(state);
        self
    }

    /// Start hidden: no renders and no timers until [`Session::show`].
    pub fn hidden(mut self) -> Harness {
        self.visible = false;
        self
    }

    /// Run on the computer's clock instead of the virtual one: timers fire
    /// as real time passes ([`Session::advance`] then waits).
    pub fn wall_clock(mut self) -> Harness {
        self.wall = true;
        self
    }

    /// Start it with tile `tile` open (its view id is the tile's id) and
    /// wait for its first frame.
    pub fn open(self, tile: &str) -> Session {
        self.start(ViewSpec::tile(tile))
    }

    /// Start it with tile `tile` open with `content` (JSON, as the user or
    /// an agent gives it), and wait for its first frame.
    pub fn open_with(self, tile: &str, content: serde_json::Value) -> Session {
        self.start(ViewSpec { content: Some(json_content(&content)), ..ViewSpec::tile(tile) })
    }

    fn start(self, view: ViewSpec) -> Session {
        let data = tempfile::tempdir().expect("a temporary data folder");
        let ext = Extension::load(&self.dir).unwrap_or_else(|e| panic!("{} can't be loaded: {e}", self.dir.display()));
        let host = TileHost::new(HostConfig { data_dir: data.path().to_path_buf(), secrets: Arc::new(FakeSecrets(self.secrets.clone())), methods: self.methods.clone() }).expect("the tile runtime starts");
        host.set_fetcher(self.net.clone());
        let store = host.extension_data(ext.id());
        for (key, value) in &self.settings {
            check_setting(&ext, key);
            store.set_setting(key, Some(value)).expect("the setting is written");
        }
        if !self.stored.is_empty() {
            store.set_storage(&self.stored).expect("the storage is written");
        }
        let clock = if self.wall { Clock::Wall } else { Clock::manual() };
        let theme = ui::default_theme();
        let grants = Grants::for_extension(ext.manifest(), self.grants.clone()).with_own_server(self.own_server);
        let options = StartOptions { views: vec![view.clone()], grants, theme, visible: self.visible, state: self.state.clone(), clock: clock.clone() };
        let (handle, rx) = launch(&host, &ext, options);
        if let Some(info) = service_info(&ext, self.own_server) {
            handle.set_service(info);
        }
        let mut s = Session {
            _data: data,
            host,
            ext,
            store,
            handle,
            rx,
            main: view.id,
            frames: BTreeMap::new(),
            revisions: HashMap::new(),
            texts: HashMap::new(),
            log: Vec::new(),
            ui: Vec::new(),
            stopped: None,
            saved: None,
            grants: self.grants,
            visible: self.visible,
            clock,
            theme,
            tile_focus: false,
            focused: HashMap::new(),
            asked: HashMap::new(),
            sent: false,
            unknown_icons: Vec::new(),
            methods: self.methods,
            got_rows: Vec::new(),
            net: self.net,
            net_seen: 0,
            own_server: self.own_server,
        };
        s.settle();
        s
    }
}

fn check_setting(ext: &Extension, key: &str) {
    match ext.manifest().settings.iter().find(|d| d.key == key) {
        Some(d) if d.kind == pitwall_core::extensions::SettingKind::Secret => panic!("“{key}” is a secret setting: use .secret()"),
        Some(_) => {}
        None => panic!("its extension.json has no setting “{key}”"),
    }
}

/// What `service.info` says in the harness: up at `own` when given, else
/// off; nothing for an extension without a service.
fn service_info(ext: &Extension, own: Option<std::net::SocketAddr>) -> Option<ServiceInfo> {
    ext.manifest().service.as_ref()?;
    Some(match own {
        Some(a) => ServiceInfo { state: ServiceState::Up, address: Some(format!("http://{a}")), version: None, why: None },
        None => ServiceInfo { state: ServiceState::Off, address: None, version: None, why: Some("not started in the test".into()) },
    })
}

fn launch(host: &TileHost, ext: &Extension, options: StartOptions) -> (TileHandle, Receiver<TileOutput>) {
    let (tx, rx) = mpsc::channel();
    let handle = host
        .start(ext, options, move |o| {
            let _ = tx.send(o);
        })
        .unwrap_or_else(|e| panic!("it can't be started: {e}"));
    (handle, rx)
}

fn json_content(v: &serde_json::Value) -> ui::Content {
    ui::Content { id: None, json: Some(v.to_string()), bytes: None }
}

struct FakeSecrets(HashMap<String, String>);

impl Secrets for FakeSecrets {
    fn secret(&self, _extension: &str, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

/// A node found in a view's newest frame.
#[derive(Debug, Clone)]
pub struct Found {
    /// The view it is in.
    pub view: String,
    /// Its index in the frame.
    pub index: usize,
    /// Its key path: the keys from the root down, joined by `/`, without
    /// the root's.
    pub key_path: String,
    /// The node.
    pub node: Node,
}

impl Found {
    /// Its text: a text node's, a button's label.
    pub fn text(&self) -> Option<&str> {
        node_text(&self.node)
    }

    /// Whether it listens for `on`.
    pub fn listens(&self, on: Listen) -> bool {
        self.node.on.iter().any(|l| l.on == on)
    }

    /// Its text field, when it is one.
    pub fn input(&self) -> Option<&Input> {
        match &self.node.element {
            Element::Input(i) => Some(i),
            _ => None,
        }
    }
}

/// A text field as the user sees it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// What it holds: the tile's last value, or what the test typed since.
    pub text: String,
    /// Several lines.
    pub multiline: bool,
    /// Its height in lines, as the tile asked.
    pub rows: Option<u16>,
    /// Its placeholder.
    pub placeholder: Option<String>,
    /// Drawn dimmed and can't be edited.
    pub disabled: bool,
    /// It has keyboard focus.
    pub focused: bool,
}

fn node_text(n: &Node) -> Option<&str> {
    match &n.element {
        Element::Text(t) => Some(t),
        Element::Button(b) => Some(&b.label),
        _ => None,
    }
}

/// Key paths of a frame's nodes, by index.
fn key_paths(f: &Frame) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(f.nodes.len());
    for n in &f.nodes {
        let path = match n.parent {
            None => String::new(),
            Some(p) => match out.get(p as usize) {
                Some(pp) if !pp.is_empty() => format!("{pp}/{}", n.key),
                _ => n.key.clone(),
            },
        };
        out.push(path);
    }
    out
}

/// A running tile, with what it has shown, logged and asked for so far.
pub struct Session {
    _data: tempfile::TempDir,
    host: TileHost,
    ext: Extension,
    store: ExtensionData,
    handle: TileHandle,
    rx: Receiver<TileOutput>,
    main: String,
    frames: BTreeMap<String, FrameUpdate>,
    /// The newest revision of each text field (view, key path).
    revisions: HashMap<(String, String), u64>,
    /// What each text field holds (view, key path).
    texts: HashMap<(String, String), String>,
    log: Vec<LogLine>,
    ui: Vec<UiRequest>,
    stopped: Option<StopReason>,
    saved: Option<Vec<u8>>,
    grants: Vec<String>,
    visible: bool,
    clock: Clock,
    theme: ThemeInfo,
    /// The tile has keyboard focus.
    tile_focus: bool,
    /// The focused node of each view (key path).
    focused: HashMap<String, String>,
    /// The focus each view's frame asked for last.
    asked: HashMap<String, Option<String>>,
    /// An event was sent while taking outputs: settle once more.
    sent: bool,
    unknown_icons: Vec<String>,
    /// The fake method table: the agents and what the tile queued.
    methods: Arc<agents::FakeMethods>,
    /// Rows the tile sent since [`Session::rows`] asked.
    got_rows: Vec<pitwall_tile_host::RowsUpdate>,
    net: Arc<FakeFetcher>,
    /// The fake requests already waited for in [`Session::settle`].
    net_seen: usize,
    own_server: Option<std::net::SocketAddr>,
}

impl Session {
    fn take(&mut self, o: TileOutput) {
        match o {
            TileOutput::Frame(f) => {
                let paths = key_paths(&f.frame);
                let view = f.view.clone();
                let mut live = std::collections::HashSet::new();
                for (i, n) in f.frame.nodes.iter().enumerate() {
                    let at = (view.clone(), paths[i].clone());
                    if let Element::Input(input) = &n.element {
                        live.insert(paths[i].clone());
                        if let Some(v) = &input.value {
                            let r = self.revisions.entry(at.clone()).or_default();
                            if input.revision >= *r {
                                self.texts.insert(at.clone(), v.clone());
                            }
                            *r = (*r).max(input.revision);
                        }
                        self.texts.entry(at).or_default();
                    }
                    let mut icons: Vec<&str> = match &n.element {
                        Element::Icon(ui::Icon::Named(name)) => vec![name.as_str()],
                        Element::Button(b) => b.icon.as_ref().map(|ui::Icon::Named(name)| name.as_str()).into_iter().collect(),
                        _ => vec![],
                    };
                    icons.extend(components::icons(&n.element));
                    for name in icons.into_iter().filter(|n| !pitwall_core::extensions::ICON_NAMES.contains(n)) {
                        if !self.unknown_icons.iter().any(|u| u == name) {
                            println!("[harness] “{}” names the icon “{name}”, which Pitwall doesn't have: it draws nothing (see the SDK's Icon)", paths[i]);
                            self.unknown_icons.push(name.to_string());
                        }
                    }
                }
                self.texts.retain(|(v, p), _| v != &view || live.contains(p));
                // A field that went away comes back new, as in Pitwall.
                self.revisions.retain(|(v, p), _| v != &view || live.contains(p));
                // The focused node went away: its blur.
                if self.focused.get(&view).is_some_and(|p| !paths.contains(p)) {
                    self.move_focus(&view, None);
                }
                // A new focus request, honoured while the tile has focus.
                if self.asked.get(&view) != Some(&f.frame.focus) {
                    self.asked.insert(view.clone(), f.frame.focus.clone());
                    if let Some(p) = f.frame.focus.clone().filter(|p| self.tile_focus && paths.contains(p)) {
                        self.move_focus(&view, Some(p));
                    }
                }
                self.frames.insert(view, f);
            }
            TileOutput::Log(l) => {
                // Shown with the test's output when it fails.
                println!("[tile {:?}] {}", l.level, l.message);
                self.log.push(l);
            }
            TileOutput::Ui(r) => self.ui.push(r),
            TileOutput::StateSaved(b) => self.saved = Some(b),
            TileOutput::Stopped(r) => self.stopped = Some(r),
            TileOutput::Preparing | TileOutput::Started { .. } => {}
            TileOutput::Rows(u) => self.got_rows.push(u),
        }
    }

    /// Tell the tile the focus in `view` moved to `to` (when it did).
    fn move_focus(&mut self, view: &str, to: Option<String>) {
        if self.focused.get(view) == to.as_ref() {
            return;
        }
        match &to {
            Some(p) => self.focused.insert(view.to_string(), p.clone()),
            None => self.focused.remove(view),
        };
        if self.stopped.is_none() && self.handle.send(Event::FocusChanged(ui::NodeFocus { view: view.to_string(), key_path: to })) {
            self.sent = true;
        }
    }

    /// The tile takes the keyboard (a click or a key in it).
    fn take_focus(&mut self) {
        if !self.tile_focus {
            self.tile_focus = true;
            self.send(Event::Focus(true));
        }
    }

    /// Take outputs until `done` says so, the tile stops, or [`WAIT`].
    fn pump(&mut self, until: Instant, mut done: impl FnMut(&Session) -> bool) -> bool {
        loop {
            if done(self) {
                return true;
            }
            if self.stopped.is_some() {
                return false;
            }
            let left = until.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(o) => self.take(o),
                Err(mpsc::RecvTimeoutError::Timeout) => return false,
                Err(mpsc::RecvTimeoutError::Disconnected) => return done(self),
            }
        }
    }

    fn check_running(&self) {
        if let Some(r) = &self.stopped {
            panic!("the tile stopped: {r}\nits Log:\n{}", self.log_text());
        }
    }

    /// Wait until the tile has handled everything sent so far and rendered
    /// what changed. Panics when it stopped (see [`Session::stopped`] to
    /// test a stop: use [`Session::try_settle`]).
    pub fn settle(&mut self) -> &mut Self {
        if let Err(r) = self.try_settle() {
            panic!("the tile stopped: {r}\nits Log:\n{}", self.log_text());
        }
        self
    }

    /// [`Session::settle`], with a stop as an error instead of a panic.
    pub fn try_settle(&mut self) -> Result<(), StopReason> {
        // Two round trips: the worker answers `save-state` in the batch
        // it takes, before rendering that batch; the second answer comes
        // after the first batch's frames and Log lines. Events sent while
        // taking them (a focus change a frame caused) need two more.
        self.sent = false;
        let mut rounds = 0;
        while rounds < 2 {
            rounds += 1;
            self.saved = None;
            if !self.handle.save_state() {
                self.pump(Instant::now() + WAIT, |s| s.stopped.is_some());
            } else {
                self.pump(Instant::now() + WAIT, |s| s.saved.is_some());
            }
            if let Some(r) = &self.stopped {
                return Err(r.clone());
            }
            if self.saved.is_none() {
                panic!("the tile didn't answer within {WAIT:?}");
            }
            if std::mem::take(&mut self.sent) {
                rounds = 0;
            }
            // A request the fake answered: its answer arrives as an event.
            let asked = self.net.requests().len();
            if asked != self.net_seen {
                self.net_seen = asked;
                rounds = 0;
            }
        }
        Ok(())
    }

    /// Let `time` pass, then settle: on the virtual clock at once (timers
    /// due meanwhile fire in order, each at its time, while the tile is
    /// shown); on the wall clock by waiting.
    pub fn advance(&mut self, time: Duration) -> &mut Self {
        if self.clock.is_manual() {
            self.check_running();
            assert!(self.handle.advance(time), "the tile isn't running");
        } else {
            self.pump(Instant::now() + time, |_| false);
        }
        self.settle()
    }

    /// [`Session::advance`]: let `time` pass.
    pub fn wait(&mut self, time: Duration) -> &mut Self {
        self.advance(time)
    }

    /// Let up to `timeout` pass until the main view shows `text`, for what
    /// a timer brings (on the virtual clock in steps of 100 ms). Panics
    /// when it doesn't.
    pub fn wait_for_text(&mut self, text: &str, timeout: Duration) -> &mut Self {
        let main = self.main.clone();
        let found = if self.clock.is_manual() {
            let mut left = timeout;
            loop {
                if self.find_text_in(&main, text).is_some() {
                    break true;
                }
                if left.is_zero() {
                    break false;
                }
                let step = left.min(Duration::from_millis(100));
                left -= step;
                self.advance(step);
            }
        } else {
            self.pump(Instant::now() + timeout, |s| s.find_text_in(&main, text).is_some())
        };
        self.check_running();
        assert!(found, "no “{text}” within {timeout:?}; it shows:\n{}", self.text_snapshot());
        self
    }

    /// Why it stopped, if it did (after the latest [`Session::settle`]).
    pub fn stopped(&self) -> Option<&StopReason> {
        self.stopped.as_ref()
    }

    // ---- What it shows -----------------------------------------------------

    /// The newest frame of the view it started with.
    pub fn frame(&self) -> &Frame {
        self.frame_of(&self.main)
    }

    /// The newest frame of view `view`. Panics when it has none.
    pub fn frame_of(&self, view: &str) -> &Frame {
        match self.frames.get(view) {
            Some(f) => &f.frame,
            None => panic!("view “{view}” has no frame (open views: {:?})", self.frames.keys().collect::<Vec<_>>()),
        }
    }

    /// The text of every text node and button of the main view, in tree
    /// order.
    pub fn texts(&self) -> Vec<String> {
        self.frame().nodes.iter().filter_map(node_text).map(str::to_string).collect()
    }

    fn find_in(&self, view: &str, pick: impl Fn(&Node, &str) -> bool) -> Option<Found> {
        let f = self.frames.get(view)?;
        let paths = key_paths(&f.frame);
        f.frame.nodes.iter().enumerate().find(|(i, n)| pick(n, &paths[*i])).map(|(i, n)| Found { view: view.to_string(), index: i, key_path: paths[i].clone(), node: n.clone() })
    }

    fn find_text_in(&self, view: &str, text: &str) -> Option<Found> {
        self.find_in(view, |n, _| node_text(n).is_some_and(|t| t.contains(text)))
    }

    /// The first text node or button in the main view whose text contains
    /// `text`.
    pub fn find_text(&self, text: &str) -> Option<Found> {
        self.find_text_in(&self.main, text)
    }

    /// The node at key path `key_path` in the main view (`"list/row-3"`),
    /// or the first whose own key is `key_path` when no path matches.
    pub fn find_key(&self, key_path: &str) -> Option<Found> {
        self.find_key_in(&self.main, key_path)
    }

    /// [`Session::find_key`] in view `view`.
    pub fn find_key_in(&self, view: &str, key_path: &str) -> Option<Found> {
        self.find_in(view, |_, p| p == key_path).or_else(|| self.find_in(view, |n, _| n.key == key_path))
    }

    /// The first node in the main view with accessible name `label`.
    pub fn find_label(&self, label: &str) -> Option<Found> {
        self.find_in(&self.main, |n, _| n.label.as_deref() == Some(label))
    }

    /// The main view as an indented outline: one node a line with its
    /// kind, text, key and listeners; a text field with its placeholder,
    /// what it holds (`text=`), `multiline`, and `focused` on the node with
    /// keyboard focus. Stable across Pitwall versions within a world, for
    /// snapshot tests.
    pub fn text_snapshot(&self) -> String {
        self.text_snapshot_of(&self.main)
    }

    /// [`Session::text_snapshot`] of view `view`.
    pub fn text_snapshot_of(&self, view: &str) -> String {
        let Some(f) = self.frames.get(view) else { return format!("(view “{view}” has no frame)\n") };
        let nodes = &f.frame.nodes;
        let paths = key_paths(&f.frame);
        let focused = self.focused.get(view);
        let mut depth = vec![0usize; nodes.len()];
        let mut out = String::new();
        for (i, n) in nodes.iter().enumerate() {
            depth[i] = n.parent.map_or(0, |p| depth[p as usize] + 1);
            let what = match &n.element {
                Element::Box => "box".to_string(),
                Element::Text(t) => format!("text {t:?}"),
                Element::Button(b) => format!("button {:?}{}", b.label, if b.disabled { " disabled" } else { "" }),
                Element::Icon(ui::Icon::Named(name)) => format!("icon {name}"),
                Element::Scroll(_) => "scroll".to_string(),
                Element::Canvas(c) => canvas::describe(c),
                Element::CodeView(c) => canvas::describe_code(c),
                Element::DiffView(d) => canvas::describe_diff(d),
                Element::Input(input) => {
                    let text = self.texts.get(&(view.to_string(), paths[i].clone())).filter(|t| !t.is_empty());
                    format!(
                        "input{}{}{}",
                        input.placeholder.as_ref().map(|p| format!(" placeholder={p:?}")).unwrap_or_default(),
                        text.map(|t| format!(" text={t:?}")).unwrap_or_default(),
                        if input.multiline { " multiline" } else { "" }
                    )
                }
                e => components::describe(e).unwrap_or_else(|| pitwall_tile_host::components::kind(e).to_string()),
            };
            let what = if focused == Some(&paths[i]) { format!("{what} focused") } else { what };
            let key = if n.key.chars().all(|c| c.is_ascii_digit()) || i == 0 { String::new() } else { format!(" #{}", n.key) };
            let on: Vec<&str> = n
                .on
                .iter()
                .map(|l| match l.on {
                    Listen::Click => "click",
                    Listen::Change => "change",
                    Listen::Select => "select",
                    Listen::Submit => "submit",
                    Listen::Scroll => "scroll",
                    Listen::Hover => "hover",
                    Listen::Pointer => "pointer",
                    Listen::Wheel => "wheel",
                    Listen::LineClick => "line-click",
                    Listen::Toggle => "toggle",
                    Listen::Link => "link",
                    Listen::Sort => "sort",
                    Listen::ColumnResized => "column-resized",
                })
                .collect();
            let on = if on.is_empty() { String::new() } else { format!(" [{}]", on.join(", ")) };
            let label = n.label.as_ref().map(|l| format!(" label={l:?}")).unwrap_or_default();
            let dnd = canvas::describe_dnd(n);
            out.push_str(&format!("{}{what}{key}{label}{dnd}{on}\n", "  ".repeat(depth[i])));
            if let Element::Canvas(c) = &n.element {
                out.push_str(&canvas::describe_regions(c, depth[i] + 1));
            }
        }
        out
    }

    // ---- What the user does ------------------------------------------------

    fn send(&mut self, e: Event) {
        self.check_running();
        assert!(self.handle.send(e), "the tile isn't running");
    }

    /// The handler of the nearest node from `found` up that listens for
    /// `on`.
    fn handler_up(&self, found: &Found, on: Listen) -> u32 {
        let nodes = &self.frame_of(&found.view).nodes;
        let mut i = Some(found.index);
        while let Some(at) = i {
            if let Some(l) = nodes[at].on.iter().find(|l| l.on == on) {
                return l.handler;
            }
            i = nodes[at].parent.map(|p| p as usize);
        }
        panic!("“{}” (and the nodes around it) don't listen for {on:?}", found.key_path)
    }

    fn click_found(&mut self, found: Option<Found>, what: &str) -> &mut Self {
        let Some(found) = found else { panic!("there is no {what}; it shows:\n{}", self.text_snapshot()) };
        if let Element::Button(b) = &found.node.element {
            assert!(!b.disabled, "the button “{}” is disabled", b.label);
        }
        let handler = self.handler_up(&found, Listen::Click);
        // As in Pitwall: the press gives the tile the keyboard, and a
        // focusable box clicked takes the focus (a button doesn't).
        self.take_focus();
        let nodes = &self.frame_of(&found.view).nodes;
        let mut at = Some(found.index);
        while let Some(i) = at {
            if nodes[i].on.iter().any(|l| l.on == Listen::Click) {
                break;
            }
            at = nodes[i].parent.map(|p| p as usize);
        }
        if let Some(i) = at.filter(|i| nodes[*i].focusable && matches!(nodes[*i].element, Element::Box | Element::Text(_) | Element::Icon(_) | Element::Scroll(_))) {
            let path = key_paths(self.frame_of(&found.view))[i].clone();
            let view = found.view.clone();
            self.move_focus(&view, Some(path));
        }
        self.send(Event::Click(ui::Click { handler, button: MouseButton::Left, modifiers: Modifiers::empty(), count: 1, at: Point { x: 1.0, y: 1.0 } }));
        self.settle()
    }

    /// Click the first text or button showing `text` (or the node around
    /// it that listens for clicks).
    pub fn click_text(&mut self, text: &str) -> &mut Self {
        let f = self.find_text(text);
        self.click_found(f, &format!("text “{text}”"))
    }

    /// Click the node at key path `key_path` (or with that key).
    pub fn click_key(&mut self, key_path: &str) -> &mut Self {
        let f = self.find_key(key_path);
        self.click_found(f, &format!("node “{key_path}”"))
    }

    /// Click the node at key path `key_path` in view `view`.
    pub fn click_key_in(&mut self, view: &str, key_path: &str) -> &mut Self {
        let f = self.find_key_in(view, key_path);
        self.click_found(f, &format!("node “{key_path}” in view “{view}”"))
    }

    /// Click the node with accessible name `label` (an icon button).
    pub fn click_label(&mut self, label: &str) -> &mut Self {
        let f = self.find_label(label);
        self.click_found(f, &format!("node labelled “{label}”"))
    }

    fn input(&self, key_path: &str) -> Found {
        match self.find_key(key_path) {
            Some(f) if matches!(f.node.element, Element::Input(_)) => f,
            Some(_) => panic!("“{key_path}” isn't a text field"),
            None => panic!("there is no text field “{key_path}”; it shows:\n{}", self.text_snapshot()),
        }
    }

    /// Replace what text field `key_path` holds with `text`, as if the
    /// user clicked into it (it takes the focus) and typed it.
    pub fn type_into(&mut self, key_path: &str, text: &str) -> &mut Self {
        let f = self.input(key_path);
        let Some(l) = f.node.on.iter().find(|l| l.on == Listen::Change) else { panic!("the text field “{key_path}” doesn't listen for changes") };
        let l = *l;
        if let Some(i) = f.input() {
            assert!(!i.disabled, "the text field “{key_path}” is disabled");
        }
        self.take_focus();
        self.move_focus(&f.view, Some(f.key_path.clone()));
        let at = (f.view.clone(), f.key_path.clone());
        self.texts.insert(at.clone(), text.to_string());
        let revision = {
            let r = self.revisions.entry(at).or_default();
            *r += 1;
            *r
        };
        let end = text.len() as u32;
        self.send(Event::InputChanged(ui::InputChange { handler: l.handler, value: text.to_string(), revision, selection: ui::TextRange { start: end, end } }));
        self.settle()
    }

    /// Press Enter (⌘Enter when multi-line) in text field `key_path`,
    /// which takes the focus first.
    pub fn submit(&mut self, key_path: &str) -> &mut Self {
        let f = self.input(key_path);
        let handler = self.handler_up(&f, Listen::Submit);
        self.take_focus();
        self.move_focus(&f.view, Some(f.key_path.clone()));
        self.settle();
        self.send(Event::Submit(handler));
        self.settle()
    }

    /// Press `keys` (as the tile bound them: `j`, `cmd-s`) in the main
    /// view, which takes the keyboard first. Panics when the view doesn't
    /// bind them (Pitwall's reserved keys are dropped from the keymap), or
    /// when they have no ⌘/Ctrl/Alt and a text field has focus: in Pitwall
    /// they would type into the field ([`Session::try_press`] to test
    /// that).
    pub fn press(&mut self, keys: &str) -> &mut Self {
        if let Err(why) = self.try_press(keys) {
            panic!("{why}");
        }
        self
    }

    /// [`Session::press`], saying why the key didn't reach the tile
    /// instead of panicking.
    pub fn try_press(&mut self, keys: &str) -> Result<&mut Self, String> {
        let Some(b) = self.frame().keymap.iter().find(|b| b.keys == keys) else {
            let why = if pitwall_tile_host::keys::reserved(keys) { " (Pitwall keeps that key for itself)" } else { "" };
            return Err(format!("the view binds no “{keys}”{why}"));
        };
        let handler = b.handler;
        let types = pitwall_tile_host::keys::parse(keys).is_some_and(|(m, _)| m.types());
        let field = self.focused.get(&self.main).filter(|p| self.find_key(p).is_some_and(|f| f.input().is_some())).cloned();
        if let (true, Some(field)) = (types, field) {
            return Err(format!("“{keys}” types into the focused text field “{field}” instead: a binding without ⌘/Ctrl/Alt doesn't fire while a field has focus"));
        }
        self.take_focus();
        self.send(Event::Key(ui::KeyEvent { handler, keys: keys.to_string() }));
        Ok(self.settle())
    }

    // ---- Focus, size and theme ---------------------------------------------

    /// The tile gets (`true`) or loses (`false`) the keyboard, as when the
    /// user clicks into it or into another tile. Losing it, the focused
    /// node hears its blur first.
    pub fn focus(&mut self, yes: bool) -> &mut Self {
        self.check_running();
        if yes {
            self.take_focus();
        } else {
            let views: Vec<String> = self.focused.keys().cloned().collect();
            for v in views {
                self.move_focus(&v, None);
            }
            if self.tile_focus {
                self.tile_focus = false;
                self.send(Event::Focus(false));
            }
        }
        self.settle()
    }

    /// Move the keyboard focus to the node at `key_path` in the main view
    /// (a text field, a focusable node), as a click or Tab would; the node
    /// that had it hears its blur.
    pub fn focus_key(&mut self, key_path: &str) -> &mut Self {
        let Some(f) = self.find_key(key_path) else { panic!("there is no node “{key_path}”; it shows:\n{}", self.text_snapshot()) };
        assert!(f.node.focusable || matches!(f.node.element, Element::Input(_) | Element::Button(_)), "“{key_path}” can't take focus: make it focusable");
        self.take_focus();
        self.move_focus(&f.view, Some(f.key_path));
        self.settle()
    }

    /// The focus leaves the main view's nodes but stays in the tile (a
    /// click on its background): the focused node hears its blur.
    pub fn blur(&mut self) -> &mut Self {
        let main = self.main.clone();
        self.move_focus(&main, None);
        self.settle()
    }

    /// Whether the tile has the keyboard.
    pub fn has_focus(&self) -> bool {
        self.tile_focus
    }

    /// The key path of the main view's focused node, if one has focus.
    pub fn focused(&self) -> Option<&str> {
        self.focused.get(&self.main).map(String::as_str)
    }

    /// What text field `key_path` holds now: the tile's last value, or what
    /// was typed since. Panics when there is no such field.
    pub fn field_text(&self, key_path: &str) -> String {
        self.field(key_path).text
    }

    /// Text field `key_path` as the user sees it. Panics when there is no
    /// such field.
    pub fn field(&self, key_path: &str) -> Field {
        let f = self.input(key_path);
        let Some(i) = f.input() else { unreachable!() };
        let at = (f.view.clone(), f.key_path.clone());
        Field {
            text: self.texts.get(&at).cloned().unwrap_or_default(),
            multiline: i.multiline,
            rows: i.rows,
            placeholder: i.placeholder.clone(),
            disabled: i.disabled,
            focused: self.focused.get(&f.view) == Some(&f.key_path),
        }
    }

    /// View `view` is now `width` × `height` logical pixels (a tile
    /// dragged wider, a window resized).
    pub fn resize(&mut self, view: &str, width: f32, height: f32) -> &mut Self {
        self.send(Event::Resize(ui::ViewSize { view: view.to_string(), size: ui::Size { width, height } }));
        self.settle()
    }

    /// The user's appearance changes (light or dark, density, motion).
    pub fn set_theme(&mut self, theme: ThemeInfo) -> &mut Self {
        self.check_running();
        self.theme = theme;
        self.handle.set_theme(theme);
        self.settle()
    }

    /// The light theme (the default theme otherwise: dark, compact).
    pub fn light(&mut self) -> &mut Self {
        let t = ThemeInfo { mode: ThemeMode::Light, ..self.theme };
        self.set_theme(t)
    }

    /// The icon names its frames used that Pitwall doesn't have (each also
    /// printed when first seen).
    pub fn unknown_icons(&self) -> &[String] {
        &self.unknown_icons
    }

    /// Open view `view` of tile `tile`, with `content` (JSON) if given.
    pub fn open_view(&mut self, view: &str, tile: &str, content: Option<serde_json::Value>) -> &mut Self {
        self.check_running();
        assert!(self.handle.open_view(ViewSpec { id: view.to_string(), tile: tile.to_string(), content: content.as_ref().map(json_content) }));
        self.settle()
    }

    /// Give view `view` new content, as an agent showing something new in
    /// the same place.
    pub fn set_content(&mut self, view: &str, content: serde_json::Value) -> &mut Self {
        self.check_running();
        assert!(self.handle.set_content(view, json_content(&content)));
        self.settle()
    }

    /// Close view `view`.
    pub fn close_view(&mut self, view: &str) -> &mut Self {
        self.check_running();
        self.handle.close_view(view);
        self.frames.remove(view);
        self.settle()
    }

    /// The user changes setting `key` to `value` (`None`: back to its
    /// default) in Settings.
    pub fn set_setting(&mut self, key: &str, value: Option<&str>) -> &mut Self {
        check_setting(&self.ext, key);
        self.store.set_setting(key, value).expect("the setting is written");
        self.check_running();
        self.handle.settings_changed();
        self.settle()
    }

    /// Show the tile (after [`Harness::hidden`] or [`Session::hide`]).
    pub fn show(&mut self) -> &mut Self {
        self.visible = true;
        self.handle.set_visible(true);
        self.settle()
    }

    /// Hide the tile: no renders, timers pause.
    pub fn hide(&mut self) -> &mut Self {
        self.visible = false;
        self.handle.set_visible(false);
        self.settle()
    }

    /// What `save-state` returns now.
    pub fn save_state(&mut self) -> Vec<u8> {
        self.check_running();
        self.saved = None;
        assert!(self.handle.save_state());
        self.pump(Instant::now() + WAIT, |s| s.saved.is_some());
        self.check_running();
        self.saved.take().expect("save-state answered")
    }

    /// Reload it as `pitwall extension dev` does: save its state, stop it,
    /// start it again from its folder with that state and the same data,
    /// and reopen the main view (without the keyboard).
    pub fn reload(&mut self) -> &mut Self {
        let state = self.save_state();
        self.start_again(Some(state))
    }

    /// Start it again with the same data but no saved state, as after
    /// Pitwall restarts or the tile was dropped without saving: what it
    /// kept in storage is all it has.
    pub fn restart(&mut self) -> &mut Self {
        self.check_running();
        self.start_again(None)
    }

    fn start_again(&mut self, state: Option<Vec<u8>>) -> &mut Self {
        self.handle.stop();
        self.pump(Instant::now() + WAIT, |s| s.stopped.is_some());
        let ext = Extension::load(self.ext.dir()).unwrap_or_else(|e| panic!("it can't be loaded again: {e}"));
        let options = StartOptions { views: vec![ViewSpec::tile(&self.main_tile())], grants: self.grants_now(&ext), theme: self.theme, visible: self.visible, state, clock: self.clock.clone() };
        let (handle, rx) = launch(&self.host, &ext, options);
        if let Some(info) = service_info(&ext, self.own_server) {
            handle.set_service(info);
        }
        (self.handle, self.rx, self.ext, self.stopped) = (handle, rx, ext, None);
        self.frames.clear();
        self.texts.clear();
        self.revisions.clear();
        self.focused.clear();
        self.asked.clear();
        self.tile_focus = false;
        self.sent = false;
        self.settle()
    }

    fn main_tile(&self) -> String {
        self.frames.get(&self.main).map(|f| f.tile.clone()).unwrap_or_else(|| self.main.clone())
    }

    // ---- What it keeps and says ---------------------------------------------

    /// Its key-value store, as written on disk.
    pub fn storage(&self) -> BTreeMap<String, Vec<u8>> {
        self.store.storage()
    }

    /// The stored value of `key`.
    pub fn stored(&self, key: &str) -> Option<Vec<u8>> {
        self.storage().remove(key)
    }

    /// The stored value of `key` as JSON (as the SDK's
    /// `storage::set_json` keeps it).
    pub fn stored_json<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.stored(key).map(|b| serde_json::from_slice(&b).unwrap_or_else(|e| panic!("“{key}” isn't JSON: {e}")))
    }

    /// Every Log line so far.
    pub fn log(&self) -> &[LogLine] {
        &self.log
    }

    /// The Log as text, one line each with its level.
    pub fn log_text(&self) -> String {
        self.log.iter().map(|l| format!("{:?}: {}\n", l.level, l.message)).collect()
    }

    /// Whether a Log line at `level` or above contains `text`.
    pub fn logged(&self, level: LogLevel, text: &str) -> bool {
        self.log.iter().any(|l| l.level >= level && l.message.contains(text))
    }

    /// The toasts it showed: text and tone.
    pub fn toasts(&self) -> Vec<(String, Tone)> {
        self.ui
            .iter()
            .filter_map(|r| match r {
                UiRequest::Toast { text, tone } => Some((text.clone(), *tone)),
                _ => None,
            })
            .collect()
    }

    /// Everything it asked of Pitwall's UI (toasts, views to open, scrolls,
    /// opening its Settings, notifications, files and pages to open,
    /// toolbar changes).
    pub fn ui_requests(&self) -> &[UiRequest] {
        &self.ui
    }

    // ---- Services -----------------------------------------------------------

    fn grants_now(&self, ext: &Extension) -> Grants {
        Grants::for_extension(ext.manifest(), self.grants.clone()).with_own_server(self.own_server)
    }

    /// The user grants permission `name` now; the tile hears
    /// `grants-changed` and its next call may use it.
    pub fn grant(&mut self, name: &str) -> &mut Self {
        if !self.grants.iter().any(|g| g == name) {
            self.grants.push(name.to_string());
        }
        self.check_running();
        self.handle.set_grants(self.grants_now(&self.ext));
        self.settle()
    }

    /// The user takes permission `name` back; the tile's next call is
    /// refused.
    pub fn revoke(&mut self, name: &str) -> &mut Self {
        self.grants.retain(|g| g != name);
        self.check_running();
        self.handle.set_grants(self.grants_now(&self.ext));
        self.settle()
    }

    /// Answer `net` requests to `url` from now on.
    pub fn respond(&mut self, url: &str, status: u16, body: impl Into<Vec<u8>>) -> &mut Self {
        self.net.respond(url, status, body);
        self
    }

    /// The `net` requests it made (that passed the checks), oldest first.
    pub fn requests(&self) -> Vec<Outgoing> {
        self.net.requests()
    }

    /// The notifications it sent (title, text), oldest first.
    pub fn notifications(&self) -> Vec<(String, String)> {
        self.ui
            .iter()
            .filter_map(|r| match r {
                UiRequest::Notify { title, body } => Some((title.clone(), body.clone())),
                _ => None,
            })
            .collect()
    }

    /// The toolbar button as the tile has set it so far.
    pub fn toolbar(&self) -> ToolbarState {
        let mut t = ToolbarState { badge: None, project_badges: BTreeMap::new(), tooltip: None, shown: None, popover_open: self.frames.contains_key(POPOVER_VIEW) };
        for r in &self.ui {
            match r {
                UiRequest::Toolbar(ToolbarRequest::Badge { project: None, badge, tone }) => t.badge = Some((*badge, *tone)),
                UiRequest::Toolbar(ToolbarRequest::Badge { project: Some(p), badge, tone }) => {
                    t.project_badges.insert(p.clone(), (*badge, *tone));
                }
                UiRequest::Toolbar(ToolbarRequest::Tooltip(s)) => t.tooltip = Some(s.clone()),
                UiRequest::Toolbar(ToolbarRequest::Shown(b)) => t.shown = Some(*b),
                _ => {}
            }
        }
        t
    }

    /// The user clicks its toolbar button: with a `popover` action the
    /// popover opens (view [`POPOVER_VIEW`]), or closes when open; with
    /// `tile` or `popover` the tile hears `toolbar-clicked` (`project`:
    /// the window's project space, if any).
    pub fn click_toolbar(&mut self, project: Option<&str>) -> &mut Self {
        use pitwall_core::extensions::toolbar::Action;
        self.check_running();
        let Some(tb) = self.ext.manifest().contributes.toolbar.clone() else { panic!("its extension.json declares no toolbar item") };
        match tb.action {
            Action::Popover { tile, .. } => {
                if self.frames.contains_key(POPOVER_VIEW) {
                    return self.close_view(POPOVER_VIEW);
                }
                self.send(Event::ToolbarClicked(project.map(String::from)));
                assert!(self.handle.open_view(ViewSpec { id: POPOVER_VIEW.to_string(), tile, content: None }));
            }
            Action::Tile(_) => {
                self.send(Event::ToolbarClicked(project.map(String::from)));
            }
            Action::Command(_) | Action::Menu(_) => panic!("its toolbar item runs commands, which go to its adapter, not the tile"),
        }
        self.settle();
        self.apply_close_popover()
    }

    /// A `close-popover` it asked for since the last action closes the
    /// popover, as Pitwall would.
    fn apply_close_popover(&mut self) -> &mut Self {
        let asked = self.ui.iter().rposition(|r| matches!(r, UiRequest::Toolbar(ToolbarRequest::ClosePopover)));
        if let Some(i) = asked {
            self.ui.remove(i);
            if self.frames.contains_key(POPOVER_VIEW) {
                return self.close_view(POPOVER_VIEW);
            }
        }
        self
    }

    /// Click the node at `key_path` in the popover (opened with
    /// [`Session::click_toolbar`]).
    pub fn click_in_popover(&mut self, key_path: &str) -> &mut Self {
        self.click_key_in(POPOVER_VIEW, key_path);
        self.apply_close_popover()
    }

    /// Its own server's state changes: the tile hears `service-changed`.
    pub fn set_service(&mut self, info: ServiceInfo) -> &mut Self {
        self.check_running();
        self.handle.set_service(info);
        self.settle()
    }

    /// The tile's database file, in the test's data folder (open it with
    /// any SQLite library to check what it keeps).
    pub fn database(&self) -> PathBuf {
        pitwall_tile_host::sql::db_file(&self.store.dir())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.handle.stop();
    }
}
