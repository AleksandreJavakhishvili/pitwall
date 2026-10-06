//! One terminal per agent: output fan-out, the headless screen, input, resize.
//!
//! The PTY and the agent's process belong to a `pitwall-hold` process (one
//! per agent, socket `~/.pitwall/run/hold/<agentId>.sock`), so the terminal
//! outlives the app: quitting, rebuilding or crashing Pitwall never kills an
//! agent (architecture.md §9, decision 1). This side keeps everything Pitwall
//! does with the bytes: ring + replay, subscribers, `Screen`, activity/echo
//! timing, paste sending and the redraw nudge.

mod fanout;

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use pitwall_detect::Screen;
use pitwall_hold::client::{self, Launch};
use pitwall_hold::{proto, Msg};

use crate::clock::Clock;
use crate::shell;
use fanout::Fanout;
pub use fanout::OutputSink;

const RING_CAP: usize = 1024 * 1024;
pub const DEFAULT_ROWS: u16 = 40;
pub const DEFAULT_COLS: u16 = 120;
/// Output this soon after user input is most likely echo, not agent work.
const ECHO_WINDOW_MS: u64 = 500;
const MAX_SUBSCRIBERS: usize = 8;
/// Grace `hangup` gives the agent before the holder kills it.
const HANGUP_GRACE: Duration = Duration::from_millis(1500);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Spawn<'a> {
    pub agent_id: &'a str,
    pub cwd: &'a str,
    /// Shell fragment run as `exec <command_line>` in a login shell.
    pub command_line: String,
    /// Initial terminal size as (cols, rows); `None` → the defaults. Start at
    /// the size the UI will show, or the first frames are drawn too wide.
    pub size: Option<(u16, u16)>,
    /// Where holder sockets live: `Paths::hold_dir` (tests: a temp dir).
    pub hold_dir: &'a Path,
    /// The `pitwall-hold` executable (the host knows where it ships it).
    pub holder: &'a Path,
    /// Passed to the agent as `PITWALL_SOCKET` (where its hooks post).
    pub hook_socket: &'a Path,
    pub clock: Arc<dyn Clock>,
}

/// PTY sizes the backend accepts, as (cols, rows).
pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(2, 1000), rows.clamp(2, 1000))
}

pub struct Session {
    output: Mutex<Fanout>,
    pub screen: Mutex<Screen>,
    /// Frames to the holder go through a writer thread so callers never block.
    to_holder: Mutex<std::sync::mpsc::Sender<Vec<u8>>>,
    /// Serialises paste+Enter sequences so two sends never interleave.
    send_lock: Arc<Mutex<()>>,
    size: Mutex<(u16, u16)>,
    /// Bumped on every chunk of output.
    pub output_seq: AtomicU64,
    /// Last output that wasn't likely echo of user input (mono ms).
    pub last_activity: AtomicU64,
    /// Last user keystroke / resize (mono ms).
    pub last_input: AtomicU64,
    /// The agent's process has ended (or its holder is gone).
    pub exited: AtomicBool,
    /// The agent's process (the login shell `exec`s into it).
    pub pid: Option<u32>,
    holder_pid: u32,
    clock: Arc<dyn Clock>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Environment variables a parent Claude Code session leaks into children.
fn is_claude_session_var(key: &str) -> bool {
    key == "CLAUDECODE" || key.starts_with("CLAUDE_CODE_")
}

pub fn launch_script(command_line: &str) -> String {
    format!("exec {command_line}")
}

impl Session {
    /// Start the agent in a new holder and connect to it.
    pub fn spawn(spec: Spawn) -> Result<Arc<Session>, String> {
        let (cols, rows) = spec.size.map_or((DEFAULT_COLS, DEFAULT_ROWS), |(c, r)| clamp_size(c, r));
        let socket = client::socket_path(spec.hold_dir, spec.agent_id);
        let (program, args) = shell::login_invocation(&launch_script(&spec.command_line));
        let args: Vec<OsString> = args.into_iter().map(OsString::from).collect();
        let mut cmd = client::command(
            spec.holder,
            &Launch {
                socket: &socket,
                cols,
                rows,
                cwd: Some(Path::new(spec.cwd)),
                grace: None,
                program: OsStr::new(&program),
                args: &args,
            },
        );
        // The holder passes its environment to the agent unchanged.
        for (key, _) in std::env::vars_os() {
            if key.to_str().is_some_and(is_claude_session_var) {
                cmd.env_remove(key);
            }
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("PITWALL_ENV", "1");
        cmd.env("PITWALL_AGENT_ID", spec.agent_id);
        cmd.env("PITWALL_SOCKET", spec.hook_socket);
        client::launch(&mut cmd).map_err(|e| format!("could not start agent: {e}"))?;
        // Even if it already exited: its output and exit still come through.
        Self::connect(spec.agent_id, &socket, true, spec.clock)
            .map_err(|e| format!("could not connect to the agent's terminal: {e}"))
    }

    /// The agent's terminal from an earlier run of the app, if its holder is
    /// still there with the agent running. Never starts anything.
    pub fn reattach(hold_dir: &Path, agent_id: &str, clock: Arc<dyn Clock>) -> Option<Arc<Session>> {
        let socket = client::socket_path(hold_dir, agent_id);
        match Self::connect(agent_id, &socket, false, clock) {
            Ok(s) => Some(s),
            Err(e) => {
                match e.kind() {
                    // No holder: the agent was stopped, or exited.
                    io::ErrorKind::NotFound => {}
                    // The holder is gone (killed); only its socket is left.
                    io::ErrorKind::ConnectionRefused => client::remove_stale(&socket),
                    _ => eprintln!("pitwall: not re-attaching agent {agent_id}: {e}"),
                }
                None
            }
        }
    }

    fn connect(agent_id: &str, socket: &Path, allow_exited: bool, clock: Arc<dyn Clock>) -> io::Result<Arc<Session>> {
        let conn = client::connect(socket, CONNECT_TIMEOUT)?;
        let info = conn.info;
        if let (Some(code), false) = (info.exit, allow_exited) {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("the agent already exited ({code})")));
        }
        let (reader, mut writer) = conn.split();
        // History first (output from before we connected), then live output.
        writer.write_all(&proto::attach(true))?;
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new()
            .name(format!("hold-in-{agent_id}"))
            .spawn(move || {
                for frame in rx {
                    if writer.write_all(&frame).is_err() {
                        break;
                    }
                }
            })?;
        let (rows, cols) = (info.rows.max(2), info.cols.max(2));
        let session = Arc::new(Session {
            output: Mutex::new(Fanout::new(RING_CAP, MAX_SUBSCRIBERS)),
            screen: Mutex::new(Screen::new(rows, cols)),
            to_holder: Mutex::new(tx),
            send_lock: Arc::new(Mutex::new(())),
            size: Mutex::new((rows, cols)),
            output_seq: AtomicU64::new(0),
            last_activity: AtomicU64::new(0),
            last_input: AtomicU64::new(0),
            exited: AtomicBool::new(false),
            pid: Some(info.child_pid),
            holder_pid: info.holder_pid,
            clock,
        });
        let s = session.clone();
        std::thread::Builder::new()
            .name(format!("hold-{agent_id}"))
            .spawn(move || s.read_loop(reader))?;
        Ok(session)
    }

    fn read_loop(&self, mut stream: impl Read) {
        while let Ok(Some((ty, body))) = proto::read_frame(&mut stream) {
            match Msg::decode(ty, body) {
                Ok(Msg::Output(chunk)) => self.on_output(&chunk, true),
                // History is not new activity.
                Ok(Msg::Replay(chunk)) => self.on_output(&chunk, false),
                Ok(Msg::Exit(_)) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        self.exited.store(true, Ordering::Relaxed);
    }

    fn on_output(&self, chunk: &[u8], live: bool) {
        lock(&self.output).push(chunk);
        lock(&self.screen).feed(chunk);
        let now = self.clock.mono_ms();
        self.output_seq.fetch_add(1, Ordering::Relaxed);
        if live && now.saturating_sub(self.last_input.load(Ordering::Relaxed)) > ECHO_WINDOW_MS {
            self.last_activity.store(now, Ordering::Relaxed);
        }
    }

    fn send_frame(&self, frame: Vec<u8>) -> Result<(), String> {
        lock(&self.to_holder)
            .send(frame)
            .map_err(|_| "agent is not accepting input".to_string())
    }

    /// Replay buffered output, then stream live output to `sink`.
    /// Returns a subscription id for `detach` (unique in this process, so a
    /// stale id never detaches a newer subscriber).
    pub fn attach(self: &Arc<Self>, sink: OutputSink) -> u64 {
        static NEXT_SUB: AtomicU64 = AtomicU64::new(1);
        let sub_id = NEXT_SUB.fetch_add(1, Ordering::Relaxed);
        if !lock(&self.output).subscribe(sub_id, sink) {
            return sub_id;
        }
        // Nudge full-screen TUIs into repainting so the replay ends clean.
        let s = self.clone();
        std::thread::spawn(move || {
            let (rows, cols) = *lock(&s.size);
            if rows < 2 || s.exited.load(Ordering::Relaxed) {
                return;
            }
            s.touch_input();
            s.pty_resize(rows - 1, cols);
            std::thread::sleep(Duration::from_millis(60));
            s.touch_input();
            s.pty_resize(rows, cols);
        });
        sub_id
    }

    pub fn detach(&self, sub_id: u64) {
        lock(&self.output).unsubscribe(sub_id);
    }

    /// The buffered output (what a new subscriber gets first).
    pub fn history(&self) -> Vec<u8> {
        lock(&self.output).history()
    }

    fn pty_resize(&self, rows: u16, cols: u16) {
        let _ = self.send_frame(proto::resize(cols, rows));
    }

    /// Current PTY size as (cols, rows). The attach redraw nudge is not reflected.
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = *lock(&self.size);
        (cols, rows)
    }

    /// Returns whether the size changed.
    pub fn resize(&self, cols: u16, rows: u16) -> bool {
        let (cols, rows) = clamp_size(cols, rows);
        {
            let mut size = lock(&self.size);
            if *size == (rows, cols) {
                return false;
            }
            *size = (rows, cols);
        }
        self.touch_input();
        self.pty_resize(rows, cols);
        lock(&self.screen).resize(rows, cols);
        true
    }

    pub fn touch_input(&self) {
        self.last_input.store(self.clock.mono_ms(), Ordering::Relaxed);
    }

    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        self.send_frame(proto::input(data))
    }

    /// Bracketed paste of `text` verbatim, then Enter ~200ms later.
    pub fn send_text(self: &Arc<Self>, text: String) {
        let s = self.clone();
        let send_lock = self.send_lock.clone();
        std::thread::spawn(move || {
            let _guard = lock(&send_lock);
            if s.write(&paste_bytes(&text)).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
            let _ = s.write(b"\r");
        });
    }

    /// Whether the agent's process has ended.
    pub fn poll_exit(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }

    /// Ask the holder to end the agent (SIGHUP, SIGKILL after a grace period).
    /// Only for an explicit stop/restart/remove — never housekeeping.
    pub fn hangup(&self) {
        if !self.poll_exit() {
            let _ = self.send_frame(proto::shutdown(HANGUP_GRACE.as_millis() as u32));
        }
    }

    /// Hang up, let the holder force the process after `grace`, and wait for
    /// it. A holder that doesn't answer is killed by its exact pid.
    pub fn terminate(&self, grace: Duration) {
        if self.poll_exit() {
            return;
        }
        let _ = self.send_frame(proto::shutdown(grace.as_millis().min(u32::MAX as u128) as u32));
        let deadline = std::time::Instant::now() + grace + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if self.poll_exit() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // Still connected, so these pids are still the holder and its child.
        if let Some(pid) = self.pid {
            client::force_kill(pid);
        }
        client::force_kill(self.holder_pid);
        self.exited.store(true, Ordering::Relaxed);
    }
}

pub fn paste_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 12);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(text.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_is_verbatim() {
        assert_eq!(paste_bytes("a'b\n"), b"\x1b[200~a'b\n\x1b[201~");
    }

    #[test]
    fn strips_claude_session_vars() {
        assert!(is_claude_session_var("CLAUDECODE"));
        assert!(is_claude_session_var("CLAUDE_CODE_ENTRYPOINT"));
        assert!(!is_claude_session_var("CLAUDE_CONFIG_DIR"));
    }

    #[test]
    fn sizes_are_clamped() {
        assert_eq!(clamp_size(0, 5000), (2, 1000));
        assert_eq!(clamp_size(80, 24), (80, 24));
    }

    #[test]
    fn no_holder_running_means_nothing_to_reattach() {
        let dir = crate::testing::TempDir::new("hold");
        let clock: Arc<dyn Clock> = Arc::new(crate::clock::SystemClock::new());
        assert!(Session::reattach(dir.path(), "never-started", clock).is_none());
    }
}
