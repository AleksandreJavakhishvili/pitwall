//! The welcome screen's choices as pure functions (a port of
//! `src/components/onboarding/projects.ts`, with its tests): default
//! selections, agent names, and what "Start Pitwall" creates.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use pitwall_core::onboarding::scan::{Conversation, RunningAgent, ScannedProject};
use pitwall_proto::ScannedSession;

const DAY_MS: u64 = 24 * 3600 * 1000;
/// Projects used this recently are ticked by default (max [`DEFAULT_MAX`]).
pub const RECENT_PROJECT_MS: u64 = 30 * DAY_MS;
pub const DEFAULT_MAX: usize = 12;
/// Conversations used this recently are ticked by default.
pub const RECENT_CONVERSATION_MS: u64 = 3 * DAY_MS;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `kind:session` (`convKey`).
pub fn conv_key(kind: &str, session_id: &str) -> String {
    format!("{kind}:{session_id}")
}

/// `provider:machine/native` (`sessionKey`).
pub fn session_key(s: &ScannedSession) -> String {
    format!("{}:{}/{}", s.provider, s.machine, s.native)
}

/// "3m ago", "2d ago" (`relTime`).
pub fn rel_time(ms: u64, now: u64) -> String {
    let s = (now.saturating_sub(ms) as f64 / 1000.0).round() as u64;
    if s < 10 {
        return "just now".into();
    }
    if s < 60 {
        return format!("{s}s ago");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m ago");
    }
    let h = m / 60;
    if h < 24 {
        return format!("{h}h ago");
    }
    format!("{}d ago", h / 24)
}

/// Default ticked projects: recent agent projects (≤ 30 days), max 12.
pub fn default_selection(projects: &[ScannedProject], now: u64) -> BTreeSet<String> {
    projects
        .iter()
        .filter(|p| !p.added)
        .filter(|p| {
            p.last_used
                .is_some_and(|t| now.saturating_sub(t) < RECENT_PROJECT_MS)
        })
        .filter(|p| p.agent_history)
        .take(DEFAULT_MAX)
        .map(|p| p.path.clone())
        .collect()
}

/// `^[a-z][a-z0-9_-]{0,31}$` (`NAME_RE`).
pub fn valid_name(n: &str) -> bool {
    let mut chars = n.chars();
    matches!(chars.next(), Some('a'..='z'))
        && n.len() <= 32
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-'))
}

/// A valid, unused agent name for `project_path` (`agentNameFor`).
pub fn agent_name_for<'a>(project_path: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let used: HashSet<&str> = taken.into_iter().collect();
    let last = project_path
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .next_back()
        .unwrap_or("agent")
        .to_lowercase();
    // Runs of anything else become one "-".
    let mut base = String::new();
    for ch in last.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-' {
            base.push(ch);
        } else if !base.ends_with('-') {
            base.push('-');
        }
    }
    let base: String = base
        .trim_start_matches(|c: char| !c.is_ascii_lowercase())
        .trim_end_matches('-')
        .chars()
        .take(26)
        .collect();
    let base = if base.is_empty() || !valid_name(&base) {
        "agent".to_string()
    } else {
        base
    };
    if !used.contains(base.as_str()) {
        return base;
    }
    (2..)
        .map(|i| format!("{base}-{i}"))
        .find(|n| !used.contains(n.as_str()))
        .expect("a free name")
}

/// Initial "Show under project…" choices: what the user picked last time.
pub fn remembered_show_under(
    conversations: &[Conversation],
    running: &[RunningAgent],
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for c in conversations {
        if let (true, Some(d)) = (c.outside_project, &c.display_project) {
            out.insert(conv_key(&c.kind, &c.session_id), d.clone());
        }
    }
    for r in running {
        if let (true, Some(s), Some(d)) = (r.outside_project, &r.session_id, &r.display_project) {
            out.insert(conv_key(&r.kind, s), d.clone());
        }
    }
    out
}

/// A Pitwall agent already works in `path` (its folder or its repo).
pub fn has_agent_in(path: &str, agents: &[(String, Option<String>)]) -> bool {
    agents
        .iter()
        .any(|(cwd, project)| cwd == path || project.as_deref() == Some(path))
}

/// Default ticked conversations: for each ticked project without a Pitwall
/// agent, its newest conversation if used in the last 3 days. Never: in
/// Pitwall, outside a project, open elsewhere, or of a kind not installed.
pub fn default_conversations(
    conversations: &[Conversation],
    ticked_projects: &BTreeSet<String>,
    agents: &[(String, Option<String>)],
    installed: &HashSet<String>,
    now: u64,
) -> BTreeSet<String> {
    let mut newest: HashMap<&str, &Conversation> = HashMap::new();
    for c in conversations {
        if c.in_pitwall || c.outside_project || c.running_elsewhere || !installed.contains(&c.kind)
        {
            continue;
        }
        let e = newest.entry(c.project_path.as_str()).or_insert(c);
        if c.last_used > e.last_used {
            *e = c;
        }
    }
    newest
        .into_iter()
        .filter(|(path, _)| ticked_projects.contains(*path) && !has_agent_in(path, agents))
        .filter(|(_, c)| now.saturating_sub(c.last_used) < RECENT_CONVERSATION_MS)
        .map(|(_, c)| conv_key(&c.kind, &c.session_id))
        .collect()
}

/// One agent "Start Pitwall" creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAgent {
    pub name: String,
    pub kind: String,
    /// Where it runs (a conversation's own cwd).
    pub project_path: String,
    /// Resume this session; `None` → a fresh agent.
    pub session_id: Option<String>,
    /// Sidebar project when it differs ("Show under project…").
    pub display_project: Option<String>,
}

pub struct PlanInput<'a> {
    pub conversations: &'a [Conversation],
    pub conv_sel: &'a BTreeSet<String>,
    pub running: &'a [RunningAgent],
    pub run_sel: &'a BTreeSet<u32>,
    /// Project path → kind, for projects with "Start a new agent" on.
    pub fresh: &'a BTreeMap<String, String>,
    pub taken: Vec<String>,
    pub show_under: &'a HashMap<String, String>,
}

/// What "Start Pitwall" creates, in order: ticked conversations, ticked
/// running sessions (each session once), then a fresh agent for each
/// project with "Start a new agent" on and nothing else starting there.
pub fn plan_agents(input: PlanInput<'_>) -> Vec<PlannedAgent> {
    struct Acc {
        used: HashSet<String>,
        sessions: HashSet<String>,
        busy: HashSet<String>,
        out: Vec<PlannedAgent>,
    }
    let mut acc = Acc {
        used: input.taken.into_iter().collect(),
        sessions: HashSet::new(),
        busy: HashSet::new(),
        out: Vec::new(),
    };
    let shown = |kind: &str, sid: &str, outside: bool| -> Option<String> {
        if !outside {
            return None;
        }
        input
            .show_under
            .get(&conv_key(kind, sid))
            .filter(|s| !s.is_empty())
            .cloned()
    };
    fn push(acc: &mut Acc, kind: &str, path: &str, sid: Option<&str>, display: Option<String>) {
        if let Some(s) = sid {
            if !acc.sessions.insert(s.to_string()) {
                return;
            }
        }
        let shown_as = display.clone().unwrap_or_else(|| path.to_string());
        let name = agent_name_for(&shown_as, acc.used.iter().map(String::as_str));
        acc.used.insert(name.clone());
        acc.busy.insert(shown_as);
        acc.out.push(PlannedAgent {
            name,
            kind: kind.into(),
            project_path: path.into(),
            session_id: sid.map(Into::into),
            display_project: display,
        });
    }
    for c in input.conversations {
        if input.conv_sel.contains(&conv_key(&c.kind, &c.session_id)) && !c.in_pitwall {
            let d = shown(&c.kind, &c.session_id, c.outside_project);
            push(&mut acc, &c.kind, &c.project_path, Some(&c.session_id), d);
        }
    }
    for r in input.running {
        if let (true, Some(sid), Some(cwd), false) = (
            input.run_sel.contains(&r.pid),
            &r.session_id,
            &r.cwd,
            r.in_pitwall,
        ) {
            let d = shown(&r.kind, sid, r.outside_project);
            push(&mut acc, &r.kind, cwd, Some(sid), d);
        }
    }
    for (path, kind) in input.fresh {
        if !acc.busy.contains(path) {
            push(&mut acc, kind, path, None, None);
        }
    }
    acc.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::onboarding::scan::RulesInfo;

    const NOW: u64 = 1_700_000_000_000;
    const H: u64 = 3600 * 1000;

    fn project(path: &str, last_used: Option<u64>, history: bool, added: bool) -> ScannedProject {
        ScannedProject {
            path: path.into(),
            display: path.into(),
            is_git: true,
            last_used,
            sources: vec!["claude"],
            agent_history: history,
            added,
            rules: RulesInfo::default(),
        }
    }

    fn conv(kind: &str, sid: &str, path: &str, ago: u64) -> Conversation {
        Conversation {
            kind: kind.into(),
            kind_name: kind.into(),
            session_id: sid.into(),
            project_path: path.into(),
            project_display: path.into(),
            title: "fix the build".into(),
            last_used: NOW - ago,
            in_pitwall: false,
            outside_project: false,
            display_project: None,
            running_elsewhere: false,
        }
    }

    #[test]
    fn default_projects_are_recent_agent_projects() {
        let ps = vec![
            project("/w/a", Some(NOW - 2 * 24 * H), true, false),
            project("/w/old", Some(NOW - 40 * 24 * H), true, false),
            project("/w/nohist", Some(NOW - H), false, false),
            project("/w/added", Some(NOW - H), true, true),
            project("/w/never", None, true, false),
        ];
        let sel = default_selection(&ps, NOW);
        assert_eq!(
            sel.into_iter().collect::<Vec<_>>(),
            vec!["/w/a".to_string()]
        );
        let many: Vec<_> = (0..20)
            .map(|i| project(&format!("/w/p{i}"), Some(NOW - H), true, false))
            .collect();
        assert_eq!(default_selection(&many, NOW).len(), DEFAULT_MAX);
    }

    #[test]
    fn names_are_valid_and_unique() {
        assert_eq!(agent_name_for("/code/My Project", []), "my-project");
        assert_eq!(agent_name_for("/code/api", ["api"]), "api-2");
        assert_eq!(agent_name_for("/code/api", ["api", "api-2"]), "api-3");
        assert_eq!(agent_name_for("/code/123", []), "agent");
        assert_eq!(agent_name_for("/", []), "agent");
        assert_eq!(agent_name_for("/x/9lives", []), "lives");
        let long = agent_name_for("/x/abcdefghijklmnopqrstuvwxyzabcdef", []);
        assert_eq!(long.len(), 26);
        assert!(valid_name(&long));
        assert!(!valid_name("Agent") && !valid_name("1a") && valid_name("a_b-1"));
    }

    #[test]
    fn default_conversations_take_the_newest_recent_one_per_ticked_project() {
        let convs = vec![
            conv("claude", "s1", "/w/a", 5 * H),
            conv("claude", "s2", "/w/a", H),
            conv("codex", "s3", "/w/b", H),
            conv("claude", "s4", "/w/c", 5 * 24 * H),
        ];
        let ticked: BTreeSet<String> = ["/w/a", "/w/b", "/w/c"]
            .into_iter()
            .map(String::from)
            .collect();
        let installed: HashSet<String> = ["claude".to_string()].into();
        let got = default_conversations(&convs, &ticked, &[], &installed, NOW);
        assert_eq!(
            got.into_iter().collect::<Vec<_>>(),
            vec!["claude:s2".to_string()]
        );
        // A project that already has a Pitwall agent gets nothing.
        let agents = vec![("/w/a".to_string(), None)];
        assert!(default_conversations(&convs, &ticked, &agents, &installed, NOW).is_empty());
    }

    #[test]
    fn the_plan_starts_conversations_then_running_then_fresh_agents() {
        let mut outside = conv("claude", "s9", "/home/u", H);
        outside.outside_project = true;
        let convs = vec![conv("claude", "s1", "/w/a", H), outside];
        let running = vec![RunningAgent {
            pid: 42,
            kind: "codex".into(),
            kind_name: "Codex".into(),
            cwd: Some("/w/b".into()),
            cwd_display: Some("~/w/b".into()),
            session_id: Some("s1".into()),
            title: None,
            in_pitwall: false,
            outside_project: false,
            display_project: None,
        }];
        let conv_sel: BTreeSet<String> = ["claude:s1".to_string(), "claude:s9".to_string()].into();
        let run_sel: BTreeSet<u32> = [42].into();
        let fresh: BTreeMap<String, String> = [
            ("/w/a".to_string(), "claude".to_string()),
            ("/w/c".to_string(), "codex".to_string()),
        ]
        .into();
        let show_under: HashMap<String, String> =
            [("claude:s9".to_string(), "/w/c".to_string())].into();
        let plan = plan_agents(PlanInput {
            conversations: &convs,
            conv_sel: &conv_sel,
            running: &running,
            run_sel: &run_sel,
            fresh: &fresh,
            taken: vec!["a".into()],
            show_under: &show_under,
        });
        let names: Vec<_> = plan.iter().map(|p| p.name.as_str()).collect();
        // s1 once (the running copy is the same session); /w/a and /w/c are
        // covered, so no fresh agents; the outside one shows under /w/c.
        assert_eq!(names, vec!["a-2", "c"]);
        assert_eq!(plan[1].project_path, "/home/u");
        assert_eq!(plan[1].display_project.as_deref(), Some("/w/c"));
        assert_eq!(plan[0].session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn relative_times() {
        assert_eq!(rel_time(NOW - 3000, NOW), "just now");
        assert_eq!(rel_time(NOW - 30_000, NOW), "30s ago");
        assert_eq!(rel_time(NOW - 5 * 60_000, NOW), "5m ago");
        assert_eq!(rel_time(NOW - 3 * H, NOW), "3h ago");
        assert_eq!(rel_time(NOW - 50 * H, NOW), "2d ago");
    }

    #[test]
    fn remembered_choices_and_keys() {
        let mut c = conv("claude", "s1", "/home/u", H);
        c.outside_project = true;
        c.display_project = Some("/w/a".into());
        let m = remembered_show_under(&[c], &[]);
        assert_eq!(m.get("claude:s1").map(String::as_str), Some("/w/a"));
        assert!(has_agent_in(
            "/w/a",
            &[("/w/a/sub".into(), Some("/w/a".into()))]
        ));
    }
}
