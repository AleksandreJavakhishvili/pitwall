//! When output repaints a terminal: on a shared frame grid, not on every
//! chunk of output.
//!
//! A GPUI view that changes dirties its ancestors too, so a terminal that
//! repaints makes the window lay out everything around it again. Busy
//! agents print 10–12 frames a second each, out of step with one another:
//! with a few of them on screen, notifying each terminal as its output
//! arrives draws the window at the display's full rate. Instead, a
//! terminal whose output changed waits for the next tick of one grid
//! ([`OUTPUT_FPS`], shared by every terminal of the window), and all the
//! terminals that changed meanwhile repaint in that one frame. Other loops
//! of the host (animations) can ride the same grid with [`next_tick`], so
//! their frames and the terminals' coincide.
//!
//! Typing stays immediate: a terminal that had input in the last
//! [`ECHO_WINDOW`] repaints at once, so echo is never held back.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::{App, EntityId, Window, WindowId};

/// Output repaints per second, at most (per window).
pub const OUTPUT_FPS: f32 = 30.0;

/// Output this soon after input repaints at once (echo).
pub const ECHO_WINDOW: Duration = Duration::from_millis(250);

/// A frame this close before a tick counts as on time (a vsync early).
const SLACK: Duration = Duration::from_millis(4);

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn period() -> Duration {
    Duration::from_secs_f32(1.0 / OUTPUT_FPS)
}

/// The first tick of the shared grid at or after `at`.
pub fn next_tick(at: Instant) -> Instant {
    let p = period().as_nanos();
    let since = at.saturating_duration_since(epoch()).as_nanos();
    let ticks = since.div_ceil(p);
    epoch() + Duration::from_nanos((ticks * p) as u64)
}

/// Whether a frame starting now is (about) at or past `due`.
pub fn is_due(now: Instant, due: Instant) -> bool {
    now + SLACK >= due
}

#[derive(Default)]
struct Gate {
    /// Views to notify at the next tick.
    pending: Vec<EntityId>,
    armed: bool,
}

thread_local! {
    static GATES: RefCell<HashMap<WindowId, Gate>> = RefCell::new(HashMap::new());
}

/// Ask for `view` (in `window`) to be notified at the next tick of the grid.
pub(crate) fn request(view: EntityId, window: &mut Window) {
    let id = window.window_handle().window_id();
    let arm = GATES.with(|g| {
        let mut g = g.borrow_mut();
        let gate = g.entry(id).or_default();
        if !gate.pending.contains(&view) {
            gate.pending.push(view);
        }
        !std::mem::replace(&mut gate.armed, true)
    });
    if arm {
        wait_for(window, id, next_tick(Instant::now()));
    }
}

fn wait_for(window: &Window, id: WindowId, due: Instant) {
    window.on_next_frame(move |window, cx| {
        if is_due(Instant::now(), due) {
            flush(id, cx);
        } else {
            wait_for(window, id, due);
        }
    });
}

fn flush(id: WindowId, cx: &mut App) {
    let views = GATES.with(|g| {
        let mut g = g.borrow_mut();
        let gate = g.entry(id).or_default();
        gate.armed = false;
        std::mem::take(&mut gate.pending)
    });
    for view in views {
        cx.notify(view);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_on_one_grid() {
        let a = next_tick(Instant::now());
        let b = next_tick(a + Duration::from_millis(1));
        let gap = b - a;
        assert!(
            (gap.as_secs_f32() - 1.0 / OUTPUT_FPS).abs() < 0.001,
            "{gap:?}"
        );
        // A tick is its own next tick.
        assert_eq!(next_tick(a), a);
        assert!(is_due(a - Duration::from_millis(2), a));
        assert!(!is_due(a - Duration::from_millis(10), a));
    }
}
