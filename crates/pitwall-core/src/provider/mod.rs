//! Places agents run (architecture.md §2.1–2.3, §3). A [`Provider`] starts,
//! attaches to and stops agents on its machines and runs commands there; the
//! engine talks to agents only through providers. A provider supplies a raw
//! terminal byte pipe ([`TermIo`]); everything Pitwall does with the bytes
//! (ring, replay, fan-out, `Screen`, activity, paste) is written once, in
//! [`TermHost`](crate::term::TermHost).
//!
//! Providers live in `pitwall-providers` (local, agw); the engine branches on
//! [`ProviderCaps`], never on a [`ProviderId`].

mod registry;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use crate::error::{ErrorCode, PwError, Result};
use crate::exec::Exec;
use crate::kind::{AgentKind, KindCatalog};
pub use pitwall_proto::{CreateChoice, CreateField, CreateForm, FieldInput, NameRule, CREATE_NEW};
pub use registry::Providers;

// ------------------------------------------------------------------ identity

/// "local", "agw", "ssh:devbox". Stable; part of persisted state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderId(pub String);

/// Within a provider: "this-mac", an agw VM name, an ssh host alias.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MachineId(pub String);

impl ProviderId {
    /// The provider that runs agents on this Mac. Named in core only because
    /// state files from before providers (v1) mean it.
    pub const LOCAL: &'static str = "local";

    pub fn new(id: &str) -> ProviderId {
        ProviderId(id.to_string())
    }
    pub fn local() -> ProviderId {
        ProviderId::new(ProviderId::LOCAL)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl MachineId {
    /// The local provider's one machine.
    pub const THIS_MAC: &'static str = "this-mac";

    pub fn new(id: &str) -> MachineId {
        MachineId(id.to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Display for MachineId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where an agent lives, in the provider's own terms. Unique across Pitwall;
/// the agent id stays the primary key (§2.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Locator {
    pub provider: ProviderId,
    pub machine: MachineId,
    /// The provider's own handle: an agw session name, a tmux session name,
    /// or (local) the Pitwall agent id, because a local PTY has no other name.
    pub native: String,
}

impl Locator {
    pub fn new(provider: &ProviderId, machine: &MachineId, native: &str) -> Locator {
        Locator { provider: provider.clone(), machine: machine.clone(), native: native.to_string() }
    }

    /// What every agent from a v1 state file is: `local:this-mac/<id>`.
    pub fn local(agent_id: &str) -> Locator {
        Locator::new(&ProviderId::local(), &MachineId::new(MachineId::THIS_MAC), agent_id)
    }
}

impl std::fmt::Display for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}/{}", self.provider, self.machine, self.native)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    pub id: MachineId,
    /// Shown to people ("This Mac", a VM name).
    pub label: String,
    /// A short note shown next to the label (where a VM lives), if any.
    pub detail: Option<String>,
}

// ------------------------------------------------------------------ capabilities

/// How hook payloads from an agent reach Pitwall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HookTransport {
    /// No hooks: status comes from the screen and output activity.
    #[default]
    None,
    /// The agent posts to Pitwall's hook socket on this Mac.
    LocalSocket,
    /// A remote agent posts to a socket forwarded back here (ssh -R).
    Forwarded,
}

/// What a provider can do (§3). Claims must be true: the contract suite
/// checks that each `false` answers `Unsupported`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCaps {
    /// Can start new agents.
    pub create: bool,
    /// New agents are made by the platform from its own form (agw:
    /// workspace, user, session template — `create_form` has no folder),
    /// not as a kind in a folder Pitwall picks. Its machines then have no
    /// "new agent or terminal in this folder" (`MachineView.can_create`).
    pub platform_create: bool,
    /// Can continue a conversation on restart.
    pub resume: bool,
    /// `start` works: a stopped agent can be started again (fresh, or
    /// however its platform continues it).
    pub start: bool,
    /// Can adopt sessions found by `discover()`: `attach` connects to them.
    pub attach_existing: bool,
    /// The session outlives its attachment: `attach` reconnects to it.
    pub survives_detach: bool,
    /// Can run commands where the agent works (diffs, Review).
    pub exec: bool,
    /// Can find where a running agent works (`process_cwd`).
    pub process_cwd: bool,
    /// Plain-text screen without attaching (`capture`).
    pub capture: bool,
    pub hooks: HookTransport,
    /// Rule files can be generated where the agent works.
    pub rules: bool,
    /// Runs any command line the user types ("Custom command").
    pub custom_command: bool,
    /// The agent's process is a process on this Mac: its pid means something
    /// to the local process table (terminals: recognising agents started by
    /// hand; worktree discovery by process cwd).
    pub local_process: bool,
    /// How often the engine may refresh an agent's git changes, in ms; 0 =
    /// the engine's default (every few seconds while the agent works or
    /// prints). A provider whose `exec` is slow (one remote round trip per
    /// call) sets it: its agents are then refreshed only while working, at
    /// most this often, plus once when a task ends and whenever asked.
    pub git_poll_ms: u32,
}

// ------------------------------------------------------------------ terminals

/// A terminal size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

impl TermSize {
    pub const DEFAULT: TermSize = TermSize { cols: 120, rows: 40 };

    /// Within the sizes Pitwall accepts (2…1000 each way).
    pub fn new(cols: u16, rows: u16) -> TermSize {
        TermSize { cols: cols.clamp(2, 1000), rows: rows.clamp(2, 1000) }
    }

    pub fn from_pair(size: Option<(u16, u16)>) -> TermSize {
        size.map_or(TermSize::DEFAULT, |(c, r)| TermSize::new(c, r))
    }

    pub fn pair(self) -> (u16, u16) {
        (self.cols, self.rows)
    }
}

/// How a terminal's process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitInfo {
    /// Exit status, or 128 + signal; `None` when unknown (a dropped link).
    pub code: Option<i32>,
}

/// A raw terminal connection: a local PTY (through its `pitwall-hold`
/// holder), or a PTY running `agw session attach <name>` /
/// `ssh -t host tmux attach -t <s>`.
pub trait TermIo: Send + Sync {
    /// Output from before this connection (a holder's or tmux's history), if
    /// the provider has any. Shown and replayed, but not counted as the agent
    /// being active. Called once, before `take_reader`.
    fn take_history(&mut self) -> Vec<u8> {
        Vec::new()
    }
    /// Live output; EOF when the connection ends. Called once.
    fn take_reader(&mut self) -> Box<dyn Read + Send>;
    /// Keystrokes and pastes for the agent. Called once. Must not block for
    /// long (buffer and send from a thread if the link is slow).
    fn take_writer(&mut self) -> Box<dyn Write + Send>;
    fn resize(&self, size: TermSize) -> Result<()>;
    /// `Some` once the connection ended; `code` when the agent exited.
    fn try_wait(&self) -> Result<Option<ExitInfo>>;
    /// Local: hang up and, after `grace`, kill the agent (blocks until it is
    /// gone). Remote: end the *attachment* only.
    fn close(&self, grace: Duration);
    /// The process at the other end, when it is a local process
    /// (`ProviderCaps::local_process`).
    fn pid(&self) -> Option<u32>;
    /// Whether EOF means "the agent exited" (local) or only "the attachment
    /// dropped" (tmux), in which case the engine asks `Provider::state` and
    /// may `attach` again.
    fn eof_is_exit(&self) -> bool;
    /// The size the other end has now (a holder knows it after a re-attach).
    fn size(&self) -> Option<TermSize> {
        None
    }
}

// ------------------------------------------------------------------ provider

/// A kind with "installed" resolved on one machine.
#[derive(Debug, Clone)]
pub struct KindOnMachine {
    pub kind: AgentKind,
    pub installed: bool,
    /// Where its program is, when known.
    pub path: Option<String>,
}

/// A session that exists on a machine but may not be in Pitwall yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub locator: Locator,
    /// What runs in it, in the platform's words ("claude-code", "shell");
    /// matched against kind ids and aliases by the engine.
    pub kind: String,
    /// Running, stopped, or `None` when the platform couldn't tell.
    pub state: Option<NativeState>,
    /// Where it works, on its machine.
    pub cwd: Option<String>,
    pub title: Option<String>,
    /// The platform's workspace it belongs to, if it has such a thing.
    pub workspace: Option<String>,
    /// The user it runs as when that isn't the machine's main user (an agw
    /// agent user).
    pub user: Option<String>,
}

/// The provider's own idea of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeState {
    Running,
    Stopped,
    /// The provider has never heard of it, or forgot it.
    Gone,
}

/// Fresh start or continue a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchIntent {
    Fresh,
    /// Continue this conversation id (the kind's resume args, or agw
    /// `start --resume-only`).
    Resume(String),
}

/// How an agent's hooks reach Pitwall (only when `caps.hooks != None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookWiring {
    /// The hook relay command (`sh ~/.pitwall/bin/pitwall-hook`).
    pub command: String,
}

/// What to run, for `create` and `start`. Providers that launch a command
/// themselves turn it into a command line with
/// [`launch::plan`](crate::kind::launch::plan); providers whose platform owns
/// the launch (agw harness integrations) use the kind and intent only.
#[derive(Debug, Clone)]
pub struct LaunchSpec<'a> {
    /// Pitwall's agent id (`PITWALL_AGENT_ID`, the local native handle).
    pub agent: &'a str,
    pub kind: &'a AgentKind,
    /// The folder it runs in, on its machine.
    pub cwd: &'a str,
    pub intent: LaunchIntent,
    /// On a fresh start: the name for the kind's own worktree flag.
    pub worktree: Option<&'a str>,
    pub size: TermSize,
    pub hooks: Option<HookWiring>,
}

#[derive(Debug, Clone)]
pub struct CreateSpec<'a> {
    pub machine: &'a MachineId,
    /// The agent's name; a platform that names its sessions (agw) uses it
    /// as the session name.
    pub name: &'a str,
    /// The machine's form values ([`Provider::create_form`], checked and
    /// defaulted by the engine); empty for Pitwall's own folder form.
    pub options: &'a BTreeMap<String, String>,
    pub launch: LaunchSpec<'a>,
}

pub struct Started {
    pub locator: Locator,
    pub conversation_id: Option<String>,
    /// The conversation was continued (else it started fresh).
    pub resumed: bool,
    /// Where it runs, on the agent's machine.
    pub cwd: String,
    /// The live terminal when the provider's session *is* the process
    /// (local); `None` ⇒ the engine calls `attach`.
    pub term: Option<Box<dyn TermIo>>,
    /// What runs, in the platform's words ("claude-code"), when the
    /// platform chose it (agw session templates); matched against kind ids
    /// and aliases like a discovered session's. `None`: the requested kind.
    pub kind: Option<String>,
}

/// A place agents run (§2.2). Blocking, like the rest of the core; the
/// engine calls it off the UI thread.
pub trait Provider: Send + Sync {
    fn id(&self) -> &ProviderId;
    fn caps(&self) -> ProviderCaps;
    /// How it is shown to people ("agw"); the id by default.
    fn label(&self) -> String {
        self.id().to_string()
    }
    /// The platform's version, when there is a platform tool to ask. Blocking.
    fn version(&self) -> Option<String> {
        None
    }

    /// Machines this provider can run on (local: exactly one).
    fn machines(&self) -> Result<Vec<Machine>>;
    /// Kinds usable on a machine, with "installed" resolved there.
    fn kinds(&self, m: &MachineId, catalog: &KindCatalog) -> Result<Vec<KindOnMachine>>;
    /// Sessions that exist on a machine but may not be in Pitwall yet.
    /// Read-only.
    fn discover(&self, m: &MachineId) -> Result<Vec<Discovered>>;

    /// How new agents are made on `m`: Pitwall's own folder form by
    /// default; a platform lists its own choices (agw: workspace, user,
    /// session template). Blocking; may be cached briefly.
    fn create_form(&self, m: &MachineId) -> Result<CreateForm> {
        Ok(CreateForm::folder(self.id().as_str(), m.as_str(), m.as_str()))
    }
    /// Start a new agent.
    fn create(&self, spec: &CreateSpec) -> Result<Started>;
    /// Start an existing agent again (`launch.intent`: fresh or resume).
    /// `Unsupported` unless `caps.start` (and `caps.resume` for a resume).
    fn start(&self, loc: &Locator, launch: &LaunchSpec) -> Result<Started>;
    /// Connect to a session that is already running: after Pitwall itself
    /// restarted (local: the agent's holder), or a dropped attachment.
    /// `NotRunning` when there is nothing to attach to.
    fn attach(&self, loc: &Locator, size: TermSize) -> Result<Box<dyn TermIo>>;
    /// End the agent (the engine closes its terminal first when closing
    /// the terminal is what ends it).
    fn stop(&self, loc: &Locator) -> Result<()>;
    /// Pitwall forgets the agent: drop what the provider keeps for it. A
    /// platform's session (agw) is never deleted here — adopted or created
    /// through Pitwall, it stays; only Pitwall's attachment ends.
    fn remove(&self, loc: &Locator) -> Result<()>;
    fn state(&self, loc: &Locator) -> Result<NativeState>;

    /// Run commands and read files where the agent works (§2.4).
    fn exec(&self, m: &MachineId) -> Result<Arc<dyn Exec>>;
    /// Commands and files where the agent at `loc` works, as the user it runs
    /// as there (git, snapshots, Review). Default: [`exec`](Self::exec) on
    /// its machine.
    fn exec_at(&self, loc: &Locator) -> Result<Arc<dyn Exec>> {
        self.exec(&loc.machine)
    }
    /// The folder a running agent is in (worktree discovery).
    fn process_cwd(&self, loc: &Locator, pid: Option<u32>) -> Result<Option<String>>;
    /// Plain-text screen without attaching.
    fn capture(&self, loc: &Locator) -> Result<String>;
}
