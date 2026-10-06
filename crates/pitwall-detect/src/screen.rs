//! Headless terminal screen, one per agent, fed from the PTY reader thread.
//!
//! Built on the `vt100` crate, which handles cursor movement, scroll regions,
//! the alternate screen, wide characters and resizing. Window titles
//! (OSC 0 / OSC 2) come from vt100's callbacks; the underlying `vte` parser
//! keeps its state between `feed` calls, so sequences split across PTY reads
//! are reassembled correctly.

/// Longest title we keep; anything longer is truncated (defensive bound).
const MAX_TITLE_CHARS: usize = 512;

#[derive(Default)]
struct TitleTracker {
    title: Option<String>,
}

impl TitleTracker {
    fn set(&mut self, raw: &[u8]) {
        let text = String::from_utf8_lossy(raw);
        let cleaned: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_TITLE_CHARS)
            .collect();
        self.title = Some(cleaned);
    }
}

impl vt100::Callbacks for TitleTracker {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.set(title);
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        // vte splits OSC payloads on ';', so a title containing ';' arrives
        // here as more than two params. Re-join the tail for OSC 0 / OSC 2.
        if let [kind, rest @ ..] = params {
            if (*kind == b"0" || *kind == b"2") && !rest.is_empty() {
                self.set(&rest.join(&b';'));
            }
        }
    }
}

pub struct Screen {
    parser: vt100::Parser<TitleTracker>,
}

impl Screen {
    pub fn new(rows: u16, cols: u16) -> Self {
        Screen {
            parser: vt100::Parser::new_with_callbacks(
                rows.max(1),
                cols.max(1),
                0,
                TitleTracker::default(),
            ),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
    }

    /// Visible screen as plain text, one line per row, trailing spaces trimmed.
    pub fn text(&self) -> String {
        let screen = self.parser.screen();
        let (_, cols) = screen.size();
        let rows: Vec<String> = screen
            .rows(0, cols)
            .map(|row| row.trim_end().to_string())
            .collect();
        rows.join("\n")
    }

    /// Last window title set via OSC 0/2, if any.
    pub fn title(&self) -> Option<String> {
        self.parser.callbacks().title.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &Screen) -> Vec<String> {
        s.text().lines().map(str::to_string).collect()
    }

    #[test]
    fn plain_text_and_trailing_spaces() {
        let mut s = Screen::new(4, 20);
        s.feed(b"hello   \r\nworld");
        let l = lines(&s);
        assert_eq!(l[0], "hello");
        assert_eq!(l[1], "world");
        assert_eq!(s.text().split('\n').count(), 4);
    }

    #[test]
    fn cursor_moves_and_overwrite() {
        let mut s = Screen::new(5, 20);
        s.feed(b"\x1b[3;5Hmid\x1b[1;1Htop\x1b[3;5HMID");
        let l = lines(&s);
        assert_eq!(l[0], "top");
        assert_eq!(l[2], "    MID");
        // Erase line.
        s.feed(b"\x1b[1;1H\x1b[2K");
        assert_eq!(lines(&s)[0], "");
    }

    #[test]
    fn alternate_screen_round_trip() {
        let mut s = Screen::new(4, 20);
        s.feed(b"shell prompt $");
        s.feed(b"\x1b[?1049h\x1b[H\x1b[2Jfull screen app");
        assert!(s.text().contains("full screen app"));
        assert!(!s.text().contains("shell prompt"));
        s.feed(b"\x1b[?1049l");
        assert!(s.text().contains("shell prompt"));
        assert!(!s.text().contains("full screen app"));
    }

    #[test]
    fn wide_chars() {
        let mut s = Screen::new(2, 20);
        s.feed("日本語 ok ❯ ✻".as_bytes());
        assert_eq!(lines(&s)[0], "日本語 ok ❯ ✻");
    }

    #[test]
    fn utf8_split_across_feeds() {
        let mut s = Screen::new(2, 20);
        let bytes = "❯ hi".as_bytes();
        s.feed(&bytes[..1]);
        s.feed(&bytes[1..]);
        assert_eq!(lines(&s)[0], "❯ hi");
    }

    #[test]
    fn resize_keeps_working() {
        let mut s = Screen::new(3, 10);
        s.feed(b"abcdefghij");
        s.resize(5, 40);
        s.feed(b"\x1b[5;1Hbottom");
        let l = lines(&s);
        assert_eq!(l.len(), 5);
        assert_eq!(l[4], "bottom");
        s.resize(0, 0); // clamped, must not panic
        s.feed(b"x");
    }

    #[test]
    fn osc_titles() {
        let mut s = Screen::new(2, 20);
        assert_eq!(s.title(), None);
        s.feed(b"\x1b]0;first\x07");
        assert_eq!(s.title().as_deref(), Some("first"));
        s.feed(b"\x1b]2;second\x1b\\");
        assert_eq!(s.title().as_deref(), Some("second"));
        // OSC 1 (icon name) does not change the title.
        s.feed(b"\x1b]1;icon\x07");
        assert_eq!(s.title().as_deref(), Some("second"));
        // Titles containing ';' survive.
        s.feed(b"\x1b]0;a;b;c\x07");
        assert_eq!(s.title().as_deref(), Some("a;b;c"));
        // Title text is not drawn on screen.
        assert_eq!(s.text().trim(), "");
    }

    #[test]
    fn osc_title_split_across_feeds() {
        let mut s = Screen::new(2, 30);
        let seq = "\x1b]0;✳ Claude Code\x07after".as_bytes();
        for chunk in seq.chunks(1) {
            s.feed(chunk);
        }
        assert_eq!(s.title().as_deref(), Some("✳ Claude Code"));
        assert_eq!(lines(&s)[0], "after");

        // Split in the middle of the ST terminator.
        s.feed(b"\x1b]2;spin");
        s.feed(b"ner \x1b");
        s.feed(b"\\");
        assert_eq!(s.title().as_deref(), Some("spinner "));
    }

    #[test]
    fn garbage_does_not_panic() {
        let mut s = Screen::new(10, 30);
        let mut x: u32 = 0x1234_5678;
        let mut buf = Vec::new();
        for _ in 0..200_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let b = (x & 0xff) as u8;
            // Bias towards escape-heavy input.
            buf.push(if b.is_multiple_of(7) { 0x1b } else { b });
        }
        for chunk in buf.chunks(97) {
            s.feed(chunk);
        }
        s.resize(3, 5);
        s.feed(&buf[..5000]);
        let _ = s.text();
        let _ = s.title();
    }
}
