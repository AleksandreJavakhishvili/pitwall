//! What the palette shows for a query (`CommandPalette.tsx`): word-AND
//! substring matching, the inline queue syntax, typed paths.

use std::sync::OnceLock;

use pitwall_proto::AgentView;

use super::registry::Command;

/// Every whitespace-separated word of `q` occurs in `search` (any case).
pub fn matches(search: &str, q: &str) -> bool {
    let s = search.to_lowercase();
    q.to_lowercase().split_whitespace().all(|w| s.contains(w))
}

/// "~", "/…" or "~/…": a folder to open a terminal in (`looksLikePath`).
pub fn looks_like_path(q: &str) -> bool {
    let t = q.trim();
    t == "~" || t.starts_with('/') || t.starts_with("~/")
}

/// "api-fix: do the thing" or "queue [for] api-fix: do the thing" → the
/// agent and the text, verbatim (`parseQueue`).
pub fn parse_queue<'a>(q: &str, agents: &'a [AgentView]) -> Option<(&'a AgentView, String)> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?is)^(?:queue\s+(?:for\s+)?)?([a-z][a-z0-9_-]{0,31})\s*:\s?(.+)$")
            .expect("a valid pattern")
    });
    let m = re.captures(q)?;
    let name = m.get(1)?.as_str();
    let text = m.get(2)?.as_str();
    let agent = agents.iter().find(|a| a.name == name)?;
    if text.trim().is_empty() {
        return None;
    }
    Some((agent, text.to_string()))
}

/// The rows to list for `q`: on an empty query everything but the
/// query-only rows; else the pinned rows first, then those matching.
pub fn visible(rows: Vec<Command>, q: &str) -> Vec<Command> {
    if q.trim().is_empty() {
        return rows.into_iter().filter(|c| !c.query_only).collect();
    }
    let (mut pinned, rest): (Vec<_>, Vec<_>) = rows.into_iter().partition(|c| c.pinned);
    pinned.extend(rest.into_iter().filter(|c| matches(&c.search, q)));
    pinned
}
