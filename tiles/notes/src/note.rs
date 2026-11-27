//! A note and what the list shows of it: its title (the first line), a
//! preview (the next line with text) and how long ago it changed. Pure, so
//! it is tested natively.

use serde::{Deserialize, Serialize};

/// The storage prefix of private notes. `_` stands for "no project" until
/// a tile learns its space's project (frd.md §7, G5).
pub const PREFIX: &str = "p/_/";

/// A private note, as the tile holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: u64,
    /// The text as last saved.
    pub text: String,
    /// When it was last saved, in milliseconds since the Unix epoch.
    pub updated: u64,
}

/// A note as kept in storage under [`key`].
#[derive(Debug, Serialize, Deserialize)]
pub struct Stored {
    pub text: String,
    #[serde(default)]
    pub updated: u64,
}

/// The storage key of note `id`.
pub fn key(id: u64) -> String {
    format!("{PREFIX}{id}")
}

/// The id in a storage key made by [`key`].
pub fn id_of(key: &str) -> Option<u64> {
    key.strip_prefix(PREFIX)?.parse().ok()
}

/// `text` with the task on line `line` (0-based) ticked (`on`) or not, as
/// a click on its box in Preview asks; `None` when that line is no task.
pub fn tick(text: &str, line: usize, on: bool) -> Option<String> {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let l = lines.get_mut(line)?;
    let indent = l.len() - l.trim_start().len();
    let rest = &l[indent..];
    let bullet = rest.chars().next().filter(|c| matches!(c, '-' | '*' | '+'))?;
    let after = rest[1..].trim_start();
    let gap = rest.len() - 1 - after.len();
    if !(after.starts_with("[ ]") || after.starts_with("[x]") || after.starts_with("[X]")) {
        return None;
    }
    *l = format!("{}{bullet}{}[{}]{}", &l[..indent], " ".repeat(gap.max(1)), if on { 'x' } else { ' ' }, &after[3..]);
    Some(lines.join("\n"))
}

/// The title: the first line with text, without Markdown's leading `#`s,
/// at most 120 characters. `None` for a note with no text yet.
pub fn title(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_start_matches('#').trim_start();
    let line = if line.is_empty() { text.lines().map(str::trim).find(|l| !l.is_empty())? } else { line };
    Some(cut(line, 120))
}

/// The title, or "New note" for a note with no text.
pub fn title_or_new(text: &str) -> String {
    title(text).unwrap_or_else(|| "New note".to_string())
}

/// The line after the title with text, at most 160 characters.
pub fn preview(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    lines.next()?;
    lines.next().map(|l| cut(l, 160))
}

/// Whether `text` matches the search `query` (already lower-case): every
/// word of it appears somewhere in the text.
pub fn matches(text: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let text = text.to_lowercase();
    query.split_whitespace().all(|w| text.contains(w))
}

/// How long ago `then` was, shortly: "now", "5 min", "3 h", "2 d", "6 w".
pub fn ago(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then) / 1000;
    match s {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{} min", s / 60),
        3600..=86_399 => format!("{} h", s / 3600),
        86_400..=604_799 => format!("{} d", s / 86_400),
        _ => format!("{} w", s / 604_800),
    }
}

/// `s` cut to `max` characters, with "…" when cut.
fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_string(),
    }
}

/// The text a draft opens with: `title` on the first line unless the text
/// already starts with it.
pub fn draft_text(title: Option<&str>, text: &str) -> String {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) if text.lines().next().map(str::trim) != Some(t) => format!("{t}\n{text}"),
        _ => text.to_string(),
    }
}

/// The note's sections under its Markdown headings (not counting a first
/// line that is the title): each heading's text and its section, the
/// heading included, up to the next heading of the same or a higher
/// level. Lines in code fences are no headings. What Send can send apart
/// from the whole note (frd.md FR-14).
pub fn sections(text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut heads: Vec<(usize, usize, String)> = Vec::new();
    let mut fence: Option<&str> = None;
    let first = lines.iter().position(|l| !l.trim().is_empty());
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        if let Some(f) = fence {
            if t.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some(&t[..3]);
            continue;
        }
        let level = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && t[level..].starts_with(' ') && Some(i) != first {
            heads.push((i, level, t[level..].trim().to_string()));
        }
    }
    heads
        .iter()
        .enumerate()
        .map(|(n, (at, level, name))| {
            let end = heads[n + 1..].iter().find(|(_, l, _)| l <= level).map_or(lines.len(), |h| h.0);
            (name.clone(), lines[*at..end].join("\n").trim_end().to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_are_under_headings() {
        let text = "# Plan\nintro\n## Today\n- a\n```\n# not a heading\n```\n### Later today\n- b\n## Tomorrow\n- c";
        let s = sections(text);
        let names: Vec<&str> = s.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Today", "Later today", "Tomorrow"]);
        assert_eq!(s[0].1, "## Today\n- a\n```\n# not a heading\n```\n### Later today\n- b");
        assert_eq!(s[2].1, "## Tomorrow\n- c");
        assert!(sections("Just a note\nno headings").is_empty());
    }

    #[test]
    fn title_is_the_first_line_with_text() {
        assert_eq!(title("\n  web: login flickers  \nmore").as_deref(), Some("web: login flickers"));
        assert_eq!(title("# Plan\n- a").as_deref(), Some("Plan"));
        assert_eq!(title("###").as_deref(), Some("###"));
        assert_eq!(title(" \n\n"), None);
        assert_eq!(title_or_new(""), "New note");
        assert_eq!(title(&"x".repeat(130)).map(|t| t.chars().count()), Some(121));
    }

    #[test]
    fn preview_is_the_next_line_with_text() {
        assert_eq!(preview("Title\n\n  second\nthird").as_deref(), Some("second"));
        assert_eq!(preview("Title only"), None);
    }

    #[test]
    fn search_needs_every_word() {
        assert!(matches("Ask web to drop the old route", "web route"));
        assert!(!matches("Ask web to drop the old route", "web 429"));
        assert!(matches("anything", ""));
    }

    #[test]
    fn ago_is_short() {
        let m = 60_000;
        assert_eq!(ago(10 * m, 10 * m), "now");
        assert_eq!(ago(10 * m, 5 * m), "5 min");
        assert_eq!(ago(200 * m, 0), "3 h");
        assert_eq!(ago(3 * 1440 * m, 0), "3 d");
        assert_eq!(ago(15 * 1440 * m, 0), "2 w");
        assert_eq!(ago(0, 5), "now");
    }

    #[test]
    fn keys_round_trip() {
        assert_eq!(id_of(&key(42)), Some(42));
        assert_eq!(id_of("p/_/x"), None);
        assert_eq!(id_of("other/1"), None);
    }

    #[test]
    fn a_draft_gets_its_title_once() {
        assert_eq!(draft_text(Some("Plan"), "- a"), "Plan\n- a");
        assert_eq!(draft_text(Some("Plan"), "Plan\n- a"), "Plan\n- a");
        assert_eq!(draft_text(None, "x"), "x");
        assert_eq!(draft_text(Some("  "), "x"), "x");
    }

    #[test]
    fn a_task_is_ticked_on_its_line() {
        assert_eq!(tick("Plan\n- [ ] one\n  * [x] two", 1, true).as_deref(), Some("Plan\n- [x] one\n  * [x] two"));
        assert_eq!(tick("Plan\n- [ ] one\n  * [x] two", 2, false).as_deref(), Some("Plan\n- [ ] one\n  * [ ] two"));
        assert_eq!(tick("Plan\n- one", 1, true), None);
        assert_eq!(tick("Plan", 4, true), None);
    }
}
