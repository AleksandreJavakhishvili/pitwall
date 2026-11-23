//! Named permissions (docs/spec/extension-api.md, "Permissions"): what
//! Pitwall gives an extension. A format 2 manifest names the ones it asks
//! for, each with its own reason; the user grants or refuses each, and the
//! decision is kept with its date ([`Decision`], in `ui.json`). Only a
//! permission this Pitwall serves, that the manifest asks for and that the
//! user granted counts ([`effective`]): one added by an update is not
//! granted until the user agrees, one a manifest no longer asks for is
//! forgotten.
//!
//! A permission bounds what Pitwall *gives*: which methods answer, which
//! events arrive. It doesn't stop the adapter, a program running as the
//! user, from reading files or reaching the network itself.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Manifest;

/// Read the agents: their names, projects, paths, branches and statuses.
pub const AGENTS_READ: &str = "agents.read";
/// Read Pitwall's projects (their folders).
pub const PROJECTS_READ: &str = "projects.read";
/// Read each project's git worktrees.
pub const WORKTREES_READ: &str = "worktrees.read";
/// Read profile and connection names (never settings, secrets or logins).
pub const PROFILES_READ: &str = "profiles.read";
/// Show the user a file or a page (a later Pitwall).
pub const SHOW: &str = "show";
/// Focus an agent's tile (a later Pitwall).
pub const AGENTS_FOCUS: &str = "agents.focus";
/// Put text into an agent's Next up (a later Pitwall).
pub const AGENTS_QUEUE: &str = "agents.queue";
/// Start an agent (a later Pitwall).
pub const AGENTS_START: &str = "agents.start";
/// Notifications (with `contributes.notifications`).
pub const NOTIFY: &str = "notify";

/// How much a permission lets an extension do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Low,
    Medium,
    High,
}

/// One permission Pitwall knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permission {
    pub name: &'static str,
    /// What it allows, as the permission screen says it ("Read your
    /// agents").
    pub title: &'static str,
    /// What it gives, and what that can leak, in one or two sentences.
    pub detail: &'static str,
    pub risk: Risk,
    /// This Pitwall serves it. A known one that isn't is shown as "not
    /// available in this Pitwall yet" and never granted.
    pub available: bool,
}

/// Every permission this Pitwall knows, the ones it serves first.
pub const ALL: [Permission; 9] = [
    Permission { name: AGENTS_READ, title: "Read your agents", detail: "Their names, projects, folders, branches and statuses, and their changes as they happen. A status can quote what an agent is waiting on (a command it asks to run). Never what is on their screens.", risk: Risk::Medium, available: true },
    Permission { name: PROJECTS_READ, title: "Read your projects", detail: "The folders of the projects in Pitwall.", risk: Risk::Low, available: true },
    Permission { name: WORKTREES_READ, title: "Read your worktrees", detail: "Each project's git worktrees: their folders, branches and the agent working in each.", risk: Risk::Low, available: true },
    Permission { name: PROFILES_READ, title: "Read profile names", detail: "The names of your profiles and of its own connections, and which of its connections each project uses. Never another extension's settings, never secrets or logins.", risk: Risk::Low, available: true },
    Permission { name: SHOW, title: "Show you files and pages", detail: "Open a file or a web page in Pitwall's viewer.", risk: Risk::Low, available: false },
    Permission { name: AGENTS_FOCUS, title: "Focus an agent", detail: "Bring an agent's tile forward when you click its item.", risk: Risk::Low, available: false },
    Permission { name: AGENTS_QUEUE, title: "Send text to your agents", detail: "Put text in an agent's Next up. Text an agent gets becomes what it does, with its own permissions in your repositories.", risk: Risk::High, available: false },
    Permission { name: AGENTS_START, title: "Start agents", detail: "Start an agent in a project, as you.", risk: Risk::High, available: false },
    Permission { name: NOTIFY, title: "Notify you", detail: "Show system notifications while Pitwall isn't in front, at most 3 a minute, labelled with its name.", risk: Risk::Low, available: true },
];

/// Permission `name`, when this Pitwall knows it.
pub fn find(name: &str) -> Option<&'static Permission> {
    ALL.iter().find(|p| p.name == name)
}

/// Whether this Pitwall serves `name` (it can be granted).
pub fn available(name: &str) -> bool {
    find(name).is_some_and(|p| p.available)
}

/// Whether the permission screen offers `name` switched on: the ones that
/// only read, yes; sending text to agents and starting them, never.
pub fn on_by_default(name: &str) -> bool {
    find(name).is_some_and(|p| p.available && p.risk != Risk::High)
}

/// Why `name` can't be granted here, in a few words; `None` when it can.
pub fn unavailable_why(name: &str) -> Option<&'static str> {
    match find(name) {
        Some(p) if p.available => None,
        Some(_) => Some("not available in this Pitwall yet"),
        None => Some("not available in this Pitwall"),
    }
}

/// The permission an event topic needs (`events.subscribe`).
pub fn of_topic(topic: &str) -> Option<&'static str> {
    match topic {
        "agents" | "attention" | "blocked" => Some(AGENTS_READ),
        _ => None,
    }
}

/// The user's answer to one permission, and when (unix seconds): a record,
/// so the dial can widen later without a new model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub granted: bool,
    #[serde(default)]
    pub at: u64,
}

impl Decision {
    /// Granted or refused now.
    pub fn now(granted: bool) -> Decision {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        Decision { granted, at }
    }
}

/// The decisions kept for an extension, by permission name.
pub type Decisions = BTreeMap<String, Decision>;

/// What `m` may use now: the permissions it asks for that this Pitwall
/// serves and the user granted, in the manifest's order.
pub fn effective(m: &Manifest, decisions: &Decisions) -> Vec<String> {
    m.permissions.iter().filter(|p| available(&p.name) && decisions.get(&p.name).is_some_and(|d| d.granted)).map(|p| p.name.clone()).collect()
}

/// The permissions `m` asks for that the user hasn't answered yet (and
/// could grant): a new install's, or ones an update added.
pub fn undecided<'a>(m: &'a Manifest, decisions: &Decisions) -> Vec<&'a super::PermissionAsk> {
    m.permissions.iter().filter(|p| available(&p.name) && !decisions.contains_key(&p.name)).collect()
}

/// `decisions` without the ones `m` no longer asks for (an update dropped
/// them: forgotten, so asking again later asks the user again).
pub fn kept(m: &Manifest, decisions: &Decisions) -> Decisions {
    decisions.iter().filter(|(n, _)| m.permissions.iter().any(|p| p.name == **n)).map(|(n, d)| (n.clone(), *d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::{PermissionAsk, Manifest};

    fn asking(names: &[&str]) -> Manifest {
        Manifest { permissions: names.iter().map(|n| PermissionAsk { name: n.to_string(), reason: format!("to use {n}") }).collect(), ..Manifest::named("x", "X") }
    }

    #[test]
    fn only_granted_known_ones_count() {
        let m = asking(&[AGENTS_READ, PROJECTS_READ, AGENTS_QUEUE, "weather.read"]);
        let mut d = Decisions::new();
        assert!(effective(&m, &d).is_empty(), "nothing until the user answers");
        let names = |v: Vec<&PermissionAsk>| v.into_iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(undecided(&m, &d)), [AGENTS_READ, PROJECTS_READ], "only ones this Pitwall can grant are asked");
        d.insert(AGENTS_READ.into(), Decision { granted: true, at: 1 });
        d.insert(PROJECTS_READ.into(), Decision { granted: false, at: 1 });
        // A later permission, even marked granted by hand, isn't served.
        d.insert(AGENTS_QUEUE.into(), Decision { granted: true, at: 1 });
        d.insert("weather.read".into(), Decision { granted: true, at: 1 });
        assert_eq!(effective(&m, &d), [AGENTS_READ]);
        assert!(undecided(&m, &d).is_empty(), "refused is answered");
        // An update adds one: it isn't granted until answered.
        let newer = asking(&[AGENTS_READ, PROJECTS_READ, WORKTREES_READ]);
        assert_eq!(effective(&newer, &d), [AGENTS_READ]);
        assert_eq!(names(undecided(&newer, &d)), [WORKTREES_READ]);
        // One dropped is forgotten.
        let fewer = asking(&[PROJECTS_READ]);
        assert_eq!(kept(&fewer, &d).keys().collect::<Vec<_>>(), [PROJECTS_READ]);
    }

    #[test]
    fn defaults_and_words() {
        assert!(on_by_default(AGENTS_READ) && on_by_default(PROFILES_READ) && on_by_default(NOTIFY));
        assert!(!on_by_default(AGENTS_QUEUE) && !on_by_default(AGENTS_START), "never on by default");
        assert_eq!(unavailable_why(AGENTS_READ), None);
        assert_eq!(unavailable_why(SHOW), Some("not available in this Pitwall yet"));
        assert_eq!(unavailable_why("weather.read"), Some("not available in this Pitwall"));
        assert_eq!((of_topic("attention"), of_topic("blocked"), of_topic("approvals")), (Some(AGENTS_READ), Some(AGENTS_READ), None));
        let d = serde_json::to_value(Decision { granted: true, at: 7 }).unwrap();
        assert_eq!(d, serde_json::json!({"granted": true, "at": 7}));
        for (i, p) in ALL.iter().enumerate() {
            assert!(ALL[i + 1..].iter().all(|q| q.name != p.name), "{} twice", p.name);
        }
    }
}
