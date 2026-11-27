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
//! Timers run on the wall clock: [`Session::wait`] lets time pass.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pitwall_tile_host::ui::{self, Element, Event, Frame, Modifiers, MouseButton, Node, Point};
use pitwall_tile_host::{Extension, ExtensionData, FrameUpdate, Grants, HostConfig, LogLevel, LogLine, Secrets, StartOptions, StopReason, TileHandle, TileHost, TileOutput, UiRequest, ViewSpec};

pub use pitwall_tile_host::ui::{Listen, Tone};
pub use pitwall_tile_host::{LogLevel as Level, StopReason as Stop};

/// The Rust target tiles are built for.
pub const WASM_TARGET: &str = "wasm32-wasip2";

/// How long one round trip may take before the test fails (a tile's own
/// deadlines are shorter: 1 s to handle, 250 ms to render).
const WAIT: Duration = Duration::from_secs(30);

/// Build the tile in `dir` (a Cargo project with an `extension.json`) with
/// `cargo build --release --target wasm32-wasip2` and copy the component
/// to where the manifest's tile entry names it (`"wasm"`, like
/// `tile.wasm`). Returns that path, or what went wrong (with the
/// compiler's output, or how to install the target).
pub fn build(dir: &Path) -> Result<PathBuf, String> {
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
    std::fs::copy(&built, &to).map_err(|e| format!("{} couldn't be copied to {}: {e}", built.display(), to.display()))?;
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
}

impl Harness {
    /// The extension in `dir`, already built (its component where its
    /// manifest names it).
    pub fn load(dir: impl AsRef<Path>) -> Harness {
        Harness { dir: dir.as_ref().to_path_buf(), settings: Vec::new(), secrets: HashMap::new(), grants: Vec::new(), state: None, visible: true }
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
        let host = TileHost::new(HostConfig { data_dir: data.path().to_path_buf(), secrets: Arc::new(FakeSecrets(self.secrets.clone())) }).expect("the tile runtime starts");
        let store = host.extension_data(ext.id());
        for (key, value) in &self.settings {
            check_setting(&ext, key);
            store.set_setting(key, Some(value)).expect("the setting is written");
        }
        let options = StartOptions { views: vec![view.clone()], grants: Grants::new(self.grants.clone()), theme: ui::default_theme(), visible: self.visible, state: self.state.clone() };
        let (handle, rx) = launch(&host, &ext, options);
        let mut s = Session { _data: data, host, ext, store, handle, rx, main: view.id, frames: BTreeMap::new(), revisions: HashMap::new(), log: Vec::new(), ui: Vec::new(), stopped: None, saved: None, grants: self.grants, visible: self.visible };
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
    log: Vec<LogLine>,
    ui: Vec<UiRequest>,
    stopped: Option<StopReason>,
    saved: Option<Vec<u8>>,
    grants: Vec<String>,
    visible: bool,
}

impl Session {
    fn take(&mut self, o: TileOutput) {
        match o {
            TileOutput::Frame(f) => {
                let paths = key_paths(&f.frame);
                for (i, n) in f.frame.nodes.iter().enumerate() {
                    if let Element::Input(input) = &n.element {
                        if input.value.is_some() {
                            let at = (f.view.clone(), paths[i].clone());
                            let r = self.revisions.entry(at).or_default();
                            *r = (*r).max(input.revision);
                        }
                    }
                }
                self.frames.insert(f.view.clone(), f);
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
        // after the first batch's frames and Log lines.
        for _ in 0..2 {
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
        }
        Ok(())
    }

    /// Let `time` pass (timers fire, while shown), then settle.
    pub fn wait(&mut self, time: Duration) -> &mut Self {
        self.pump(Instant::now() + time, |_| false);
        self.settle()
    }

    /// Wait up to `timeout` for a view to show `text`, for what a timer
    /// brings. Panics when it doesn't.
    pub fn wait_for_text(&mut self, text: &str, timeout: Duration) -> &mut Self {
        let until = Instant::now() + timeout;
        let main = self.main.clone();
        let found = self.pump(until, |s| s.find_text_in(&main, text).is_some());
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
    /// kind, text, key and listeners. Stable across Pitwall versions within
    /// a world, for snapshot tests.
    pub fn text_snapshot(&self) -> String {
        self.text_snapshot_of(&self.main)
    }

    /// [`Session::text_snapshot`] of view `view`.
    pub fn text_snapshot_of(&self, view: &str) -> String {
        let Some(f) = self.frames.get(view) else { return format!("(view “{view}” has no frame)\n") };
        let nodes = &f.frame.nodes;
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
                Element::Input(input) => format!("input{}", input.placeholder.as_ref().map(|p| format!(" placeholder={p:?}")).unwrap_or_default()),
            };
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
                })
                .collect();
            let on = if on.is_empty() { String::new() } else { format!(" [{}]", on.join(", ")) };
            let label = n.label.as_ref().map(|l| format!(" label={l:?}")).unwrap_or_default();
            out.push_str(&format!("{}{what}{key}{label}{on}\n", "  ".repeat(depth[i])));
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
    /// user typed it.
    pub fn type_into(&mut self, key_path: &str, text: &str) -> &mut Self {
        let f = self.input(key_path);
        let Some(l) = f.node.on.iter().find(|l| l.on == Listen::Change) else { panic!("the text field “{key_path}” doesn't listen for changes") };
        let at = (f.view.clone(), f.key_path.clone());
        let revision = {
            let r = self.revisions.entry(at).or_default();
            *r += 1;
            *r
        };
        let end = text.len() as u32;
        self.send(Event::InputChanged(ui::InputChange { handler: l.handler, value: text.to_string(), revision, selection: ui::TextRange { start: end, end } }));
        self.settle()
    }

    /// Press Enter in text field `key_path`.
    pub fn submit(&mut self, key_path: &str) -> &mut Self {
        let f = self.input(key_path);
        let handler = self.handler_up(&f, Listen::Submit);
        self.send(Event::Submit(handler));
        self.settle()
    }

    /// Press `keys` (as the tile bound them: `j`, `cmd-r`) while the main
    /// view has focus.
    pub fn press(&mut self, keys: &str) -> &mut Self {
        let Some(b) = self.frame().keymap.iter().find(|b| b.keys == keys) else { panic!("the view binds no “{keys}”") };
        let handler = b.handler;
        self.send(Event::Key(ui::KeyEvent { handler, keys: keys.to_string() }));
        self.settle()
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
    /// and reopen the main view.
    pub fn reload(&mut self) -> &mut Self {
        let state = self.save_state();
        self.handle.stop();
        self.pump(Instant::now() + WAIT, |s| s.stopped.is_some());
        let ext = Extension::load(self.ext.dir()).unwrap_or_else(|e| panic!("it can't be loaded again: {e}"));
        let options = StartOptions { views: vec![ViewSpec::tile(&self.main_tile())], grants: Grants::new(self.grants.clone()), theme: ui::default_theme(), visible: self.visible, state: Some(state) };
        let (handle, rx) = launch(&self.host, &ext, options);
        (self.handle, self.rx, self.ext, self.stopped) = (handle, rx, ext, None);
        self.frames.clear();
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
    /// opening its Settings).
    pub fn ui_requests(&self) -> &[UiRequest] {
        &self.ui
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.handle.stop();
    }
}
