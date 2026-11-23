//! An adapter's process (docs/spec/extensions.md, "Adapter protocol"):
//! started in its extension's folder in a process group (a job, on
//! Windows) of its own, its stdin, stdout and stderr piped, and stopped
//! with everything it started. The app's adapters and
//! `pitwall extension test` both start adapters this way; the OS work is
//! in `platform/group.rs`.

use std::io::{self, Read, Write};
use std::path::Path;

/// A started adapter.
pub struct Process {
    inner: crate::platform::Grouped,
}

/// Its pipes: lines to it, its lines, its stderr.
pub struct Pipes {
    pub stdin: Box<dyn Write + Send>,
    pub stdout: Box<dyn Read + Send>,
    pub stderr: Box<dyn Read + Send>,
}

/// Start `program` (found already: [`super::plan::resolve`]) with `args` in
/// `dir`, with `PATH` set to `path` when given.
pub fn start(program: &Path, args: &[String], dir: &Path, path: Option<&str>) -> io::Result<(Process, Pipes)> {
    let (inner, stdin, stdout, stderr) = crate::platform::spawn_grouped(program, args, dir, path)?;
    Ok((Process { inner }, Pipes { stdin: Box::new(stdin), stdout: Box::new(stdout), stderr: Box::new(stderr) }))
}

impl Process {
    /// Its process id.
    pub fn id(&self) -> u32 {
        self.inner.id()
    }

    /// `Some(exit code)` once it has exited (`Some(None)`: killed by a
    /// signal); `None` while it runs.
    pub fn exited(&mut self) -> Option<Option<i32>> {
        self.inner.exited()
    }

    /// Kill it and whatever it started that is still in its group.
    pub fn stop(&mut self) {
        self.inner.kill_group();
    }
}
