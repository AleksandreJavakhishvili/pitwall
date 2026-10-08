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
//! - exec (slice b, diffs and Review): [`AgwExec`], agw's own `vm exec` /
//!   `agent exec` (never ssh), as the user the session runs as — its agent's
//!   user when it has one, else the admin — from the session's workspace
//!   (`agw session describe`, asked once per session and again after a
//!   start or a new attachment). Each call is a round trip of about a
//!   second, so git is polled rarely (`git_poll_ms`).
//!
//! - create (slice c, [`create`]): the New-agent form for a VM lists its
//!   workspaces, agent users and templates (`agw workspace list`, `agw
//!   agent list`, `agw resource list`, read-only, cached briefly); creating
//!   runs `agw session create … --vm <vm>` (which also starts it), then
//!   Pitwall attaches exactly as to an added session. A created session
//!   belongs to agw like an added one: removing it from Pitwall only closes
//!   the attachment. Deleting it on agw (`agw session delete <name> --yes`)
//!   is not offered yet.
//!
//! Not here yet: rules and hooks (d), `process_cwd` (so no worktree
//! discovery, hence no merge). Locator: `agw:<vm>/<session name>` (agw
//! session names are unique).

pub mod create;
mod exec;
mod parse;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use pitwall_core::exec::{Cmd, Exec, LocalExec, Out};
use pitwall_core::kind::KindCatalog;
use pitwall_core::provider::{
    CreateForm, CreateSpec, Discovered, ErrorCode, ExitInfo, HookTransport, KindOnMachine, LaunchIntent, LaunchSpec, Locator,
    Machine, MachineId, NativeState, Provider, ProviderCaps, ProviderId, PwError, Result, Started, TermIo, TermSize,
};
use pitwall_hold::client;

use crate::hold::{self, HoldTerm, Program};
pub use exec::{AgwExec, User, QUICK as EXEC_QUICK};
pub use parse::{Session, Status, Vm};

const LIST_TIMEOUT: Duration = Duration::from_secs(30);
const LIFECYCLE_TIMEOUT: Duration = Duration::from_secs(180);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(20);
/// How often an agw agent's changes may be refreshed while it works: every
/// refresh is an agw round trip to the VM.
pub const GIT_POLL_MS: u32 = 15_000;
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
    /// Execs by VM (`m:<vm>`, the admin) and by session (`s:<locator>`).
    execs: Mutex<HashMap<String, Arc<AgwExec>>>,
    /// New-agent forms by VM, with when they were listed.
    forms: Mutex<HashMap<String, (Instant, CreateForm)>>,
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
        AgwProvider { id: ProviderId::new(Self::ID), cfg, agw: OnceLock::new(), execs: Mutex::default(), forms: Mutex::default() }
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
        self.json_within(args, LIST_TIMEOUT)
    }

    fn json_within(&self, args: &[&str], timeout: Duration) -> Result<serde_json::Value> {
        let mut args = args.to_vec();
        args.extend(["--output", "json"]);
        let out = self.checked(&args, timeout)?;
        parse::json(&out.stdout_text()).ok_or_else(|| PwError::other("agw printed no JSON"))
    }

    /// What `vm` has to create sessions with: its workspaces and agent
    /// users, and agw's templates (read-only, asked in parallel).
    fn listings(&self, vm: &str) -> create::Listings {
        let t = create::OPTIONS_TIMEOUT;
        let msg = |e: PwError| e.message.strip_prefix("agw: ").map(String::from).unwrap_or(e.message);
        std::thread::scope(|s| {
            let ws = s.spawn(|| self.json_within(&["workspace", "list", "--vm", vm], t).and_then(|v| parse::workspaces(&v).ok_or_else(|| PwError::other("no workspaces in agw's answer"))));
            let agents = s.spawn(|| self.json_within(&["agent", "list", "--vm", vm], t).and_then(|v| parse::agents(&v).ok_or_else(|| PwError::other("no agents in agw's answer"))));
            let kinds = "session-template,workspace-template,agent-template";
            let templates = self
                .json_within(&["resource", "list", "--kind", kinds], t)
                .and_then(|v| parse::resources(&v).ok_or_else(|| PwError::other("no templates in agw's answer")));
            let join = |h: std::thread::ScopedJoinHandle<'_, Result<Vec<parse::Named>>>| h.join().unwrap_or_else(|_| Err(PwError::other("listing failed")));
            create::Listings { workspaces: join(ws).map_err(msg), agents: join(agents).map_err(msg), templates: templates.map_err(msg) }
        })
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

    fn cached_exec(&self, key: &str, make: impl FnOnce() -> Result<AgwExec>) -> Result<Arc<AgwExec>> {
        if let Some(x) = self.execs.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
            return Ok(x.clone());
        }
        let x = Arc::new(make()?);
        self.execs.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_string(), x.clone());
        Ok(x)
    }

    /// Ask agw again who `loc` runs as and where (after a start or a new
    /// attachment: the session may have been recreated meanwhile).
    fn forget_exec(&self, loc: &Locator) {
        self.execs.lock().unwrap_or_else(|e| e.into_inner()).remove(&format!("s:{loc}"));
    }

    /// The exec for session `loc`: as its agent's user (or the admin) in its
    /// workspace, per `agw session describe`.
    pub fn session_exec(&self, loc: &Locator) -> Result<Arc<AgwExec>> {
        self.mine(loc)?;
        let agw = self.agw()?;
        self.cached_exec(&format!("s:{loc}"), || {
            let s = parse::described(&self.json(&["session", "describe", &loc.native])?)
                .ok_or_else(|| PwError::other(format!("agw described no session {}", loc.native)))?;
            let user = match s.agent {
                Some(agent) => User::Agent { agent },
                None if s.vm.is_empty() => User::Admin { vm: loc.machine.to_string() },
                None => User::Admin { vm: s.vm },
            };
            Ok(AgwExec::new(&agw, self.cfg.exec.clone(), user, s.workspace.as_deref()))
        })
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
            create: true,
            platform_create: true,
            resume: false,
            start: true,
            attach_existing: true,
            survives_detach: true,
            exec: true,
            process_cwd: false,
            capture: true,
            hooks: HookTransport::None,
            rules: false,
            custom_command: false,
            local_process: false,
            git_poll_ms: GIT_POLL_MS,
            fs_events: false,
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

    /// The VM's workspaces, agent users and templates as choices (cached
    /// for [`create::FORM_TTL`]). Fails only when agw itself is missing.
    fn create_form(&self, m: &MachineId) -> Result<CreateForm> {
        self.agw()?;
        let vm = m.to_string();
        if let Some((at, f)) = self.forms.lock().unwrap_or_else(|e| e.into_inner()).get(&vm) {
            if at.elapsed() < create::FORM_TTL {
                return Ok(f.clone());
            }
        }
        let mut f = create::form(&vm, &self.listings(&vm));
        f.provider = self.id.to_string();
        self.forms.lock().unwrap_or_else(|e| e.into_inner()).insert(vm, (Instant::now(), f.clone()));
        Ok(f)
    }

    /// `agw session create` (which also starts it), then what agw says runs
    /// there and where. The terminal comes from `attach`.
    fn create(&self, spec: &CreateSpec) -> Result<Started> {
        let vm = spec.machine.as_str();
        let args = create::args(spec.name, vm, spec.options)?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.run(&args, create::CREATE_TIMEOUT).map_err(|e| {
            PwError::new(e.code, format!("agw session create {} on {vm} didn't finish: {} (see `agw session list --vm {vm}`)", spec.name, e.message))
        })?;
        // Whatever happened, the VM's workspaces and agents may have changed.
        self.forms.lock().unwrap_or_else(|e| e.into_inner()).remove(vm);
        if !out.ok() {
            let stderr = out.stderr_text();
            let msg = parse::error_line(&stderr);
            let code = if msg.contains("already exists") { ErrorCode::Conflict } else { ErrorCode::Other };
            let hint = parse::hint_line(&stderr).map(|h| format!(" ({h})")).unwrap_or_default();
            return Err(PwError::new(code, format!("agw couldn't create \"{}\" on {vm}: {msg}{hint}", spec.name)));
        }
        let loc = Locator::new(&self.id, spec.machine, spec.name);
        let described = self.json(&["session", "describe", spec.name]).ok().and_then(|v| parse::described(&v));
        let workspace = described.as_ref().and_then(|s| s.workspace.clone());
        let cwd = workspace.and_then(|w| self.workspace_paths(BTreeSet::from([w.clone()])).remove(&w)).unwrap_or_default();
        Ok(Started { locator: loc, conversation_id: None, resumed: false, cwd, term: None, kind: described.and_then(|s| s.harness) })
    }

    /// `agw session start`: its harness integration continues the
    /// conversation when it can. The terminal comes from `attach`.
    fn start(&self, loc: &Locator, launch: &LaunchSpec) -> Result<Started> {
        self.mine(loc)?;
        if let LaunchIntent::Resume(_) = launch.intent {
            return Err(PwError::unsupported("agw continues conversations itself: start the session instead"));
        }
        self.checked(&["session", "start", &loc.native], LIFECYCLE_TIMEOUT)?;
        self.forget_exec(loc);
        Ok(Started { locator: loc.clone(), conversation_id: None, resumed: false, cwd: launch.cwd.to_string(), term: None, kind: None })
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
        self.forget_exec(loc);
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

    /// The VM as its admin user (agw's `vm exec`).
    fn exec(&self, m: &MachineId) -> Result<Arc<dyn Exec>> {
        let agw = self.agw()?;
        let x = self.cached_exec(&format!("m:{m}"), || {
            Ok(AgwExec::new(&agw, self.cfg.exec.clone(), User::Admin { vm: m.to_string() }, None))
        })?;
        Ok(x)
    }

    /// As the user the session runs as, in its workspace.
    fn exec_at(&self, loc: &Locator) -> Result<Arc<dyn Exec>> {
        let x = self.session_exec(loc)?;
        Ok(x)
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
    use std::collections::BTreeMap;

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
        x.on(&["/opt/agw", "--non-interactive", "session", "list", "--vm", "my-vm", "--status", "--output", "json"], LIST);
        x.on(&["/opt/agw", "--non-interactive", "workspace", "describe", "api-session", "--output", "json"], include_str!("fixtures/workspace_describe.json"));
        x.on_exit(&["/opt/agw", "--non-interactive", "workspace", "describe", "work", ".."], 1, "", "Error: boom");
        x.on(&["/opt/agw", "--non-interactive", "--version"], "0.19.0\n");
        let p = provider(&x);
        assert_eq!(p.version().as_deref(), Some("0.19.0"));
        let m = p.machines().unwrap();
        assert_eq!((m[0].id.as_str(), m[0].label.as_str(), m[0].detail.as_deref()), ("my-vm", "my-vm", Some("local-site")));
        let d = p.discover(&MachineId::new("my-vm")).unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].locator.to_string(), "agw:my-vm/api-session");
        assert_eq!((d[0].kind.as_str(), d[0].state), ("claude-code", Some(NativeState::Running)));
        assert_eq!(d[0].cwd.as_deref(), Some("/opt/agentworks/workspaces/api-session"));
        assert_eq!((d[1].state, d[1].cwd.as_deref(), d[1].user.as_deref()), (Some(NativeState::Stopped), None, None));
        // Only read-only commands ran.
        assert!(x.calls().iter().all(|c| ["vm", "session", "workspace", "--version"].contains(&c.argv[2].as_str())));
        assert!(x.calls().iter().all(|c| !c.argv.iter().any(|a| ["start", "stop", "delete", "create", "attach"].contains(&a.as_str()))));
    }

    #[test]
    fn state_follows_agw_status() {
        let x = FakeExec::new();
        x.on(&["/opt/agw", "--non-interactive", "session", "list", "--vm", "my-vm", "--status", "--output", "json"], LIST);
        x.on_exit(&["/opt/agw", "--non-interactive", "session", "list", "--vm", "gone", ".."], 1, "", "Error: unknown VM 'gone'\n  Hint: x");
        x.on(
            &["/opt/agw", "--non-interactive", "session", "list", "--vm", "dim", ".."],
            include_str!("fixtures/session_list_nostatus.json").replace("my-vm", "dim").as_str(),
        );
        let p = provider(&x);
        assert_eq!(p.state(&loc("my-vm", "api-session")).unwrap(), NativeState::Running);
        assert_eq!(p.state(&loc("my-vm", "work")).unwrap(), NativeState::Stopped);
        assert_eq!(p.state(&loc("my-vm", "nope")).unwrap(), NativeState::Gone);
        assert_eq!(p.state(&loc("gone", "x")).unwrap(), NativeState::Gone);
        assert!(p.state(&loc("dim", "api-session")).unwrap_err().is(ErrorCode::Unreachable));
        // Not running: nothing to attach to, and nothing is started.
        assert!(p.attach(&loc("my-vm", "work"), TermSize::DEFAULT).err().unwrap().is(ErrorCode::NotRunning));
        let local = Locator::local("a");
        assert!(p.state(&local).unwrap_err().is(ErrorCode::NotFound));
    }

    #[test]
    fn stop_and_start_are_agws_own_commands() {
        let x = FakeExec::new();
        x.on(&["/opt/agw", "--non-interactive", "session", "stop", "api-session"], "");
        x.on(&["/opt/agw", "--non-interactive", "session", "start", "api-session"], "");
        x.on_exit(&["/opt/agw", "--non-interactive", "session", "start", "work"], 1, "", "Error: VM is stopped\n");
        let p = provider(&x);
        p.stop(&loc("my-vm", "api-session")).unwrap();
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
        let s = p.start(&loc("my-vm", "api-session"), &spec).unwrap();
        assert!(s.term.is_none() && !s.resumed && s.conversation_id.is_none());
        assert_eq!(p.start(&loc("my-vm", "work"), &spec).err().unwrap().message, "agw: VM is stopped");
        spec.intent = LaunchIntent::Resume("c".into());
        assert!(p.start(&loc("my-vm", "api-session"), &spec).err().unwrap().is(ErrorCode::Unsupported));
        assert_eq!(x.ran(&["/opt/agw", "--non-interactive", "session", ".."]), 3, "a resume never reaches agw");
        assert!(!x.calls().iter().any(|c| c.argv.iter().any(|a| a == "--force-new" || a == "delete")));
    }

    #[test]
    fn capture_reads_the_session_socket_over_agws_ssh_alias() {
        let x = FakeExec::new();
        x.on(&["/opt/agw", "--non-interactive", "session", "list", "--vm", "my-vm", "--output", "json"], LIST);
        x.on(&["ssh", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "awvm--my-vm", "--", "*"], "screen\n");
        let p = provider(&x);
        assert_eq!(p.capture(&loc("my-vm", "api-session")).unwrap(), "screen\n");
        let script = x.calls().last().unwrap().argv.last().unwrap().clone();
        assert_eq!(script, capture_script("api-session", None));
        assert!(script.contains("/run/agentworks/admin-tmux-sockets/\"$(id -un)\"/'api-session'.sock") && script.contains("-t '=api-session:'"));
        assert!(capture_script("s", Some("bot")).contains("/run/agentworks/agent-tmux-sockets/'agt-bot'/'s'.sock"));
        assert!(p.capture(&loc("my-vm", "nope")).err().unwrap().is(ErrorCode::NotRunning));
    }

    #[test]
    fn what_is_not_here_yet_is_unsupported() {
        let x = FakeExec::new();
        let p = provider(&x);
        assert!(p.process_cwd(&loc("my-vm", "api-session"), None).unwrap_err().is(ErrorCode::Unsupported));
        let caps = p.caps();
        assert!(caps.create && caps.platform_create && !caps.resume && !caps.rules && !caps.process_cwd && caps.hooks == HookTransport::None);
        assert!(caps.exec && caps.git_poll_ms >= 10_000, "diffs and Review, polled rarely");
        assert!(caps.attach_existing && caps.start && caps.capture && !caps.local_process);
        assert!(p.remove(&loc("my-vm", "api-session")).is_ok(), "nothing attached: nothing to do");
        assert!(x.calls().is_empty(), "remove never asks agw to delete anything");
    }

    #[test]
    fn exec_runs_as_the_sessions_user_in_its_workspace() {
        let x = FakeExec::new();
        let describe = include_str!("fixtures/session_describe.json");
        x.on(&["/opt/agw", "--non-interactive", "session", "describe", "work", "--output", "json"], describe);
        let as_agent = describe.replace("\"name\":\"work\"", "\"name\":\"bot-s\"").replace("\"mode\":\"admin\",\"agent_name\":null", "\"mode\":\"agent\",\"agent_name\":\"bot\"");
        x.on(&["/opt/agw", "--non-interactive", "session", "describe", "bot-s", "--output", "json"], &as_agent);
        x.on(&["/opt/agw", "--non-interactive", "session", "start", "work"], "");
        let p = provider(&x);
        let admin = p.session_exec(&loc("my-vm", "work")).unwrap();
        assert_eq!((admin.user(), admin.workspace()), (&User::Admin { vm: "my-vm".into() }, Some("work")));
        assert_eq!(admin.argv("s")[2..8], ["vm", "exec", "--workspace", "work", "my-vm", "--"]);
        let agent = p.session_exec(&loc("my-vm", "bot-s")).unwrap();
        assert_eq!(agent.user(), &User::Agent { agent: "bot".into() });
        assert_eq!(agent.argv("s")[2..8], ["agent", "exec", "--workspace", "work", "bot", "--"]);
        // Asked once per session; again after a start.
        p.session_exec(&loc("my-vm", "work")).unwrap();
        let described = || x.ran(&["/opt/agw", "--non-interactive", "session", "describe", "work", ".."]);
        assert_eq!(described(), 1);
        let kind = pitwall_core::kind::custom_kind("x");
        let spec = LaunchSpec { agent: "a", kind: &kind, cwd: "/w", intent: LaunchIntent::Fresh, worktree: None, size: TermSize::DEFAULT, hooks: None };
        p.start(&loc("my-vm", "work"), &spec).unwrap();
        p.exec_at(&loc("my-vm", "work")).unwrap();
        assert_eq!(described(), 2);
        // The machine's exec is the admin's, with no workspace.
        assert!(p.exec(&MachineId::new("my-vm")).is_ok());
        assert!(p.exec_at(&Locator::local("a")).is_err_and(|e| e.is(ErrorCode::NotFound)));
        // Nothing but describe/start reached agw: no command ran on the VM.
        assert!(x.calls().iter().all(|c| c.argv[2] == "session"));
    }

    const AGW: &str = "/opt/agw";

    fn listings_on(x: &FakeExec) {
        x.on(&[AGW, "--non-interactive", "workspace", "list", "--vm", "my-vm", "--output", "json"], include_str!("fixtures/workspace_list.json"));
        x.on(&[AGW, "--non-interactive", "agent", "list", "--vm", "my-vm", "--output", "json"], include_str!("fixtures/agent_list_empty.json"));
        x.on(
            &[AGW, "--non-interactive", "resource", "list", "--kind", "session-template,workspace-template,agent-template", "--output", "json"],
            include_str!("fixtures/resource_list.json"),
        );
    }

    #[test]
    fn the_create_form_comes_from_read_only_listings_and_is_cached() {
        let x = FakeExec::new();
        listings_on(&x);
        let p = provider(&x);
        let f = p.create_form(&MachineId::new("my-vm")).unwrap();
        assert_eq!((f.provider.as_str(), f.machine.as_str(), f.folder, f.error.as_deref()), ("agw", "my-vm", false, None));
        let ws: Vec<_> = f.field("workspace").unwrap().choices.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(ws, ["api-session", "work", "+new"]);
        let run_as: Vec<_> = f.field("runAs").unwrap().choices.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(run_as, ["admin", "+new"], "no agent users on this VM yet");
        assert_eq!(x.calls().len(), 3);
        p.create_form(&MachineId::new("my-vm")).unwrap();
        assert_eq!(x.calls().len(), 3, "asked again within a few seconds: cached");
        // Only read-only listings ran, each time-boxed.
        assert!(x.calls().iter().all(|c| c.argv.contains(&"list".to_string()) && c.timeout <= create::OPTIONS_TIMEOUT));
    }

    #[test]
    fn a_vm_that_cant_be_listed_still_gets_a_form() {
        let x = FakeExec::new();
        x.on_exit(&[AGW, "--non-interactive", "workspace", "list", ".."], 1, "", "Error: VM 'my-vm' is stopped\n  Hint: agw vm start my-vm");
        x.on_exit(&[AGW, "--non-interactive", "agent", "list", ".."], 1, "", "Error: VM 'my-vm' is stopped");
        x.on(&[AGW, "--non-interactive", "resource", "list", ".."], include_str!("fixtures/resource_list.json"));
        let f = provider(&x).create_form(&MachineId::new("my-vm")).unwrap();
        assert_eq!(
            f.error.as_deref(),
            Some("couldn't list workspaces: VM 'my-vm' is stopped; couldn't list agents: VM 'my-vm' is stopped")
        );
        assert_eq!(f.field("workspace").unwrap().default.as_deref(), Some("+new"));
    }

    #[test]
    fn create_runs_agws_create_then_reads_what_runs_there() {
        let x = FakeExec::new();
        listings_on(&x);
        let created = [AGW, "--non-interactive", "session", "create", "--vm", "my-vm", "--workspace", "work", "--template", "claude", "--admin", "api-fix"];
        x.on(&created, "Created session 'api-fix'\n");
        let describe = include_str!("fixtures/session_describe.json").replace("\"name\":\"work\"", "\"name\":\"api-fix\"");
        x.on(&[AGW, "--non-interactive", "session", "describe", "api-fix", "--output", "json"], &describe);
        x.on(&[AGW, "--non-interactive", "workspace", "describe", "work", "--output", "json"], include_str!("fixtures/workspace_describe.json"));
        let p = provider(&x);
        p.create_form(&MachineId::new("my-vm")).unwrap();
        let kind = pitwall_core::kind::custom_kind("x");
        let options: BTreeMap<String, String> =
            [("workspace", "work"), ("runAs", "admin"), ("template", "claude")].into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        let spec = CreateSpec {
            machine: &MachineId::new("my-vm"),
            name: "api-fix",
            options: &options,
            launch: LaunchSpec { agent: "a", kind: &kind, cwd: "", intent: LaunchIntent::Fresh, worktree: None, size: TermSize::DEFAULT, hooks: None },
        };
        let s = p.create(&spec).unwrap();
        assert_eq!(s.locator.to_string(), "agw:my-vm/api-fix");
        assert_eq!((s.kind.as_deref(), s.cwd.as_str()), (Some("claude-code"), "/opt/agentworks/workspaces/api-session"));
        assert!(s.term.is_none(), "the terminal comes from attach");
        let call = x.calls().into_iter().find(|c| c.argv.contains(&"create".to_string())).unwrap();
        assert_eq!(call.argv, created);
        assert_eq!(call.timeout, create::CREATE_TIMEOUT);
        // The VM changed: its form is listed again next time.
        let before = x.ran(&[AGW, "--non-interactive", "workspace", "list", ".."]);
        p.create_form(&MachineId::new("my-vm")).unwrap();
        assert_eq!(x.ran(&[AGW, "--non-interactive", "workspace", "list", ".."]), before + 1);

        // agw's refusal, in its own words (notices before it are skipped).
        x.on_exit(
            &created,
            1,
            "Checking session-template/claude...\n",
            "Notice: x\nError: session 'api-fix' already exists\n  Hint: Delete it first: agw session delete api-fix\n",
        );
        let e = p.create(&spec).err().unwrap();
        assert!(e.is(ErrorCode::Conflict));
        assert_eq!(e.message, "agw couldn't create \"api-fix\" on my-vm: session 'api-fix' already exists (Delete it first: agw session delete api-fix)");
        // A bad name never reaches agw.
        let n = x.calls().len();
        assert!(p.create(&CreateSpec { name: "Bad Name", ..spec.clone() }).is_err());
        assert_eq!(x.calls().len(), n);
        assert!(!x.calls().iter().any(|c| c.argv.iter().any(|a| a == "delete" || a == "--yes")), "nothing is ever deleted");
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
        assert!(p.create_form(&MachineId::new("vm")).unwrap_err().is(ErrorCode::Unsupported));
        assert_eq!(p.version(), None);
    }
}
