//! An adapter's log for its author (docs/spec/extensions.md, "Dev mode"):
//! the last [`KEEP`] protocol lines both ways and its stderr, each cut to
//! [`MAX_TEXT`] characters, with secrets taken out before anything is
//! kept ([`redact`]): `hello`'s secrets, the values of the extension's
//! secret settings wherever they show up, and anything shaped like a key.
//! Kept in memory only, never written to a file.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde_json::Value;

/// How many lines are kept.
pub const KEEP: usize = 500;
/// The longest line kept (cut with "…").
pub const MAX_TEXT: usize = 2000;

/// Which way a line went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Pitwall to the adapter (its stdin).
    ToAdapter,
    /// The adapter to Pitwall (its stdout).
    FromAdapter,
    /// What it printed on stderr.
    Stderr,
    /// Pitwall's own word: started, stopped and why.
    Note,
}

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub dir: Dir,
    pub at: SystemTime,
    pub text: String,
}

/// The log of one adapter.
#[derive(Debug, Default)]
pub struct Log {
    lines: VecDeque<Line>,
    /// The secrets to take out (the values of its secret settings).
    secrets: Vec<String>,
}

/// A log shared by the threads that read and write an adapter.
pub type Shared = Arc<Mutex<Log>>;

impl Log {
    /// The values to take out of every line from now on.
    pub fn set_secrets(&mut self, secrets: Vec<String>) {
        self.secrets = secrets.into_iter().filter(|s| !s.is_empty()).collect();
    }

    /// Keep `text` (redacted, cleaned, cut), dropping the oldest beyond
    /// [`KEEP`].
    pub fn push(&mut self, dir: Dir, text: &str) {
        let text = super::protocol::clean(&redact(text, &self.secrets), MAX_TEXT, false);
        if text.is_empty() {
            return;
        }
        self.lines.push_back(Line { dir, at: SystemTime::now(), text });
        while self.lines.len() > KEEP {
            self.lines.pop_front();
        }
    }

    /// Its lines, oldest first.
    pub fn lines(&self) -> Vec<Line> {
        self.lines.iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Everything forgotten (the extension was turned off).
    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

/// `line` with secrets taken out: a `hello`'s `secrets` values, each of
/// `secrets` wherever it shows (as it is and as JSON writes it), and
/// anything shaped like a key or token ([`looks_like_key`]); each becomes
/// `[secret]`.
pub fn redact(line: &str, secrets: &[String]) -> String {
    let mut out = line.to_string();
    // A hello: its secrets' values, whatever they are.
    if line.contains("\"hello\"") {
        if let Ok(mut v) = serde_json::from_str::<Value>(line) {
            if v.get("type").and_then(Value::as_str) == Some("hello") {
                if let Some(Value::Object(s)) = v.get_mut("secrets") {
                    for val in s.values_mut() {
                        *val = Value::String("[secret]".into());
                    }
                    out = v.to_string();
                }
            }
        }
    }
    for s in secrets.iter().filter(|s| !s.is_empty()) {
        out = out.replace(s.as_str(), "[secret]");
        let escaped = serde_json::to_string(s).unwrap_or_default();
        let escaped = escaped.trim_matches('"');
        if !escaped.is_empty() && escaped != s {
            out = out.replace(escaped, "[secret]");
        }
    }
    key_pattern().replace_all(&out, |c: &regex::Captures| if looks_like_key(&c[0]) { "[secret]".to_string() } else { c[0].to_string() }).into_owned()
}

/// Shapes keys and tokens commonly take: `<prefix>_<at least 8 key
/// characters>`, `sk-…`/`pk-…`/`rk-…`, and what follows `Bearer `.
fn key_pattern() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\b[a-z][a-z0-9]{1,19}_[A-Za-z0-9_-]{8,}|\b[spr]k-[A-Za-z0-9_-]{16,}|\bBearer [A-Za-z0-9._~+/=-]{8,}").expect("a valid pattern"))
}

/// A match of [`key_pattern`] that is a key, not a snake_case word: its
/// part after the prefix has a digit or a capital letter (a bearer token
/// always is).
pub fn looks_like_key(m: &str) -> bool {
    if m.starts_with("Bearer ") {
        return true;
    }
    let tail = m.split_once(['_', '-']).map_or(m, |(_, t)| t);
    tail.chars().any(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_s_secrets_and_known_values_are_taken_out() {
        let hello = r#"{"type":"hello","protocol":1,"settings":{"server":"https://x.example"},"secrets":{"apiKey":"hunter2"}}"#;
        let r = redact(hello, &[]);
        assert!(!r.contains("hunter2") && r.contains("[secret]") && r.contains("https://x.example"), "{r}");
        // Anywhere else too, once known, as it is and as JSON escapes it.
        let secrets = vec!["hunter2".to_string(), "a\"b".to_string()];
        assert_eq!(redact("error: key hunter2 refused", &secrets), "error: key [secret] refused");
        assert_eq!(redact(r#"{"echo":"a\"b"}"#, &secrets), r#"{"echo":"[secret]"}"#);
    }

    #[test]
    fn things_shaped_like_keys_are_taken_out() {
        assert_eq!(redact("using tool_AbCdEf123456 now", &[]), "using [secret] now");
        assert_eq!(redact("sk-abcdefghijklmnop1234", &[]), "[secret]");
        assert_eq!(redact("Authorization: Bearer abc.def.ghi", &[]), "Authorization: [secret]");
        // Plain snake_case words and ids stay.
        for keep in ["the session_cache_store is full", r#"{"type":"threads","openAsks":1}"#, "thread demo/1", "2026-11-23T10:00:00Z"] {
            assert_eq!(redact(keep, &[]), keep);
        }
    }

    #[test]
    fn the_last_lines_are_kept_cut_and_cleaned() {
        let mut log = Log::default();
        log.set_secrets(vec!["s3cret".into()]);
        for i in 0..(KEEP + 20) {
            log.push(Dir::FromAdapter, &format!("line {i} s3cret"));
        }
        assert_eq!(log.len(), KEEP);
        let lines = log.lines();
        assert_eq!(lines[0].text, "line 20 [secret]");
        log.push(Dir::Stderr, &"x".repeat(MAX_TEXT + 10));
        assert_eq!(log.lines().last().unwrap().text.chars().count(), MAX_TEXT);
        log.push(Dir::Stderr, "a\u{202E}b\u{7}");
        assert_eq!(log.lines().last().unwrap().text, "ab");
        log.push(Dir::Stderr, "   ");
        assert_eq!(log.lines().last().unwrap().text, "ab", "an empty line isn't kept");
        log.clear();
        assert!(log.is_empty());
    }
}
