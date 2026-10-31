//! Pure helpers of the main screen: sidebar groups with listed projects,
//! machine and account sections, collapsed summaries, the status strip, rail initials,
//! agent and terminal names, shortcut labels (ports of `src/lib/groups.ts`,
//! `statusStrip.ts`, `terminals.ts`, `host.ts` `keys()` and the New-agent
//! dialog's name helpers).

use pitwall_core::onboarding::project_list::Project;
use pitwall_proto::peer::LinkView;
use pitwall_proto::{AgentView, MachineView, Status};

use crate::agents::{status_word, ProjectGroup};

/// Sidebar groups = agent groups + listed projects without agents (`withProjects`).
pub fn with_projects(groups: Vec<ProjectGroup>, projects: &[Project]) -> Vec<ProjectGroup> {
    let have: Vec<&str> = groups
        .iter()
        .filter(|g| g.can_create)
        .map(|g| g.project.as_str())
        .collect();
    let empty: Vec<ProjectGroup> = projects
        .iter()
        .filter(|p| !have.contains(&p.path.as_str()))
        .map(|p| ProjectGroup {
            key: p.path.clone(),
            project: p.path.clone(),
            display: p.display.clone(),
            machine: None,
            can_create: true,
            agents: vec![],
            blocked: 0,
        })
        .collect();
    if empty.is_empty() {
        return groups;
    }
    let mut all = groups;
    all.extend(empty);
    let label = |g: &ProjectGroup| {
        if g.can_create {
            String::new()
        } else {
            g.machine
                .as_ref()
                .map(|m| m.label.clone())
                .unwrap_or_default()
        }
    };
    all.sort_by(|a, b| {
        b.can_create
            .cmp(&a.can_create)
            .then_with(|| label(a).cmp(&label(b)))
            .then_with(|| a.display.to_lowercase().cmp(&b.display.to_lowercase()))
    });
    all
}

/// A group's key without its account section's prefix (`acct:<id>|`): the
/// same project under every account.
pub fn base_key(key: &str) -> &str {
    key.strip_prefix("acct:").and_then(|r| r.split_once('|')).map_or(key, |(_, k)| k)
}

/// The account section a group key sits in (`acct:<id>|…`; `None` when
/// grouped by project).
pub fn section_of(key: &str) -> Option<&str> {
    key.strip_prefix("acct:")?.split_once('|').map(|(a, _)| a)
}

/// Groups in the order the user dragged them (`order`: project keys and
/// agent ids): projects stay in their machine's block, agents in their
/// project; what `order` doesn't name follows, projects as before and
/// agents oldest first.
pub fn in_order(mut groups: Vec<ProjectGroup>, order: &[String]) -> Vec<ProjectGroup> {
    let rank = |k: &str| order.iter().position(|o| o == k).unwrap_or(usize::MAX);
    let label = |g: &ProjectGroup| {
        (!g.can_create)
            .then(|| g.machine.as_ref().map(|m| m.label.clone()))
            .flatten()
            .unwrap_or_default()
    };
    groups.sort_by_cached_key(|g| (!g.can_create, label(g), rank(base_key(&g.key))));
    for g in &mut groups {
        g.agents.sort_by_cached_key(|a| (rank(&a.id), a.created_at));
    }
    groups
}

/// `order` with `item` placed just before `before` (`None`: last) among
/// `siblings`, in their shown order; other entries keep their places.
pub fn reorder(order: &[String], siblings: &[String], item: &str, before: Option<&str>) -> Vec<String> {
    let mut seq: Vec<String> = siblings.iter().filter(|s| *s != item).cloned().collect();
    let at = before.and_then(|b| seq.iter().position(|s| s == b)).unwrap_or(seq.len());
    seq.insert(at, item.to_string());
    let mut out: Vec<String> = order.iter().filter(|o| !seq.contains(o)).cloned().collect();
    out.extend(seq);
    out
}

/// One account in the sidebar grouped by account: its project groups.
#[derive(Debug, Clone)]
pub struct AccountSection {
    /// `account_key` of its agents (`default`: the user's own logins).
    pub account: String,
    /// Its collapse key (`acct:<account>`; on another machine than this
    /// Mac `acct:<account>@<machine>`, so each folds on its own).
    pub key: String,
    /// Another machine's Pitwall's account: its name there, and nothing
    /// in it can be changed from here.
    pub peer_name: Option<String>,
    pub groups: Vec<ProjectGroup>,
    pub agents: usize,
    pub blocked: usize,
}

/// The sidebar grouped by account, then project (`with_projects` within
/// each): Default first, then the accounts in `order`, then any other.
/// Projects without agents go under their account (Default when they have
/// none). Group keys are prefixed with the section's, so each folds on its
/// own.
pub fn by_account(agents: &[AgentView], projects: &[Project], order: &[String]) -> Vec<AccountSection> {
    use super::workspace::account_key;
    use pitwall_core::accounts::DEFAULT;
    let mut keys: Vec<String> = vec![];
    for a in agents {
        let k = account_key(a);
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let empties: Vec<(String, ProjectGroup)> = with_projects(crate::agents::group_by_project(agents), projects)
        .into_iter()
        .filter(|g| g.agents.is_empty())
        .map(|g| (crate::accounts::project_account(&g.project).unwrap_or_else(|| DEFAULT.into()), g))
        .collect();
    for (k, _) in &empties {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }
    let rank = |k: &String| match order.iter().position(|o| o == k) {
        _ if k == DEFAULT || k.ends_with(PEER_DEFAULT) => (0, 0, String::new()),
        Some(i) => (1, i, String::new()),
        None => (2, 0, k.clone()),
    };
    keys.sort_by_key(rank);
    keys.into_iter()
        .map(|account| {
            let mine: Vec<AgentView> = agents.iter().filter(|a| account_key(a) == account).cloned().collect();
            let mut groups = crate::agents::group_by_project(&mine);
            groups.extend(empties.iter().filter(|(k, _)| *k == account).map(|(_, g)| g.clone()));
            let key = format!("acct:{account}");
            for g in &mut groups {
                g.key = format!("{key}|{}", g.key);
            }
            let blocked = groups.iter().map(|g| g.blocked).sum();
            let peer_name = mine.iter().find(|a| crate::agents::mirrored(a)).map(|a| peer_account(a).1);
            AccountSection { account, key, peer_name, groups, agents: mine.len(), blocked }
        })
        .collect()
}

/// The end of a peer's Default account's key.
const PEER_DEFAULT: &str = "/default";

/// An agent of another machine's Pitwall: its account there, as a key
/// (`peer:<peer>/default`, `peer:<peer>/@<name>`, or `peer:<peer>/~` when
/// an older peer sent only an id) and the name shown. This Mac's account
/// ids are never matched to it.
pub fn peer_account(a: &AgentView) -> (String, String) {
    let peer = a.machine.link.as_ref().map_or("", |l| l.peer.as_str());
    match (&a.account, &a.account_name) {
        (None, _) => (format!("peer:{peer}{PEER_DEFAULT}"), "Default".into()),
        (Some(_), Some(n)) => (format!("peer:{peer}/@{}", n.replace('|', "/")), n.clone()),
        (Some(_), None) => (format!("peer:{peer}/~"), "Unnamed account".into()),
    }
}

fn machine_key(m: &MachineView) -> String {
    format!("{}:{}", m.provider, m.id)
}

/// One machine in the sidebar: its heading (when there are 2+ machines,
/// [`machine_level`]) over its accounts or projects.
#[derive(Debug, Clone)]
pub struct MachineSection {
    /// Its collapse key (`machine:<provider>:<id>`; `machine:peer:<peer>`
    /// for a paired machine with nothing to show).
    pub key: String,
    pub label: String,
    /// This Mac: its agents and listed projects.
    pub home: bool,
    /// Another machine's Pitwall: how the link is. Read-only.
    pub link: Option<LinkView>,
    /// Grouped by project: its project groups.
    pub groups: Vec<ProjectGroup>,
    /// Grouped by account: its accounts, each with its project groups.
    pub accounts: Vec<AccountSection>,
}

impl MachineSection {
    /// Its project groups, under its accounts or not.
    pub fn all_groups(&self) -> impl Iterator<Item = &ProjectGroup> {
        self.groups.iter().chain(self.accounts.iter().flat_map(|s| s.groups.iter()))
    }

    pub fn agents(&self) -> Vec<AgentView> {
        self.all_groups().flat_map(|g| g.agents.iter().cloned()).collect()
    }

    pub fn blocked(&self) -> usize {
        self.all_groups().map(|g| g.blocked).sum()
    }
}

/// The machine a group is on: (its key, label, link, this Mac). Listed
/// projects without agents are this Mac's.
fn machine_of(m: Option<&MachineView>) -> (String, String, Option<LinkView>, bool) {
    match m {
        Some(m) if m.provider != pitwall_core::provider::ProviderId::LOCAL => {
            (format!("machine:{}", machine_key(m)), m.label.clone(), m.link.clone(), false)
        }
        m => ("machine:this-mac".into(), m.map_or("This Mac".into(), |m| m.label.clone()), None, true),
    }
}

/// This Mac first, then its other machines (agw), then other machines'
/// Pitwalls (a peer's VMs, `studio › vm-1`, after it), each by name.
fn sort_machines(sections: &mut [MachineSection]) {
    sections.sort_by_cached_key(|m| (if m.home { 0 } else if m.link.is_none() { 1 } else { 2 }, m.label.to_lowercase()));
}

/// The sidebar grouped by project, split by machine (`groups` in sidebar
/// order; each keeps its place within its machine).
pub fn by_machine(groups: Vec<ProjectGroup>) -> Vec<MachineSection> {
    let mut out: Vec<MachineSection> = vec![];
    for g in groups {
        let (key, label, link, home) = machine_of(g.machine.as_ref());
        match out.iter_mut().find(|m| m.key == key) {
            Some(m) => m.groups.push(g),
            None => out.push(MachineSection { key, label, home, link, groups: vec![g], accounts: vec![] }),
        }
    }
    sort_machines(&mut out);
    out
}

/// The sidebar grouped by account, split by machine: each machine's own
/// accounts ([`by_account`]; listed projects go to this Mac's).
pub fn by_machine_account(agents: &[AgentView], projects: &[Project], order: &[String]) -> Vec<MachineSection> {
    let mut out: Vec<MachineSection> = vec![];
    let mut mine: Vec<Vec<AgentView>> = vec![];
    for a in agents {
        let (key, label, link, home) = machine_of(Some(&a.machine));
        match out.iter().position(|m| m.key == key) {
            Some(i) => mine[i].push(a.clone()),
            None => {
                out.push(MachineSection { key, label, home, link, groups: vec![], accounts: vec![] });
                mine.push(vec![a.clone()]);
            }
        }
    }
    if !out.iter().any(|m| m.home) && !projects.is_empty() {
        let (key, label, link, home) = machine_of(None);
        out.push(MachineSection { key, label, home, link, groups: vec![], accounts: vec![] });
        mine.push(vec![]);
    }
    for (m, agents) in out.iter_mut().zip(mine) {
        m.accounts = by_account(&agents, if m.home { projects } else { &[] }, order);
        if !m.home {
            let at = m.key.trim_start_matches("machine:").to_string();
            for s in &mut m.accounts {
                s.key = format!("{}@{at}", s.key);
            }
        }
    }
    out.retain(|m| m.accounts.iter().any(|s| !s.groups.is_empty()));
    sort_machines(&mut out);
    out
}

/// Paired machines with nothing to show (connecting, asleep, no agents
/// running) still get their section: `paired` is (id, label, link).
pub fn with_paired(mut sections: Vec<MachineSection>, paired: &[(String, String, LinkView)]) -> Vec<MachineSection> {
    for (id, label, link) in paired {
        if !sections.iter().any(|m| m.link.as_ref().is_some_and(|l| l.peer == *id)) {
            sections.push(MachineSection {
                key: format!("machine:peer:{id}"),
                label: label.clone(),
                home: false,
                link: Some(link.clone()),
                groups: vec![],
                accounts: vec![],
            });
        }
    }
    sort_machines(&mut sections);
    sections
}

/// Whether the sidebar names its machines: 2+ of them, or another
/// machine's Pitwall (always named, with its link). This Mac alone: no
/// machine level at all.
pub fn machine_level(sections: &[MachineSection]) -> bool {
    sections.len() > 1 || sections.iter().any(|m| m.link.is_some())
}

const ORDER: [Status; 7] = [
    Status::Blocked,
    Status::Done,
    Status::Working,
    Status::Idle,
    Status::Unknown,
    Status::Exited,
    Status::Stopped,
];

/// "1 needs you · 2 idle" for a collapsed group.
pub fn summarize(agents: &[AgentView]) -> String {
    ORDER
        .iter()
        .filter_map(|s| {
            let n = agents.iter().filter(|a| a.status == *s).count();
            (n > 0).then(|| format!("{n} {}", status_word(*s)))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Calm,
    Blocked,
    Done,
}

/// The bottom strip (`stripState`): `agents` in sidebar order.
#[derive(Debug, Clone, PartialEq)]
pub struct Strip {
    pub tone: Tone,
    pub lead: Option<AgentView>,
    /// Others in the lead's state.
    pub more: usize,
    pub working: usize,
    pub total: usize,
}

pub fn strip(agents: &[AgentView]) -> Strip {
    let of = |s: Status| agents.iter().filter(move |a| a.status == s);
    let blocked = of(Status::Blocked).count();
    let done = of(Status::Done).count();
    let lead = of(Status::Blocked)
        .next()
        .or_else(|| of(Status::Done).next())
        .cloned();
    let (tone, same) = if blocked > 0 {
        (Tone::Blocked, blocked)
    } else if done > 0 {
        (Tone::Done, done)
    } else {
        (Tone::Calm, 0)
    };
    Strip {
        tone,
        lead,
        more: same.saturating_sub(1),
        working: of(Status::Working).count(),
        total: agents.len(),
    }
}

/// Rail initials: first letters of the first two `-`/`_` parts, else the
/// first two letters.
pub fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.split(['-', '_']).filter(|p| !p.is_empty()).collect();
    let s: String = if parts.len() > 1 {
        parts[0]
            .chars()
            .take(1)
            .chain(parts[1].chars().take(1))
            .collect()
    } else {
        name.chars().take(2).collect()
    };
    s.to_uppercase()
}

/// A folder's last part as an agent name: lower-case, `[a-z0-9_-]`, starts
/// with a letter (the New-agent dialog's `slug`).
pub fn slug(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in s.to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-' {
            out.push(ch);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_start_matches(|c: char| !c.is_ascii_lowercase());
    let out: String = out.chars().take(max).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "agent".into()
    } else {
        out
    }
}

/// An unused name for an agent in `path` (`suggestName` / `agentNameFor`).
pub fn suggest_name(path: &str, taken: &[String]) -> String {
    let last = path
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|p| !p.is_empty())
        .unwrap_or("agent");
    let base = slug(last, 28);
    unique(base, taken)
}

fn unique(base: String, taken: &[String]) -> String {
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|i| format!("{base}-{i}"))
        .find(|n| !taken.contains(n))
        .expect("a free name")
}

/// A plain terminal's name: its folder's ("home" for `~`), -2, -3… when taken.
pub fn terminal_name(path: &str, taken: &[String]) -> String {
    let p = path.trim().trim_end_matches('/');
    let p = if p == "~" || p.is_empty() { "home" } else { p };
    // `terminalName` → `agentNameFor` (26 characters).
    crate::settings::onboarding::plan::agent_name_for(p, taken.iter().map(String::as_str))
}

/// Why `name` doesn't fit `pattern`/`max_len`, or `None`.
pub fn name_problem(rule: &pitwall_proto::NameRule, name: &str) -> Option<String> {
    if name.chars().count() > rule.max_len as usize {
        return Some(format!("At most {} characters", rule.max_len));
    }
    match regex::Regex::new(&rule.pattern) {
        Ok(re) if !re.is_match(name) => Some(rule.hint.clone()),
        _ => None,
    }
}

/// `~/work/alpha` → (`work/`, `alpha`) for file rows (`splitPath`).
pub fn split_path(p: &str) -> (&str, &str) {
    match p.rfind('/') {
        Some(i) => (&p[..=i], &p[i + 1..]),
        None => ("", p),
    }
}

/// "updated 12 s ago" (`updatedLabel`).
pub use crate::kit::updated_label;

/// The dev-server addresses a pane header offers ("Open preview"): none
/// once the agent stopped.
pub fn previews(a: &AgentView) -> &[String] {
    if a.running { &a.preview_urls } else { &[] }
}

/// The pane header's preview chip: the address's port (":5173").
pub fn preview_port(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split('/').next().unwrap_or(rest);
    host.rfind(':').map_or(host, |i| &host[i..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::group_by_project;
    use crate::agents::tests::agent;

    fn on(mut a: AgentView, account: Option<&str>) -> AgentView {
        a.account = account.map(String::from);
        a
    }

    #[test]
    fn a_preview_is_named_by_its_port() {
        assert_eq!(preview_port("http://localhost:5173/"), ":5173");
        assert_eq!(preview_port("https://127.0.0.1:8443/app/x"), ":8443");
        assert_eq!(preview_port("http://[::1]:4000"), ":4000");
        let mut a = agent("a", "/work/alpha", "idle", 1, true);
        a.preview_urls = vec!["http://localhost:3000".into()];
        assert_eq!(previews(&a).len(), 1);
        a.running = false;
        assert!(previews(&a).is_empty(), "gone with the agent");
    }

    #[test]
    fn grouped_by_account_then_project() {
        let agents = vec![
            on(agent("a", "/work/alpha", "idle", 1, true), Some("claude-work")),
            on(agent("b", "/work/alpha", "blocked", 2, true), None),
            on(agent("c", "/work/beta", "idle", 3, true), Some("claude-work")),
            on(agent("d", "/work/gamma", "idle", 4, true), Some("codex-play")),
            on(agent("e", "/work/alpha", "idle", 5, true), Some("claude-home")),
        ];
        let empty = Project { path: "/work/empty".into(), display: "empty".into(), is_git: true, added_at: 0 };
        let alpha = Project { path: "/work/alpha".into(), display: "alpha".into(), is_git: true, added_at: 0 };
        let order = ["claude-home".to_string(), "claude-work".to_string()];
        let s = by_account(&agents, &[empty, alpha], &order);
        let accounts: Vec<&str> = s.iter().map(|s| s.account.as_str()).collect();
        assert_eq!(accounts, ["default", "claude-home", "claude-work", "codex-play"], "Default, the user's order, then the rest");
        let projects = |i: usize| s[i].groups.iter().map(|g| g.project.clone()).collect::<Vec<_>>();
        assert_eq!(projects(0), ["/work/alpha", "/work/empty"], "only projects without any agent are added, under Default");
        assert_eq!(projects(2), ["/work/alpha", "/work/beta"]);
        assert_eq!((s[0].agents, s[0].blocked, s[2].agents), (1, 1, 2));
        assert_eq!(s[2].groups[0].key, "acct:claude-work|/work/alpha", "folds apart from the same project elsewhere");
        assert!(by_account(&[], &[], &order).is_empty());
    }

    #[test]
    fn terminals_and_empty_projects_go_with_their_projects_account() {
        crate::accounts::set_project_accounts(&[("/acct-t/alpha", "claude-work"), ("/acct-t/quiet", "claude-work")]);
        let mut term = agent("t", "/acct-t/alpha", "idle", 2, true);
        term.terminal = true;
        let agents = vec![on(agent("a", "/acct-t/alpha", "idle", 1, true), Some("claude-work")), term, agent("b", "/acct-t/beta", "idle", 3, true)];
        let quiet = Project { path: "/acct-t/quiet".into(), display: "quiet".into(), is_git: true, added_at: 0 };
        let s = by_account(&agents, &[quiet], &["claude-work".to_string()]);
        let projects = |i: usize| s[i].groups.iter().map(|g| g.project.clone()).collect::<Vec<_>>();
        assert_eq!(s.iter().map(|s| s.account.as_str()).collect::<Vec<_>>(), ["default", "claude-work"]);
        assert_eq!(projects(0), ["/acct-t/beta"], "an agent on Default stays there");
        assert_eq!(projects(1), ["/acct-t/alpha", "/acct-t/quiet"], "the terminal and the empty project follow the project's account");
        assert_eq!(s[1].agents, 2);
    }

    #[test]
    fn listed_projects_join_the_groups() {
        let groups = group_by_project(&[agent("a", "/work/beta", "idle", 0, true)]);
        let project = |path: &str, display: &str| Project {
            path: path.into(),
            display: display.into(),
            is_git: true,
            added_at: 0,
        };
        let projects = vec![
            project("/work/alpha", "alpha"),
            project("/work/beta", "beta"),
        ];
        let all = with_projects(groups, &projects);
        let names: Vec<_> = all.iter().map(|g| g.display.as_str()).collect();
        assert_eq!(names, ["alpha", "beta"]);
        assert!(all[0].agents.is_empty() && all[0].machine.is_none());
    }

    /// `studio`'s agent `id` on its account (`None`: its Default), and
    /// that account's name if studio sent one.
    fn peer(id: &str, project: &str, account: Option<&str>, name: Option<&str>) -> AgentView {
        let mut a = agent(&format!("peer:studio:{id}"), project, "idle", 1, false);
        a.machine.provider = "peer:studio".into();
        a.machine.id = "this-mac".into();
        a.machine.label = "studio".into();
        a.machine.link = Some(LinkView { peer: "studio".into(), state: pitwall_proto::peer::LinkState::Connected, since: 0, last_seen: None });
        a.account = account.map(String::from);
        a.account_name = name.map(String::from);
        a
    }

    #[test]
    fn this_mac_alone_has_no_machine_level() {
        let agents = [agent("a", "/work/a", "idle", 0, true), agent("b", "/work/b", "idle", 0, true)];
        let s = by_machine(group_by_project(&agents));
        assert_eq!(s.len(), 1);
        assert!(s[0].home && !machine_level(&s));
        assert_eq!(s[0].groups.len(), 2, "its projects as before");
        let s = by_machine_account(&agents, &[], &[]);
        assert!(!machine_level(&s));
        assert_eq!(s[0].accounts[0].key, "acct:default", "this Mac's keys as before");
        assert_eq!(s[0].accounts[0].groups[0].key, "acct:default|/work/a");
    }

    #[test]
    fn machines_then_projects() {
        let mut vm2 = agent("w", "/srv/w", "idle", 0, false);
        vm2.machine.id = "m2".into();
        vm2.machine.label = "alpha-vm".into();
        let agents = [
            peer("p1", "/Users/me/api", None, None),
            agent("v", "/srv/c", "blocked", 0, false),
            agent("a", "/work/a", "idle", 0, true),
            vm2,
        ];
        let s = by_machine(group_by_project(&agents));
        let labels: Vec<&str> = s.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["This Mac", "alpha-vm", "vm-1", "studio"], "this Mac, its machines, then other Pitwalls");
        assert!(machine_level(&s));
        assert_eq!((s[2].key.as_str(), s[2].blocked(), s[2].agents().len()), ("machine:agw:m1", 1, 1));
        assert!(s[3].link.is_some() && !s[3].home);
        // A peer alone is still named, with its link.
        assert!(machine_level(&by_machine(group_by_project(&agents[..1]))));
        // Listed projects without agents are this Mac's.
        let p = Project { path: "/work/new".into(), display: "new".into(), is_git: true, added_at: 0 };
        let s = by_machine(with_projects(group_by_project(&agents[..2]), &[p]));
        assert_eq!(s.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(), ["This Mac", "vm-1", "studio"]);
    }

    #[test]
    fn machines_then_accounts_and_a_peers_accounts_are_its_own() {
        // studio's "Work" has the same id as this Mac's: never matched.
        let agents = [
            on(agent("a", "/work/a", "idle", 1, true), Some("claude-work")),
            agent("b", "/work/b", "idle", 2, true),
            agent("v", "/srv/c", "idle", 3, false),
            peer("p1", "/Users/me/api", Some("claude-work"), Some("Work")),
            peer("p2", "/Users/me/api", None, None),
            peer("p3", "/Users/me/web", Some("codex-old"), None),
            peer("p4", "/Users/me/web", Some("other-id"), None),
        ];
        let order = ["claude-work".to_string()];
        let s = by_machine_account(&agents, &[], &order);
        assert_eq!(s.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(), ["This Mac", "vm-1", "studio"]);
        let accounts = |i: usize| s[i].accounts.iter().map(|a| a.key.clone()).collect::<Vec<_>>();
        assert_eq!(accounts(0), ["acct:default", "acct:claude-work"]);
        assert_eq!(s[0].accounts[1].agents, 1, "only this Mac's agent on this Mac's account");
        assert_eq!(accounts(1), ["acct:default@agw:m1"], "the VM's Default folds apart from this Mac's");
        assert_eq!(
            accounts(2),
            ["acct:peer:studio/default@peer:studio:this-mac", "acct:peer:studio/@Work@peer:studio:this-mac", "acct:peer:studio/~@peer:studio:this-mac"],
            "studio's Default, its accounts by name, then one for those an older studio didn't name"
        );
        let names: Vec<Option<&str>> = s[2].accounts.iter().map(|a| a.peer_name.as_deref()).collect();
        assert_eq!(names, [Some("Default"), Some("Work"), Some("Unnamed account")]);
        assert_eq!(s[2].accounts[2].agents, 2);
        assert!(s[0].accounts.iter().all(|a| a.peer_name.is_none()));
        // Each group key still says its account (drag and drop).
        assert_eq!(section_of(&s[2].accounts[1].groups[0].key), Some("peer:studio/@Work"));
    }

    #[test]
    fn a_paired_machine_without_agents_keeps_its_section() {
        let link = LinkView { peer: "studio".into(), state: pitwall_proto::peer::LinkState::Offline, since: 0, last_seen: None };
        let mut quiet = link.clone();
        quiet.peer = "quiet".into();
        let paired = [("studio".to_string(), "studio".to_string(), link), ("quiet".to_string(), "attic".to_string(), quiet)];
        let s = with_paired(by_machine(group_by_project(&[agent("a", "/work/a", "idle", 0, true), peer("p1", "/x", None, None)])), &paired);
        let labels: Vec<(&str, usize)> = s.iter().map(|m| (m.label.as_str(), m.agents().len())).collect();
        assert_eq!(labels, [("This Mac", 1), ("attic", 0), ("studio", 1)], "studio once, attic empty");
        assert_eq!(s[1].key, "machine:peer:quiet");
    }

    #[test]
    fn summaries_and_the_strip() {
        let list = vec![
            agent("a", "/w", "idle", 0, true),
            agent("b", "/w", "blocked", 0, true),
            agent("c", "/w", "idle", 0, true),
            agent("d", "/w", "done", 0, true),
            agent("e", "/w", "blocked", 0, true),
        ];
        assert_eq!(summarize(&list), "2 needs you · 1 done · 2 idle");
        let s = strip(&list);
        assert_eq!(s.tone, Tone::Blocked);
        assert_eq!(s.lead.unwrap().id, "b");
        assert_eq!(s.more, 1);
        let calm = strip(&list[..1]);
        assert_eq!((calm.tone, calm.total, calm.working), (Tone::Calm, 1, 0));
        let done = strip(&list[3..4]);
        assert_eq!((done.tone, done.more), (Tone::Done, 0));
    }

    #[test]
    fn names() {
        assert_eq!(initials("api-server"), "AS");
        assert_eq!(initials("web"), "WE");
        assert_eq!(slug("My Project!", 28), "my-project");
        assert_eq!(slug("2024 app", 28), "app");
        assert_eq!(slug("!!!", 28), "agent");
        let taken = vec!["alpha".to_string(), "alpha-2".to_string()];
        assert_eq!(suggest_name("/work/alpha/", &taken), "alpha-3");
        assert_eq!(terminal_name("~", &[]), "home");
        assert_eq!(terminal_name("~/", &["home".into()]), "home-2");
        let rule = pitwall_proto::CreateForm::pitwall_name();
        assert_eq!(name_problem(&rule, "ok-name"), None);
        assert!(name_problem(&rule, "Bad").is_some());
        assert!(name_problem(&rule, &"a".repeat(40)).unwrap().contains("32"));
    }

    #[test]
    fn the_sidebar_keeps_the_dragged_order() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let agents = vec![
            agent("a1", "/work/alpha", "idle", 1, true),
            agent("a2", "/work/alpha", "blocked", 2, true),
            agent("a3", "/work/alpha", "working", 3, true),
            agent("b1", "/work/beta", "idle", 1, true),
            agent("r1", "/srv/remote", "idle", 1, false),
        ];
        let names = |gs: &[ProjectGroup]| gs.iter().map(|g| g.display.clone()).collect::<Vec<_>>();
        let rows = |gs: &[ProjectGroup]| gs[0].agents.iter().map(|a| a.id.clone()).collect::<Vec<_>>();

        let plain = in_order(group_by_project(&agents), &[]);
        assert_eq!(rows(&plain), ["a1", "a2", "a3"], "oldest first: needs-you doesn't jump");

        let order = reorder(&[], &ids(&["/work/alpha", "/work/beta"]), "/work/beta", Some("/work/alpha"));
        let order = reorder(&order, &ids(&["a1", "a2", "a3"]), "a3", Some("a1"));
        let g = in_order(group_by_project(&agents), &order);
        assert_eq!(names(&g)[..2], ["beta", "alpha"]);
        assert_eq!(rows(&g[1..]), ["a3", "a1", "a2"]);
        assert!(!g[2].can_create, "another machine's projects stay after this Mac's");

        let last = reorder(&order, &ids(&["a3", "a1", "a2"]), "a3", None);
        assert_eq!(rows(&in_order(group_by_project(&agents), &last)[1..]), ["a1", "a2", "a3"]);
        assert_eq!(last.iter().filter(|o| *o == "a3").count(), 1);

        assert_eq!(base_key("acct:claude-work|/work/alpha"), "/work/alpha");
        assert_eq!(base_key("/work/alpha"), "/work/alpha");
        assert_eq!(section_of("acct:default|agw:m1|/srv"), Some("default"));
        assert_eq!(section_of("/work/alpha"), None);
    }

    #[test]
    fn shortcut_labels_per_desktop() {
        assert_eq!(split_path("src/app/main.rs"), ("src/app/", "main.rs"));
        assert_eq!(updated_label(12), "updated 12 s ago");
    }
}
