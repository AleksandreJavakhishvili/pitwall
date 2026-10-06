//! This Mac, for the local provider (`pitwall-providers`): process facts the
//! platform layer reads (architecture.md §9 decision 7). Nothing in the
//! engine calls these for an agent; it asks the agent's provider.

use std::time::Duration;

use crate::platform;

/// The current folder of each of `pids` that could be read (`lsof`);
/// `None` when the lookup itself failed.
pub fn process_cwds(pids: &[u32], timeout: Duration) -> Option<Vec<(u32, String)>> {
    platform::process_cwds(pids, timeout)
}

/// The current folder of process `pid`.
pub fn process_cwd(pid: u32, timeout: Duration) -> Option<String> {
    process_cwds(&[pid], timeout)?.into_iter().find(|(p, _)| *p == pid).map(|(_, cwd)| cwd)
}
