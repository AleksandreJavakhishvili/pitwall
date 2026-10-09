//! Pure helpers of the main screen: sidebar groups with listed projects,
//! machine headings, collapsed summaries, the status strip, rail initials,
//! agent and terminal names, shortcut labels (ports of `src/lib/groups.ts`,
//! `statusStrip.ts`, `terminals.ts`, `host.ts` `keys()` and the New-agent
//! dialog's name helpers).

use pitwall_core::onboarding::project_list::Project;
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

fn machine_key(m: &MachineView) -> String {
    format!("{}:{}", m.provider, m.id)
}

/// The machine heading above `groups[i]`: when agents run on 2+ machines,
/// each machine's first group names it.
pub fn machine_heading(groups: &[ProjectGroup], i: usize) -> Option<String> {
    let mut keys: Vec<String> = groups
        .iter()
        .filter_map(|g| g.machine.as_ref().map(machine_key))
        .collect();
    keys.sort();
    keys.dedup();
    if keys.len() < 2 {
        return None;
    }
    let mine = groups.get(i)?.machine.as_ref().map(machine_key)?;
    let first = groups
        .iter()
        .position(|g| g.machine.as_ref().map(machine_key).as_deref() == Some(mine.as_str()))?;
    (first == i)
        .then(|| groups[i].machine.as_ref().map(|m| m.label.clone()))
        .flatten()
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

/// Changes polled this often while the panel is shown.
pub const CHANGES_POLL: std::time::Duration = std::time::Duration::from_secs(5);

/// "updated 12 s ago" (`updatedLabel`).
pub use crate::kit::updated_label;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::group_by_project;
    use crate::agents::tests::agent;

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

    #[test]
    fn machine_headings_only_with_two_machines() {
        let one = group_by_project(&[agent("a", "/work/a", "idle", 0, true)]);
        assert_eq!(machine_heading(&one, 0), None);
        let two = group_by_project(&[
            agent("a", "/work/a", "idle", 0, true),
            agent("b", "/work/b", "idle", 0, true),
            agent("v", "/srv/c", "idle", 0, false),
        ]);
        assert_eq!(machine_heading(&two, 0).as_deref(), Some("This Mac"));
        assert_eq!(machine_heading(&two, 1), None);
        assert_eq!(machine_heading(&two, 2).as_deref(), Some("vm-1"));
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
    fn shortcut_labels_per_desktop() {
        assert_eq!(split_path("src/app/main.rs"), ("src/app/", "main.rs"));
        assert_eq!(updated_label(12), "updated 12 s ago");
    }
}
