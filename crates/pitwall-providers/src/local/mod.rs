//! The local provider: agents on this Mac (architecture.md §2.7, "The local
//! provider is today's code"). `create`/`start` turn the kind into a command
//! line with `launch::plan` and run it through the login shell in a new
//! `pitwall-hold` holder; the terminal is a connection to that holder, so
//! agents outlive Pitwall and `attach` finds them again after a restart
//! (§9 decision 1). `exec` is `LocalExec`; `process_cwd` uses lsof.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pitwall_core::exec::{Exec, LocalExec};
use pitwall_core::kind::catalog::program;
use pitwall_core::kind::{launch, KindCatalog};
use pitwall_core::provider::{
    CreateSpec, Discovered, ErrorCode, HookTransport, KindOnMachine, LaunchIntent, LaunchSpec, Locator, Machine,
    MachineId, NativeState, Provider, ProviderCaps, ProviderId, PwError, Result, Started, TermIo, TermSize,
};
use crate::hold;
pub use crate::hold::HoldTerm;
use pitwall_hold::client;

/// How long `stop` gives an agent after the hang-up before it is killed.
const STOP_GRACE: Duration = Duration::from_millis(1500);
const LSOF_TIMEOUT: Duration = Duration::from_secs(3);

/// Where the local provider keeps its holders and what it hands agents.
#[derive(Debug, Clone)]
pub struct LocalConfig {
    /// Holder sockets: `<agentId>.sock` each (`Paths::hold_dir`).
    pub hold_dir: PathBuf,
    /// The `pitwall-hold` executable, or why there is none (reported when an
    /// agent is started).
    pub holder_bin: std::result::Result<PathBuf, String>,
    /// `PITWALL_SOCKET` for agents: where their hooks post (`Paths::hook_socket`).
    pub hook_socket: PathBuf,
    /// `PITWALL_CLI_SOCKET` for agents: where the `pitwall` CLI connects
    /// (`Paths::cli_socket`).
    pub cli_socket: PathBuf,
}

pub struct LocalProvider {
    id: ProviderId,
    machine: Machine,
    cfg: LocalConfig,
    exec: Arc<LocalExec>,
    /// Program → where it is installed (resolving spawns a login shell, so
    /// each program is looked up once).
    resolved: Mutex<HashMap<String, Option<String>>>,
}

fn resolve(program: &str) -> Option<String> {
    if program.is_empty() {
        return None;
    }
    if Path::new(program).is_absolute() {
        return Path::new(program).exists().then(|| program.to_string());
    }
    pitwall_core::shell::which(program)
}

impl LocalProvider {
    pub fn new(cfg: LocalConfig) -> LocalProvider {
        LocalProvider {
            id: ProviderId::local(),
            machine: Machine { id: MachineId::new(MachineId::THIS_MAC), label: pitwall_core::host::machine_label().into(), detail: None },
            cfg,
            exec: Arc::new(LocalExec),
            resolved: Mutex::default(),
        }
    }

    fn socket(&self, loc: &Locator) -> PathBuf {
        client::socket_path(&self.cfg.hold_dir, &loc.native)
    }

    fn mine(&self, loc: &Locator) -> Result<()> {
        if loc.provider != self.id || loc.machine != self.machine.id {
            return Err(PwError::not_found(format!("{loc} is not on this Mac")));
        }
        Ok(())
    }

    fn launch(&self, loc: Locator, spec: &LaunchSpec) -> Result<Started> {
        let holder = self.cfg.holder_bin.as_deref().map_err(|e| PwError::unsupported(e.clone()))?;
        let (session_id, resume) = match &spec.intent {
            LaunchIntent::Fresh => (None, false),
            LaunchIntent::Resume(id) => (Some(id.as_str()), true),
        };
        let hook = spec.hooks.as_ref().map_or("", |h| h.command.as_str());
        let plan = launch::plan(spec.kind, session_id, resume, hook, spec.worktree);
        let socket = self.socket(&loc);
        let term = hold::spawn(hold::Spawn {
            agent_id: spec.agent,
            cwd: spec.cwd,
            command_line: &plan.command_line,
            size: spec.size,
            socket: &socket,
            holder,
            hook_socket: &self.cfg.hook_socket,
            cli_socket: &self.cfg.cli_socket,
        })?;
        Ok(Started {
            locator: loc,
            conversation_id: plan.session_id,
            resumed: plan.resumed,
            cwd: spec.cwd.to_string(),
            term: Some(Box::new(term)),
            kind: None,
        })
    }

    /// The agent's process, from its holder.
    fn child_pid(&self, loc: &Locator) -> Option<u32> {
        let conn = client::connect(&self.socket(loc), hold::CONNECT_TIMEOUT).ok()?;
        conn.info.exit.is_none().then_some(conn.info.child_pid)
    }
}

impl Provider for LocalProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn caps(&self) -> ProviderCaps {
        ProviderCaps {
            create: true,
            platform_create: false,
            resume: true,
            start: true,
            // Agents started elsewhere on this Mac are listed by the onboarding
            // scan, not adopted (their terminal belongs to another app).
            attach_existing: false,
            // The holder keeps the agent when Pitwall goes away.
            survives_detach: true,
            exec: true,
            process_cwd: true,
            capture: false,
            hooks: HookTransport::LocalSocket,
            rules: true,
            custom_command: true,
            local_process: true,
            git_poll_ms: 0,
        }
    }

    fn machines(&self) -> Result<Vec<Machine>> {
        Ok(vec![self.machine.clone()])
    }

    /// Kinds with "installed" resolved in the user's login shell (each
    /// program once; the first call is slow).
    fn kinds(&self, _m: &MachineId, catalog: &KindCatalog) -> Result<Vec<KindOnMachine>> {
        let kinds = catalog.kinds();
        let mut resolved = self.resolved.lock().unwrap_or_else(|e| e.into_inner());
        Ok(kinds
            .into_iter()
            .map(|kind| {
                let p = program(&kind.command);
                let path = resolved.entry(p.clone()).or_insert_with(|| resolve(&p)).clone();
                KindOnMachine { installed: path.is_some(), path, kind }
            })
            .collect())
    }

    /// Nothing yet: CLIs running in other terminals are listed by the
    /// onboarding scan and the sidebar's "Elsewhere" group
    /// (`onboarding::elsewhere`); they move here with agw's discovery (step 8).
    fn discover(&self, _m: &MachineId) -> Result<Vec<Discovered>> {
        Ok(Vec::new())
    }

    fn create(&self, spec: &CreateSpec) -> Result<Started> {
        if *spec.machine != self.machine.id {
            return Err(PwError::not_found(format!("no machine {} here", spec.machine)));
        }
        if !spec.options.is_empty() {
            return Err(PwError::unsupported("agents on this Mac work in a folder"));
        }
        let loc = Locator::new(&self.id, &self.machine.id, spec.launch.agent);
        self.launch(loc, &spec.launch)
    }

    fn start(&self, loc: &Locator, launch: &LaunchSpec) -> Result<Started> {
        self.mine(loc)?;
        self.launch(loc.clone(), launch)
    }

    /// The agent's holder from an earlier run of Pitwall, while the agent
    /// runs. Never starts anything.
    fn attach(&self, loc: &Locator, _size: TermSize) -> Result<Box<dyn TermIo>> {
        self.mine(loc)?;
        let socket = self.socket(loc);
        match HoldTerm::connect(&socket, false) {
            Ok(t) => Ok(Box::new(t)),
            Err(e) => Err(match e.kind() {
                // No holder: the agent was stopped, or exited.
                std::io::ErrorKind::NotFound => PwError::not_running(),
                // The holder is gone (killed); only its socket is left.
                std::io::ErrorKind::ConnectionRefused => {
                    client::remove_stale(&socket);
                    PwError::not_running()
                }
                _ => PwError::new(ErrorCode::Other, format!("could not attach to {loc}: {e}")),
            }),
        }
    }

    fn stop(&self, loc: &Locator) -> Result<()> {
        self.mine(loc)?;
        hold::shutdown(&self.socket(loc), STOP_GRACE)
    }

    /// A local agent leaves nothing behind once stopped.
    fn remove(&self, loc: &Locator) -> Result<()> {
        self.mine(loc)
    }

    fn state(&self, loc: &Locator) -> Result<NativeState> {
        self.mine(loc)?;
        Ok(match client::connect(&self.socket(loc), hold::CONNECT_TIMEOUT) {
            Ok(c) if c.info.exit.is_none() => NativeState::Running,
            Ok(_) => NativeState::Stopped,
            Err(_) => NativeState::Gone,
        })
    }

    fn exec(&self, m: &MachineId) -> Result<Arc<dyn Exec>> {
        if *m != self.machine.id {
            return Err(PwError::not_found(format!("no machine {m} here")));
        }
        Ok(self.exec.clone())
    }

    fn process_cwd(&self, loc: &Locator, pid: Option<u32>) -> Result<Option<String>> {
        self.mine(loc)?;
        let Some(pid) = pid.or_else(|| self.child_pid(loc)) else { return Ok(None) };
        Ok(pitwall_core::host::process_cwd(pid, LSOF_TIMEOUT))
    }

    fn process_cwds(&self, asks: &[(Locator, Option<u32>)]) -> Vec<Option<String>> {
        let pids: Vec<Option<u32>> =
            asks.iter().map(|(loc, pid)| self.mine(loc).ok().and_then(|_| pid.or_else(|| self.child_pid(loc)))).collect();
        let known: Vec<u32> = pids.iter().flatten().copied().collect();
        let found = if known.is_empty() { None } else { pitwall_core::host::process_cwds(&known, LSOF_TIMEOUT) };
        let found = found.unwrap_or_default();
        pids.iter().map(|p| p.and_then(|p| found.iter().find(|(q, _)| *q == p).map(|(_, cwd)| cwd.clone()))).collect()
    }

    fn capture(&self, _loc: &Locator) -> Result<String> {
        Err(PwError::unsupported("this Mac's terminals are read through their holder, not captured"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(holder: std::result::Result<PathBuf, String>) -> LocalProvider {
        let dir = std::env::temp_dir().join(format!("pw-local-{}", std::process::id()));
        LocalProvider::new(LocalConfig { hold_dir: dir.join("hold"), holder_bin: holder, hook_socket: dir.join("hook.sock"), cli_socket: dir.join("cli.sock") })
    }

    #[test]
    fn without_a_holder_binary_nothing_starts() {
        let p = provider(Err("the terminal holder (pitwall-hold) is missing".into()));
        let kind = pitwall_core::kind::custom_kind("true");
        let m = MachineId::new(MachineId::THIS_MAC);
        let spec = CreateSpec {
            machine: &m,
            name: "x",
            options: &Default::default(),
            launch: LaunchSpec {
                agent: "a1",
                kind: &kind,
                cwd: "/",
                intent: LaunchIntent::Fresh,
                worktree: None,
                size: TermSize::DEFAULT,
                hooks: None,
            },
        };
        let e = p.create(&spec).err().unwrap();
        assert_eq!((e.code, e.message.as_str()), (ErrorCode::Unsupported, "the terminal holder (pitwall-hold) is missing"));
        let options = std::collections::BTreeMap::from([("workspace".to_string(), "w".to_string())]);
        let other = CreateSpec { options: &options, ..spec.clone() };
        assert!(p.create(&other).err().unwrap().is(ErrorCode::Unsupported));
    }

    #[test]
    fn other_places_are_not_this_mac() {
        let p = provider(Err("none".into()));
        let elsewhere = Locator::new(&ProviderId::new("agw"), &MachineId::new("vm"), "s");
        assert!(p.state(&elsewhere).unwrap_err().is(ErrorCode::NotFound));
        assert!(p.exec(&MachineId::new("vm")).is_err());
        let here = Locator::local("never-started");
        assert_eq!(p.state(&here).unwrap(), NativeState::Gone);
        assert!(p.attach(&here, TermSize::DEFAULT).err().unwrap().is(ErrorCode::NotRunning));
        assert!(p.stop(&here).is_ok());
        assert!(p.capture(&here).unwrap_err().is(ErrorCode::Unsupported));
        assert_eq!(p.process_cwd(&here, None).unwrap(), None);
    }

    #[test]
    fn process_folders_are_asked_in_one_go() {
        let p = provider(Err("none".into()));
        let here = Locator::local("never-started");
        let elsewhere = Locator::new(&ProviderId::new("agw"), &MachineId::new("vm"), "s");
        let me = std::process::id();
        let got = p.process_cwds(&[(here.clone(), Some(me)), (here, None), (elsewhere, Some(me))]);
        let cwd = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].as_deref().map(std::path::Path::new), Some(cwd.as_path()));
        assert_eq!((got[1].as_deref(), got[2].as_deref()), (None, None), "no process; not this Mac");
    }
}
