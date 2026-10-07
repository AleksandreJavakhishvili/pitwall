//! [`Exec`] on this machine: `std::process` and `std::fs`. The only place in
//! the core (besides `platform/`) that starts processes.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{Cmd, Exec, FileKind, Out, PwError, Result, Stat};
use crate::platform;

/// This machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalExec;

fn err(path: &str, e: std::io::Error) -> PwError {
    PwError::from(e).context(path)
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
}

impl Exec for LocalExec {
    fn run(&self, cmd: &Cmd) -> Result<Out> {
        let (program, args) = cmd.argv.split_first().ok_or("empty command")?;
        let mut c = Command::new(program);
        platform::hide_console(&mut c);
        c.args(args)
            .stdin(if cmd.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = cmd.cwd {
            c.current_dir(dir);
        }
        for (k, v) in cmd.env {
            c.env(k, v);
        }
        let mut child = c.spawn().map_err(|e| PwError::from(e).context(program))?;
        let writer = cmd.stdin.zip(child.stdin.take()).map(|(bytes, mut pipe)| {
            let bytes = bytes.to_vec();
            std::thread::spawn(move || {
                let _ = pipe.write_all(&bytes);
            })
        });
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        // `None`: no deadline (a timeout too large to add).
        let deadline = Instant::now().checked_add(cmd.timeout);
        let mut nap = Duration::from_millis(1);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if deadline.is_none_or(|d| Instant::now() < d) => {
                    std::thread::sleep(nap);
                    nap = (nap * 2).min(Duration::from_millis(25));
                }
                res => {
                    // Only ever the child we spawned ourselves.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(match res {
                        Err(e) => PwError::from(e).context(program),
                        _ => PwError::other(format!("{program}: timed out after {}s", cmd.timeout.as_secs_f32())),
                    });
                }
            }
        };
        if let Some(w) = writer {
            let _ = w.join();
        }
        Ok(Out {
            status: status.code().unwrap_or(-1),
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
        })
    }

    fn read_file(&self, path: &str, max: u64) -> Result<Option<Vec<u8>>> {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(err(path, e)),
        };
        let mut buf = Vec::new();
        file.take(max).read_to_end(&mut buf).map_err(|e| err(path, e))?;
        Ok(Some(buf))
    }

    fn write_file(&self, path: &str, bytes: &[u8]) -> Result<()> {
        if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| err(&dir.to_string_lossy(), e))?;
        }
        std::fs::write(path, bytes).map_err(|e| err(path, e))
    }

    fn remove_file(&self, path: &str) -> Result<()> {
        std::fs::remove_file(path).map_err(|e| err(path, e))
    }

    fn remove_dir(&self, path: &str) -> Result<()> {
        std::fs::remove_dir(path).map_err(|e| err(path, e))
    }

    fn copy_file(&self, from: &str, to: &str) -> Result<()> {
        std::fs::copy(from, to).map(|_| ()).map_err(|e| err(from, e))
    }

    fn stat(&self, path: &str) -> Result<Option<Stat>> {
        match std::fs::symlink_metadata(path) {
            Ok(m) => {
                let t = m.file_type();
                let kind = if t.is_symlink() {
                    FileKind::Symlink
                } else if t.is_dir() {
                    FileKind::Dir
                } else if t.is_file() {
                    FileKind::File
                } else {
                    FileKind::Other
                };
                Ok(Some(Stat { kind, len: m.len() }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(err(path, e)),
        }
    }

    fn real_path(&self, path: &str) -> Result<String> {
        platform::canonicalize(std::path::Path::new(path))
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| err(path, e))
    }

    fn temp_dir(&self) -> Result<String> {
        Ok(std::env::temp_dir().to_string_lossy().into_owned())
    }

    fn home(&self) -> Result<String> {
        Ok(platform::home_dir().to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{exists, is_dir, join};
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn runs_with_args_cwd_env_and_stdin() {
        let dir = TempDir::new("exec");
        let cwd = LocalExec.real_path(&dir.path().to_string_lossy()).unwrap();
        let out = LocalExec
            .run(&Cmd::new(&["sh", "-c", "pwd; printf %s \"$PW_X\"; cat; echo oops >&2; exit 3"])
                .cwd(&cwd)
                .env(&[("PW_X", "x=1")])
                .stdin(b"in\n"))
            .unwrap();
        assert_eq!(out.status, 3);
        assert!(!out.ok());
        assert_eq!(out.stdout_text(), format!("{cwd}\nx=1in\n"));
        assert_eq!(out.stderr_text(), "oops");
        assert!(LocalExec.run(&Cmd::new(&[])).is_err());
        assert!(LocalExec.run(&Cmd::new(&["/no/such/program"])).unwrap_err().starts_with("/no/such/program: "));
    }

    #[test]
    fn kills_its_own_child_on_timeout() {
        let started = Instant::now();
        let res = LocalExec.run(&Cmd::new(&["sleep", "5"]).timeout(Duration::from_millis(100)));
        assert!(res.unwrap_err().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(3));
        // A timeout too large for a deadline means none.
        assert!(LocalExec.run(&Cmd::new(&["true"]).timeout(Duration::MAX)).unwrap().ok());
    }

    #[test]
    fn files_round_trip() {
        let dir = TempDir::new("exec-files");
        let root = dir.path().to_string_lossy().into_owned();
        let f = join(&root, "a/b/c.txt");
        assert_eq!(LocalExec.read_file(&f, 10).unwrap(), None);
        assert_eq!(LocalExec.stat(&f).unwrap(), None);
        LocalExec.write_file(&f, b"hello world").unwrap();
        assert_eq!(LocalExec.read_file(&f, 100).unwrap().as_deref(), Some(&b"hello world"[..]));
        assert_eq!(LocalExec.read_file(&f, 5).unwrap().as_deref(), Some(&b"hello"[..]), "cut at max");
        assert_eq!(LocalExec.stat(&f).unwrap(), Some(Stat { kind: FileKind::File, len: 11 }));
        assert!(is_dir(&LocalExec, &join(&root, "a/b")) && !is_dir(&LocalExec, &f));
        let g = join(&root, "copy.txt");
        LocalExec.copy_file(&f, &g).unwrap();
        assert!(exists(&LocalExec, &g));
        assert!(LocalExec.remove_dir(&join(&root, "a/b")).is_err(), "not empty");
        LocalExec.remove_file(&f).unwrap();
        LocalExec.remove_dir(&join(&root, "a/b")).unwrap();
        assert!(!exists(&LocalExec, &join(&root, "a/b")));
        assert!(LocalExec.real_path(&join(&root, "gone")).is_err());
        assert!(!LocalExec.temp_dir().unwrap().is_empty() && !LocalExec.home().unwrap().is_empty());
    }
}
