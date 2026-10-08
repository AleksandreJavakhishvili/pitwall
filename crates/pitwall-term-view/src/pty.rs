//! A local PTY as a [`TermStream`] (feature `pty`): for the example, tests
//! and benchmarks. Pitwall itself feeds terminals from its holders instead.

use std::io::{self, Read, Write};
use std::sync::Mutex;
use std::thread;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::terminal::{Feed, TermSize, TermStream};

/// A child process on a PTY of its own. Dropping it kills the child (only
/// this child: it was started here).
pub struct LocalPty {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    reader: Mutex<Option<Box<dyn Read + Send>>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
}

fn pty_size(s: TermSize) -> PtySize {
    PtySize {
        rows: s.rows,
        cols: s.cols,
        pixel_width: (s.cell_width * s.cols as f32).round() as u16,
        pixel_height: (s.cell_height * s.rows as f32).round() as u16,
    }
}

impl LocalPty {
    /// Start `cmd` on a new PTY of `size`. `TERM=xterm-256color` and
    /// `COLORTERM=truecolor` are set unless `cmd` sets them.
    pub fn spawn(mut cmd: CommandBuilder, size: TermSize) -> io::Result<LocalPty> {
        let pair = native_pty_system().openpty(pty_size(size)).map_err(io::Error::other)?;
        if cmd.get_env("TERM").is_none() {
            cmd.env("TERM", "xterm-256color");
        }
        if cmd.get_env("COLORTERM").is_none() {
            cmd.env("COLORTERM", "truecolor");
        }
        let child = pair.slave.spawn_command(cmd).map_err(io::Error::other)?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let writer = pair.master.take_writer().map_err(io::Error::other)?;
        Ok(LocalPty {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            reader: Mutex::new(Some(reader)),
            child: Mutex::new(child),
        })
    }

    /// The login shell (`$SHELL`, else `/bin/sh`) in `cwd` (or the current folder).
    pub fn shell(size: TermSize) -> io::Result<LocalPty> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut cmd = CommandBuilder::new(shell);
        cmd.arg("-l");
        if let Ok(dir) = std::env::current_dir() {
            cmd.cwd(dir);
        }
        LocalPty::spawn(cmd, size)
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.lock().unwrap().process_id()
    }
}

/// Read `reader` on a thread of its own into `feed` until EOF.
pub fn pump(mut reader: impl Read + Send + 'static, feed: Feed) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("term-read".into())
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => feed.push(&buf[..n]),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            feed.close();
        })
        .expect("spawn reader thread")
}

impl TermStream for LocalPty {
    fn attach(&self, feed: Feed) {
        if let Some(reader) = self.reader.lock().unwrap().take() {
            pump(reader, feed);
        }
    }

    fn write(&self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    fn resize(&self, size: TermSize) {
        let _ = self.master.lock().unwrap().resize(pty_size(size));
    }
}

impl Drop for LocalPty {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap();
        if let Ok(None) = child.try_wait() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::terminal::{Terminal, TerminalConfig};
    use std::time::{Duration, Instant};

    #[test]
    fn runs_a_command_and_reports_exit() {
        let mut cmd = CommandBuilder::new("printf");
        cmd.arg("ready\\n");
        let pty = LocalPty::spawn(cmd, TermSize::new(20, 3)).unwrap();
        let t = Terminal::new(pty, TermSize::new(20, 3), TerminalConfig::default());
        let start = Instant::now();
        while !t.is_closed() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(t.is_closed());
        assert!(t.screen_text().starts_with("ready"), "{:?}", t.screen_text());
    }
}
