//! What a terminal's scrollback costs, measured with a counting allocator:
//! a terminal no view looks at keeps it frozen (`frozen.rs`), a fraction
//! of alacritty's full rows of cells.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use pitwall_term_view::alacritty_terminal::grid::Dimensions;
use pitwall_term_view::{NullStream, TermSize, Terminal, TerminalConfig};

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as isize, Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as isize, Ordering::Relaxed);
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        LIVE.fetch_add(new as isize - l.size() as isize, Ordering::Relaxed);
        System.realloc(p, l, new)
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Tests here measure the whole process: one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn lines(t: &Terminal, n: usize) {
    for i in 0..n {
        t.feed().push(
            format!("\x1b[32m+\x1b[0m made-up output line {i:05}: compiling the demo module\r\n")
                .as_bytes(),
        );
    }
}

/// Live heap bytes of a 140×40 terminal after 5 000 lines of output.
fn terminal_bytes(freeze_after: Duration) -> (usize, Terminal) {
    let before = LIVE.load(Ordering::Relaxed);
    let config = TerminalConfig {
        scrollback: 5_000,
        freeze_after,
        ..TerminalConfig::default()
    };
    let t = Terminal::new(NullStream, TermSize::new(140, 40), config);
    lines(&t, 5_000);
    t.freeze_if_unseen();
    let used = (LIVE.load(Ordering::Relaxed) - before) as usize;
    (used, t)
}

#[test]
fn unseen_scrollback_is_kept_compact() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (full, t) = terminal_bytes(Duration::from_secs(3600));
    assert_eq!(t.frozen_rows(), 0);
    drop(t);
    let (frozen, t) = terminal_bytes(Duration::ZERO);
    assert_eq!(t.frozen_rows(), 4_961);
    // ~17 MB of cells against ~100 bytes a line plus a screen of cells and
    // alacritty's spare rows.
    assert!(full > 16 << 20, "{} KB", full / 1024);
    assert!(
        frozen < full / 4,
        "frozen {} KB, full {} KB",
        frozen / 1024,
        full / 1024
    );
    eprintln!("full {} KB, frozen {} KB", full / 1024, frozen / 1024);
    // Seen again: the scrollback is all there.
    assert_eq!(t.lock().term().history_size(), 4_961);
}
