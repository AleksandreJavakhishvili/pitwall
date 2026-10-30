//! Local dev-server addresses in an agent's output (docs/spec/terminals.md
//! "Open preview"): `http://localhost:3000`, `127.0.0.1:5173/`, `0.0.0.0`,
//! `[::1]`, https too, and a bare `localhost:3000`. Read incrementally as
//! output comes: escape sequences are dropped on the way (a colour change
//! inside the URL, as Vite bolds its port, joins up; a cursor move splits
//! words), each finished line is matched once, and the unfinished one only
//! for a URL already followed by something.

use std::sync::OnceLock;

use regex::Regex;

/// Addresses kept, most recent first.
pub const MAX_URLS: usize = 4;
/// A line longer than this is matched and dropped (a TUI redrawing without
/// newlines); a URL across the cut is lost.
const LINE_CAP: usize = 2048;

/// What the parser is in the middle of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Text,
    /// After ESC.
    Start,
    /// `ESC [` … final byte.
    Csi,
    /// `ESC ]`, `ESC P` … BEL or ST: dropped whole.
    Str,
    /// ESC inside a string (the start of ST).
    StrEsc,
    /// `ESC (` etc.: one more byte.
    Charset,
}

#[derive(Debug, Default)]
pub struct PreviewScan {
    line: String,
    /// Bytes of an unfinished UTF-8 character.
    utf8: Vec<u8>,
    esc: Mode,
    /// Bytes of the CSI being read (a runaway one is given up).
    csi_len: usize,
    urls: Vec<String>,
    /// Addresses printed so far, each counted once per line.
    seen: u32,
    /// The addresses counted in the line being read.
    line_seen: Vec<String>,
}

impl PreviewScan {
    /// The addresses seen so far, most recent first.
    pub fn urls(&self) -> &[String] {
        &self.urls
    }

    /// How many times an address was printed (once per line each): a dev
    /// server printing its address again (a restart) counts, even when the
    /// address list stays the same. The preview reloads on it.
    pub fn seen(&self) -> u32 {
        self.seen
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        for &b in chunk {
            match self.esc {
                Mode::Text => match b {
                    0x1b => self.esc = Mode::Start,
                    b'\n' | b'\r' => self.end_line(),
                    b'\t' => self.push_char(' '),
                    0x00..=0x1f | 0x7f => {}
                    0x20..=0x7e => {
                        self.utf8.clear();
                        self.push_char(b as char);
                    }
                    _ => {
                        self.utf8.push(b);
                        match std::str::from_utf8(&self.utf8) {
                            Ok(s) => {
                                let c = s.chars().next().unwrap_or(' ');
                                self.utf8.clear();
                                self.push_char(c);
                            }
                            Err(e) if e.error_len().is_some() || self.utf8.len() >= 4 => self.utf8.clear(),
                            Err(_) => {}
                        }
                    }
                },
                Mode::Start => {
                    self.esc = match b {
                        b'[' => {
                            self.csi_len = 0;
                            Mode::Csi
                        }
                        b']' | b'P' | b'X' | b'^' | b'_' => Mode::Str,
                        b'(' | b')' | b'*' | b'+' => Mode::Charset,
                        _ => Mode::Text,
                    }
                }
                Mode::Csi => {
                    self.csi_len += 1;
                    if (0x40..=0x7e).contains(&b) {
                        self.esc = Mode::Text;
                        // Colours vanish; anything else (cursor moves,
                        // erases) parts what is on either side.
                        if b != b'm' {
                            self.push_char(' ');
                        }
                    } else if self.csi_len > 64 {
                        self.esc = Mode::Text;
                    }
                }
                Mode::Str => match b {
                    0x07 => self.esc = Mode::Text,
                    0x1b => self.esc = Mode::StrEsc,
                    _ => {}
                },
                Mode::StrEsc => self.esc = if b == b'\\' { Mode::Text } else { Mode::Str },
                Mode::Charset => self.esc = Mode::Text,
            }
        }
        // The unfinished line: only URLs already followed by something
        // (`localhost:30` may be the start of `:3000`).
        if !self.line.is_empty() {
            for url in find(&self.line, false) {
                self.note(url);
            }
        }
    }

    fn push_char(&mut self, c: char) {
        if c == ' ' && self.line.ends_with(' ') {
            return;
        }
        self.line.push(c);
        if self.line.len() >= LINE_CAP {
            self.end_line();
        }
    }

    fn end_line(&mut self) {
        if self.line.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.line);
        for url in find(&line, true) {
            self.note(url);
        }
        self.line_seen.clear();
    }

    fn note(&mut self, url: String) {
        // The unfinished line is matched again on every read: count once.
        if !self.line_seen.contains(&url) {
            self.line_seen.push(url.clone());
            self.seen = self.seen.wrapping_add(1);
        }
        if self.urls.first() == Some(&url) {
            return;
        }
        self.urls.retain(|u| *u != url);
        self.urls.insert(0, url);
        self.urls.truncate(MAX_URLS);
    }
}

fn pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(https?://)?(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1?\]):(\d{1,5})(/[^\s<>`]*)?")
            .expect("preview pattern")
    })
}

/// The dev-server addresses in `line`, in order, as URLs to open. `whole`:
/// the line is finished (a match may end it).
pub fn find(line: &str, whole: bool) -> Vec<String> {
    let mut out = Vec::new();
    for c in pattern().captures_iter(line) {
        let m = c.get(0).expect("match");
        let scheme = c.get(1).map(|s| s.as_str().to_ascii_lowercase());
        let before = line[..m.start()].chars().next_back();
        // A bare address only standing on its own: not `user@localhost:5432`,
        // `redis://localhost:6379` or `db.localhost:1`.
        let free = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || "@/.:-_\\".contains(c));
        if scheme.is_none() && !free(before) {
            continue;
        }
        if scheme.is_some() && before.is_some_and(|c| c.is_alphanumeric()) {
            continue;
        }
        // The port ends there (`:3000x` is something else).
        let after_port = line[c.get(3).expect("port").end()..].chars().next();
        if c.get(4).is_none() && after_port.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let Ok(port) = c[3].parse::<u16>() else { continue };
        if port == 0 {
            continue;
        }
        let path = c.get(4).map_or("", |p| p.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '\'', '"']));
        if !whole && m.end() == line.len() {
            continue;
        }
        let host = c[2].to_ascii_lowercase();
        // Listening on every interface: open it here.
        let host = if host == "0.0.0.0" { "localhost" } else { &host };
        out.push(format!("{}{host}:{port}{path}", scheme.as_deref().unwrap_or("http://")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> Option<String> {
        find(line, true).pop()
    }

    #[test]
    fn finds_dev_server_addresses() {
        assert_eq!(one("  ➜  Local:   http://localhost:5173/").as_deref(), Some("http://localhost:5173/"));
        assert_eq!(one("ready - started server on http://127.0.0.1:3000").as_deref(), Some("http://127.0.0.1:3000"));
        assert_eq!(one("Listening on http://0.0.0.0:8080.").as_deref(), Some("http://localhost:8080"));
        assert_eq!(one("serving at http://[::1]:4000/app, ok").as_deref(), Some("http://[::1]:4000/app"));
        assert_eq!(one("(https://localhost:8443)").as_deref(), Some("https://localhost:8443"));
        assert_eq!(one("Server running at localhost:3000").as_deref(), Some("http://localhost:3000"));
        assert_eq!(one("HTTP://LOCALHOST:3000").as_deref(), Some("http://localhost:3000"));
    }

    #[test]
    fn leaves_other_addresses_alone() {
        for line in [
            "postgres://user@localhost:5432/db",
            "redis://localhost:6379",
            "ws://localhost:3000",
            "http://localhost/",
            "http://example.com:3000",
            "localhost:3000x",
            "http://localhost:0",
            "http://localhost:99999",
            "app.localhost:3000",
            "mylocalhost:3000",
        ] {
            assert_eq!(one(line), None, "{line}");
        }
    }

    #[test]
    fn reads_across_chunks_and_escapes() {
        let mut s = PreviewScan::default();
        // Vite: a bold port, a colour, the line in two reads.
        s.feed(b"  \x1b[32m\xe2\x9e\x9c\x1b[39m  Local:   \x1b[36mhttp://localhost:\x1b[1m51");
        assert!(s.urls().is_empty());
        s.feed(b"73\x1b[22m/\x1b[39m\r\n");
        assert_eq!(s.urls(), ["http://localhost:5173/"]);
        // A title and a cursor move: neither joins the address to text.
        s.feed(b"\x1b]0;http://localhost:1111\x07\x1b[2;1Hlocalhost:4000\x1b[3;1Hnext\r\n");
        assert_eq!(s.urls(), ["http://localhost:4000", "http://localhost:5173/"]);
        // Seen again: most recent first, once.
        s.feed(b"http://localhost:5173/\n");
        assert_eq!(s.urls(), ["http://localhost:5173/", "http://localhost:4000"]);
    }

    #[test]
    fn an_unfinished_line_counts_once_followed() {
        let mut s = PreviewScan::default();
        s.feed(b"open http://localhost:3000");
        assert!(s.urls().is_empty());
        s.feed(b" now");
        assert_eq!(s.urls(), ["http://localhost:3000"]);
    }

    #[test]
    fn counts_each_printing_once() {
        let mut s = PreviewScan::default();
        s.feed(b"open http://localhost:3000");
        s.feed(b" now");
        s.feed(b" really\n");
        assert_eq!(s.seen(), 1, "one line, read three times");
        // A restart prints it again: the list stays, the count moves.
        s.feed(b"  Local: http://localhost:3000\n");
        assert_eq!(s.urls(), ["http://localhost:3000"]);
        assert_eq!(s.seen(), 2);
        s.feed(b"compiled\n");
        assert_eq!(s.seen(), 2);
    }

    #[test]
    fn keeps_a_few() {
        let mut s = PreviewScan::default();
        for p in 3000..3010 {
            s.feed(format!("http://localhost:{p}\n").as_bytes());
        }
        assert_eq!(s.urls().len(), MAX_URLS);
        assert_eq!(s.urls()[0], "http://localhost:3009");
    }
}
