//! `PITWALL_FRAME_STATS=1`: log frame timing to stderr every 2 s while the
//! window draws (paint-to-paint interval and the time from the start of a
//! frame's render to its paint, p50 / p95 / max). A tool for perf work;
//! does nothing unless the variable is set.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use gpui::{canvas, prelude::*, AnyElement, Styled};

#[derive(Default)]
struct Stats {
    last_paint: Option<Instant>,
    render_at: Option<Instant>,
    intervals: Vec<f64>,
    work: Vec<f64>,
    since: Option<Instant>,
    /// Frames that started off the shared frame grid (not output or a
    /// ring frame: input, a store change, a hover, a timer).
    off_grid: usize,
}

thread_local! {
    static STATS: RefCell<Stats> = RefCell::new(Stats::default());
}

pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PITWALL_FRAME_STATS").is_some())
}

/// Call at the start of the root view's render.
pub fn render_started() {
    if enabled() {
        let now = Instant::now();
        // On the grid: a tick in the last vsync (120 Hz) or just ahead.
        let tick = pitwall_term_view::frames::next_tick(now - Duration::from_millis(9));
        let on_grid = tick <= now + Duration::from_millis(4);
        STATS.with(|s| {
            let mut s = s.borrow_mut();
            s.render_at = Some(now);
            if !on_grid {
                s.off_grid += 1;
            }
        });
    }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() as f64 - 1.) * p).round() as usize]
}

/// Paint this last in the root view.
pub fn probe() -> Option<AnyElement> {
    if !enabled() {
        return None;
    }
    Some(
        canvas(
            |_, _, _| {},
            |_, _, _, _| {
                let now = Instant::now();
                STATS.with(|s| {
                    let mut s = s.borrow_mut();
                    if let Some(at) = s.render_at.take() {
                        s.work.push((now - at).as_secs_f64() * 1000.);
                    }
                    if let Some(last) = s.last_paint {
                        let gap = (now - last).as_secs_f64() * 1000.;
                        // Idle gaps aren't frames of an animation or scroll.
                        if gap < 100. {
                            s.intervals.push(gap);
                        }
                    }
                    s.last_paint = Some(now);
                    let since = *s.since.get_or_insert(now);
                    if now - since >= Duration::from_secs(2) && s.intervals.len() >= 10 {
                        let n = s.intervals.len();
                        let (mut i, mut w) = (s.intervals.clone(), s.work.clone());
                        eprintln!(
                            "frames {n} in 2s ({} off the grid) · interval p50 {:.1} p95 {:.1} max {:.1} ms · render→paint p50 {:.1} p95 {:.1} max {:.1} ms",
                            s.off_grid,
                            pct(&mut i, 0.5), pct(&mut i, 0.95), pct(&mut i, 1.),
                            pct(&mut w, 0.5), pct(&mut w, 0.95), pct(&mut w, 1.)
                        );
                        s.intervals.clear();
                        s.work.clear();
                        s.off_grid = 0;
                        s.since = Some(now);
                    } else if now - since >= Duration::from_secs(2) {
                        s.intervals.clear();
                        s.work.clear();
                        s.off_grid = 0;
                        s.since = Some(now);
                    }
                });
            },
        )
        .absolute()
        .size_0()
        .into_any_element(),
    )
}
