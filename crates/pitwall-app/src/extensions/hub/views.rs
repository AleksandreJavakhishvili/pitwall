//! What an extension's adapters draw beside its inbox (docs/extensions/
//! views.md): its views, merged over its connections, the badges and
//! notes it may put on sidebar rows, and the limits on its notifications.
//! The hub keeps the state; this is the plain part of it.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use pitwall_core::extensions::permissions;
use pitwall_core::extensions::view::{Open, Root, RowBadge, Target};
use pitwall_core::extensions::Manifest;

use super::thread_id;

/// A view as tiles read it: its content, or why it is refused (said in
/// the tile).
pub type ViewData = Result<Root, String>;

/// Notifications an extension may show a minute; more are dropped (and
/// logged).
pub const NOTIFY_PER_MINUTE: usize = 3;
/// The same notification (its id, the same words) isn't shown again
/// within this.
pub const SAME_WITHIN: Duration = Duration::from_secs(10 * 60);
/// Badges are applied at most this often; a newer set waits for it.
pub const BADGES_EVERY: Duration = Duration::from_millis(250);

/// Ids as views know them: an adapter's own, with its connection.
fn ids(conn: &str, root: &mut Root) {
    if conn.is_empty() {
        return;
    }
    if let Root::List { list, .. } = root {
        for s in &mut list.sections {
            for i in &mut s.items {
                i.id = thread_id(conn, &i.id);
            }
        }
    }
}

/// Each view of an extension, from its adapters' (connection id, name,
/// views) in order: a list's sections from each connection one after
/// another (named after it when there are several), anything else the
/// first connection's that sent one.
pub fn merge(parts: &[(&str, &str, &HashMap<String, ViewData>)]) -> HashMap<String, ViewData> {
    let several = parts.len() > 1;
    let mut out: HashMap<String, ViewData> = HashMap::new();
    for (conn, name, views) in parts {
        for (id, v) in views.iter() {
            let mut v = v.clone();
            if let Ok(root) = &mut v {
                ids(conn, root);
                if several {
                    if let Root::List { list, .. } = root {
                        for s in &mut list.sections {
                            s.title = if s.title.is_empty() { name.to_string() } else { format!("{name}: {}", s.title) };
                        }
                    }
                }
            }
            match (out.get_mut(id), v) {
                (None, v) => {
                    out.insert(id.clone(), v);
                }
                (Some(Ok(Root::List { list, detail })), Ok(Root::List { list: more, detail: d })) => {
                    list.sections.extend(more.sections);
                    if detail.is_none() {
                        *detail = d;
                    }
                }
                // A refused one gives way to a connection's that isn't.
                (Some(e @ Err(_)), v @ Ok(_)) => *e = v,
                _ => {}
            }
        }
    }
    out
}

/// What `open` names, as views know it (an adapter of connection `conn`
/// sent it).
pub fn open_as_seen(conn: &str, open: Open) -> Open {
    if conn.is_empty() {
        return open;
    }
    match open {
        Open::View { view, item } => Open::View { view, item: item.map(|i| thread_id(conn, &i)) },
        Open::Thread(t) => Open::Thread(thread_id(conn, &t)),
    }
}

/// Whether extension `m`, granted `grants`, may put `b` on a row: an
/// agent by Pitwall's id needs agents.read, a project projects.read, an
/// agent by name its `matchAgents`. `Err`: why not, for its activity.
pub fn allowed(m: &Manifest, grants: &[String], b: &RowBadge) -> Result<(), String> {
    let need = |p: &str| if grants.iter().any(|g| g == p) { Ok(()) } else { Err(format!("a badge on {} needs the \"{p}\" permission", if p == permissions::PROJECTS_READ { "a project" } else { "an agent" })) };
    match &b.target {
        Target::Agent(_) => need(permissions::AGENTS_READ),
        Target::Project(_) => need(permissions::PROJECTS_READ),
        Target::From(_) if m.match_agents.is_some() => Ok(()),
        Target::From(_) => Err("a badge on an agent by name needs \"matchAgents\" in its manifest".into()),
    }
}

/// Whether one more notification fits in the last minute's (`shown`,
/// kept to it); it is counted when it does.
pub fn may_notify(shown: &mut VecDeque<Instant>, now: Instant) -> bool {
    while shown.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60)) {
        shown.pop_front();
    }
    if shown.len() >= NOTIFY_PER_MINUTE {
        return false;
    }
    shown.push_back(now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::extensions::protocol::From;
    use pitwall_core::extensions::view::root;
    use serde_json::json;

    fn list(ids: &[&str]) -> ViewData {
        root(&json!({"kind": "list", "sections": [{"title": "Runs", "items": ids.iter().map(|i| json!({"id": i, "title": i})).collect::<Vec<_>>()}]}))
    }

    /// One connection: as sent. Several: each one's sections, named, the
    /// ids with their connection; a refused one gives way.
    #[test]
    fn views_merge_over_connections() {
        let one: HashMap<String, ViewData> = [("runs".to_string(), list(&["a"]))].into();
        let m = merge(&[("", "X", &one)]);
        assert_eq!(m["runs"], list(&["a"]));
        let two: HashMap<String, ViewData> = [("runs".to_string(), list(&["b"])), ("other".to_string(), Err("too big".into()))].into();
        let m = merge(&[("c1", "Work", &one), ("c2", "Home", &two)]);
        let Ok(Root::List { list: l, .. }) = &m["runs"] else { panic!("{:?}", m["runs"]) };
        let got: Vec<(&str, Vec<&str>)> = l.sections.iter().map(|s| (s.title.as_str(), s.items.iter().map(|i| i.id.as_str()).collect())).collect();
        assert_eq!(got, [("Work: Runs", vec!["c1:a"]), ("Home: Runs", vec!["c2:b"])]);
        assert_eq!(m["other"], Err("too big".into()));
        let ok: HashMap<String, ViewData> = [("other".to_string(), list(&["c"]))].into();
        assert!(merge(&[("c2", "Home", &two), ("c3", "More", &ok)])["other"].is_ok());
    }

    #[test]
    fn badges_need_what_names_their_row() {
        let mut m = Manifest::named("x", "X");
        let b = |target| RowBadge { target, count: Some(1), tone: Default::default(), icon: None, note: String::new(), tooltip: String::new(), open: None };
        let agent = b(Target::Agent("a1".into()));
        assert!(allowed(&m, &[], &agent).unwrap_err().contains("agents.read"));
        assert!(allowed(&m, &["agents.read".into()], &agent).is_ok());
        assert!(allowed(&m, &["agents.read".into()], &b(Target::Project("/p".into()))).unwrap_err().contains("projects.read"));
        let by_name = b(Target::From(From { agent: "a".into(), session: "s".into() }));
        assert!(allowed(&m, &[], &by_name).unwrap_err().contains("matchAgents"));
        m.match_agents = Some(pitwall_core::extensions::MatchAgents { agent: "{project}".into(), session: "{agent_id}".into() });
        assert!(allowed(&m, &[], &by_name).is_ok());
    }

    #[test]
    fn three_notifications_a_minute() {
        let mut shown = VecDeque::new();
        let t = Instant::now();
        assert!((0..3).all(|i| may_notify(&mut shown, t + Duration::from_secs(i))));
        assert!(!may_notify(&mut shown, t + Duration::from_secs(30)));
        assert!(may_notify(&mut shown, t + Duration::from_secs(61)), "the first is a minute old");
        assert_eq!(open_as_seen("c1", Open::View { view: "v".into(), item: Some("i".into()) }), Open::View { view: "v".into(), item: Some("c1:i".into()) });
    }
}
