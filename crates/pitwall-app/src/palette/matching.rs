//! What the palette shows for a query (`CommandPalette.tsx`): word-AND
//! substring matching, typed paths.

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
