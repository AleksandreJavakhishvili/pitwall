//! A program started in a process group of its own (an extension's
//! adapter: docs/spec/extensions.md, "Adapter protocol"), its stdin,
//! stdout and stderr piped, so that stopping it stops whatever it started
//! too. Unix: its own process group (`setpgid(0, 0)` at start), the group
//! killed with `SIGKILL`. Windows: no console window, and a job object it
//! is put in right after it starts, killed with `TerminateJobObject` (and
//! by the system when Pitwall's handle closes: `KILL_ON_JOB_CLOSE`).
//! A program that leaves its group on purpose (`setsid`, breaking away
//! from the job) isn't stopped: only a sandbox stops that.

use std::io;
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};

/// A started program and its group.
pub(crate) struct Grouped {
    child: Child,
    /// Its group was killed (once only: its id may be reused after).
    killed: bool,
    /// The job it is in (a `HANDLE`; `None` when it couldn't be put in one).
    #[cfg(windows)]
    job: Option<isize>,
}

/// Start `program` with `args` in `dir`, `PATH` set to `path` when given,
/// in a group of its own, its stdio piped.
pub(crate) fn spawn_grouped(program: &Path, args: &[String], dir: &Path, path: Option<&str>) -> io::Result<(Grouped, ChildStdin, ChildStdout, ChildStderr)> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(p) = path {
        cmd.env("PATH", p);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;
    let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("no pipes"));
    };
    #[cfg(windows)]
    let grouped = Grouped { job: windows_job::put_in_job(&child), child, killed: false };
    #[cfg(not(windows))]
    let grouped = Grouped { child, killed: false };
    Ok((grouped, stdin, stdout, stderr))
}

impl Grouped {
    /// Its process id.
    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    /// `Some(exit code)` once it has exited (`Some(None)`: by a signal).
    pub(crate) fn exited(&mut self) -> Option<Option<i32>> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code()),
            Ok(None) => None,
            Err(_) => Some(None),
        }
    }

    /// Kill it and everything still in its group, then reap it (once).
    pub(crate) fn kill_group(&mut self) {
        if std::mem::replace(&mut self.killed, true) {
            return;
        }
        #[cfg(unix)]
        {
            // Its group's id is its pid (`process_group(0)`). The group
            // outlives it while something it started is in it.
            let pgid = self.child.id() as libc::pid_t;
            if pgid > 0 {
                // SAFETY: a plain syscall on a group this process made.
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
        }
        #[cfg(windows)]
        {
            if let Some(job) = self.job {
                windows_job::terminate(job);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(windows)]
impl Drop for Grouped {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            windows_job::close(job);
        }
    }
}

#[cfg(windows)]
mod windows_job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};

    /// A new job that kills its processes when its last handle closes,
    /// with `child` in it (`None` when that fails: it is then stopped on
    /// its own).
    pub(super) fn put_in_job(child: &Child) -> Option<isize> {
        // SAFETY: plain Win32 calls on a handle we own; the struct is
        // zeroed and sized as the call asks.
        unsafe {
            let job: HANDLE = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const core::ffi::c_void, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
            if set == 0 || AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) == 0 {
                CloseHandle(job);
                return None;
            }
            Some(job as isize)
        }
    }

    pub(super) fn terminate(job: isize) {
        // SAFETY: a job handle this module made and hasn't closed.
        unsafe {
            TerminateJobObject(job as HANDLE, 1);
        }
    }

    pub(super) fn close(job: isize) {
        // SAFETY: as above; closed once (taken out of `Grouped`).
        unsafe {
            CloseHandle(job as HANDLE);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only asks whether the process is there.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// What the program started stops with it: its group is killed.
    #[test]
    fn stopping_stops_what_it_started() {
        let t = crate::testing::TempDir::new("group-kill");
        let script = "sleep 30 &\necho $!\nwait\n";
        let (mut g, stdin, stdout, _err) = spawn_grouped(Path::new("/bin/sh"), &["-c".into(), script.into()], t.path(), None).unwrap();
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line).unwrap();
        let child: i32 = line.trim().parse().unwrap();
        assert!(alive(child), "its child runs");
        assert_eq!(g.exited(), None);
        drop(stdin);
        g.kill_group();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // Not our child: it is reaped by init, so it may linger a moment.
        while alive(child) && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!alive(child), "its child stopped with it");
        assert!(g.exited().is_some());
    }
}
