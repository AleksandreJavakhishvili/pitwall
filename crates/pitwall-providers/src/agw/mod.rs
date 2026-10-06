//! The agw provider, slice (a) of step 8 (architecture.md §2.7): sessions
//! that already exist on agw VMs become Pitwall agents. Pitwall *adopts*
//! them — it never creates or deletes agw sessions here — and talks to agw
//! only through agw's own commands, so agw's rules for admin and agent users
//! (§9 decision 4) and its VM gate always apply:
//!
//! - machines, sessions, state: `agw vm list` / `agw session list --vm <vm>
//!   --status` / `agw workspace describe` (`--output json`, read-only).
//! - terminal: `agw session attach <name>` (a tmux client over agw's ssh)
//!   running in a `pitwall-hold` holder, exactly like a local agent's PTY.
//!   Input is written to that stream (Next up, typing); the attachment's
//!   EOF only means it dropped (`eof_is_exit` false) — the engine asks
//!   `state` and attaches again. The holder keeps the attachment across
//!   Pitwall restarts; closing it detaches the tmux client and leaves the
//!   session running.
//! - status: screen detection on that stream (no hooks yet, decision 5).
//! - stop / start: `agw session stop|start <name>` (agw's harness integration
//!   decides whether a start continues the conversation, so `resume` is not
//!   a capability: Pitwall can't pick a conversation).
//! - capture: `tmux capture-pane -p` over agw's ssh host alias for the VM
//!   (`awvm--<vm>`, the admin user, decision 3), on the session's own tmux
//!   socket (agw's layout: admin sessions under
//!   `/run/agentworks/admin-tmux-sockets/<admin>/`, agent sessions under
//!   `/run/agentworks/agent-tmux-sockets/agt-<agent>/`, reachable by the
//!   admin through agw's group), else the default server.
//!
//! Not here yet: diffs/Review (`exec`, slice b), create (c), rules and hooks
//! (d). Locator: `agw:<vm>/<session name>` (agw session names are unique).

mod parse;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use pitwall_core::exec::{Cmd, Exec, LocalExec, Out};
use pitwall_core::kind::KindCatalog;
use pitwall_core::provider::{
    CreateSpec, Discovered, ErrorCode, ExitInfo, HookTransport, KindOnMachine, LaunchIntent, LaunchSpec, Locator, Machine,
    MachineId, NativeState, Provider, ProviderCaps, ProviderId, PwError, Result, Started, TermIo, TermSize,
};
use pitwall_hold::client;

use crate::hold::{self, HoldTerm, Program};
pub use parse::{Session, Status, Vm};

const LIST_TIMEOUT: Duration = Duration::from_secs(30);
const LIFECYCLE_TIMEOUT: Duration = Duration::from_secs(180);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a closing attachment gets before its holder kills it.
const DETACH_GRACE: Duration = Duration::from_millis(1500);

/// Where agw and its pieces are.
#[derive(Clone)]
pub struct AgwConfig {
    /// The agw program; `None` = looked up on the login shell's PATH on
    /// first use (not installed → every call answers `Unsupported`).
    pub agw: Option<PathBuf>,
    /// ssh, for screen captures.
    pub ssh: PathBuf,
    /// agw's ssh host alias for a VM is this + the VM name (agw writes these
    /// into `~/.ssh/config.d/agentworks.conf`).
    pub ssh_alias_prefix: String,
    /// Attachment holders: `agw-<hash>.sock` each (`Paths::hold_dir`).
    pub hold_dir: PathBuf,
    /// The `pitwall-hold` executable, or why there is none.
    pub holder_bin: std::result::Result<PathBuf, String>,
    /// Runs agw and ssh (this Mac; a script in tests).
    pub exec: Arc<dyn Exec>,
}

impl AgwConfig {
    pub fn new(hold_dir: PathBuf, holder_bin: std::result::Result<PathBuf, String>) -> AgwConfig {
        AgwConfig {
            agw: None,
            ssh: PathBuf::from("ssh"),
            ssh_alias_prefix: "awvm--".into(),
            hold_dir,
            holder_bin,
            exec: Arc::new(LocalExec),
        }
    }
}

pub struct AgwProvider {
    id: ProviderId,
    cfg: AgwConfig,
    agw: OnceLock<Option<String>>,
}

/// Single-quoted for a POSIX shell, always (tmux's `=name` must never be
/// expanded by zsh).
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// FNV-1a: a short, stable file name for a locator.
fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The shell script that prints a session's screen on its VM (run as the
/// admin user).
pub fn capture_script(session: &str, agent: Option<&str>) -> String {
    let socket = match agent {
        Some(a) => format!("/run/agentworks/agent-tmux-sockets/{}/{}.sock", sq(&format!("agt-{a}")), sq(session)),
        None => format!("/run/agentworks/admin-tmux-sockets/\"$(id -un)\"/{}.sock", sq(session)),
    };
    let target = sq(&format!("={session}:"));
    format!(
        "s={socket}; if [ -S \"$s\" ]; then exec tmux -S \"$s\" capture-pane -p -t {target}; else exec tmux capture-pane -p -t {target}; fi"
    )
}

impl AgwProvider {
    pub const ID: &'static str = "agw";

    pub fn new(cfg: AgwConfig) -> AgwProvider {
        AgwProvider { id: ProviderId::new(Self::ID), cfg, agw: OnceLock::new() }
    }

    fn agw(&self) -> Result<String> {
        if let Some(p) = &self.cfg.agw {
            return Ok(p.to_string_lossy().into_owned());
        }
        self.agw
            .get_or_init(|| pitwall_core::shell::which("agw"))
            .clone()
            .ok_or_else(|| PwError::unsupported("agw is not installed (not on your shell's PATH)"))
    }

    fn run(&self, args: &[&str], timeout: Duration) -> Result<Out> {
        let agw = self.agw()?;
        let mut argv = vec![agw.as_str(), "--non-interactive"];
        argv.extend_from_slice(args);
        self.cfg.exec.run(&Cmd::new(&argv).timeout(timeout))
    }

    /// Run an agw command; its error output becomes the error.
    fn checked(&self, args: &[&str], timeout: Duration) -> Result<Out> {
        let out = self.run(args, timeout)?;
        if out.ok() {
            return Ok(out);
        }
        let msg = parse::error_line(&out.stderr_text());
        let lower = msg.to_ascii_lowercase();
        let code = if lower.contains("not found") || lower.contains("unknown") { ErrorCode::NotFound } else { ErrorCode::Other };
        Err(PwError::new(code, format!("agw: {msg}")))
    }

    fn json(&self, args: &[&str]) -> Result<serde_json::Value> {
        let mut args = args.to_vec();
        args.extend(["--output", "json"]);
        let out = self.checked(&args, LIST_TIMEOUT)?;
        parse::json(&out.stdout_text()).ok_or_else(|| PwError::other("agw printed no JSON"))
    }

    /// Sessions on `vm` (`status`: ask agw for their live state).
    pub fn sessions(&self, vm: &str, status: bool) -> Result<Vec<Session>> {
        let mut args = vec!["session", "list", "--vm", vm];
        if status {
            args.push("--status");
        }
        parse::sessions(&self.json(&args)?).ok_or_else(|| PwError::other("agw listed no sessions"))
    }

    fn session(&self, loc: &Locator, status: bool) -> Result<Option<Session>> {
        match self.sessions(loc.machine.as_str(), status) {
            Ok(list) => Ok(list.into_iter().find(|s| s.name == loc.native)),
            Err(e) if e.is(ErrorCode::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn mine(&self, loc: &Locator) -> Result<()> {
        if loc.provider != self.id {
            return Err(PwError::not_found(format!("{loc} is not an agw session")));
        }
        Ok(())
    }

    /// The holder of Pitwall's attachment to `loc`.
    fn socket(&self, loc: &Locator) -> PathBuf {
        client::socket_path(&self.cfg.hold_dir, &format!("agw-{:016x}", hash(&loc.to_string())))
    }

    /// Workspace → its folder on the VM (asked in parallel; failures skipped).
    fn workspace_paths(&self, names: BTreeSet<String>) -> BTreeMap<String, String> {
        std::thread::scope(|s| {
            let asks: Vec<_> = names
                .into_iter()
                .map(|ws| {
                    s.spawn(move || {
                        let path = self.json(&["workspace", "describe", &ws]).ok().and_then(|v| parse::workspace_path(&v));
                        (ws, path)
                    })
                })
                .collect();
            asks.into_iter().filter_map(|a| a.join().ok()).filter_map(|(ws, p)| Some((ws, p?))).collect()
        })
    }
}

impl Provider for AgwProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn caps(&self) -> ProviderCaps {
        ProviderCaps {
            create: false,
            resume: false,
            start: true,
            attach_existing: true,
            survives_detach: true,
            exec: false,
            process_cwd: false,
            capture: true,
            hooks: HookTransport::None,
            rules: false,
            custom_command: false,
            local_process: false,
        }
    }

    fn version(&self) -> Option<String> {
        let out = self.run(&["--version"], LIST_TIMEOUT).ok()?;
        pitwall_core::onboarding::scan::version_line(&out.stdout_text())
    }

    fn machines(&self) -> Result<Vec<Machine>> {
        let vms = parse::vms(&self.json(&["vm", "list"])?).ok_or_else(|| PwError::other("agw listed no VMs"))?;
        Ok(vms.into_iter().map(|v| Machine { id: MachineId::new(&v.name), label: v.name, detail: v.site }).collect())
    }

    /// agw's harness integrations decide what a session runs; Pitwall can't
    /// tell which are set up on a VM, so none is claimed as installed.
    fn kinds(&self, _m: &MachineId, catalog: &KindCatalog) -> Result<Vec<KindOnMachine>> {
        Ok(catalog.kinds().into_iter().map(|kind| KindOnMachine { kind, installed: false, path: None }).collect())
    }

    fn discover(&self, m: &MachineId) -> Result<Vec<Discovered>> {
        let sessions = self.sessions(m.as_str(), true)?;
        let paths = self.workspace_paths(sessions.iter().filter_map(|s| s.workspace.clone()).collect());
        Ok(sessions
            .into_iter()
            .map(|s| Discovered {
                locator: Locator::new(&self.id, m, &s.name),
                kind: s.harness.clone().unwrap_or_else(|| "shell".into()),
                state: match s.status {
                    Status::Running => Some(NativeState::Running),
                    Status::Stopped => Some(NativeState::Stopped),
                    Status::Unknown => None,
                },
                cwd: s.workspace.as_ref().and_then(|w| paths.get(w).cloned()),
                title: None,
                workspace: s.workspace,
                user: s.agent,
            })
            .collect())
    }

    fn create(&self, _spec: &CreateSpec) -> Result<Started> {
        Err(PwError::unsupported("Pitwall adds existing agw sessions; creating them comes later"))
    }

    /// `agw session start`: its harness integration continues the
    /// conversation when it can. The terminal comes from `attach`.
    fn start(&self, loc: &Locator, launch: &LaunchSpec) -> Result<Started> {
        self.mine(loc)?;
        if let LaunchIntent::Resume(_) = launch.intent {
            return Err(PwError::unsupported("agw continues conversations itself: start the session instead"));
        }
        self.checked(&["session", "start", &loc.native], LIFECYCLE_TIMEOUT)?;
        Ok(Started { locator: loc.clone(), conversation_id: None, resumed: false, cwd: launch.cwd.to_string(), term: None })
    }

    /// Pitwall's attachment from before (its holder outlives Pitwall), else
    /// a new `agw session attach` while the session runs. Never starts it.
    fn attach(&self, loc: &Locator, size: TermSize) -> Result<Box<dyn TermIo>> {
        self.mine(loc)?;
        let socket = self.socket(loc);
        match HoldTerm::connect(&socket, false) {
            Ok(t) => return Ok(Box::new(AgwTerm(t))),
            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => client::remove_stale(&socket),
            Err(_) => {}
        }
        if self.state(loc)? != NativeState::Running {
            return Err(PwError::not_running());
        }
        let holder = self.cfg.holder_bin.as_deref().map_err(|e| PwError::unsupported(e.clone()))?;
        let agw = self.agw()?;
        let args = ["session".to_string(), "attach".to_string(), loc.native.clone()];
        let home = std::env::var_os("HOME").map(PathBuf::from);
        hold::run(holder, &socket, size, Program { program: &agw, args: &args, cwd: home.as_deref(), env: &[] })
            .map_err(|e| PwError::other(format!("could not attach to {loc}: {e}")))?;
        let t = HoldTerm::connect(&socket, true).map_err(|e| PwError::other(format!("could not attach to {loc}: {e}")))?;
        Ok(Box::new(AgwTerm(t)))
    }

    fn stop(&self, loc: &Locator) -> Result<()> {
        self.mine(loc)?;
        self.checked(&["session", "stop", &loc.native], LIFECYCLE_TIMEOUT).map(|_| ())
    }

    /// Pitwall forgets the session: only its attachment is closed. The agw
    /// session itself is never deleted here.
    fn remove(&self, loc: &Locator) -> Result<()> {
        self.mine(loc)?;
        hold::shutdown(&self.socket(loc), DETACH_GRACE)
    }

    fn state(&self, loc: &Locator) -> Result<NativeState> {
        self.mine(loc)?;
        match self.session(loc, true)? {
            None => Ok(NativeState::Gone),
            Some(s) => match s.status {
                Status::Running => Ok(NativeState::Running),
                Status::Stopped => Ok(NativeState::Stopped),
                Status::Unknown => Err(PwError::unreachable(format!("agw can't tell whether {} runs", loc.native))),
            },
        }
    }

    fn exec(&self, _m: &MachineId) -> Result<Arc<dyn Exec>> {
        Err(PwError::unsupported("diffs and Review on agw VMs come later"))
    }

    fn process_cwd(&self, _loc: &Locator, _pid: Option<u32>) -> Result<Option<String>> {
        Err(PwError::unsupported("finding an agw session's folder comes later"))
    }

    fn capture(&self, loc: &Locator) -> Result<String> {
        self.mine(loc)?;
        let s = self.session(loc, false)?.ok_or_else(PwError::not_running)?;
        let alias = format!("{}{}", self.cfg.ssh_alias_prefix, loc.machine);
        let script = capture_script(&s.name, s.agent.as_deref());
        let ssh = self.cfg.ssh.to_string_lossy().into_owned();
        let argv = [ssh.as_str(), "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", &alias, "--", &script];
        let out = self.cfg.exec.run(&Cmd::new(&argv).timeout(CAPTURE_TIMEOUT))?;
        if !out.ok() {
            return Err(PwError::new(ErrorCode::Unreachable, format!("could not read {}'s screen: {}", loc.native, out.stderr_text())));
        }
        Ok(out.stdout_text())
    }
}

/// An attachment to an agw session: a holder running `agw session attach`.
/// Its end is the attachment's, not the agent's.
pub struct AgwTerm(HoldTerm);

impl TermIo for AgwTerm {
    fn take_history(&mut self) -> Vec<u8> {
        self.0.take_history()
    }
    fn take_reader(&mut self) -> Box<dyn Read + Send> {
        self.0.take_reader()
    }
    fn take_writer(&mut self) -> Box<dyn Write + Send> {
        self.0.take_writer()
    }
    fn resize(&self, size: TermSize) -> Result<()> {
        self.0.resize(size)
    }
    fn try_wait(&self) -> Result<Option<ExitInfo>> {
        Ok(self.0.try_wait()?.map(|_| ExitInfo { code: None }))
    }
    /// Hangs up the tmux client; the session keeps running.
    fn close(&self, grace: Duration) {
        self.0.close(grace)
    }
    /// A process on this Mac, but not the agent's.
    fn pid(&self) -> Option<u32> {
        None
    }
    fn eof_is_exit(&self) -> bool {
        false
    }
    fn size(&self) -> Option<TermSize> {
        self.0.size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::testing::FakeExec;

    const LIST: &str = include_str!("fixtures/session_list.json");

    fn provider(x: &Arc<FakeExec>) -> AgwProvider {
        let dir = std::env::temp_dir().join(format!("pw-agw-unit-{}", std::process::id()));
        let mut cfg = AgwConfig::new(dir, Err("no holder in unit tests".into()));
        cfg.agw = Some("/opt/agw".into());
        cfg.exec = x.clone();
        AgwProvider::new(cfg)
    }

    fn loc(vm: &str, name: &str) -> Locator {
        Locator::new(&ProviderId::new("agw"), &MachineId::new(vm), name)
    }

    #[test]
    fn machines_and_sessions_come_from_agw_listings() {
        let x = FakeExec::new();
        x.on(&["/opt/agw", "--non-interactive", "vm", "list", "--output", "json"], include_str!("fixtures/vm_list.json"));
        x.on(&["/opt/agw", "--non-interactive", "session", "list", "--vm", "shakedown", "--status", "--output", "json"], LIST);
        x.on(&["/opt/agw", "--non-interactive", "workspace", "describe", "gwlive", "--output", "json"], include_str!("fixtures/workspace_describe.json"));
        x.on_exit(&["/opt/agw", "--non-interactive", "workspace", "describe", "work", ".."], 1, "", "Error: boom");
        x.on(&["/opt/agw", "--non-interactive", "--version"], "0.19.0\n");
        let p = provider(&x);
        assert_eq!(p.version().as_deref(), Some("0.19.0"));
        let m = p.machines().unwrap();
        assert_eq!((m[0].id.as_str(), m[0].label.as_str(), m[0].detail.as_deref()), ("shakedown", "shakedown", Some("lima-local")));
        let d = p.discover(&MachineId::new("shakedown")).unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].locator.to_string(), "agw:shakedown/gwlive");
        assert_eq!((d[0].kind.as_str(), d[0].state), ("claude-code", Some(NativeState::Running)));
        assert_eq!(d[0].cwd.as_deref(), Some("/opt/agentworks/workspaces/gwlive"));
        assert_eq!((d[1].state, d[1].cwd.as_deref(), d[1].user.as_deref()), (Some(NativeState::Stopped), None, None));
        // Only read-only commands ran.
        assert!(x.calls().iter().all(|c| ["vm", "session", "workspace", "--version"].contains(&c.argv[2].as_str())));
        assert!(x.calls().iter().all(|c| !c.argv.iter().any(|a| ["start", "stop", "delete", "create", "attach"].contains(&a.as_str()))));
    }

    #[test]
    fn state_follows_agw_status() {
        let x = FakeExec::new();
        x.on(&["/opt/agw", "--non-interactive", "session", "list", "--vm", "shakedown", "--status", "--output", "json"], LIST);
        x.on_exit(&["/opt/agw", "--non-interactive", "session", "list", "--vm", "gone", ".."], 1, "", "Error: unknown VM 'gone'\n  Hint: x");
        x.on(
            &["/opt/agw", "--non-interactive", "session", "list", "--vm", "dim", ".."],
            include_str!("fixtures/session_list_nostatus.json").replace("shakedown", "dim").as_str(),
        );
        let p = provider(&x);
        assert_eq!(p.state(&loc("shakedown", "gwlive")).unwrap(), NativeState::Running);
        assert_eq!(p.state(&loc("shakedown", "work")).unwrap(), NativeState::Stopped);
        assert_eq!(p.state(&loc("shakedown", "nope")).unwrap(), NativeState::Gone);
        assert_eq!(p.state(&loc("gone", "x")).unwrap(), NativeState::Gone);
        assert!(p.state(&loc("dim", "gwlive")).unwrap_err().is(ErrorCode::Unreachable));
        // Not running: nothing to attach to, and nothing is started.
        assert!(p.attach(&loc("shakedown", "work"), TermSize::DEFAULT).err().unwrap().is(ErrorCode::NotRunning));
        let local = Locator::local("a");
        assert!(p.state(&local).unwrap_err().is(ErrorCode::NotFound));
    }

    #[test]
    fn stop_and_start_are_agws_own_commands() {
        let x = FakeExec::new();
        x.on(&["/opt/agw", "--non-interactive", "session", "stop", "gwlive"], "");
        x.on(&["/opt/agw", "--non-interactive", "session", "start", "gwlive"], "");
        x.on_exit(&["/opt/agw", "--non-interactive", "session", "start", "work"], 1, "", "Error: VM is stopped\n");
        let p = provider(&x);
        p.stop(&loc("shakedown", "gwlive")).unwrap();
        let kind = pitwall_core::kind::custom_kind("x");
        let mut spec = LaunchSpec {
            agent: "a",
            kind: &kind,
            cwd: "/opt/w",
            intent: LaunchIntent::Fresh,
            worktree: None,
            size: TermSize::DEFAULT,
            hooks: None,
        };
        let s = p.start(&loc("shakedown", "gwlive"), &spec).unwrap();
        assert!(s.term.is_none() && !s.resumed && s.conversation_id.is_none());
        assert_eq!(p.start(&loc("shakedown", "work"), &spec).err().unwrap().message, "agw: VM is stopped");
        spec.intent = LaunchIntent::Resume("c".into());
        assert!(p.start(&loc("shakedown", "gwlive"), &spec).err().unwrap().is(ErrorCode::Unsupported));
        assert_eq!(x.ran(&["/opt/agw", "--non-interactive", "session", ".."]), 3, "a resume never reaches agw");
        assert!(!x.calls().iter().any(|c| c.argv.iter().any(|a| a == "--force-new" || a == "delete")));
    }

    #[test]
    fn capture_reads_the_session_socket_over_agws_ssh_alias() {
        let x = FakeExec::new();
        x.on(&["/opt/agw", "--non-interactive", "session", "list", "--vm", "shakedown", "--output", "json"], LIST);
        x.on(&["ssh", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "awvm--shakedown", "--", "*"], "screen\n");
        let p = provider(&x);
        assert_eq!(p.capture(&loc("shakedown", "gwlive")).unwrap(), "screen\n");
        let script = x.calls().last().unwrap().argv.last().unwrap().clone();
        assert_eq!(script, capture_script("gwlive", None));
        assert!(script.contains("/run/agentworks/admin-tmux-sockets/\"$(id -un)\"/'gwlive'.sock") && script.contains("-t '=gwlive:'"));
        assert!(capture_script("s", Some("bot")).contains("/run/agentworks/agent-tmux-sockets/'agt-bot'/'s'.sock"));
        assert!(p.capture(&loc("shakedown", "nope")).err().unwrap().is(ErrorCode::NotRunning));
    }

    #[test]
    fn what_slice_a_does_not_do_is_unsupported() {
        let x = FakeExec::new();
        let p = provider(&x);
        let m = MachineId::new("shakedown");
        assert!(p.exec(&m).is_err_and(|e| e.is(ErrorCode::Unsupported)));
        assert!(p.process_cwd(&loc("shakedown", "gwlive"), None).unwrap_err().is(ErrorCode::Unsupported));
        let caps = p.caps();
        assert!(!caps.create && !caps.resume && !caps.exec && !caps.rules && caps.hooks == HookTransport::None);
        assert!(caps.attach_existing && caps.start && caps.capture && !caps.local_process);
        assert!(p.remove(&loc("shakedown", "gwlive")).is_ok(), "nothing attached: nothing to do");
        assert!(x.calls().is_empty(), "remove never asks agw to delete anything");
    }

    #[test]
    fn without_agw_nothing_is_offered() {
        let x = FakeExec::new();
        let mut cfg = AgwConfig::new(std::env::temp_dir(), Err("none".into()));
        cfg.exec = x.clone();
        let p = AgwProvider::new(cfg);
        // Pretend the PATH lookup already found nothing.
        let _ = p.agw.set(None);
        assert!(p.machines().unwrap_err().is(ErrorCode::Unsupported));
        assert_eq!(p.version(), None);
    }
}
