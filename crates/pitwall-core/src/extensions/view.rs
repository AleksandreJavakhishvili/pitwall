//! What an extension draws besides its inbox (docs/spec/extension-api.md,
//! "Contribution points" 2, 4 and 5): a view tile's content, badges and
//! short notes on sidebar rows, and notifications, as its adapter sends
//! them. Pitwall draws all of it itself from a small set of components:
//! nothing in it is a layout, a colour, a font or a script.
//!
//! A **view** is sent whole each time (`{"type":"view","view":id,"root":…}`,
//! never patched) and replaces the last one. Its root is a [`List`] with an
//! optional pane beside it (the selected item's [`Detail`], [`Markdown`]
//! text or a [`Status`]), or one such pane on its own. `form` and `table`
//! are kinds a later Pitwall draws: a view of either is refused, saying so,
//! as is a view over its limits ([`MAX_VIEW_BYTES`], [`MAX_ITEMS`],
//! [`MAX_SECTIONS`]): the tile shows why instead.
//!
//! Everything is untrusted, as the inbox's lines are
//! ([`super::protocol`]): strings are cut to their limits with control and
//! bidirectional-override characters removed, lists cut to their lengths,
//! an item or an action without a usable id dropped, an icon name Pitwall
//! doesn't have drawn as the extension icon, a link that isn't http(s)
//! dropped, and Markdown drawn by Pitwall's inert renderer.

use serde_json::Value;

use super::protocol::{clean, From};

/// A view's `root`, as JSON, at most this many bytes.
pub const MAX_VIEW_BYTES: usize = 256 * 1024;
/// The most items a list shows (all sections together).
pub const MAX_ITEMS: usize = 500;
/// The most sections in a list.
pub const MAX_SECTIONS: usize = 50;
/// The most badges on an item.
pub const MAX_ITEM_BADGES: usize = 3;
/// The most actions on an item or a detail.
pub const MAX_ACTIONS: usize = 5;
/// The most actions in a status.
pub const MAX_STATUS_ACTIONS: usize = 2;
/// The most label/value rows in a detail.
pub const MAX_FIELDS: usize = 30;
/// The longest Markdown (a detail's body, a markdown pane).
pub const MAX_MARKDOWN: usize = 40_000;
/// The most badges and notes in one `badges` line.
pub const MAX_ROW_BADGES: usize = 500;
/// A row note's longest text.
pub const MAX_NOTE: usize = 16;
/// The longest id (longer ones are dropped, never cut: ids are matched).
pub const MAX_ID: usize = 200;
/// What Pitwall draws in a view.
pub const KINDS: [&str; 4] = ["list", "detail", "markdown", "status"];
/// Kinds a later Pitwall draws.
pub const LATER_KINDS: [&str; 2] = ["form", "table"];

/// How an item, a badge or a row's badge looks: its colour, from Pitwall's
/// own palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    None,
    Good,
    Bad,
    Warn,
    Busy,
}

impl Tone {
    fn of(v: &Value) -> Tone {
        match v.get("tone").and_then(Value::as_str) {
            Some("good") => Tone::Good,
            Some("bad") => Tone::Bad,
            Some("warn") => Tone::Warn,
            Some("busy") => Tone::Busy,
            _ => Tone::None,
        }
    }
}

/// A small label on an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub text: String,
    pub tone: Tone,
}

/// A button: pressing it sends `action` back to the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub label: String,
    /// The item's main action (Enter on the item).
    pub primary: bool,
}

/// A row of a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub tone: Tone,
    /// One of Pitwall's icon names (an unknown one draws the extension's).
    pub icon: Option<String>,
    pub badges: Vec<Badge>,
    /// When (ISO 8601, as the adapter says), drawn as "5m".
    pub time: String,
    pub actions: Vec<Action>,
}

/// A titled group of items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub items: Vec<Item>,
}

/// Sections of items; `empty` is what it says when there are none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct List {
    pub sections: Vec<Section>,
    pub empty: String,
}

impl List {
    /// Every item, in order.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.sections.iter().flat_map(|s| s.items.iter())
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items().find(|i| i.id == id)
    }
}

/// A label and its value in a detail, the value maybe a link (http(s)
/// only, opened in the browser on a click).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub value: String,
    pub link: Option<String>,
}

/// One thing in full: a title, label/value rows, Markdown, buttons.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Detail {
    pub title: String,
    pub fields: Vec<Field>,
    pub body: String,
    pub actions: Vec<Action>,
}

/// What a status says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusKind {
    /// Nothing to show ("Nothing failing").
    #[default]
    Empty,
    Loading,
    /// Something is wrong ("Signed out").
    Error,
}

/// An empty, loading or error state: an icon, words, at most two buttons.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    pub kind: StatusKind,
    pub icon: Option<String>,
    pub title: String,
    pub text: String,
    pub actions: Vec<Action>,
}

/// What fills a view, or the side of a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pane {
    Detail(Detail),
    Markdown(String),
    Status(Status),
}

/// A view's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Root {
    /// A list, with what is beside it (the selected item's detail).
    List { list: List, detail: Option<Pane> },
    Pane(Pane),
}

/// A `view` line: which view, and its content or why it is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub id: String,
    pub root: Result<Root, String>,
}

/// What a badge, a note or a notification opens when clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Open {
    /// One of the extension's views, at an item.
    View { view: String, item: Option<String> },
    /// A thread of its inbox.
    Thread(String),
}

/// Which sidebar row a badge is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// An agent by Pitwall's id (`agents.read`).
    Agent(String),
    /// An agent by the names its `matchAgents` gives it (no permission).
    From(From),
    /// A project by its folder (`projects.read`).
    Project(String),
}

/// A badge (a count) and/or a short note on a sidebar row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowBadge {
    pub target: Target,
    pub count: Option<u32>,
    pub tone: Tone,
    pub icon: Option<String>,
    /// At most [`MAX_NOTE`] characters, beside the name.
    pub note: String,
    pub tooltip: String,
    pub open: Option<Open>,
}

/// A notification the adapter asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notify {
    /// The adapter's id for it: the same id again replaces rather than
    /// repeats.
    pub id: String,
    pub title: String,
    pub body: String,
    pub open: Option<Open>,
}

fn text(v: &Value, key: &str, max: usize) -> String {
    v.get(key).and_then(Value::as_str).map(|s| clean(s, max, false)).unwrap_or_default()
}

fn long_text(v: &Value, key: &str, max: usize) -> String {
    v.get(key).and_then(Value::as_str).map(|s| clean(s, max, true)).unwrap_or_default()
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// An id: a non-empty string of at most [`MAX_ID`] characters without
/// control characters.
pub fn id_of(v: &Value, key: &str) -> Option<String> {
    let s = v.get(key)?.as_str()?;
    (!s.is_empty() && s.chars().count() <= MAX_ID && !s.chars().any(char::is_control)).then(|| s.to_string())
}

/// An icon name: like an extension id (else none: the extension's icon).
fn icon(v: &Value) -> Option<String> {
    let s = v.get("icon")?.as_str()?;
    super::check_word("icon", s).ok().map(|_| s.to_string())
}

/// An http(s) address without spaces, at most 2000 characters.
pub fn link(s: &str) -> Option<String> {
    let ok = (s.starts_with("https://") || s.starts_with("http://")) && s.len() <= 2000 && !s.chars().any(|c| c.is_whitespace() || c.is_control());
    ok.then(|| s.to_string())
}

fn actions(v: &Value, max: usize) -> Vec<Action> {
    let mut out: Vec<Action> = Vec::new();
    for a in list(v, "actions") {
        let Some(id) = id_of(a, "id") else { continue };
        if out.iter().any(|x| x.id == id) {
            continue;
        }
        let label = text(a, "label", 40);
        out.push(Action { label: if label.is_empty() { id.clone() } else { label }, id, primary: a.get("primary").and_then(Value::as_bool).unwrap_or(false) });
        if out.len() == max {
            break;
        }
    }
    // One primary at most: the first marked.
    let mut seen = false;
    for a in &mut out {
        let first = a.primary && !seen;
        seen |= a.primary;
        a.primary = first;
    }
    out
}

fn item(v: &Value) -> Option<Item> {
    Some(Item {
        id: id_of(v, "id")?,
        title: text(v, "title", 200),
        subtitle: text(v, "subtitle", 300),
        tone: Tone::of(v),
        icon: icon(v),
        badges: list(v, "badges").iter().filter_map(|b| Some(Badge { text: text(b, "text", 24), tone: Tone::of(b) }).filter(|b| !b.text.is_empty())).take(MAX_ITEM_BADGES).collect(),
        time: text(v, "time", 40),
        actions: actions(v, MAX_ACTIONS),
    })
}

fn detail(v: &Value) -> Detail {
    Detail {
        title: text(v, "title", 200),
        fields: list(v, "fields")
            .iter()
            .filter_map(|f| {
                let label = text(f, "label", 60);
                let value = text(f, "value", 500);
                (!label.is_empty() || !value.is_empty()).then(|| Field { label, value, link: f.get("link").and_then(Value::as_str).and_then(link) })
            })
            .take(MAX_FIELDS)
            .collect(),
        body: long_text(v, "body", MAX_MARKDOWN),
        actions: actions(v, MAX_ACTIONS),
    }
}

fn status(v: &Value) -> Status {
    Status {
        kind: match v.get("state").and_then(Value::as_str) {
            Some("loading") => StatusKind::Loading,
            Some("error") => StatusKind::Error,
            _ => StatusKind::Empty,
        },
        icon: icon(v),
        title: text(v, "title", 120),
        text: long_text(v, "text", 1000),
        actions: actions(v, MAX_STATUS_ACTIONS),
    }
}

/// Why kind `k` isn't drawn.
fn refused_kind(k: &str) -> String {
    if LATER_KINDS.contains(&k) {
        format!("It sent a {k}, which a later Pitwall draws. This one draws a list, a detail, Markdown and a status.")
    } else {
        format!("It sent a view of kind “{k}”, which Pitwall doesn't know. It draws a list, a detail, Markdown and a status.")
    }
}

fn pane(v: &Value) -> Result<Pane, String> {
    match v.get("kind").and_then(Value::as_str).unwrap_or_default() {
        "detail" => Ok(Pane::Detail(detail(v))),
        "markdown" => Ok(Pane::Markdown(long_text(v, "text", MAX_MARKDOWN))),
        "status" => Ok(Pane::Status(status(v))),
        "" => Err("It sent a view without a \"kind\".".into()),
        "list" => Err("It sent a list beside a list: only a detail, Markdown or a status goes beside one.".into()),
        k => Err(refused_kind(k)),
    }
}

/// A view's `root`, or why it isn't drawn.
pub fn root(v: &Value) -> Result<Root, String> {
    if !v.is_object() {
        return Err("It sent a view without a \"root\".".into());
    }
    let size = v.to_string().len();
    if size > MAX_VIEW_BYTES {
        return Err(format!("It sent a view of {} KiB; a view is at most {} KiB.", size.div_ceil(1024), MAX_VIEW_BYTES / 1024));
    }
    if v.get("kind").and_then(Value::as_str) != Some("list") {
        return pane(v).map(Root::Pane);
    }
    let sections = list(v, "sections");
    if sections.len() > MAX_SECTIONS {
        return Err(format!("It sent {} sections; a list has at most {MAX_SECTIONS}.", sections.len()));
    }
    let count: usize = sections.iter().map(|s| list(s, "items").len()).sum();
    if count > MAX_ITEMS {
        return Err(format!("It sent {count} items; a view shows at most {MAX_ITEMS}."));
    }
    let mut seen = std::collections::HashSet::new();
    let sections = sections
        .iter()
        .map(|s| Section { title: text(s, "title", 120), items: list(s, "items").iter().filter_map(item).filter(|i| seen.insert(i.id.clone())).collect() })
        .collect();
    let detail = match v.get("detail") {
        None | Some(Value::Null) => None,
        Some(d) => Some(pane(d)?),
    };
    Ok(Root::List { list: List { sections, empty: text(v, "empty", 200) }, detail })
}

/// A `view` line: `Err` when it names no usable view (it is dropped).
pub fn parse_view(v: &Value) -> Result<View, String> {
    let id = v.get("view").and_then(Value::as_str).filter(|s| super::check_word("view", s).is_ok()).ok_or("a view without a usable \"view\" id")?;
    Ok(View { id: id.to_string(), root: v.get("root").map_or_else(|| Err("It sent a view without a \"root\".".to_string()), root) })
}

/// What `open` names, when it is usable.
pub fn open_of(v: &Value) -> Option<Open> {
    let o = v.get("open")?;
    if let Some(t) = id_of(o, "thread") {
        return Some(Open::Thread(t));
    }
    let view = o.get("view").and_then(Value::as_str).filter(|s| super::check_word("view", s).is_ok())?;
    Some(Open::View { view: view.to_string(), item: id_of(o, "item") })
}

fn target(v: &Value) -> Option<Target> {
    if let Some(a) = v.get("agent") {
        if let Some(id) = id_of(a, "agentId") {
            return Some(Target::Agent(id));
        }
        let f = a.get("from")?;
        let name = |k: &str| f.get(k).and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 100 && !s.chars().any(char::is_control)).map(String::from);
        return Some(Target::From(From { agent: name("agent")?, session: name("session")? }));
    }
    let p = v.get("project")?;
    let path = p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control))?;
    Some(Target::Project(path.to_string()))
}

/// A `badges` line's set (the whole set each time): those without a
/// usable row, or with neither a count nor a note, are dropped.
pub fn parse_badges(v: &Value) -> Vec<RowBadge> {
    list(v, "badges")
        .iter()
        .filter_map(|b| {
            let count = b.get("count").and_then(Value::as_u64).map(|n| n.min(9999) as u32);
            let note = text(b, "note", MAX_NOTE);
            if count.is_none() && note.is_empty() {
                return None;
            }
            Some(RowBadge { target: target(b)?, count, tone: Tone::of(b), icon: icon(b), note, tooltip: text(b, "tooltip", 120), open: open_of(b) })
        })
        .take(MAX_ROW_BADGES)
        .collect()
}

/// A `notify` line: `Err` without a usable id or title.
pub fn parse_notify(v: &Value) -> Result<Notify, String> {
    let id = id_of(v, "id").ok_or("a notification without a usable \"id\"")?;
    let title = text(v, "title", 100);
    if title.is_empty() {
        return Err("a notification without a \"title\"".into());
    }
    Ok(Notify { id, title, body: text(v, "body", 300), open: open_of(v) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The proposal's example: a list with a section of items, each with
    /// its tone, badges and actions, and a detail beside it.
    #[test]
    fn a_list_view_is_read_and_cut_to_its_limits() {
        let v = parse_view(&json!({"type": "view", "view": "runs", "root": {
            "kind": "list", "empty": "Nothing failing", "sections": [{"title": "Failing", "items": [
                {"id": "r812", "title": "api: tests failed", "subtitle": "agent/fix-auth · 3 min ago", "tone": "bad", "icon": "check",
                 "badges": [{"text": "2 jobs"}, {"text": ""}, {"text": "a", "tone": "warn"}, {"text": "b"}, {"text": "c"}],
                 "time": "2026-11-23T10:00:00Z",
                 "actions": [{"id": "open", "label": "Open log", "primary": true}, {"id": "rerun", "label": "Re-run", "primary": true}, {"label": "no id"}, {"id": "open"}]},
                {"title": "no id"},
                {"id": "r812", "title": "twice"},
                {"id": "r9", "title": "evil\u{202E}txt\nnext", "icon": "Not An Icon"}]}],
            "detail": {"kind": "markdown", "text": "Select a run\n\n**bold**"}}}))
        .unwrap();
        assert_eq!(v.id, "runs");
        let Ok(Root::List { list, detail }) = v.root else { panic!("{:?}", v.root) };
        assert_eq!(list.empty, "Nothing failing");
        let items: Vec<&Item> = list.items().collect();
        assert_eq!(items.len(), 2, "no id and a second r812 are dropped");
        let i = items[0];
        assert_eq!((i.tone, i.icon.as_deref(), i.badges.len()), (Tone::Bad, Some("check"), 3));
        assert_eq!(i.badges[1], Badge { text: "a".into(), tone: Tone::Warn });
        assert_eq!(i.actions.iter().map(|a| (a.id.as_str(), a.primary)).collect::<Vec<_>>(), [("open", true), ("rerun", false)], "one primary, ids unique");
        assert_eq!((items[1].title.as_str(), items[1].icon.as_deref()), ("eviltxt next", None));
        assert_eq!(detail, Some(Pane::Markdown("Select a run\n\n**bold**".into())));
        assert_eq!(list.item("r9").map(|i| i.id.as_str()), Some("r9"));
    }

    #[test]
    fn details_markdown_and_statuses() {
        let r = root(&json!({"kind": "detail", "title": "Run 812", "body": "Log:\n```\nfail\n```",
            "fields": [{"label": "Branch", "value": "agent/fix-auth"}, {"label": "Link", "value": "CI", "link": "https://ci.example.com/812"}, {"label": "Bad", "value": "x", "link": "javascript:alert(1)"}, {}],
            "actions": [{"id": "rerun", "label": "Re-run"}]}))
        .unwrap();
        let Root::Pane(Pane::Detail(d)) = r else { panic!() };
        assert_eq!(d.fields.len(), 3, "an empty row is dropped");
        assert_eq!((d.fields[1].link.as_deref(), d.fields[2].link.as_deref()), (Some("https://ci.example.com/812"), None));
        assert!(d.body.contains('\n'));
        let r = root(&json!({"kind": "status", "state": "error", "icon": "lock", "title": "Signed out", "text": "Open Settings", "actions": [{"id": "a"}, {"id": "b"}, {"id": "c"}]})).unwrap();
        let Root::Pane(Pane::Status(s)) = r else { panic!() };
        assert_eq!((s.kind, s.actions.len(), s.actions[0].label.as_str()), (StatusKind::Error, 2, "a"));
        assert_eq!(root(&json!({"kind": "markdown", "text": "# Hi"})).unwrap(), Root::Pane(Pane::Markdown("# Hi".into())));
    }

    /// Form and table are refused, saying a later Pitwall draws them; so
    /// is a view over its limits, saying which.
    #[test]
    fn views_pitwall_doesnt_draw_are_refused_plainly() {
        let why = |v: Value| root(&v).unwrap_err();
        assert!(why(json!({"kind": "form", "fields": []})).contains("a form, which a later Pitwall draws"));
        assert!(why(json!({"kind": "table"})).contains("a table, which a later Pitwall draws"));
        assert!(why(json!({"kind": "chart"})).contains("“chart”, which Pitwall doesn't know"));
        assert!(why(json!({"kind": "list", "sections": [], "detail": {"kind": "list"}})).contains("beside a list"));
        assert!(why(json!({"kind": "list", "sections": [], "detail": {"kind": "form"}})).contains("a form"));
        let items: Vec<Value> = (0..501).map(|i| json!({"id": i.to_string()})).collect();
        assert_eq!(why(json!({"kind": "list", "sections": [{"items": items}]})), "It sent 501 items; a view shows at most 500.");
        let sections: Vec<Value> = (0..51).map(|_| json!({"items": []})).collect();
        assert!(why(json!({"kind": "list", "sections": sections})).contains("at most 50"));
        let big = "x".repeat(MAX_VIEW_BYTES);
        assert!(why(json!({"kind": "markdown", "text": big})).contains("at most 256 KiB"));
        // Without a usable view id the line is dropped; without a root the
        // view says so.
        assert!(parse_view(&json!({"type": "view", "view": "Runs"})).is_err());
        assert!(parse_view(&json!({"type": "view", "view": "runs"})).unwrap().root.unwrap_err().contains("without a \"root\""));
    }

    #[test]
    fn badges_and_notifications_are_read() {
        let b = parse_badges(&json!({"type": "badges", "badges": [
            {"agent": {"agentId": "a1"}, "count": 2, "tone": "bad", "tooltip": "2 failing runs", "open": {"view": "runs", "item": "r812"}},
            {"agent": {"from": {"agent": "api", "session": "a7f3"}}, "note": "PR #42 ✓ and a long tail"},
            {"project": {"path": "/code/api"}, "note": "deploying", "icon": "rocket"},
            {"agent": {"agentId": "a2"}},
            {"note": "nowhere"},
            {"agent": {"from": {"agent": "half"}}, "count": 1}
        ]}));
        assert_eq!(b.len(), 3, "without a count or note, or a row, dropped");
        assert_eq!((b[0].target.clone(), b[0].count, b[0].tone), (Target::Agent("a1".into()), Some(2), Tone::Bad));
        assert_eq!(b[0].open, Some(Open::View { view: "runs".into(), item: Some("r812".into()) }));
        assert_eq!(b[1].note.chars().count(), MAX_NOTE, "a note is cut");
        assert_eq!(b[1].target, Target::From(From { agent: "api".into(), session: "a7f3".into() }));
        assert_eq!((b[2].target.clone(), b[2].icon.as_deref()), (Target::Project("/code/api".into()), Some("rocket")));
        let n = parse_notify(&json!({"type": "notify", "id": "r812", "title": "api: tests failed", "body": "agent/fix-auth", "open": {"thread": "t1"}})).unwrap();
        assert_eq!((n.title.as_str(), n.open), ("api: tests failed", Some(Open::Thread("t1".into()))));
        assert!(parse_notify(&json!({"type": "notify", "title": "x"})).is_err());
        assert!(parse_notify(&json!({"type": "notify", "id": "x"})).is_err());
    }
}
