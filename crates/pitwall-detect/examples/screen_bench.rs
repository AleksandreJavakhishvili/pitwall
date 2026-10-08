//! Parser benchmark (docs/spec/perf.md): what keeping one agent's headless
//! screen costs, per emulator, on a recording of busy TUI output.
//!
//!   python3 scripts/tui-agent.py --frames 3000 --cols 100 > /tmp/tui.bin
//!   cargo run --release -p pitwall-detect --example screen_bench -- /tmp/tui.bin
//!
//! Each frame of the recording is fed as one chunk (like one PTY read);
//! status rules run every 4th frame (the ticker reads the screen every
//! 400 ms, a TUI draws ~10 frames/s). Reports time per MB and per frame,
//! what that means for 20 agents at the recording's live rate, and heap
//! allocations per frame.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(n as u64, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

const ROWS: u16 = 30;
const COLS: u16 = 100;
/// The recording's live pace (tui-agent.py default).
const FPS: f64 = 10.0;

trait Emu {
    fn feed(&mut self, b: &[u8]);
    fn text(&self) -> (String, Option<String>);
}

struct Vt(vt100::Parser);
impl Emu for Vt {
    fn feed(&mut self, b: &[u8]) {
        self.0.process(b);
    }
    fn text(&self) -> (String, Option<String>) {
        let s = self.0.screen();
        let rows: Vec<String> = s.rows(0, s.size().1).map(|r| r.trim_end().to_string()).collect();
        (rows.join("\n"), None)
    }
}

struct Ours(pitwall_detect::Screen);
impl Emu for Ours {
    fn feed(&mut self, b: &[u8]) {
        self.0.feed(b);
    }
    fn text(&self) -> (String, Option<String>) {
        (self.0.text(), self.0.title())
    }
}

/// Split the recording into frames: each starts at an Ink-style erase (ESC[2K)
/// run, roughly like the PTY reads a live agent produces.
fn frames(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 1;
    while i + 4 < data.len() {
        if &data[i..i + 4] == b"\x1b[2K" && data[i - 1] != b'A' && data[i - 1] != b'K' {
            out.push(&data[start..i]);
            start = i;
        }
        i += 1;
    }
    out.push(&data[start..]);
    out
}

fn run(name: &str, emu: &mut dyn Emu, frames: &[&[u8]], detect: bool) -> f64 {
    let bytes: usize = frames.iter().map(|f| f.len()).sum();
    let (a0, b0) = (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed));
    let t = Instant::now();
    let mut matched = 0usize;
    for (i, f) in frames.iter().enumerate() {
        emu.feed(f);
        if detect && i % 4 == 3 {
            let (text, title) = emu.text();
            matched += pitwall_detect::detect("claude", &text, title.as_deref()).state.is_some() as usize;
        }
    }
    let secs = t.elapsed().as_secs_f64();
    let allocs = ALLOCS.load(Ordering::Relaxed) - a0;
    let abytes = BYTES.load(Ordering::Relaxed) - b0;
    let n = frames.len() as f64;
    let per_frame_us = secs * 1e6 / n;
    // 20 agents at FPS frames/s: share of one core.
    let core20 = per_frame_us * FPS * 20.0 / 1e6 * 100.0;
    println!(
        "{name:<34} {:7.1} MB/s  {:6.2} µs/frame  20 agents @{FPS} fps: {:5.2} % of a core  allocs/frame {:6.1} ({:7.0} B){}",
        bytes as f64 / secs / 1e6,
        per_frame_us,
        core20,
        allocs as f64 / n,
        abytes as f64 / n,
        if detect { format!("  [{matched} rule hits]") } else { String::new() },
    );
    secs
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: screen_bench <recording> (see the header)");
    let data = std::fs::read(&path).expect("read recording");
    let frames = frames(&data);
    println!("{} frames, {:.1} KB/frame, {} rows × {} cols", frames.len(), data.len() as f64 / frames.len() as f64 / 1024.0, ROWS, COLS);
    // Warm up both, then measure.
    for _ in 0..2 {
        run("warm-up vt100", &mut Vt(vt100::Parser::new(ROWS, COLS, 0)), &frames, false);
        run("warm-up alacritty", &mut Ours(pitwall_detect::Screen::new(ROWS, COLS)), &frames, false);
    }
    println!("--");
    run("vt100: feed", &mut Vt(vt100::Parser::new(ROWS, COLS, 0)), &frames, false);
    run("alacritty (Screen): feed", &mut Ours(pitwall_detect::Screen::new(ROWS, COLS)), &frames, false);
    run("vt100: feed + text + rules", &mut Vt(vt100::Parser::new(ROWS, COLS, 0)), &frames, true);
    run("alacritty (Screen): feed + text + rules", &mut Ours(pitwall_detect::Screen::new(ROWS, COLS)), &frames, true);

    // Snapshots for the Wall (10 per second per visible tile).
    let mut s = pitwall_detect::Screen::new(ROWS, COLS);
    let (a0, t) = (ALLOCS.load(Ordering::Relaxed), Instant::now());
    for f in &frames {
        s.feed(f);
        std::hint::black_box(s.snapshot());
    }
    let per = t.elapsed().as_secs_f64() * 1e6 / frames.len() as f64;
    println!(
        "alacritty (Screen): feed + snapshot   {per:6.2} µs/frame  allocs/frame {:.1}",
        (ALLOCS.load(Ordering::Relaxed) - a0) as f64 / frames.len() as f64
    );
}
