//! Phase 0 spike of the Tile SDK (docs/spec/tile-sdk/README.md §10.2,
//! results in docs/spec/tile-sdk/spike-results.md): Wasmtime running a
//! tile's WebAssembly component, the way runtime.md §2 describes it.
//!
//! - One [`Engine`] per app, with epoch interruption on and an
//!   [`EpochTicker`] thread bumping its epoch every [`EPOCH_TICK`].
//! - [`Runtime`]: the engine plus a linker holding the host imports (the
//!   tile's `log`, and WASI 0.2 with nothing granted).
//! - [`TileInstance`]: one `Store` and instance; every call has a deadline.
//! - [`Worker`]: the store on its own thread (`tile:<id>`); the UI thread
//!   only posts events and receives frames (runtime.md §2.4).
//! - [`diff`]: the host-side keyed diff of two frames.
//!
//! No GPUI here (runtime.md §2.2); `examples/tile_gpui.rs` maps a frame to
//! GPUI elements. A prototype: nothing ships with it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline,
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

pub mod diff;

mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "tile",
        additional_derives: [PartialEq],
    });
}

pub use bindings::pitwall::tile_spike::log::Level;
pub use bindings::pitwall::tile_spike::types::{
    Button, Configure, Element, Event, Frame, Length, Node, Style, TextSize, Token,
};
pub use wasmtime::{Error, Result};

/// How often the ticker bumps the engine's epoch (runtime.md §2.5).
pub const EPOCH_TICK: Duration = Duration::from_millis(10);
/// Hard deadline of `on-event` (runtime.md §2.5: 1 s).
pub const HANDLE_DEADLINE: Duration = Duration::from_secs(1);
/// Hard deadline of `render` (runtime.md §2.5: 250 ms).
pub const RENDER_DEADLINE: Duration = Duration::from_millis(250);
/// Linear memory per tile (runtime.md §2.5: 64 MiB by default).
pub const MEMORY_LIMIT: usize = 64 << 20;

/// The engine: Cranelift, epoch interruption on, and Wasmtime's own
/// compiled-code cache in `cache_dir` when given (only with the
/// `wasmtime-cache` feature; ignored otherwise).
pub fn engine(cache_dir: Option<&Path>) -> Result<Engine> {
    engine_with(cache_dir, |_| ())
}

/// [`engine`] with more settings, for the spike's comparisons (Winch,
/// fuel).
pub fn engine_with(cache_dir: Option<&Path>, more: impl FnOnce(&mut Config)) -> Result<Engine> {
    let mut config = Config::new();
    config.epoch_interruption(true);
    config.parallel_compilation(true);
    #[cfg(feature = "wasmtime-cache")]
    if let Some(dir) = cache_dir {
        let mut cache = wasmtime::CacheConfig::new();
        cache.with_directory(dir);
        config.cache(Some(wasmtime::Cache::new(cache)?));
    }
    #[cfg(not(feature = "wasmtime-cache"))]
    let _ = cache_dir;
    more(&mut config);
    Engine::new(&config)
}

/// Bumps an engine's epoch every [`EPOCH_TICK`] until dropped.
pub struct EpochTicker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl EpochTicker {
    pub fn start(engine: &Engine) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let engine = engine.clone();
        let flag = stop.clone();
        let thread = std::thread::Builder::new().name("tile-epoch".into()).spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(EPOCH_TICK);
                engine.increment_epoch();
            }
        })?;
        Ok(Self { stop, thread: Some(thread) })
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

/// What a tile's store holds: WASI with nothing granted, its limits and
/// the lines it logged since the host last took them.
pub struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    logs: Vec<(Level, String)>,
    /// When the current call must end (wall clock).
    deadline: Instant,
    /// Spike comparison: count ticks only, as the docs' recipe does.
    ticks_only: bool,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl bindings::pitwall::tile_spike::log::Host for HostState {
    fn log(&mut self, level: Level, message: String) {
        // A real host caps this (runtime.md §2.5: 200 host calls a second).
        if self.logs.len() < 1000 {
            self.logs.push((level, message));
        }
    }
}

impl bindings::pitwall::tile_spike::types::Host for HostState {}

/// The engine and the linker every tile instance is made from.
#[derive(Clone)]
pub struct Runtime {
    engine: Engine,
    linker: Linker<HostState>,
    /// Spike comparison: fuel given to each store (the engine must have
    /// `consume_fuel` on).
    fuel: Option<u64>,
}

impl Runtime {
    pub fn new(engine: Engine) -> Result<Self> {
        let mut linker = Linker::new(&engine);
        // WasiCtxBuilder's defaults: no stdin/out/err, env, args, preopens,
        // name lookup or sockets (runtime.md §3.1).
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        bindings::Tile::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s)?;
        Ok(Self { engine, linker, fuel: None })
    }

    /// Spike comparison: give each new store this much fuel.
    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel = Some(fuel);
        self
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Compile with Cranelift (a cache hit if the engine has Wasmtime's
    /// cache and has seen these bytes).
    pub fn compile(&self, wasm: &[u8]) -> Result<Component> {
        Component::new(&self.engine, wasm)
    }

    /// Pitwall's own cache (runtime.md §2.6): `<dir>/<hash>.cwasm`, written
    /// after a compile, memory-mapped on the next load. The spike keys by a
    /// 64-bit FNV hash of the bytes; the real one by SHA-256.
    pub fn load_or_compile(&self, wasm: &[u8], dir: &Path) -> Result<(Component, bool)> {
        let path = cache_path(dir, wasm);
        if path.exists() {
            // SAFETY: the file is written only by `serialize` below, in a
            // folder only this process writes; Wasmtime checks its header
            // against this engine's version and settings.
            match unsafe { Component::deserialize_file(&self.engine, &path) } {
                Ok(c) => return Ok((c, true)),
                Err(_) => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        let component = self.compile(wasm)?;
        std::fs::create_dir_all(dir)?;
        std::fs::write(&path, component.serialize()?)?;
        Ok((component, false))
    }

    pub fn instantiate(&self, component: &Component) -> Result<TileInstance> {
        let state = HostState {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new()
                .memory_size(MEMORY_LIMIT)
                .trap_on_grow_failure(true)
                .instances(16)
                .build(),
            logs: Vec::new(),
            deadline: Instant::now() + Duration::from_secs(2),
            ticks_only: false,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        // Each tick checks the wall clock: the ticker thread's sleeps run
        // long under load and on macOS (spike-results.md, "Epoch"), so
        // counting ticks alone overshoots a deadline by 40 % or more.
        store.epoch_deadline_callback(|cx| {
            let s = cx.data();
            if s.ticks_only || Instant::now() >= s.deadline {
                Ok(UpdateDeadline::Interrupt)
            } else {
                Ok(UpdateDeadline::Continue(1))
            }
        });
        store.set_epoch_deadline(1);
        if let Some(fuel) = self.fuel {
            store.set_fuel(fuel)?;
        }
        let tile = bindings::Tile::instantiate(&mut store, component, &self.linker)?;
        Ok(TileInstance { store, tile })
    }
}

fn cache_path(dir: &Path, wasm: &[u8]) -> PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in wasm {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    dir.join(format!("{h:016x}.cwasm"))
}

/// Epoch ticks for a deadline, at least one.
fn ticks(d: Duration) -> u64 {
    (d.as_nanos().div_ceil(EPOCH_TICK.as_nanos()) as u64).max(1)
}

impl TileInstance {
    fn arm(&mut self, deadline: Duration) {
        let s = self.store.data_mut();
        s.deadline = Instant::now() + deadline;
        let n = if s.ticks_only { ticks(deadline) } else { 1 };
        self.store.set_epoch_deadline(n);
    }

    /// Spike comparison only: end calls after the deadline's number of
    /// ticks, without looking at the clock.
    pub fn use_tick_deadlines(&mut self) {
        self.store.data_mut().ticks_only = true;
    }
}

/// One running tile: its store and instance. After a trap the instance is
/// dead (the caller drops it, runtime.md §2.3 "Killed on a hang").
pub struct TileInstance {
    store: Store<HostState>,
    tile: bindings::Tile,
}

impl TileInstance {
    pub fn render(&mut self, deadline: Duration) -> Result<Frame> {
        self.arm(deadline);
        self.tile.call_render(&mut self.store)
    }

    pub fn on_event(&mut self, event: Event, deadline: Duration) -> Result<bool> {
        self.arm(deadline);
        self.tile.call_on_event(&mut self.store, event)
    }

    /// Spike comparison: refill the fuel (for engines with fuel on).
    pub fn set_fuel(&mut self, fuel: u64) -> Result<()> {
        self.store.set_fuel(fuel)
    }

    pub fn take_logs(&mut self) -> Vec<(Level, String)> {
        std::mem::take(&mut self.store.data_mut().logs)
    }
}

/// From the UI thread to a tile's worker.
pub enum ToWorker {
    Event(Event),
}

/// From a tile's worker to the UI thread.
#[derive(Debug)]
pub enum FromWorker {
    /// A new tree, and how long `on-event` + `render` took on the worker.
    Frame { frame: Frame, took: Duration },
    Log(Level, String),
    /// The instance trapped, hung or failed to start; it is gone.
    Stopped { reason: String },
}

/// A tile's store on its own thread (runtime.md §2.4). The UI thread
/// calls [`Worker::send`], which never waits; replies go to the `sink`
/// the app gives (in the app: a channel its foreground task drains, then
/// `cx.notify()`).
pub struct Worker {
    tx: Option<mpsc::Sender<ToWorker>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(
        id: &str,
        runtime: Runtime,
        component: Component,
        mut sink: impl FnMut(FromWorker) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<ToWorker>();
        let thread = std::thread::Builder::new().name(format!("tile:{id}")).spawn(move || {
            // The store is created, used and dropped on this thread only.
            let mut tile = match runtime.instantiate(&component) {
                Ok(t) => t,
                Err(e) => return sink(FromWorker::Stopped { reason: format!("didn't start: {e}") }),
            };
            let mut changed = true;
            let mut started = Instant::now();
            loop {
                if changed {
                    let frame = tile.render(RENDER_DEADLINE);
                    for (level, line) in tile.take_logs() {
                        sink(FromWorker::Log(level, line));
                    }
                    match frame {
                        Ok(frame) => sink(FromWorker::Frame { frame, took: started.elapsed() }),
                        Err(e) => return sink(FromWorker::Stopped { reason: stop_reason(&e) }),
                    }
                }
                // Block for the next message, then take every one already
                // queued: a burst of events makes one render (§2.7).
                let Ok(first) = rx.recv() else { return };
                started = Instant::now();
                changed = false;
                for msg in std::iter::once(first).chain(rx.try_iter()) {
                    let ToWorker::Event(event) = msg;
                    match tile.on_event(event, HANDLE_DEADLINE) {
                        Ok(c) => changed |= c,
                        Err(e) => return sink(FromWorker::Stopped { reason: stop_reason(&e) }),
                    }
                }
                for (level, line) in tile.take_logs() {
                    sink(FromWorker::Log(level, line));
                }
            }
        })?;
        Ok(Self { tx: Some(tx), thread: Some(thread) })
    }

    /// Post an event; never blocks. False once the worker has stopped.
    pub fn send(&self, event: Event) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.send(ToWorker::Event(event)).is_ok())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the channel ends the loop once the current call returns
        // (a hung call ends at its deadline).
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn stop_reason(e: &Error) -> String {
    match e.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => "it didn't answer in time".into(),
        Some(trap) => format!("it crashed ({trap})"),
        None => format!("{e:#}"),
    }
}

/// Compile, start and render a tile once, click its button, render again:
/// the whole path linked into the app for the size measurement
/// (`pitwall` built with `--features tile-spike`, run with
/// `PITWALL_TILE_SPIKE=<tile.wasm>`). Returns a process exit code.
pub fn smoke(wasm_path: &Path) -> i32 {
    match smoke_inner(wasm_path) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            0
        }
        Err(e) => {
            eprintln!("tile spike: {e:#}");
            1
        }
    }
}

fn smoke_inner(wasm_path: &Path) -> Result<Vec<String>> {
    let wasm = std::fs::read(wasm_path)?;
    let cache = std::env::temp_dir().join("pitwall-tile-spike-cache");
    let runtime = Runtime::new(engine(Some(&cache.join("wasmtime")))?)?;
    let _ticker = EpochTicker::start(runtime.engine())?;
    let t = Instant::now();
    let (component, hit) = runtime.load_or_compile(&wasm, &cache.join("pitwall"))?;
    let mut out = vec![format!("loaded in {:?} (cache hit: {hit})", t.elapsed())];
    let (tx, rx) = mpsc::channel();
    let worker = Worker::spawn("smoke", runtime, component, move |m| {
        let _ = tx.send(m);
    })?;
    let mut frames = 0;
    while frames < 2 {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(FromWorker::Frame { frame, took }) => {
                frames += 1;
                out.push(format!("frame of {} nodes in {took:?}", frame.nodes.len()));
                worker.send(Event::Click(1));
            }
            Ok(FromWorker::Log(level, line)) => out.push(format!("log {level:?}: {line}")),
            Ok(FromWorker::Stopped { reason }) => return Err(Error::msg(reason)),
            Err(e) => return Err(Error::msg(format!("no frame: {e}"))),
        }
    }
    Ok(out)
}
