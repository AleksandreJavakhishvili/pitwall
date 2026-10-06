//! Unix (macOS) process queries for recognising agents in terminals
//! (docs/spec/terminals.md). Each returns raw `ps` output; `crate::procs`
//! parses it. Cheap: one `ps` per call, for exactly the given pids.

use std::time::Duration;

use crate::exec::local_stdout;

const PS_TIMEOUT: Duration = Duration::from_secs(2);

fn ps_for(pids: &[u32], fields: &str) -> Option<String> {
    if pids.is_empty() {
        return Some(String::new());
    }
    let list: Vec<String> = pids.iter().map(u32::to_string).collect();
    let list = list.join(",");
    // `ps` exits 1 when one of the pids is gone; the others are still printed.
    local_stdout(&["/bin/ps", "-ww", "-o", fields, "-p", &list], PS_TIMEOUT)
}

/// `pid tpgid` per pid: the foreground process group of its terminal
/// (`ps -o pid=,tpgid=`). Parsed by `procs::parse_pid_numbers`.
pub fn terminal_groups(pids: &[u32]) -> Option<String> {
    ps_for(pids, "pid=,tpgid=")
}

/// `pid args` per pid (`ps -ww -o pid=,args=`). Parsed by `procs::parse_pid_args`.
pub fn process_args(pids: &[u32]) -> Option<String> {
    ps_for(pids, "pid=,args=")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::{parse_pid_args, parse_pid_numbers};

    #[test]
    fn reads_groups_and_args_of_given_pids_only() {
        // A harmless process of our own; killed by its exact pid below.
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().expect("spawn sleep");
        let pid = child.id();
        let args = parse_pid_args(&process_args(&[pid, std::process::id()]).unwrap());
        let groups = parse_pid_numbers(&terminal_groups(&[pid]).unwrap());
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(args.get(&pid).map(String::as_str), Some("/bin/sleep 30"));
        assert!(args.contains_key(&std::process::id()));
        assert!(groups.contains_key(&pid));
        assert_eq!(process_args(&[]).as_deref(), Some(""));
    }
}
