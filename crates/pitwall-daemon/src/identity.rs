//! Who is calling. The server asks the OS for the connecting process (the
//! socket's peer pid), walks up its parents, and looks for the process one
//! of Pitwall's terminal holders runs for an agent: a CLI started inside an
//! agent or terminal descends from it. Nothing the caller says about itself
//! (`PITWALL_AGENT_ID`, the hello's role) counts.

use std::collections::HashMap;
use std::sync::Arc;

use pitwall_core::Shared;
use pitwall_proto::{Requester, RequesterKind};

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
}

impl Caller {
    /// Someone outside Pitwall's agents (pid unknown).
    pub fn outside() -> Caller {
        Caller { pid: None, agent: None, process: None, ui: false }
    }

    /// How the approval dialog names it.
    pub fn requester(&self) -> Requester {
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

/// The real thing: the process table and the engine's agents.
pub struct ProcessIdentity {
    engine: Shared,
}

impl ProcessIdentity {
    pub fn new(engine: Shared) -> Arc<ProcessIdentity> {
        Arc::new(ProcessIdentity { engine })
    }
}

impl Identify for ProcessIdentity {
    fn identify(&self, pid: Option<u32>) -> Caller {
        let Some(pid) = pid else { return Caller::outside() };
        let table = platform::process_table().unwrap_or_default();
        let chain = ancestors(pid, &table);
        let agent = agent_of(&chain, &self.engine.agent_pids());
        let process = table.get(&pid).map(|p| p.name.clone());
        // In-process server: only this very process is Pitwall's UI. (A
        // standalone daemon will check the signed app's path instead.)
        let ui = pid == std::process::id() && agent.is_none();
        Caller { pid: Some(pid), agent, process, ui }
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

    #[test]
    fn callers_are_named_for_the_dialog() {
        let c = Caller { pid: Some(7), agent: Some(("a1".into(), "api".into())), process: Some("pitwall".into()), ui: false };
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
