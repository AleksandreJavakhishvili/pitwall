//! How much an account has used, read from what its agent tool records on
//! disk (no network, no credentials): Claude Code's conversation logs
//! (`projects/**/*.jsonl`: each reply's token counts), Codex's session logs
//! (`sessions/YYYY/MM/DD/rollout-*.jsonl`: token counts, and the plan's
//! limits as Codex last saw them). Other tools record neither, so they have
//! no usage here.
//!
//! Files are parsed once per (size, mtime) and kept in a [`UsageCache`]:
//! logs only grow, so a refresh reads only what changed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::accounts::{dir_of, Account};
use crate::kind::AgentKind;

const HOUR: u64 = 3_600_000;
/// The longest window shown: a week.
const WEEK: u64 = 7 * 24 * HOUR;

/// Tokens of some replies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    /// Fresh input (not from the cache).
    pub input: u64,
    pub output: u64,
    /// Input read from the prompt cache.
    pub cache_read: u64,
    /// Input written to the prompt cache.
    pub cache_write: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }
}

/// One of a plan's limits, as the tool last saw it (Codex: its 5-hour and
/// weekly windows).
#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    pub used_percent: f64,
    /// How long the window is.
    pub window_minutes: u64,
    /// When it starts over (ms since the epoch).
    pub resets_at: Option<u64>,
}

/// An account's usage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub last_5h: Tokens,
    pub last_24h: Tokens,
    pub last_7d: Tokens,
    /// The plan's limits, where the tool records them.
    pub limits: Vec<Limit>,
    /// The plan's name ("pro"), where recorded.
    pub plan: Option<String>,
    /// When the limits were read by the tool (ms since the epoch).
    pub limits_at: Option<u64>,
    /// The newest reply counted (ms since the epoch).
    pub last_used: Option<u64>,
}

/// Whether Pitwall can read `kind`'s usage.
pub fn readable(kind: &str) -> bool {
    matches!(kind, "claude" | "codex")
}

/// One reply's tokens at a time.
#[derive(Debug, Clone)]
struct Entry {
    at: u64,
    /// Claude Code logs a reply once per content block: the same message
    /// and request id, counted once.
    key: Option<String>,
    tokens: Tokens,
}

/// A Codex session's view of the plan's limits.
#[derive(Debug, Clone)]
struct Seen {
    at: u64,
    limits: Vec<Limit>,
    plan: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Parsed {
    entries: Vec<Entry>,
    seen: Option<Seen>,
}

/// Parsed log files by path, with the (size, mtime) they were parsed at.
#[derive(Debug, Default)]
pub struct UsageCache {
    files: HashMap<PathBuf, (u64, SystemTime, Parsed)>,
}

impl UsageCache {
    pub fn new() -> UsageCache {
        UsageCache::default()
    }

    /// `kind`'s usage on `account` (`None`: the user's own login) at `now`
    /// (ms since the epoch). `None`: a tool whose usage Pitwall can't read.
    pub fn usage(&mut self, kind: &AgentKind, account: Option<&Account>, now: u64) -> Option<Usage> {
        let dir = dir_of(kind, account)?;
        let files = match kind.id.as_str() {
            "claude" => logs(&dir.join("projects"), now, 3),
            "codex" => logs(&dir.join("sessions"), now, 4),
            _ => return None,
        };
        let codex = kind.id == "codex";
        let mut u = Usage::default();
        let mut keys = HashSet::new();
        let mut seen: Option<Seen> = None;
        for (path, len, mtime) in files {
            let fresh = self.files.get(&path).is_some_and(|(l, m, _)| *l == len && *m == mtime);
            if !fresh {
                let p = std::fs::read_to_string(&path).map(|t| if codex { parse_codex(&t) } else { parse_claude(&t) }).unwrap_or_default();
                self.files.insert(path.clone(), (len, mtime, p));
            }
            let p = &self.files[&path].2;
            for e in &p.entries {
                if e.key.as_ref().is_some_and(|k| !keys.insert(k.clone())) {
                    continue;
                }
                let age = now.saturating_sub(e.at);
                if age <= WEEK {
                    u.last_7d.add(&e.tokens);
                    u.last_used = u.last_used.max(Some(e.at));
                }
                if age <= 24 * HOUR {
                    u.last_24h.add(&e.tokens);
                }
                if age <= 5 * HOUR {
                    u.last_5h.add(&e.tokens);
                }
            }
            if let Some(s) = &p.seen {
                if seen.as_ref().is_none_or(|b| s.at > b.at) {
                    seen = Some(s.clone());
                }
            }
        }
        if let Some(s) = seen {
            u.limits = s.limits;
            u.plan = s.plan;
            u.limits_at = Some(s.at);
        }
        Some(u)
    }
}

/// `.jsonl` files under `root` (at most `depth` folders down) changed in
/// the last week, with their size and mtime.
fn logs(root: &Path, now: u64, depth: usize) -> Vec<(PathBuf, u64, SystemTime)> {
    let since = UNIX_EPOCH + Duration::from_millis(now.saturating_sub(WEEK));
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((dir, d)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            let path = e.path();
            if meta.is_dir() {
                if d < depth {
                    stack.push((path, d + 1));
                }
            } else if path.extension().is_some_and(|x| x == "jsonl") {
                let Ok(mtime) = meta.modified() else { continue };
                if mtime >= since {
                    out.push((path, meta.len(), mtime));
                }
            }
        }
    }
    out
}

fn num(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

/// Claude Code's log: each assistant reply's `message.usage`.
fn parse_claude(text: &str) -> Parsed {
    let mut p = Parsed::default();
    for line in text.lines().filter(|l| l.contains("\"usage\"")) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(at) = v.get("timestamp").and_then(Value::as_str).and_then(iso_ms) else { continue };
        let Some(msg) = v.get("message") else { continue };
        let Some(usage) = msg.get("usage") else { continue };
        let key = match (msg.get("id").and_then(Value::as_str), v.get("requestId").and_then(Value::as_str)) {
            (Some(m), Some(r)) => Some(format!("{m}:{r}")),
            (Some(m), None) => Some(m.to_string()),
            _ => None,
        };
        p.entries.push(Entry {
            at,
            key,
            tokens: Tokens {
                input: num(usage, "input_tokens"),
                output: num(usage, "output_tokens"),
                cache_read: num(usage, "cache_read_input_tokens"),
                cache_write: num(usage, "cache_creation_input_tokens"),
            },
        });
    }
    p
}

/// Codex's log: `token_count` events (each turn's `last_token_usage`, and
/// the plan's `rate_limits` then).
fn parse_codex(text: &str) -> Parsed {
    let mut p = Parsed::default();
    for line in text.lines().filter(|l| l.contains("\"token_count\"")) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(payload) = v.get("payload") else { continue };
        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        let Some(at) = v.get("timestamp").and_then(Value::as_str).and_then(iso_ms) else { continue };
        if let Some(last) = payload.get("info").and_then(|i| i.get("last_token_usage")) {
            // Codex counts cached input inside input_tokens.
            let (input, cached) = (num(last, "input_tokens"), num(last, "cached_input_tokens"));
            p.entries.push(Entry {
                at,
                key: None,
                tokens: Tokens {
                    input: input.saturating_sub(cached),
                    output: num(last, "output_tokens"),
                    cache_read: cached,
                    cache_write: num(last, "cache_write_input_tokens"),
                },
            });
        }
        if let Some(rl) = payload.get("rate_limits").filter(|r| r.is_object()) {
            let limits = ["primary", "secondary"]
                .iter()
                .filter_map(|k| rl.get(*k).filter(|w| w.is_object()))
                .map(|w| Limit {
                    used_percent: w.get("used_percent").and_then(Value::as_f64).unwrap_or(0.),
                    window_minutes: num(w, "window_minutes"),
                    resets_at: w.get("resets_at").and_then(Value::as_u64).map(|s| s * 1000),
                })
                .collect();
            let plan = rl.get("plan_type").and_then(Value::as_str).map(String::from);
            if p.seen.as_ref().is_none_or(|s| at >= s.at) {
                p.seen = Some(Seen { at, limits, plan });
            }
        }
    }
    p
}

/// `2026-10-23T20:36:23.668Z` (or with `+hh:mm`) in ms since the epoch.
pub fn iso_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, sec) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let mut rest = &s[19..];
    let mut ms = 0i64;
    if let Some(f) = rest.strip_prefix('.') {
        let digits: String = f.chars().take_while(|c| c.is_ascii_digit()).collect();
        ms = format!("{digits:0<3}")[..3].parse().ok()?;
        rest = &f[digits.len()..];
    }
    let offset = match rest {
        "" | "Z" => 0,
        o if o.len() == 6 && (o.starts_with('+') || o.starts_with('-')) => {
            let m = o[1..3].parse::<i64>().ok()? * 60 + o[4..6].parse::<i64>().ok()?;
            if o.starts_with('-') { -m } else { m }
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let t = ((days * 24 + h) * 60 + mi - offset) * 60 + sec;
    u64::try_from(t * 1000 + ms).ok()
}

/// "1.2M", "34k", "512".
pub fn short(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 10_000 => format!("{}k", n / 1000),
        n if n >= 1_000 => format!("{:.1}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn iso_times() {
        assert_eq!(iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_ms("2026-10-23T20:36:23.668Z"), Some(1_792_787_783_668));
        assert_eq!(iso_ms("2026-10-23T22:36:23.668+02:00"), Some(1_792_787_783_668));
        assert_eq!(iso_ms("2026-10-23T20:36:23Z"), Some(1_792_787_783_000));
        assert_eq!(iso_ms("2026-10-23"), None);
    }

    #[test]
    fn short_numbers() {
        assert_eq!(short(512), "512");
        assert_eq!(short(1_500), "1.5k");
        assert_eq!(short(34_000), "34k");
        assert_eq!(short(1_234_567), "1.2M");
    }

    fn kind(id: &str) -> AgentKind {
        crate::kind::load_kinds(Path::new("/nonexistent/pitwall-agents")).into_iter().find(|k| k.id == id).unwrap()
    }

    fn account(dir: &Path) -> Account {
        Account { id: "a".into(), kind: "claude".into(), name: "A".into(), dir: dir.to_string_lossy().into_owned() }
    }

    const NOW: u64 = 1_792_787_783_668; // 2026-10-23T20:36:23.668Z

    #[test]
    fn claude_counts_each_reply_once_by_window() {
        let tmp = TempDir::new("usage");
        let dir = tmp.path().join("projects/-p");
        std::fs::create_dir_all(dir.join("s1/subagents")).unwrap();
        let reply = |at: &str, id: &str, out: u64| {
            format!(
                r#"{{"type":"assistant","timestamp":"{at}","requestId":"r{id}","message":{{"id":"m{id}","usage":{{"input_tokens":10,"output_tokens":{out},"cache_read_input_tokens":100,"cache_creation_input_tokens":5}}}}}}"#
            )
        };
        let log = [
            reply("2026-10-23T20:00:00Z", "1", 1),
            // The same reply again (another content block): counted once.
            reply("2026-10-23T20:00:01Z", "1", 1),
            reply("2026-10-23T10:00:00Z", "2", 2),
            reply("2026-10-20T10:00:00Z", "3", 3),
            r#"{"type":"user","message":{"content":"usage"}}"#.into(),
        ]
        .join("\n");
        std::fs::write(dir.join("s1.jsonl"), log).unwrap();
        std::fs::write(dir.join("s1/subagents/a.jsonl"), reply("2026-10-23T19:00:00Z", "4", 4)).unwrap();
        let mut c = UsageCache::new();
        let u = c.usage(&kind("claude"), Some(&account(tmp.path())), NOW).unwrap();
        assert_eq!(u.last_5h.output, 1 + 4);
        assert_eq!(u.last_24h.output, 1 + 2 + 4);
        assert_eq!(u.last_7d.output, 1 + 2 + 3 + 4);
        assert_eq!(u.last_7d.total(), 4 * 115 + 10);
        assert!(u.limits.is_empty());
        assert_eq!(u.last_used, iso_ms("2026-10-23T20:00:00Z"));
        // Cached: the same again.
        assert_eq!(c.usage(&kind("claude"), Some(&account(tmp.path())), NOW).unwrap(), u);
    }

    #[test]
    fn codex_reads_tokens_and_the_newest_limits() {
        let tmp = TempDir::new("usage");
        let dir = tmp.path().join("sessions/2026/10/23");
        std::fs::create_dir_all(&dir).unwrap();
        let ev = |at: &str, used: f64, out: u64| {
            format!(
                r#"{{"timestamp":"{at}","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"input_tokens":100,"cached_input_tokens":60,"output_tokens":{out}}}}},"rate_limits":{{"primary":{{"used_percent":{used},"window_minutes":300,"resets_at":1793230000}},"secondary":{{"used_percent":40.0,"window_minutes":10080,"resets_at":1793500000}},"plan_type":"pro"}}}}}}"#
            )
        };
        let noise = r#"{"timestamp":"2026-10-23T20:00:00Z","type":"response_item","payload":{"type":"message","text":"token_count"}}"#;
        std::fs::write(dir.join("rollout-a.jsonl"), [ev("2026-10-23T19:00:00Z", 10.0, 5), noise.into()].join("\n")).unwrap();
        std::fs::write(dir.join("rollout-b.jsonl"), ev("2026-10-23T20:30:00Z", 25.5, 7)).unwrap();
        let mut c = UsageCache::new();
        let u = c.usage(&kind("codex"), Some(&account(tmp.path())), NOW).unwrap();
        assert_eq!(u.last_5h.output, 12);
        assert_eq!(u.last_5h.input, 80, "cached input apart");
        assert_eq!(u.last_5h.cache_read, 120);
        assert_eq!(u.plan.as_deref(), Some("pro"));
        assert_eq!(u.limits.len(), 2);
        assert_eq!(u.limits[0].used_percent, 25.5, "the newest");
        assert_eq!(u.limits[0].window_minutes, 300);
        assert_eq!(u.limits[1].resets_at, Some(1_793_500_000_000));
    }

    #[test]
    fn other_tools_have_none() {
        let mut c = UsageCache::new();
        assert!(c.usage(&kind("gemini"), None, NOW).is_none());
        assert!(!readable("gemini") && readable("claude") && readable("codex"));
    }
}
