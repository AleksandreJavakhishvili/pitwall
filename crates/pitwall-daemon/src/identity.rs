//! Who is calling. The server asks the OS for the connecting process (the
//! socket's peer pid), walks up its parents, and looks for the process one
//! of Pitwall's terminal holders runs for an agent: a CLI started inside an
//! agent or terminal descends from it. Nothing the caller says about itself
//! (`PITWALL_AGENT_ID`, the hello's role) counts.

use std::collections::HashMap;
use std::sync::Arc;

use pitwall_core::Shared;
use pitwall_proto::{Requester, RequesterKind};

use crate::extensions::Registry;
use crate::platform;

/// A connection's caller, as the server established it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// The connecting process (`None`: the OS didn't say).
    pub pid: Option<u32>,
    /// The Pitwall agent it runs in: (id, name).
    pub agent: Option<(String, String)>,
    /// The connecting program's name.
    pub process: Option<String>,
    /// Pitwall's own UI: may answer approvals.
    pub ui: bool,
    /// The extension (id, name) whose adapter it is or descends from: its
    /// permissions apply ([`crate::extensions`]).
    pub extension: Option<(String, String)>,
}

impl Caller {
    /// Someone outside Pitwall's agents (pid unknown).
    pub fn outside() -> Caller {
        Caller { pid: None, agent: None, process: None, ui: false, extension: None }
    }

    /// How the approval dialog names it.
    pub fn requester(&self) -> Requester {
        if let Some((_, name)) = &self.extension {
            return Requester { kind: RequesterKind::Extension, agent_id: None, name: name.clone(), pid: self.pid, process: self.process.clone() };
        }
        match &self.agent {
            Some((id, name)) => Requester {
                kind: RequesterKind::Agent,
                agent_id: Some(id.clone()),
                name: name.clone(),
                pid: self.pid,
                process: self.process.clone(),
            },
            None => Requester {
                kind: RequesterKind::Outside,
                agent_id: None,
                name: "A terminal outside Pitwall".into(),
                pid: self.pid,
                process: self.process.clone(),
            },
        }
    }

    /// Remembered approvals are kept per key: each agent is its own caller;
    /// everything outside Pitwall's agents is one.
    pub fn key(&self) -> String {
        if let Some((id, _)) = &self.extension {
            return format!("ext:{id}");
        }
        match &self.agent {
            Some((id, _)) => format!("agent:{id}"),
            None => "outside".into(),
        }
    }
}

/// Turns a peer pid into a [`Caller`]. The server's is [`ProcessIdentity`];
/// tests plug in their own.
pub trait Identify: Send + Sync {
    fn identify(&self, pid: Option<u32>) -> Caller;
}

/// One process: its parent and program name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub ppid: u32,
    pub name: String,
}

/// `pid` and its parents, nearest first (stops at pid 1, a loop, or a
/// process missing from `table`).
pub fn ancestors(pid: u32, table: &HashMap<u32, Proc>) -> Vec<u32> {
    let mut chain = vec![pid];
    let mut at = pid;
    while let Some(p) = table.get(&at) {
        if p.ppid <= 1 || chain.contains(&p.ppid) {
            break;
        }
        chain.push(p.ppid);
        at = p.ppid;
    }
    chain
}

/// The agent the nearest ancestor in `chain` belongs to. `agents`: (agent
/// id, name, pid of the process its holder runs).
pub fn agent_of(chain: &[u32], agents: &[(String, String, u32)]) -> Option<(String, String)> {
    chain.iter().find_map(|pid| agents.iter().find(|(_, _, p)| p == pid).map(|(id, name, _)| (id.clone(), name.clone())))
}

/// An id and its name.
pub type Named = (String, String);

/// Who `chain` (a process and its parents, nearest first) is: the agent
/// and the extension whose process is nearest in it. A process an
/// extension's adapter started inside an agent is the extension's; an
/// adapter's process is never an agent's.
pub fn attribute(chain: &[u32], agents: &[(String, String, u32)], extensions: Option<&Registry>) -> (Option<Named>, Option<Named>) {
    let agent_at = chain.iter().position(|pid| agents.iter().any(|(_, _, p)| p == pid));
    let ext = extensions.and_then(|r| r.of_chain(chain));
    match (agent_at, ext) {
        (Some(a), Some((e, x))) if e <= a => (None, Some(x)),
        (None, Some((_, x))) => (None, Some(x)),
        _ => (agent_of(chain, agents), None),
    }
}

/// The real thing: the process table, the engine's agents and the
/// extensions' adapters.
pub struct ProcessIdentity {
    engine: Shared,
    extensions: Option<Arc<Registry>>,
}

impl ProcessIdentity {
    pub fn new(engine: Shared) -> Arc<ProcessIdentity> {
        Arc::new(ProcessIdentity { engine, extensions: None })
    }

    /// [`ProcessIdentity::new`] that also tells processes of extensions'
    /// adapters (`extensions`).
    pub fn with_extensions(engine: Shared, extensions: Arc<Registry>) -> Arc<ProcessIdentity> {
        Arc::new(ProcessIdentity { engine, extensions: Some(extensions) })
    }
}

impl Identify for ProcessIdentity {
    fn identify(&self, pid: Option<u32>) -> Caller {
        let Some(pid) = pid else { return Caller::outside() };
        let table = platform::process_table().unwrap_or_default();
        let chain = ancestors(pid, &table);
        let (agent, extension) = attribute(&chain, &self.engine.agent_pids(), self.extensions.as_deref());
        let process = table.get(&pid).map(|p| p.name.clone());
        // In-process server: only this very process is Pitwall's UI. (A
        // standalone daemon will check the signed app's path instead.)
        let ui = pid == std::process::id() && agent.is_none() && extension.is_none();
        Caller { pid: Some(pid), agent, process, ui, extension }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(rows: &[(u32, u32)]) -> HashMap<u32, Proc> {
        rows.iter().map(|&(pid, ppid)| (pid, Proc { ppid, name: format!("p{pid}") })).collect()
    }

    #[test]
    fn walks_up_to_the_agent_its_holder_runs() {
        // launchd(1) → holder(100) → shell(101) → claude(102) → sh(103) → pitwall(104)
        let t = table(&[(100, 1), (101, 100), (102, 101), (103, 102), (104, 103), (200, 1), (201, 200)]);
        assert_eq!(ancestors(104, &t), [104, 103, 102, 101, 100]);
        let agents = vec![("a1".to_string(), "api".to_string(), 101), ("a2".into(), "web".into(), 201)];
        assert_eq!(agent_of(&ancestors(104, &t), &agents), Some(("a1".into(), "api".into())));
        assert_eq!(agent_of(&ancestors(201, &t), &agents), Some(("a2".into(), "web".into())));
        // Outside every agent (the user's own terminal), or a process that
        // isn't in the table any more.
        assert_eq!(agent_of(&ancestors(100, &t), &agents), None);
        assert_eq!(ancestors(999, &t), [999]);
    }

    #[test]
    fn the_nearest_agent_wins_and_loops_end() {
        // A terminal (301) running an agent that is itself tracked (303).
        let t = table(&[(301, 1), (302, 301), (303, 302), (304, 303)]);
        let agents = vec![("outer".to_string(), "term".to_string(), 301), ("inner".into(), "claude".into(), 303)];
        assert_eq!(agent_of(&ancestors(304, &t), &agents).unwrap().0, "inner");
        let looped = table(&[(5, 6), (6, 5)]);
        assert_eq!(ancestors(5, &looped), [5, 6]);
    }

    /// Decision 5: a process whose parent chain reaches an adapter Pitwall
    /// started is that extension's caller; the user's own terminal stays
    /// outside; a detached process (reparented to launchd) is outside too.
    #[test]
    fn a_process_an_adapter_started_is_its_extensions() {
        // launchd(1) → pitwall(10) → adapter(20) → node(21) → pitwall-cli(22)
        //            → terminal(30) → shell(31) → pitwall-cli(32)
        //            → detached(40), started by the adapter, now under launchd
        let t = table(&[(10, 1), (20, 10), (21, 20), (22, 21), (30, 1), (31, 30), (32, 31), (40, 1), (50, 10), (51, 50), (52, 51)]);
        let reg = Registry::new();
        reg.set("ci", "CI", vec!["agents.read".into()], None);
        reg.adapter_started("ci", 20);
        let agents = vec![("a1".to_string(), "api".to_string(), 50)];
        let who = |pid| attribute(&ancestors(pid, &t), &agents, Some(&reg));
        assert_eq!(who(22), (None, Some(("ci".into(), "CI".into()))));
        assert_eq!(who(20).1.unwrap().0, "ci", "the adapter itself");
        assert_eq!(who(32), (None, None), "the user's own terminal");
        assert_eq!(who(40), (None, None), "detached: not stopped by this");
        assert_eq!(who(52), (Some(("a1".into(), "api".into())), None), "an agent's process stays the agent's");
        // An adapter started inside an agent: its processes are the
        // extension's (the nearest wins).
        reg.adapter_started("ci", 51);
        assert_eq!(who(52).1.unwrap().0, "ci");
        assert_eq!(attribute(&ancestors(22, &t), &agents, None), (None, None), "no registry: as before");
        let c = Caller { extension: Some(("ci".into(), "CI".into())), pid: Some(22), ..Caller::outside() };
        assert_eq!((c.key(), c.requester().kind, c.requester().name), ("ext:ci".to_string(), RequesterKind::Extension, "CI".to_string()));
    }

    #[test]
    fn callers_are_named_for_the_dialog() {
        let c = Caller { pid: Some(7), agent: Some(("a1".into(), "api".into())), process: Some("pitwall".into()), ui: false, extension: None };
        let r = c.requester();
        assert_eq!((r.kind, r.name.as_str(), r.agent_id.as_deref()), (RequesterKind::Agent, "api", Some("a1")));
        assert_eq!(c.key(), "agent:a1");
        assert_eq!(Caller::outside().key(), "outside");
        assert_eq!(Caller::outside().requester().kind, RequesterKind::Outside);
    }

    /// The real process table: this test's own process has a parent.
    #[test]
    fn reads_the_process_table() {
        let t = platform::process_table().expect("ps works");
        let me = std::process::id();
        assert!(t.contains_key(&me));
        assert!(ancestors(me, &t).len() >= 2);
    }
}
