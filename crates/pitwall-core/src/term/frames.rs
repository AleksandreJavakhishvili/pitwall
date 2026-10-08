//! Styled frames of an agent's screen for UIs that draw it without a
//! terminal emulator (the Wall, docs/spec/wall.md): each watcher gets the
//! whole screen first, then only the rows that changed, from its own thread,
//! at most one frame per `gap`, and only when something changed.

use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Weak;
use std::time::Duration;

use pitwall_detect::screen::{Color, Run, Snapshot};
use pitwall_proto::{screen_attr, ScreenFrame, ScreenLine, ScreenRun};

use super::{lock, TermHost};

/// Receives frames. Returning `false` ends the watch (its consumer is gone).
pub type FrameSink = Box<dyn FnMut(&ScreenFrame) -> bool + Send>;

pub(super) struct Watcher {
    pub id: u64,
    wake: SyncSender<()>,
}

impl Watcher {
    /// Tell the watcher the screen changed; `false` once its thread is gone.
    pub fn wake(&self) -> bool {
        !matches!(self.wake.try_send(()), Err(TrySendError::Disconnected(_)))
    }
}

pub(super) fn spawn(id: u64, host: Weak<TermHost>, sink: FrameSink, gap: Duration) -> Option<Watcher> {
    let (tx, rx) = mpsc::sync_channel(1);
    let _ = tx.try_send(()); // the first frame right away
    std::thread::Builder::new().name("term-frames".into()).spawn(move || run(rx, host, sink, gap)).ok()?;
    Some(Watcher { id, wake: tx })
}

fn run(rx: Receiver<()>, host: Weak<TermHost>, mut sink: FrameSink, gap: Duration) {
    let mut last = Frames::default();
    // Ends when the watch is dropped (unwatch, or the host is gone).
    while rx.recv().is_ok() {
        let Some(h) = host.upgrade() else { return };
        let snap = lock(&h.screen).snapshot();
        drop(h);
        if let Some(frame) = last.next(&snap) {
            if !sink(&frame) {
                return;
            }
        }
        std::thread::sleep(gap);
    }
}

/// The whole screen as one frame (what a new watcher gets first).
pub fn full_frame(snap: &Snapshot) -> ScreenFrame {
    Frames::default().next(snap).expect("a first frame is always full")
}

/// What a watcher was sent last, to send only what changed.
#[derive(Default)]
pub(super) struct Frames {
    size: Option<(u16, u16)>,
    cursor: Option<(u16, u16)>,
    rows: Vec<Vec<ScreenRun>>,
}

impl Frames {
    /// The frame that brings the watcher from what it has to `snap`; `None`
    /// when nothing changed.
    pub fn next(&mut self, snap: &Snapshot) -> Option<ScreenFrame> {
        let rows: Vec<Vec<ScreenRun>> = snap.lines.iter().map(|runs| runs.iter().map(wire_run).collect()).collect();
        let full = self.size != Some((snap.cols, snap.rows));
        let lines: Vec<ScreenLine> =
            rows.iter().enumerate().filter(|(i, r)| full || self.rows.get(*i) != Some(*r)).map(|(i, r)| ScreenLine(i as u16, r.clone())).collect();
        if !full && lines.is_empty() && self.cursor == snap.cursor {
            return None;
        }
        self.size = Some((snap.cols, snap.rows));
        self.cursor = snap.cursor;
        self.rows = rows;
        Some(ScreenFrame { cols: snap.cols, rows: snap.rows, cursor: snap.cursor, full, lines })
    }
}

fn wire_color(c: Color) -> u32 {
    match c {
        Color::Default => 0,
        Color::Palette(i) => i as u32 + 1,
        Color::Rgb(r, g, b) => screen_attr::RGB | (r as u32) << 16 | (g as u32) << 8 | b as u32,
    }
}

fn wire_run(r: &Run) -> ScreenRun {
    // pitwall-detect's attribute bits are the wire's (checked in the tests).
    let mut attrs = r.style.attrs;
    if r.alone() {
        attrs |= if r.cells == 2 { screen_attr::WIDE } else { screen_attr::CLUSTER };
    }
    ScreenRun(r.text.clone(), wire_color(r.style.fg), wire_color(r.style.bg), attrs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_detect::screen::attr;
    use pitwall_detect::Screen;

    #[test]
    fn attribute_bits_match_the_wire() {
        assert_eq!(
            [attr::BOLD, attr::ITALIC, attr::DIM, attr::INVERSE, attr::HIDDEN, attr::STRIKE, attr::UNDERLINE_SHIFT, attr::UNDERLINE_MASK],
            [
                screen_attr::BOLD,
                screen_attr::ITALIC,
                screen_attr::DIM,
                screen_attr::INVERSE,
                screen_attr::HIDDEN,
                screen_attr::STRIKE,
                screen_attr::UNDERLINE_SHIFT,
                screen_attr::UNDERLINE_MASK
            ]
        );
    }

    #[test]
    fn full_then_changed_rows_only() {
        let mut s = Screen::new(3, 20);
        s.feed(b"\x1b[1;32mok\x1b[0m \x1b[38;2;1;2;3mx\x1b[0m\r\n\xe6\x97\xa5\r\nthree");
        let mut f = Frames::default();
        let first = f.next(&s.snapshot()).unwrap();
        assert!(first.full);
        assert_eq!((first.cols, first.rows, first.cursor), (20, 3, Some((5, 2))));
        assert_eq!(
            first.lines[0],
            ScreenLine(
                0,
                vec![
                    ScreenRun("ok".into(), 3, 0, screen_attr::BOLD),
                    ScreenRun(" ".into(), 0, 0, 0),
                    ScreenRun("x".into(), screen_attr::RGB | 0x010203, 0, 0),
                ]
            )
        );
        assert_eq!(first.lines[1], ScreenLine(1, vec![ScreenRun("日".into(), 0, 0, screen_attr::WIDE)]));
        assert_eq!(f.next(&s.snapshot()), None, "nothing changed: nothing to send");

        s.feed(b"\x1b[2;1Hnew");
        let delta = f.next(&s.snapshot()).unwrap();
        assert!(!delta.full);
        assert_eq!(delta.lines, vec![ScreenLine(1, vec![ScreenRun("new".into(), 0, 0, 0)])]);
        assert_eq!(delta.cursor, Some((3, 1)));

        // Only the cursor moved.
        s.feed(b"\x1b[1;1H");
        let moved = f.next(&s.snapshot()).unwrap();
        assert!(moved.lines.is_empty());
        assert_eq!(moved.cursor, Some((0, 0)));

        s.resize(4, 20);
        assert!(f.next(&s.snapshot()).unwrap().full, "a new size sends every row");
    }
}
