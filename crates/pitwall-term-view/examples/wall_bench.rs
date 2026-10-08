//! Wall-tile benchmark: N read-only tiles, each a terminal fed by its own
//! local PTY running the repo's synthetic busy agent (`scripts/tui-agent.py`:
//! ~10 redraws/s of Claude-Code-style output, ~12 KB/s; it only prints).
//! Measures this process only (the generators are children and excluded,
//! like agents are in docs/spec/perf.md): CPU, memory footprint, frames and
//! the CPU time each frame takes (render + layout + prepaint + paint).
//!
//!   cargo run --release -p pitwall-term-view --example wall_bench -- [--tiles 20] [--secs 10] [--settle 5] [--no-cache] [--window W H] [--idle]
//!
//! The window closes and every generator it started is stopped at the end.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::*, px, size, AnyView, App, Application, Bounds, Context, Element, Entity, GlobalElementId,
    InspectorElementId, LayoutId, Pixels, StyleRefinement, Window, WindowBounds, WindowOptions,
};
use pitwall_term_view::pty::LocalPty;
use pitwall_term_view::{TermFont, TermSize, Terminal, TerminalConfig, TerminalView, ViewMode, ViewSettings};
use portable_pty::CommandBuilder;

struct Opts {
    tiles: usize,
    secs: u64,
    settle: u64,
    cache: bool,
    cols: u16,
    rows: u16,
    win: (f32, f32),
    idle: bool,
}

fn opts() -> Opts {
    let mut o =
        Opts { tiles: 20, secs: 10, settle: 5, cache: true, cols: 100, rows: 30, win: (1600.0, 1000.0), idle: false };
    let mut it = std::env::args().skip(1);
    fn num<T: std::str::FromStr>(it: &mut impl Iterator<Item = String>) -> T {
        it.next().and_then(|v| v.parse().ok()).expect("number")
    }
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tiles" => o.tiles = num(&mut it),
            "--secs" => o.secs = num(&mut it),
            "--settle" => o.settle = num(&mut it),
            "--cols" => o.cols = num(&mut it),
            "--rows" => o.rows = num(&mut it),
            "--no-cache" => o.cache = false,
            "--idle" => o.idle = true,
            "--window" => o.win = (num(&mut it), num(&mut it)),
            _ => panic!("unknown argument {a}"),
        }
    }
    o
}

/// Frame timing: the root's render marks the start, a last child's paint the end.
#[derive(Default)]
struct Frames {
    start: Option<Instant>,
    times_us: Vec<u64>,
    recording: bool,
}

struct FrameEnd(Rc<RefCell<Frames>>);

impl IntoElement for FrameEnd {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for FrameEnd {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (window.request_layout(gpui::Style::default(), [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui::Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui::Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
        let mut f = self.0.borrow_mut();
        if let Some(start) = f.start.take() {
            if f.recording {
                f.times_us.push(start.elapsed().as_micros() as u64);
            }
        }
    }
}

struct Wall {
    tiles: Vec<Entity<TerminalView>>,
    frames: Rc<RefCell<Frames>>,
    cache: bool,
    per_row: usize,
}

impl Render for Wall {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.frames.borrow_mut().start = Some(Instant::now());
        let mut grid = div().size_full().flex().flex_wrap().bg(gpui::rgb(0x0d0f12));
        let w = 100.0 / self.per_row as f32;
        for t in &self.tiles {
            let view: AnyView = t.clone().into();
            let view = if self.cache { view.cached(StyleRefinement::default().size_full()) } else { view };
            grid = grid.child(div().w(gpui::relative(w / 100.0)).h(gpui::relative(0.25)).p(px(2.0)).child(view));
        }
        grid.child(FrameEnd(self.frames.clone()))
    }
}

// ── process measurements (macOS) ───────────────────────────────────────────

#[derive(Clone, Copy)]
struct Sample {
    at: Instant,
    cpu_s: f64,
    footprint: u64,
    rss: u64,
}

fn sample() -> Sample {
    let (footprint, rss) = mem();
    Sample { at: Instant::now(), cpu_s: cpu_s(), footprint, rss }
}

/// This process's user + system CPU time in seconds.
#[cfg(unix)]
fn cpu_s() -> f64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

/// Not measured on Windows (no getrusage); frames and frame times still are.
#[cfg(not(unix))]
fn cpu_s() -> f64 {
    0.0
}

/// Stop a generator this run started.
#[cfg(unix)]
fn stop(pid: u32) {
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
}

#[cfg(not(unix))]
fn stop(pid: u32) {
    let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).status();
}

#[cfg(target_os = "macos")]
fn mem() -> (u64, u64) {
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let r = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as i32,
            libc::RUSAGE_INFO_V2,
            &mut info as *mut _ as *mut libc::rusage_info_t,
        )
    };
    if r == 0 {
        (info.ri_phys_footprint, info.ri_resident_size)
    } else {
        (0, 0)
    }
}

#[cfg(not(target_os = "macos"))]
fn mem() -> (u64, u64) {
    (0, 0)
}

fn pct(v: &mut [u64], p: f64) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let o = opts();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/tui-agent.py");
    Application::new().run(move |cx: &mut App| {
        let size0 = TermSize::new(o.cols, o.rows);
        let mut terminals = Vec::new();
        let mut pids = Vec::new();
        for _ in 0..o.tiles {
            let mut cmd = if o.idle {
                CommandBuilder::new("cat") // prints nothing: an idle terminal
            } else {
                let mut c = CommandBuilder::new("python3");
                c.arg(script);
                c
            };
            cmd.env("PYTHONDONTWRITEBYTECODE", "1");
            let pty = LocalPty::spawn(cmd, size0).expect("start generator");
            pids.extend(pty.pid());
            terminals.push(Terminal::new(pty, size0, TerminalConfig { scrollback: 500, ..Default::default() }));
        }
        let frames = Rc::new(RefCell::new(Frames::default()));
        let bounds = Bounds::centered(None, size(px(o.win.0), px(o.win.1)), cx);
        let f2 = frames.clone();
        let cache = o.cache;
        let window = cx
            .open_window(
                WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
                |window, cx| {
                    window.set_window_title("wall bench");
                    let settings =
                        ViewSettings { font: TermFont { size: 13.0, ..Default::default() }, ..Default::default() };
                    let tiles = terminals
                        .iter()
                        .map(|t| {
                            cx.new(|cx| {
                                TerminalView::new(
                                    t.clone(),
                                    ViewMode::Tile { min_scale: 0.3 },
                                    settings.clone(),
                                    window,
                                    cx,
                                )
                            })
                        })
                        .collect();
                    cx.new(|_| Wall { tiles, frames: f2, cache, per_row: 5 })
                },
            )
            .expect("window");
        cx.activate(true);

        let (settle, secs, n) = (o.settle, o.secs, o.tiles);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(settle)).await;
            frames.borrow_mut().recording = true;
            let (shaped0, tile_frames0) = stats(&window, cx);
            let a = sample();
            let mut peak = a.footprint;
            let mut foot_sum = 0u64;
            let mut k = 0u64;
            let end = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < end {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let s = sample();
                peak = peak.max(s.footprint);
                foot_sum += s.footprint;
                k += 1;
            }
            let b = sample();
            frames.borrow_mut().recording = false;
            let (shaped1, tile_frames1) = stats(&window, cx);
            let wall = b.at.duration_since(a.at).as_secs_f64();
            let mut times = std::mem::take(&mut frames.borrow_mut().times_us);
            let nframes = times.len();
            let mean = times.iter().sum::<u64>() as f64 / nframes.max(1) as f64;
            let (p50, p95, p99, max) =
                (pct(&mut times, 0.5), pct(&mut times, 0.95), pct(&mut times, 0.99), pct(&mut times, 1.0));
            println!(
                "wall_bench: {n} tiles, {}x{} each, view cache {}",
                o.cols,
                o.rows,
                if cache { "on" } else { "off" }
            );
            println!("  cpu        {:5.1} % of one core", (b.cpu_s - a.cpu_s) / wall * 100.0);
            println!(
                "  footprint  {:5.1} MB avg, {:5.1} MB peak (rss {:5.1} MB)",
                foot_sum as f64 / k.max(1) as f64 / 1048576.0,
                peak as f64 / 1048576.0,
                b.rss as f64 / 1048576.0
            );
            println!("  frames     {:5.1} /s", nframes as f64 / wall);
            println!(
                "  frame cpu  mean {:.2} ms, p50 {:.2}, p95 {:.2}, p99 {:.2}, max {:.2} ms",
                mean / 1000.0,
                p50 as f64 / 1000.0,
                p95 as f64 / 1000.0,
                p99 as f64 / 1000.0,
                max as f64 / 1000.0
            );
            println!(
                "  rows shaped {:.0} /s, tile repaints {:.0} /s",
                (shaped1 - shaped0) as f64 / wall,
                (tile_frames1 - tile_frames0) as f64 / wall
            );
            // Stop the generators this run started (by PID), then quit.
            for pid in &pids {
                stop(*pid);
            }
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
}

fn stats(window: &gpui::WindowHandle<Wall>, cx: &mut gpui::AsyncApp) -> (u64, u64) {
    window
        .update(cx, |wall, _, cx| {
            wall.tiles.iter().fold((0, 0), |(s, f), t| {
                let st = t.read(cx).render_stats();
                (s + st.rows_shaped, f + st.frames)
            })
        })
        .unwrap_or((0, 0))
}
