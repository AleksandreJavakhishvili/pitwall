//! Process facts from a Linux `/proc` tree, in the same text shapes the
//! macOS `ps` / `lsof` queries return, so `crate::procs` parses both alike.
//! The root is a parameter: tests read a fixture tree
//! (`fixtures/proc/`) on any Unix; Linux reads `/proc`.
//!
//! No subprocesses: a few small file reads per process. A process can vanish
//! between listing and reading; it is then skipped, like `ps` would.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::procs::{cmdline_args, parse_proc_stat, ProcStat};

pub struct ProcFs {
    root: PathBuf,
}

impl ProcFs {
    pub fn new(root: impl Into<PathBuf>) -> ProcFs {
        ProcFs { root: root.into() }
    }

    fn dir(&self, pid: u32) -> PathBuf {
        self.root.join(pid.to_string())
    }

    /// Every numeric entry of the root, ascending.
    pub fn pids(&self) -> Vec<u32> {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return Vec::new() };
        let mut pids: Vec<u32> = entries.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect();
        pids.sort_unstable();
        pids
    }

    pub fn stat(&self, pid: u32) -> Option<ProcStat> {
        parse_proc_stat(&std::fs::read_to_string(self.dir(pid).join("stat")).ok()?)
    }

    /// The command line as `ps -o args=` shows it.
    pub fn args(&self, pid: u32, comm: &str) -> Option<String> {
        Some(cmdline_args(&std::fs::read(self.dir(pid).join("cmdline")).ok()?, comm))
    }

    /// The process's current folder; `None` when unreadable (another
    /// user's process) or deleted since.
    pub fn cwd(&self, pid: u32) -> Option<String> {
        let target = std::fs::read_link(self.dir(pid).join("cwd")).ok()?;
        let target = target.to_string_lossy().into_owned();
        (!target.ends_with(" (deleted)")).then_some(target)
    }

    /// `pid ppid args` per process, like `ps -axww -o pid=,ppid=,args=`.
    pub fn table(&self) -> Option<String> {
        if !self.root.is_dir() {
            return None;
        }
        let mut out = String::new();
        for pid in self.pids() {
            let Some(st) = self.stat(pid) else { continue };
            let Some(args) = self.args(pid, &st.comm) else { continue };
            let _ = writeln!(out, "{pid} {} {args}", st.ppid);
        }
        Some(out)
    }

    /// `pid tpgid` per pid, like `ps -o pid=,tpgid= -p …`.
    pub fn terminal_groups(&self, pids: &[u32]) -> String {
        let mut out = String::new();
        for &pid in pids {
            if let Some(st) = self.stat(pid) {
                let _ = writeln!(out, "{pid} {}", st.tpgid);
            }
        }
        out
    }

    /// `pid args` per pid, like `ps -ww -o pid=,args= -p …`.
    pub fn process_args(&self, pids: &[u32]) -> String {
        let mut out = String::new();
        for &pid in pids {
            let Some(st) = self.stat(pid) else { continue };
            if let Some(args) = self.args(pid, &st.comm) {
                let _ = writeln!(out, "{pid} {args}");
            }
        }
        out
    }

    /// `(pid, cwd)` of each of `pids` whose folder could be read.
    pub fn cwds(&self, pids: &[u32]) -> Vec<(u32, String)> {
        pids.iter().filter_map(|&pid| Some((pid, self.cwd(pid)?))).collect()
    }
}

/// The fixture tree (`fixtures/proc/`): a terminal holder running bash
/// running claude, a codex in a deleted folder, a kernel thread, a process
/// that vanished mid-read.
#[cfg(test)]
pub(crate) fn fixture() -> ProcFs {
    ProcFs::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform/fixtures/proc"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::{parse_pid_args, parse_pid_numbers, parse_table, ProcRow};

    #[test]
    fn lists_processes_like_ps() {
        let fs = fixture();
        assert_eq!(fs.pids(), [1, 2, 3900, 4100, 4242, 4300, 5000]);
        let rows = parse_table(&fs.table().unwrap());
        let row = |pid: u32, ppid: u32, args: &str| ProcRow { pid, ppid, args: args.into() };
        assert_eq!(
            rows,
            vec![
                row(1, 0, "/sbin/init splash"),
                row(2, 0, "[kthreadd]"),
                row(3900, 1, "/usr/bin/pitwall-hold --socket /home/dev/.local/share/pitwall/run/hold/a1.sock"),
                row(4100, 3900, "-bash"),
                row(4242, 4100, "claude --resume s9"),
                row(4300, 1, "node /usr/lib/node_modules/@openai/codex/bin/codex.js"),
            ],
            "5000 has no stat (gone mid-read) and `sys` is not a process"
        );
        assert_eq!(ProcFs::new("/nonexistent/proc").table(), None);
    }

    #[test]
    fn terminal_groups_and_args_of_given_pids_only() {
        let fs = fixture();
        let groups = parse_pid_numbers(&fs.terminal_groups(&[4100, 4300, 9999]));
        assert_eq!(groups.len(), 2);
        assert_eq!((groups[&4100], groups[&4300]), (4242, -1));
        let args = parse_pid_args(&fs.process_args(&[4242, 5000]));
        assert_eq!(args.len(), 1);
        assert_eq!(args[&4242], "claude --resume s9");
        assert_eq!(fs.terminal_groups(&[]), "");
    }

    #[test]
    fn current_folders() {
        let fs = fixture();
        assert_eq!(
            fs.cwds(&[4100, 4242, 4300, 2, 9999]),
            vec![(4100, "/home/dev/code/orders-api".to_string()), (4242, "/home/dev/My Projects/web app".to_string())],
            "deleted, unreadable and missing folders are left out"
        );
    }
}
