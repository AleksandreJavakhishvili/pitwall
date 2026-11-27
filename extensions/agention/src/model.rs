//! What the tile draws from Agention's threads: groups by agent and
//! session, which of Pitwall's agents a thread is from, and the open asks
//! for Pitwall's floating control. Plain Rust, tested natively.

use std::collections::BTreeMap;

use crate::api::{Ask, AskState, Message, Thread};

/// `raw` as a name, as Pitwall writes `{project}` and `{agent_id}` into
/// the variables the bridge reads (`AGENTWORKS_AGENT`,
/// `AGENTWORKS_SESSION`): lower case; runs of anything but `a-z`, `0-9`,
/// `_` and `-` become one `-`; it starts with a letter or digit, doesn't
/// end with `-` or `_` and has at most 63 characters (`agent` when
/// nothing is left).
pub fn name_form(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars().flat_map(char::to_lowercase) {
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' { c } else { '-' };
        if (c == '-' || c == '_') && out.is_empty() {
            continue;
        }
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    out.truncate(63);
    while out.ends_with('-') || out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("agent");
    }
    out
}

/// One of Pitwall's agents, as far as matching needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PitwallAgent {
    pub id: String,
    pub name: String,
    /// Its project's folder.
    pub project: String,
}

impl PitwallAgent {
    /// The names Agention knows it by: its project folder's last part and
    /// its id, each in name form.
    pub fn names(&self) -> (String, String) {
        let folder = self.project.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
        (name_form(folder), name_form(&self.id))
    }
}

/// Pitwall's agent a thread's agent and session stand for, if any.
pub fn matching<'a>(agents: &'a [PitwallAgent], agent: &str, session: &str) -> Option<&'a PitwallAgent> {
    agents.iter().find(|a| {
        let (p, s) = a.names();
        p == agent && s == session
    })
}

/// The threads of one session, under its agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub agent: String,
    pub session: String,
    /// What the list calls it: Pitwall's agent's name when it is one of
    /// Pitwall's, else Agention's agent and session.
    pub label: String,
    /// Pitwall's id for the agent, when matched.
    pub pitwall: Option<String>,
    /// Its session's status title from Agention.
    pub status: String,
    /// Thread ids, newest activity first.
    pub threads: Vec<String>,
    pub open_asks: u32,
    pub blocking: bool,
}

/// The threads in groups by agent and session: groups waiting on the user
/// first (blocking, then open asks), then by newest activity; `only_open`
/// keeps the threads with open asks or unread messages.
pub fn groups(threads: &[Thread], agents: &[PitwallAgent], titles: &BTreeMap<(String, String), String>, only_open: bool) -> Vec<Group> {
    let mut by: BTreeMap<(String, String), (Group, i64)> = BTreeMap::new();
    let mut sorted: Vec<&Thread> = threads.iter().filter(|t| !t.archived && (!only_open || t.open_asks > 0 || t.unread > 0)).collect();
    sorted.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then_with(|| a.at.cmp(&b.at)));
    for t in sorted {
        let key = (t.at.agent.clone(), t.at.session.clone());
        let (g, newest) = by.entry(key.clone()).or_insert_with(|| {
            let m = matching(agents, &t.at.agent, &t.at.session);
            let label = m.map_or_else(|| format!("{} · {}", t.at.agent, t.at.session), |a| a.name.clone());
            (Group { agent: key.0.clone(), session: key.1.clone(), label, pitwall: m.map(|a| a.id.clone()), status: titles.get(&key).cloned().unwrap_or_default(), threads: Vec::new(), open_asks: 0, blocking: false }, t.updated_ms)
        });
        g.threads.push(t.id());
        g.open_asks += t.open_asks;
        g.blocking |= t.blocking && t.open_asks > 0;
        *newest = (*newest).max(t.updated_ms);
    }
    let mut out: Vec<(Group, i64)> = by.into_values().collect();
    out.sort_by(|(a, an), (b, bn)| b.blocking.cmp(&a.blocking).then((b.open_asks > 0).cmp(&(a.open_asks > 0))).then(bn.cmp(an)).then_with(|| a.label.cmp(&b.label)));
    out.into_iter().map(|(g, _)| g).collect()
}

/// An open ask, with where it is.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenAsk {
    /// `<thread id>#<ask number>`: the floating control's id for it.
    pub id: String,
    pub thread: String,
    pub title: String,
    pub ask: Ask,
    /// Pitwall's agent that asks, when matched.
    pub from: Option<String>,
    pub updated_ms: i64,
}

/// The floating control's id for ask `number` of thread `thread`.
pub fn ask_id(thread: &str, number: u64) -> String {
    format!("{thread}#{number}")
}

/// From an id made by [`ask_id`].
pub fn parse_ask_id(id: &str) -> Option<(String, u64)> {
    let (t, n) = id.rsplit_once('#')?;
    Some((t.to_string(), n.parse().ok()?))
}

/// The open asks in the messages read of each thread, blocking first, then
/// newest, at most `max`.
pub fn open_asks(threads: &[Thread], messages: &BTreeMap<String, Vec<Message>>, agents: &[PitwallAgent], max: usize) -> Vec<OpenAsk> {
    let mut out = Vec::new();
    for t in threads.iter().filter(|t| t.open_asks > 0 && !t.archived) {
        let id = t.id();
        let from = matching(agents, &t.at.agent, &t.at.session).map(|a| a.id.clone());
        for m in messages.get(&id).into_iter().flatten() {
            for a in m.asks.iter().filter(|a| a.state == AskState::Open) {
                out.push(OpenAsk { id: ask_id(&id, a.number), thread: id.clone(), title: t.title.clone(), ask: a.clone(), from: from.clone(), updated_ms: t.updated_ms });
            }
        }
    }
    out.sort_by(|a, b| b.ask.blocking.cmp(&a.ask.blocking).then(b.updated_ms.cmp(&a.updated_ms)).then_with(|| a.id.cmp(&b.id)));
    out.truncate(max);
    out
}

/// Open asks per Pitwall agent (by id), and whether any of them blocks:
/// the sidebar's badges.
pub fn waiting_by_agent(threads: &[Thread], agents: &[PitwallAgent]) -> BTreeMap<String, (u32, bool)> {
    let mut out: BTreeMap<String, (u32, bool)> = BTreeMap::new();
    for t in threads.iter().filter(|t| t.open_asks > 0 && !t.archived) {
        if let Some(a) = matching(agents, &t.at.agent, &t.at.session) {
            let e = out.entry(a.id.clone()).or_default();
            e.0 += t.open_asks;
            e.1 |= t.blocking;
        }
    }
    out
}

/// A key for a thread in the element tree (keys can't hold `/`).
pub fn key_of(thread: &str) -> String {
    thread.replace('/', "~")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AskState, Author, ThreadRef};

    fn t(agent: &str, session: &str, n: u64, open: u32, blocking: bool, at: i64) -> Thread {
        Thread { at: ThreadRef { agent: agent.into(), session: session.into(), number: n }, title: format!("T{n}"), unread: 0, open_asks: open, open_ask_seqs: vec![], blocking, archived: false, updated_ms: at, last_seq: 1 }
    }

    #[test]
    fn names_as_pitwall_writes_them() {
        assert_eq!(name_form("My App"), "my-app");
        assert_eq!(name_form("__web__"), "web");
        assert_eq!(name_form("ÄÖ"), "agent");
        assert_eq!(name_form("a1B2"), "a1b2");
        let a = PitwallAgent { id: "A7f".into(), name: "fix login".into(), project: "/Users/sam/code/My App/".into() };
        assert_eq!(a.names(), ("my-app".into(), "a7f".into()));
        assert!(matching(std::slice::from_ref(&a), "my-app", "a7f").is_some());
        assert!(matching(&[a], "my-app", "other").is_none());
    }

    #[test]
    fn groups_waiting_first() {
        let agents = vec![PitwallAgent { id: "a1".into(), name: "fix login".into(), project: "/p/web".into() }];
        let threads = vec![t("api", "s9", 1, 0, false, 50), t("web", "a1", 1, 1, false, 10), t("web", "a1", 2, 0, false, 20), t("docs", "s2", 1, 1, true, 5)];
        let g = groups(&threads, &agents, &BTreeMap::new(), false);
        assert_eq!(g.iter().map(|g| g.label.as_str()).collect::<Vec<_>>(), ["docs · s2", "fix login", "api · s9"]);
        assert_eq!(g[1].threads, ["web/a1/2", "web/a1/1"]);
        assert_eq!((g[1].pitwall.as_deref(), g[1].open_asks), (Some("a1"), 1));
        assert_eq!(groups(&threads, &agents, &BTreeMap::new(), true).len(), 2, "only what waits on the user");
        assert_eq!(waiting_by_agent(&threads, &agents), BTreeMap::from([("a1".to_string(), (1, false))]));
    }

    #[test]
    fn asks_for_the_floating_control() {
        let threads = vec![t("web", "a1", 1, 1, false, 10), t("docs", "s2", 1, 1, true, 5)];
        let ask = |n, blocking| crate::api::Ask { number: n, question: "Q?".into(), suggestions: vec![], multiple: false, blocking, state: AskState::Open };
        let msg = |asks| Message { seq: 1, author: Author::Agent { agent: "x".into(), session: "y".into() }, time_ms: 0, text: String::new(), detail: String::new(), asks, answers: vec![] };
        let read = BTreeMap::from([("web/a1/1".to_string(), vec![msg(vec![ask(1, false)])]), ("docs/s2/1".to_string(), vec![msg(vec![ask(2, true)])])]);
        let open = open_asks(&threads, &read, &[], 50);
        assert_eq!(open.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["docs/s2/1#2", "web/a1/1#1"]);
        assert_eq!(parse_ask_id("web/a1/1#1"), Some(("web/a1/1".into(), 1)));
        assert_eq!(open_asks(&threads, &read, &[], 1).len(), 1);
    }
}
