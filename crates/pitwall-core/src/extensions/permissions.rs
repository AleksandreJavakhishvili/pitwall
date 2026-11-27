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
/// Show the user a file or a page in Pitwall (`viewer.open`, `web.open`).
pub const SHOW: &str = "show";
/// Focus an agent's tile while Pitwall is in front (`agent.focus`).
pub const AGENTS_FOCUS: &str = "agents.focus";
/// Put text into an agent's Next up, marked as the extension's
/// (`queue.add`); never sent without the user unless they allow it.
pub const AGENTS_QUEUE: &str = "agents.queue";
/// Ask to start an agent, approved by the user every time
/// (`agent.create`).
pub const AGENTS_START: &str = "agents.start";
/// Notifications (with `contributes.notifications`).
pub const NOTIFY: &str = "notify";
/// Linear memory for its tiles above the 64 MiB every tile gets, up to
/// the size its manifest names (`{"reason": …, "mib": 256}`; docs/spec/
/// tile-sdk/permissions.md §5.1). Shown on the permission screen with
/// its size, not a switch: granted when the extension is turned on.
pub const MEMORY: &str = "memory";
/// Its tiles connect to the hosts the manifest lists (`{"reason": …,
/// "hosts": [...]}`; docs/spec/tile-sdk/permissions.md §5.1): https, and
/// plain http only to this computer.
pub const NET: &str = "net";
/// Its tiles' database may grow past the 100 MiB every extension's gets,
/// up to the size its manifest names (`{"reason": …, "mib": 500}`;
/// platform-services.md §14.2). Shown with its size, not a switch, as
/// [`MEMORY`].
pub const SQL: &str = "sql";

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
    /// What can go wrong, in plain words, for the ones that act on agents:
    /// shown beside it on the permission screen.
    pub risk_note: Option<&'static str>,
    /// This Pitwall serves it. A known one that isn't is shown as "not
    /// available in this Pitwall yet" and never granted.
    pub available: bool,
}

/// The methods that act, each with the permission an extension needs for
/// it (the daemon's method table says the same; its tests hold them
/// together). `pitwall extension test`'s fake Pitwall side refuses them
/// without the grant, as Pitwall does.
pub const ACTION_METHODS: [(&str, &str); 5] = [("queue.add", AGENTS_QUEUE), ("agent.create", AGENTS_START), ("agent.focus", AGENTS_FOCUS), ("viewer.open", SHOW), ("web.open", SHOW)];

/// The permission method `m` needs when it acts.
pub fn action_needs(m: &str) -> Option<&'static str> {
    ACTION_METHODS.iter().find(|(name, _)| *name == m).map(|(_, p)| *p)
}

/// Every permission this Pitwall knows, the ones it serves first.
pub const ALL: [Permission; 12] = [
    Permission { name: AGENTS_READ, title: "Read your agents", detail: "Their names, projects, folders, branches and statuses, and their changes as they happen. A status can quote what an agent is waiting on (a command it asks to run). Never what is on their screens.", risk: Risk::Medium, risk_note: None, available: true },
    Permission { name: PROJECTS_READ, title: "Read your projects", detail: "The folders of the projects in Pitwall.", risk: Risk::Low, risk_note: None, available: true },
    Permission { name: WORKTREES_READ, title: "Read your worktrees", detail: "Each project's git worktrees: their folders, branches and the agent working in each.", risk: Risk::Low, risk_note: None, available: true },
    Permission { name: PROFILES_READ, title: "Read profile names", detail: "The names of your profiles and of its own connections, and which of its connections each project uses. Never another extension's settings, never secrets or logins.", risk: Risk::Low, risk_note: None, available: true },
    Permission { name: SHOW, title: "Show you files and pages", detail: "Open a file (at a line) or a web page in Pitwall, next to an agent, as agents' own show tools do. It never brings Pitwall forward or takes the keyboard.", risk: Risk::Low, risk_note: None, available: true },
    Permission { name: AGENTS_FOCUS, title: "Focus an agent", detail: "Move the focus to an agent's tile while Pitwall is in front, as a click on its row does. Never brings Pitwall forward from another app.", risk: Risk::Low, risk_note: None, available: true },
    Permission { name: AGENTS_QUEUE, title: "Send text to your agents", detail: "Put text in an agent's Next up, marked with its name. Nothing it puts there is sent until you press Send, unless you let it send without asking.", risk: Risk::High, risk_note: Some("Text an agent gets becomes what it does: it acts on it with its own permissions in your repositories (edit files, run commands, push). A service that is wrong or compromised could slip instructions in."), available: true },
    Permission { name: AGENTS_START, title: "Start agents", detail: "Ask to start an agent in one of your projects, maybe with a first prompt. Every request asks you first, showing the project, the tool and the prompt in full.", risk: Risk::High, risk_note: Some("An agent it starts runs as you, with the project's accounts, in your repository. Allowed without looking, it can set work going that you didn't ask for."), available: true },
    Permission { name: NOTIFY, title: "Notify you", detail: "Show system notifications while Pitwall isn't in front, at most 3 a minute, labelled with its name.", risk: Risk::Low, risk_note: None, available: true },
    Permission { name: MEMORY, title: "Use more memory", detail: "Its tiles may use more memory than the 64 MiB every tile gets, up to the size shown. It gives no access to anything else.", risk: Risk::Low, risk_note: None, available: true },
    Permission { name: NET, title: "Connect to these servers", detail: "Its tiles may send requests to the servers listed here, exactly as written, and read the answers (https; plain http only to this computer). What its tiles have, from you or from other servers, can be sent to them.", risk: Risk::Medium, risk_note: None, available: true },
    Permission { name: SQL, title: "Keep a larger database", detail: "Its tiles' own database may grow to the size shown instead of the 100 MiB every extension gets. Only its own data: it gives no access to anything else.", risk: Risk::Low, risk_note: None, available: true },
];

/// Whether `name` is shown on the permission screen without a switch:
/// granted with the extension ([`MEMORY`], [`SQL`]).
pub fn shown_only(name: &str) -> bool {
    name == MEMORY || name == SQL
}

/// What the permission screen shows beside `name` for `m` instead of a
/// switch: the memory its tiles may use ("256 MiB").
pub fn amount(m: &Manifest, name: &str) -> Option<String> {
    match name {
        MEMORY => Some(format!("{} MiB", m.memory_mib.unwrap_or(super::TILE_MEMORY_MIB))),
        SQL => Some(format!("{} MiB", m.sql_mib.unwrap_or(super::TILE_SQL_MIB))),
        _ => None,
    }
}

/// What the permission screen lists under `name` for `m`, as the manifest
/// wrote it: `net`'s hosts.
pub fn listed(m: &Manifest, name: &str) -> Vec<String> {
    match name {
        NET => m.net_hosts.iter().map(|h| h.written()).collect(),
        _ => vec![],
    }
}

/// Permission `name`, when this Pitwall knows it.
pub fn find(name: &str) -> Option<&'static Permission> {
    ALL.iter().find(|p| p.name == name)
}

/// Whether this Pitwall serves `name` (it can be granted).
pub fn available(name: &str) -> bool {
    find(name).is_some_and(|p| p.available)
}

/// Whether the permission screen offers `name` switched on: the ones that
/// only read (and notifications), yes; any that acts (showing, focusing,
/// queueing, starting), never.
pub fn on_by_default(name: &str) -> bool {
    find(name).is_some_and(|p| p.available && p.risk != Risk::High) && !ACTION_METHODS.iter().any(|(_, p)| *p == name)
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
/// serves and the user granted (and the ones granted with it, like
/// [`MEMORY`]), in the manifest's order.
pub fn effective(m: &Manifest, decisions: &Decisions) -> Vec<String> {
    m.permissions.iter().filter(|p| available(&p.name) && (shown_only(&p.name) || decisions.get(&p.name).is_some_and(|d| d.granted))).map(|p| p.name.clone()).collect()
}

/// The permissions `m` asks for that the user hasn't answered yet (and
/// could grant): a new install's, or ones an update added.
pub fn undecided<'a>(m: &'a Manifest, decisions: &Decisions) -> Vec<&'a super::PermissionAsk> {
    m.permissions.iter().filter(|p| available(&p.name) && !shown_only(&p.name) && !decisions.contains_key(&p.name)).collect()
}

/// `decisions` without the ones `m` no longer asks for (an update dropped
/// them: forgotten, so asking again later asks the user again).
pub fn kept(m: &Manifest, decisions: &Decisions) -> Decisions {
    decisions.iter().filter(|(n, _)| m.permissions.iter().any(|p| p.name == **n)).map(|(n, d)| (n.clone(), *d)).collect()
}

/// `decisions` after an update from `old` to `new` (docs/spec/tile-sdk/
/// packaging.md §6.4): [`kept`], and a grant whose permission now asks
/// for more forgotten, so it is asked again before it is used: `net`
/// listing a host it didn't.
pub fn after_update(old: &Manifest, new: &Manifest, decisions: &Decisions) -> Decisions {
    let mut out = kept(new, decisions);
    let was: Vec<String> = old.net_hosts.iter().map(|h| h.written()).collect();
    if new.net_hosts.iter().any(|h| !was.contains(&h.written())) {
        out.remove(NET);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An update that adds a `net` host asks again; one that keeps them
    /// keeps the grant.
    #[test]
    fn a_new_host_is_asked_again() {
        let net = |hosts: &str| crate::extensions::parse(&format!(r#"{{"v": 2, "id": "x", "name": "X", "permissions": {{"net": {{"reason": "Its API", "hosts": [{hosts}]}}}}, "contributes": {{"tiles": [{{"id": "m", "title": "M", "wasm": "t.wasm"}}]}}}}"#)).unwrap();
        let mut d = Decisions::new();
        d.insert(NET.into(), Decision { granted: true, at: 1 });
        let (a, ab) = (net(r#""api.example.com""#), net(r#""api.example.com", "evil.example.net""#));
        assert!(after_update(&a, &a, &d).contains_key(NET));
        assert!(after_update(&ab, &a, &d).contains_key(NET), "fewer hosts: kept");
        assert!(!after_update(&a, &ab, &d).contains_key(NET), "a new host: asked again");
    }
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
        assert_eq!(names(undecided(&m, &d)), [AGENTS_READ, PROJECTS_READ, AGENTS_QUEUE], "only ones this Pitwall can grant are asked");
        d.insert(AGENTS_READ.into(), Decision { granted: true, at: 1 });
        d.insert(PROJECTS_READ.into(), Decision { granted: false, at: 1 });
        // One this Pitwall doesn't know, even marked granted by hand, isn't
        // served; sending text to agents is, once granted.
        d.insert("weather.read".into(), Decision { granted: true, at: 1 });
        assert_eq!(names(undecided(&m, &d)), [AGENTS_QUEUE]);
        assert_eq!(effective(&m, &d), [AGENTS_READ]);
        d.insert(AGENTS_QUEUE.into(), Decision { granted: true, at: 1 });
        assert_eq!(effective(&m, &d), [AGENTS_READ, AGENTS_QUEUE]);
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
        // Memory is granted with the extension, shown with its size.
        assert!(available(MEMORY) && on_by_default(MEMORY) && shown_only(MEMORY) && !shown_only(AGENTS_READ));
        let big = Manifest { memory_mib: Some(256), ..Manifest::named("x", "X") };
        assert_eq!((amount(&big, MEMORY).as_deref(), amount(&big, NOTIFY)), (Some("256 MiB"), None));
        let asks = Manifest { permissions: vec![PermissionAsk { name: MEMORY.into(), reason: "big files".into() }], ..big };
        assert_eq!(effective(&asks, &Decisions::new()), vec![MEMORY.to_string()], "granted with the extension");
        assert!(undecided(&asks, &Decisions::new()).is_empty(), "never asked on its own");
        for p in [SHOW, AGENTS_FOCUS, AGENTS_QUEUE, AGENTS_START] {
            assert!(!on_by_default(p), "{p} acts: never on by default");
        }
        assert_eq!(unavailable_why(AGENTS_READ), None);
        assert_eq!(unavailable_why(SHOW), None);
        for p in [SHOW, AGENTS_FOCUS, AGENTS_QUEUE, AGENTS_START] {
            assert!(available(p), "{p} is served");
        }
        // The two that act on agents say what can go wrong.
        assert!(find(AGENTS_QUEUE).unwrap().risk_note.is_some() && find(AGENTS_START).unwrap().risk_note.is_some());
        assert!(ALL.iter().filter(|p| p.risk == Risk::High).all(|p| p.risk_note.is_some()));
        assert_eq!(unavailable_why("weather.read"), Some("not available in this Pitwall"));
        assert_eq!((of_topic("attention"), of_topic("blocked"), of_topic("approvals")), (Some(AGENTS_READ), Some(AGENTS_READ), None));
        let d = serde_json::to_value(Decision { granted: true, at: 7 }).unwrap();
        assert_eq!(d, serde_json::json!({"granted": true, "at": 7}));
        for (i, p) in ALL.iter().enumerate() {
            assert!(ALL[i + 1..].iter().all(|q| q.name != p.name), "{} twice", p.name);
        }
    }
}
