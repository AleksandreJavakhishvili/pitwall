//! A terminal in a `pitwall-hold` holder, as a raw byte pipe ([`TermIo`]).
//! The holder owns the PTY and the process, so closing this connection
//! never ends the process — only an explicit [`close`](TermIo::close)
//! (hang-up, then kill) does. The local provider runs agents in holders;
//! the agw provider runs its attachments (`agw session attach`) in them.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pitwall_core::provider::{ExitInfo, PwError, Result, TermIo, TermSize};
use pitwall_hold::client::{self, Conn, Launch, Reader};
use pitwall_hold::{proto, Msg};

/// How long a holder may take to answer the handshake.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Environment variables a parent Claude Code session leaks into children.
pub fn is_claude_session_var(key: &str) -> bool {
    key == "CLAUDECODE" || key.starts_with("CLAUDE_CODE_")
}

pub fn launch_script(command_line: &str) -> String {
    format!("exec {command_line}")
}

/// What to start in a new holder.
pub struct Spawn<'a> {
    pub agent_id: &'a str,
    pub cwd: &'a str,
    /// Shell fragment run as `exec <command_line>` in a login shell.
    pub command_line: &'a str,
    pub size: TermSize,
    pub socket: &'a Path,
    /// The `pitwall-hold` executable.
    pub holder: &'a Path,
    /// Passed to the agent as `PITWALL_SOCKET` (where its hooks post).
    pub hook_socket: &'a Path,
    /// Passed to the agent as `PITWALL_CLI_SOCKET` (where the `pitwall`
    /// CLI run inside it connects).
    pub cli_socket: &'a Path,
}

/// Start the agent in a new holder (through the user's login shell) and
/// connect to it.
pub fn spawn(spec: Spawn) -> Result<HoldTerm> {
    let (program, args) = pitwall_core::shell::login_invocation(&launch_script(spec.command_line));
    let env = [
        ("PITWALL_ENV", OsStr::new("1")),
        ("PITWALL_AGENT_ID", OsStr::new(spec.agent_id)),
        ("PITWALL_SOCKET", spec.hook_socket.as_os_str()),
        ("PITWALL_CLI_SOCKET", spec.cli_socket.as_os_str()),
    ];
    let program = Program { program: &program, args: &args, cwd: Some(Path::new(spec.cwd)), env: &env };
    run(spec.holder, spec.socket, spec.size, program).map_err(|e| PwError::other(format!("could not start agent: {e}")))?;
    // Even if it already exited: its output and exit still come through.
    HoldTerm::connect(spec.socket, true).map_err(|e| PwError::other(format!("could not connect to the agent's terminal: {e}")))
}

/// A program to run in a holder, as it is (no shell around it).
pub struct Program<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub cwd: Option<&'a Path>,
    /// Added to the environment (which is Pitwall's own, minus a parent
    /// Claude Code session's variables, with a 256-colour `TERM`).
    pub env: &'a [(&'a str, &'a OsStr)],
}

/// Start `p` in a new holder listening on `socket`.
pub fn run(holder: &Path, socket: &Path, size: TermSize, p: Program) -> io::Result<()> {
    let args: Vec<OsString> = p.args.iter().map(OsString::from).collect();
    let mut cmd = client::command(
        holder,
        &Launch {
            socket,
            cols: size.cols,
            rows: size.rows,
            cwd: p.cwd,
            grace: None,
            program: OsStr::new(p.program),
            args: &args,
        },
    );
    // The holder passes its environment to the process unchanged.
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(is_claude_session_var) {
            cmd.env_remove(key);
        }
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    for (k, v) in p.env {
        cmd.env(k, v);
    }
    client::launch(&mut cmd).map(|_| ())
}

/// Exit status and "the connection ended", shared with the reader.
#[derive(Default)]
struct End {
    ended: AtomicBool,
    code: Mutex<Option<i32>>,
}

impl End {
    fn finish(&self, code: Option<i32>) {
        if let Some(c) = code {
            *lock(&self.code) = Some(c);
        }
        self.ended.store(true, Ordering::Relaxed);
    }
    fn ended(&self) -> bool {
        self.ended.load(Ordering::Relaxed)
    }
}

pub struct HoldTerm {
    /// Frames to the holder go through a writer thread so callers never block.
    tx: Mutex<Sender<Vec<u8>>>,
    history: Vec<u8>,
    /// Live output that arrived before the replay was known to be complete.
    early: Vec<u8>,
    reader: Option<Reader>,
    end: Arc<End>,
    pid: u32,
    holder_pid: u32,
    size: TermSize,
}

impl HoldTerm {
    /// Connect to the holder at `socket`. Without `allow_exited`, a holder
    /// whose agent already exited counts as nothing to attach to
    /// (`NotFound`).
    pub fn connect(socket: &Path, allow_exited: bool) -> io::Result<HoldTerm> {
        let mut conn = client::connect(socket, CONNECT_TIMEOUT)?;
        let info = conn.info;
        if let (Some(code), false) = (info.exit, allow_exited) {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("the agent already exited ({code})")));
        }
        // History first (output from before we connected), then live output.
        // STATUS is answered after every REPLAY frame, so INFO marks where
        // history ends.
        conn.set_recv_timeout(Some(CONNECT_TIMEOUT))?;
        conn.send(&proto::attach(true))?;
        conn.send(&proto::status())?;
        let end = Arc::new(End::default());
        let (mut history, mut early) = (Vec::new(), Vec::new());
        loop {
            match conn.recv()? {
                Some(Msg::Replay(b)) => history.extend_from_slice(&b),
                Some(Msg::Output(b)) => early.extend_from_slice(&b),
                Some(Msg::Exit(code)) => *lock(&end.code) = Some(code),
                Some(Msg::Info(_)) => break,
                Some(_) => {}
                None => {
                    end.finish(None);
                    break;
                }
            }
        }
        conn.set_recv_timeout(None)?;
        let exited = lock(&end.code).is_some();
        Self::start(conn, history, early, end, exited, info)
    }

    fn start(conn: Conn, history: Vec<u8>, early: Vec<u8>, end: Arc<End>, exited: bool, info: pitwall_hold::Info) -> io::Result<HoldTerm> {
        let (reader, mut writer) = conn.split();
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new().name("hold-in".into()).spawn(move || {
            for frame in rx {
                if writer.write_all(&frame).is_err() {
                    break;
                }
            }
        })?;
        Ok(HoldTerm {
            tx: Mutex::new(tx),
            history,
            early,
            // An agent that is already gone has nothing more to say.
            reader: (!exited && !end.ended()).then_some(reader),
            end,
            pid: info.child_pid,
            holder_pid: info.holder_pid,
            size: TermSize::new(info.cols, info.rows),
        })
    }

    fn send(&self, frame: Vec<u8>) -> Result<()> {
        lock(&self.tx).send(frame).map_err(|_| PwError::not_running())
    }
}

/// Live output from the holder; EOF on EXIT or when the holder goes away.
struct HoldReader {
    pending: Vec<u8>,
    stream: Option<Reader>,
    end: Arc<End>,
}

impl Read for HoldReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if !self.pending.is_empty() {
                let n = buf.len().min(self.pending.len());
                buf[..n].copy_from_slice(&self.pending[..n]);
                self.pending.drain(..n);
                return Ok(n);
            }
            let Some(stream) = self.stream.as_mut() else {
                self.end.finish(None);
                return Ok(0);
            };
            match proto::read_frame(stream) {
                Ok(Some((ty, body))) => match Msg::decode(ty, body) {
                    Ok(Msg::Output(b)) | Ok(Msg::Replay(b)) => self.pending = b,
                    Ok(Msg::Exit(code)) => {
                        self.stream = None;
                        self.end.finish(Some(code));
                        return Ok(0);
                    }
                    Ok(_) => {}
                    Err(_) => self.stream = None,
                },
                Ok(None) | Err(_) => self.stream = None,
            }
        }
    }
}

struct HoldWriter(Sender<Vec<u8>>);

impl Write for HoldWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.send(proto::input(buf)).map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl TermIo for HoldTerm {
    fn take_history(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.history)
    }

    fn take_reader(&mut self) -> Box<dyn Read + Send> {
        Box::new(HoldReader { pending: std::mem::take(&mut self.early), stream: self.reader.take(), end: self.end.clone() })
    }

    fn take_writer(&mut self) -> Box<dyn Write + Send> {
        Box::new(HoldWriter(lock(&self.tx).clone()))
    }

    fn resize(&self, size: TermSize) -> Result<()> {
        self.send(proto::resize(size.cols, size.rows))
    }

    fn try_wait(&self) -> Result<Option<ExitInfo>> {
        Ok(self.end.ended().then(|| ExitInfo { code: *lock(&self.end.code) }))
    }

    /// Hang up, let the holder force the process after `grace`, and wait for
    /// it. A holder that doesn't answer is killed by its exact pid.
    fn close(&self, grace: Duration) {
        if self.end.ended() {
            return;
        }
        let _ = self.send(proto::shutdown(grace.as_millis().min(u32::MAX as u128) as u32));
        let deadline = Instant::now() + grace + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.end.ended() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // Still connected, so these pids are still the holder and its child.
        client::force_kill(self.pid);
        client::force_kill(self.holder_pid);
        self.end.finish(None);
    }

    fn pid(&self) -> Option<u32> {
        Some(self.pid)
    }

    fn eof_is_exit(&self) -> bool {
        true
    }

    fn size(&self) -> Option<TermSize> {
        Some(self.size)
    }
}

/// Ask the holder at `socket` to end its agent and wait until it has (a
/// holder that doesn't answer is killed by its exact pid). Nothing there,
/// or already exited: nothing to do.
pub fn shutdown(socket: &Path, grace: Duration) -> Result<()> {
    let mut conn = match client::connect(socket, CONNECT_TIMEOUT) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
            client::remove_stale(socket);
            return Ok(());
        }
        // Nothing there, or a holder on its way out.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe
            ) =>
        {
            return Ok(())
        }
        Err(e) => return Err(e.into()),
    };
    if conn.info.exit.is_some() {
        return Ok(());
    }
    conn.send(&proto::attach(false))?;
    conn.send(&proto::shutdown(grace.as_millis().min(u32::MAX as u128) as u32))?;
    let deadline = Instant::now() + grace + Duration::from_secs(2);
    conn.set_recv_timeout(Some(Duration::from_millis(200)))?;
    while Instant::now() < deadline {
        match conn.recv() {
            Ok(Some(Msg::Exit(_))) | Ok(None) => return Ok(()),
            Ok(_) => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(_) => return Ok(()),
        }
    }
    client::force_kill(conn.info.child_pid);
    client::force_kill(conn.info.holder_pid);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_claude_session_vars() {
        assert!(is_claude_session_var("CLAUDECODE"));
        assert!(is_claude_session_var("CLAUDE_CODE_ENTRYPOINT"));
        assert!(!is_claude_session_var("CLAUDE_CONFIG_DIR"));
    }

    #[test]
    fn no_holder_means_nothing_to_attach_or_stop() {
        let dir = std::env::temp_dir().join(format!("pw-hold-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = client::socket_path(&dir, "never-started");
        assert_eq!(HoldTerm::connect(&socket, false).err().map(|e| e.kind()), Some(io::ErrorKind::NotFound));
        assert!(shutdown(&socket, Duration::from_millis(100)).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
