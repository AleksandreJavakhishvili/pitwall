//! Terminal modes that outlive the replay ring.
//!
//! A program sets most of its terminal modes once, when it starts: Claude
//! Code turns on bracketed paste (`CSI ? 2004 h`) and hides the cursor, a TUI
//! switches to the alternate screen, asks for mouse or focus reports. Once
//! more output than the ring holds has gone by, those sequences are no longer
//! in it, and a terminal that attaches later replays a stream that never
//! turns them on: it pastes without the `ESC[200~ … ESC[201~` brackets (each
//! line break in a long paste then reads as Enter), sends the wrong cursor
//! keys or no mouse reports.
//!
//! [`Modes`] is fed the bytes that leave the ring and keeps the state of the
//! DEC private modes that matter to a terminal attaching later; its
//! [`preamble`](Modes::preamble) restores that state in front of the replay.
//!
//! This file exists twice, byte for byte: `pitwall-hold/src/modes.rs` (the
//! holder's ring) and `pitwall-core/src/term/modes.rs` (the engine's ring;
//! the core never depends on the holder, architecture.md §1). A test in
//! pitwall-core checks that they match.

/// The DEC private modes kept, with their power-on state.
const TRACKED: &[(u16, bool)] = &[
    (1, false),    // application cursor keys
    (7, true),     // autowrap
    (25, true),    // cursor visible
    (47, false),   // alternate screen
    (66, false),   // application keypad
    (1000, false), // mouse: clicks
    (1002, false), // mouse: drags
    (1003, false), // mouse: all motion
    (1004, false), // focus reports
    (1005, false), // mouse: UTF-8 encoding
    (1006, false), // mouse: SGR encoding
    (1007, false), // alternate scroll
    (1015, false), // mouse: urxvt encoding
    (1016, false), // mouse: SGR pixels
    (1047, false), // alternate screen
    (1049, false), // alternate screen, cursor saved
    (2004, false), // bracketed paste
];

/// Longest parameter list followed; longer sequences are not mode changes.
const MAX_PARAMS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    /// After `ESC [`: the first byte decides whether it is private (`?`).
    CsiStart,
    /// Inside `ESC [ ? …`.
    Private,
    /// Inside some other control sequence: skipped to its final byte.
    Other,
}

/// DEC private mode state, fed a byte stream in order (sequences may be
/// split across calls).
#[derive(Clone, Debug)]
pub struct Modes {
    on: [bool; TRACKED.len()],
    state: State,
    params: Vec<u8>,
}

impl Default for Modes {
    fn default() -> Self {
        Modes::new()
    }
}

impl Modes {
    pub fn new() -> Modes {
        let mut on = [false; TRACKED.len()];
        for (i, (_, default)) in TRACKED.iter().enumerate() {
            on[i] = *default;
        }
        Modes { on, state: State::Ground, params: Vec::new() }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    fn byte(&mut self, b: u8) {
        // ESC starts over anywhere; CAN and SUB cancel a sequence.
        match b {
            0x1b => {
                self.state = State::Esc;
                return;
            }
            0x18 | 0x1a => {
                self.state = State::Ground;
                return;
            }
            _ => {}
        }
        self.state = match self.state {
            State::Ground => State::Ground,
            State::Esc if b == b'[' => State::CsiStart,
            State::Esc => State::Ground,
            State::CsiStart if b == b'?' => {
                self.params.clear();
                State::Private
            }
            State::CsiStart | State::Other => {
                if (0x40..=0x7e).contains(&b) {
                    State::Ground
                } else {
                    State::Other
                }
            }
            State::Private => match b {
                b'0'..=b'9' | b';' if self.params.len() < MAX_PARAMS => {
                    self.params.push(b);
                    State::Private
                }
                b'h' | b'l' => {
                    self.set(b == b'h');
                    State::Ground
                }
                0x40..=0x7e => State::Ground,
                _ => State::Other,
            },
        };
    }

    fn set(&mut self, on: bool) {
        for p in self.params.split(|&c| c == b';') {
            let Some(mode) = std::str::from_utf8(p).ok().and_then(|s| s.parse::<u16>().ok()) else {
                continue;
            };
            if let Some(i) = TRACKED.iter().position(|(m, _)| *m == mode) {
                self.on[i] = on;
            }
        }
    }

    /// Sequences that bring a terminal in its power-on state to this state
    /// (empty when nothing differs).
    pub fn preamble(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, (mode, default)) in TRACKED.iter().enumerate() {
            if self.on[i] != *default {
                let end = if self.on[i] { 'h' } else { 'l' };
                out.extend_from_slice(format!("\x1b[?{mode}{end}").as_bytes());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_set_nothing_to_restore() {
        let mut m = Modes::new();
        m.feed(b"plain text \x1b[1;31mred\x1b[0m \x1b[2J\x1b[H");
        assert!(m.preamble().is_empty());
    }

    #[test]
    fn modes_follow_the_stream() {
        let mut m = Modes::new();
        m.feed(b"\x1b[?2004h\x1b[?25l\x1b[?1000;1006h\x1b[?9999h");
        assert_eq!(m.preamble(), b"\x1b[?25l\x1b[?1000h\x1b[?1006h\x1b[?2004h");
        m.feed(b"\x1b[?1000;1006l\x1b[?25h");
        assert_eq!(m.preamble(), b"\x1b[?2004h");
        m.feed(b"\x1b[?2004l");
        assert!(m.preamble().is_empty());
    }

    #[test]
    fn sequences_split_across_chunks() {
        let mut m = Modes::new();
        for b in b"x\x1b[?20" {
            m.feed(&[*b]);
        }
        m.feed(b"04");
        m.feed(b"h");
        assert_eq!(m.preamble(), b"\x1b[?2004h");
    }

    #[test]
    fn other_sequences_are_not_mode_changes() {
        let mut m = Modes::new();
        // Not private, a cancelled one, an ESC restarting one, a DCS-ish string.
        m.feed(b"\x1b[2004h\x1b[?2004\x18h\x1b[?20\x1b[0m04h\x1b[>1u\x1b]0;t?2004h\x07");
        assert!(m.preamble().is_empty());
        // Overlong parameter lists are dropped.
        let mut long = b"\x1b[?".to_vec();
        long.extend(std::iter::repeat_n(b';', 100));
        long.extend_from_slice(b"2004h");
        m.feed(&long);
        assert!(m.preamble().is_empty());
    }
}
