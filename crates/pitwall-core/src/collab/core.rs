//! The conversation core (§6, §7, §8.3): one place for audiences, limits,
//! conversations, turns and the sessions behind them, whichever front a
//! message came through. [`Collab`] implements [`Conversations`] for the
//! fronts and the owner's share methods for the daemon.
//!
//! A conversation is one caller's (person, plus the asking agent's label)
//! with one share. Its session is an ordinary engine agent with
//! `AgentRecord.shared` ([`SessionHost`]), started in its own copy inside
//! the OS sandbox, resumed with the kind's resume after an idle stop.
//!
//! Locking: one mutex over the state. Nothing blocking happens under it;
//! host calls, copies and launches are [`Action`]s run after it is released
//! (inline, or on a thread of their own in the background mode).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::Duration;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use super::audit::{AuditLine, AuditLog};
use super::live::LiveBlock;
use super::people::{audience_via, People, Person};
use super::session::{
    can_resume, kind_argv, share_kind, HostLaunch, OwnerAgents, SessionEvent, SessionHost, SessionSkills, SharedSession, Snapshots,
    StartAs, EgressProxies,
};
use super::shares::ShareStore;
use super::store::{read_json, write_json, CollabPaths};
use super::teams::TeamStore;
use super::time::{self, WallClock};
use super::turn::{
    Ask, Asker, ChangesPart, ConversationId, ConversationSummary, Failure, Left, Refusal, ShareCard, Turn, TurnId,
    TurnStatus, Who,
};
use super::usage::{Usage, UsageRow};
use super::Conversations;
use crate::error::{PwError, Result};
use crate::kind::AgentKind;
use crate::net::{Caller, Identity, Network, NetworkId, PersonId, Revocation};
use crate::sandbox::{Egress, ExecRule, LocalEndpoint, SandboxPolicy, SessionSandbox};
use pitwall_proto::collab::{
    Capability, Front, Programs, RefusalReason, SessionCaller, SessionState, SessionUsage, ShareDiff, ShareSettings, ShareState,
    SharedSessionView, Start,
};

/// Shared conversations on this machine at once, by default (§5.8).
pub const DEFAULT_MAX_SESSIONS: u32 = 4;
/// A vendor limit with no reset time shown is assumed to hold this long.
const LIMIT_WITHOUT_TIME: Duration = Duration::from_secs(30 * 60);

/// How hooks reach Pitwall from a shared session (§8.5): the relay command
/// and the hook socket the sandbox lets it connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookWiring {
    pub command: String,
    pub socket: PathBuf,
    /// What the relay command runs or reads besides the shell the agent
    /// runs hooks with: readable and runnable inside the sandbox.
    pub programs: Vec<PathBuf>,
}

impl HookWiring {
    /// The `pitwall-hook` relay binary: the session runs it and nothing
    /// else for hooks (no `curl` in its programs).
    pub fn binary(relay: &Path, socket: &Path) -> HookWiring {
        HookWiring {
            command: crate::shell::quote(&relay.to_string_lossy()),
            socket: socket.to_path_buf(),
            programs: vec![relay.to_path_buf()],
        }
    }

    /// The `sh` + `curl` relay script (when the binary isn't there).
    pub fn script(script: &Path, socket: &Path) -> HookWiring {
        HookWiring {
            command: format!("sh {}", crate::shell::quote(&script.to_string_lossy())),
            socket: socket.to_path_buf(),
            programs: vec![script.to_path_buf(), PathBuf::from("/usr/bin/curl")],
        }
    }
}

/// Settings → Collaboration, as the core needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollabSettings {
    /// "At most N shared sessions on this machine" (§6.6).
    pub max_sessions: u32,
    /// The owner's short name for status lines ("waiting for alice").
    pub owner_name: String,
}

impl Default for CollabSettings {
    fn default() -> CollabSettings {
        CollabSettings { max_sessions: DEFAULT_MAX_SESSIONS, owner_name: "the owner".into() }
    }
}

/// Where blocking work runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// On the calling thread (tests: deterministic).
    Inline,
    /// On a thread of its own, so `ask` returns at once (the daemon).
    Background,
}

/// What [`Collab`] is built from.
pub struct CollabDeps {
    pub paths: CollabPaths,
    pub clock: Arc<dyn WallClock>,
    pub host: Arc<dyn SessionHost>,
    pub agents: Arc<dyn OwnerAgents>,
    pub sandbox: Arc<dyn SessionSandbox>,
    pub snapshots: Arc<dyn Snapshots>,
    pub egress: Arc<dyn EgressProxies>,
    pub skills: Option<Arc<dyn SessionSkills>>,
    pub teams: Option<Arc<TeamStore>>,
    pub hooks: Option<HookWiring>,
    pub settings: CollabSettings,
    pub run: RunMode,
}

/// One turn as the owner sees it ([`Collab::owner_turns`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerTurn {
    pub turn: Turn,
    /// What the caller wrote.
    pub text: String,
    /// What was typed into the session, once delivered.
    pub delivered: Option<String>,
    /// Pitwall's own (the approved summary as a first message).
    pub internal: bool,
}

/// Something the owner should hear about (badge, notification, §7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// "api-expert: bob@example.com's conversation needs you".
    NeedsYou { share: String, conversation: ConversationId, caller: String, detail: Option<String> },
    /// A turn ended with nothing captured; the session is flagged.
    NoAnswer { share: String, conversation: ConversationId },
    /// The share's agent time for today is used up: it pauses until tomorrow.
    ShareTotal { share: String },
    /// The agent's vendor limit.
    AgentLimit { share: String, until: Option<String> },
}

// ------------------------------------------------------------------ state

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    #[default]
    Caller,
    /// The share's base conversation: the approved summary, forked by new
    /// callers' sessions where the kind can (§6.4).
    Base,
}

/// The session process, as the core tracks it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Proc {
    #[default]
    Stopped,
    Starting,
    Ready,
    Working,
    Blocked,
}

impl Proc {
    fn running(self) -> bool {
        self != Proc::Stopped
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnRec {
    turn: Turn,
    /// The caller's text, as written.
    text: String,
    /// What was typed into the session (the "From" line and factual lines
    /// first), once delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delivered: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    /// Pitwall's own turn (the approved summary as a first message): never
    /// shown to callers, never counted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    internal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Conv {
    id: ConversationId,
    share: String,
    #[serde(default)]
    role: Role,
    /// The caller: person id and identity when the conversation began, and
    /// the asking agent's label (§6.1).
    person: String,
    identity: Identity,
    #[serde(default)]
    agent_label: Option<String>,
    caller: SessionCaller,
    /// The engine agent this conversation's session is.
    agent: String,
    /// The kind's conversation id, for its resume.
    #[serde(default)]
    session_id: Option<String>,
    /// A message reached the session, so resuming it means something.
    #[serde(default)]
    has_conversation: bool,
    /// Which approved summary the session started from (`approvedAt`).
    #[serde(default)]
    briefing: Option<String>,
    #[serde(default)]
    copy: Option<String>,
    current: bool,
    created_at: String,
    #[serde(default)]
    last_at: Option<String>,
    #[serde(default)]
    turns: Vec<TurnRec>,
    #[serde(skip)]
    proc: Proc,
    /// The last proof a network gave for this caller (audience re-checks).
    #[serde(skip)]
    proof: Option<Caller>,
    #[serde(skip)]
    last_used: i64,
    /// Restart with the kind's resume before the next turn (settings changed).
    #[serde(skip)]
    relaunch: bool,
    /// Rebuild the copy before the next turn (scope changed).
    #[serde(skip)]
    rebuild: bool,
    /// Factual lines for the next message (§8.3.1).
    #[serde(skip)]
    lines: Vec<String>,
    /// A turn ended with nothing captured (flag for the owner).
    #[serde(skip)]
    flagged: bool,
    /// Stops we asked for while it restarts: their `Exited` is expected.
    #[serde(skip)]
    stopping: u32,
    /// The session stopped and its kind can't resume: the next turn starts
    /// a new conversation, and the caller is told.
    #[serde(skip)]
    lost: bool,
}

impl Conv {
    fn active(&self) -> Option<usize> {
        self.turns.iter().position(|t| matches!(t.turn.status, TurnStatus::Working | TurnStatus::WaitingForOwner))
    }

    fn busy(&self) -> bool {
        self.active().is_some() || self.proc == Proc::Starting
    }

    fn belongs_to(&self, person: &Person) -> bool {
        self.role == Role::Caller && (person.is(&self.person) || person.has(&self.identity))
    }

    fn turn_mut(&mut self, id: &TurnId) -> Option<&mut TurnRec> {
        self.turns.iter_mut().find(|t| &t.turn.id == id)
    }

    fn public_turns(&self) -> impl Iterator<Item = &TurnRec> {
        self.turns.iter().filter(|t| !t.internal)
    }
}

#[derive(Debug, Clone)]
struct Limit {
    until: Option<String>,
    ends: Option<DateTime<FixedOffset>>,
}

#[derive(Default)]
struct State {
    convs: BTreeMap<ConversationId, Conv>,
    by_agent: HashMap<String, ConversationId>,
    /// Queued turns, oldest first, across every share (FIFO, §6.6).
    queue: Vec<TurnId>,
    watchers: HashMap<TurnId, Vec<Sender<Turn>>>,
    share_subs: Vec<Sender<()>>,
    notice_subs: Vec<Sender<Notice>>,
    /// Shares whose agent shows its vendor limit.
    limited: HashMap<String, Limit>,
    /// (day, share) the owner was told about `share_total`.
    told_total: HashSet<(String, String)>,
    seq: i64,
    usage: Option<Usage>,
}

/// Blocking work, run after the lock is released.
enum Action {
    Launch(Box<LaunchJob>),
    Send { agent: String, text: String },
    Interrupt { agent: String },
    Stop { agent: String },
    Remove { agent: String, folder: PathBuf },
    Save(Box<Conv>),
    Delete(PathBuf),
    Changed(Box<Turn>),
    SharesChanged,
    Notice(Notice),
    Audit(Box<AuditLine>),
}

struct LaunchJob {
    conv: Conv,
    share: ShareSettings,
    start: StartAs,
    make_copy: bool,
}

/// The collaboration core on the owner's side.
pub struct Collab {
    paths: CollabPaths,
    clock: Arc<dyn WallClock>,
    host: Arc<dyn SessionHost>,
    agents: Arc<dyn OwnerAgents>,
    sandbox: Arc<dyn SessionSandbox>,
    snapshots: Arc<dyn Snapshots>,
    egress: Arc<dyn EgressProxies>,
    skills: Option<Arc<dyn SessionSkills>>,
    teams: Option<Arc<TeamStore>>,
    hooks: Option<HookWiring>,
    settings: RwLock<CollabSettings>,
    run: RunMode,
    shares: ShareStore,
    people: People,
    audit: Arc<AuditLog>,
    networks: RwLock<Vec<Arc<dyn Network>>>,
    state: Mutex<State>,
    this: Weak<Collab>,
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix} in line")
}

fn refusal(reason: RefusalReason, message: String, resets_at: Option<String>, until: Option<String>) -> TurnStatus {
    TurnStatus::Refused(Refusal { reason, message, resets_at, until })
}

/// `bob@example.com`: how a person is named in lines and views.
fn login(identity: &Identity) -> String {
    identity.email.clone().unwrap_or_else(|| identity.subject.clone())
}

/// A reset time the agent showed ("2pm", "14:30") as the next such time.
fn parse_until(text: &str, now: DateTime<FixedOffset>) -> Option<DateTime<FixedOffset>> {
    let t = text.trim().to_lowercase();
    let (body, pm) = match t.strip_suffix("pm").or_else(|| t.strip_suffix(" pm")) {
        Some(b) => (b.trim().to_string(), Some(true)),
        None => match t.strip_suffix("am") {
            Some(b) => (b.trim().to_string(), Some(false)),
            None => (t.clone(), None),
        },
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok()?, 0),
    };
    let h = match pm {
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    let at = now.date_naive().and_hms_opt(h, m, 0)?;
    let at = now.offset().from_local_datetime(&at).single()?;
    Some(if at <= now { at + chrono::Duration::days(1) } else { at })
}

use chrono::TimeZone as _;

impl Collab {
    /// Load shares, conversations and usage. Sessions that were running
    /// when Pitwall last stopped are treated as stopped (their next message
    /// resumes them); turns that were in flight fail.
    pub fn open(deps: CollabDeps) -> Arc<Collab> {
        let CollabDeps { paths, clock, host, agents, sandbox, snapshots, egress, skills, teams, hooks, settings, run } = deps;
        let shares = ShareStore::open(paths.clone());
        let people = People::open(paths.people_file(), teams.clone());
        let audit = Arc::new(AuditLog::new(paths.audit_file()));
        let mut state = State { usage: Some(Usage::open(paths.usage_file())), ..State::default() };
        let now = time::rfc3339(clock.now());
        for share in shares.list() {
            let Ok(entries) = std::fs::read_dir(paths.share_dir(&share.slug).join("s")) else { continue };
            for entry in entries.flatten() {
                let Ok(Some(mut conv)) = read_json::<Conv>(&entry.path().join("conversation.json")) else { continue };
                for t in conv.turns.iter_mut().filter(|t| !t.turn.status.is_final()) {
                    t.turn.status = TurnStatus::Failed { reason: Failure::Crashed };
                    t.turn.status_message = Some("Pitwall restarted during this turn".into());
                    t.turn.done_at = Some(now.clone());
                }
                state.by_agent.insert(conv.agent.clone(), conv.id.clone());
                state.convs.insert(conv.id.clone(), conv);
            }
        }
        let collab = Arc::new_cyclic(|this| Collab {
            paths,
            clock,
            host,
            agents,
            sandbox,
            snapshots,
            egress,
            skills,
            teams,
            hooks,
            settings: RwLock::new(settings),
            run,
            shares,
            people,
            audit,
            networks: RwLock::default(),
            state: Mutex::new(state),
            this: this.clone(),
        });
        let weak = Arc::downgrade(&collab);
        collab.host.subscribe(Arc::new(move |agent: &str, event: SessionEvent| {
            if let Some(c) = weak.upgrade() {
                c.on_event(agent, event);
            }
        }));
        // Stop sessions a previous run left behind: their next message resumes them.
        let agents: Vec<String> = collab.lock().convs.values().map(|c| c.agent.clone()).collect();
        for agent in agents {
            let _ = collab.host.stop(&agent);
        }
        collab
    }

    /// Tick every second in the background: timeouts, idle stops, limits.
    pub fn start(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            match weak.upgrade() {
                Some(c) => c.tick(),
                None => return,
            }
        });
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self) -> DateTime<FixedOffset> {
        self.clock.now()
    }

    fn settings(&self) -> CollabSettings {
        self.settings.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Settings → Collaboration changed.
    pub fn set_settings(&self, settings: CollabSettings) {
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = settings;
        let actions = self.schedule(&mut self.lock());
        self.run(actions);
    }

    /// The networks this Pitwall serves (for the owner's identity on cards).
    pub fn set_networks(&self, networks: Vec<Arc<dyn Network>>) {
        *self.networks.write().unwrap_or_else(|e| e.into_inner()) = networks;
    }

    pub fn people(&self) -> &People {
        &self.people
    }

    pub fn audit_log(&self) -> &AuditLog {
        &self.audit
    }

    pub fn share_store(&self) -> &ShareStore {
        &self.shares
    }

    /// The person an identity is (for [`Handler::person_of`](crate::net::Handler::person_of)).
    pub fn person_of(&self, identity: &Identity) -> PersonId {
        self.people.person_of(identity)
    }

    fn person(&self, who: &Who) -> Person {
        match who {
            Who::Network(caller) => self.people.person(caller.identity()),
            Who::Guest => {
                let identity = Identity { name: Some("guest".into()), ..Identity::new("guest", "owner") };
                Person { id: PersonId(identity.key()), identities: vec![identity] }
            }
        }
    }

    fn owner_identity(&self, network: Option<&NetworkId>) -> Identity {
        let networks = self.networks.read().unwrap_or_else(|e| e.into_inner()).clone();
        networks
            .iter()
            .filter(|n| network.is_none_or(|id| &n.id() == id))
            .chain(networks.iter())
            .find_map(|n| n.me().map(|(i, _)| i))
            .unwrap_or_else(|| Identity::new("local", "owner"))
    }

    fn audit_line(&self, asker: Option<&Asker>, method: &str, share: Option<&str>, outcome: &str) -> AuditLine {
        let mut line = AuditLine::new(time::rfc3339(self.now()), method, outcome);
        line.agent = share.map(String::from);
        if let Some(a) = asker {
            line.front = Some(match a.front {
                Front::A2a => "a2a".into(),
                Front::Mcp => "mcp".into(),
            });
            match &a.who {
                Who::Network(c) => {
                    line.peer = c.network().0.clone();
                    line.login = c.identity().key();
                    line.node = c.device().map(|d| d.node.clone());
                }
                Who::Guest => line.login = "guest".into(),
            }
        }
        line
    }

    // ------------------------------------------------------------ audience

    /// Whether `asker` may see `share` at all (else `NotFound`): live (or
    /// paused) and in the audience; a guest sees drafts too. Blocked people
    /// and overrides with `blocked` see nothing.
    fn visible(&self, share: &ShareSettings, asker: &Asker, person: &Person) -> bool {
        if self.people.is_blocked(person) || share.limits.overrides.iter().any(|(k, o)| o.blocked && person.is(k)) {
            return false;
        }
        match &asker.who {
            Who::Guest => true,
            Who::Network(caller) => {
                matches!(share.state, ShareState::Live | ShareState::Paused)
                    && !audience_via(&share.audience, person, caller, self.teams.as_deref()).is_empty()
            }
        }
    }

    fn messages_per_day(share: &ShareSettings, person: &Person) -> u32 {
        share
            .limits
            .overrides
            .iter()
            .find(|(k, _)| person.is(k))
            .and_then(|(_, o)| o.messages_per_day)
            .unwrap_or(share.limits.per_caller.messages_per_day)
    }

    fn left(&self, st: &State, share: &ShareSettings, person: &Person) -> Left {
        let now = self.now();
        let used = st.usage.as_ref().map_or(0, |u| u.messages_of(&time::day(now), &share.slug, person));
        Left {
            messages_today: Self::messages_per_day(share, person).saturating_sub(used),
            resets_at: time::rfc3339(self.clock.next_midnight(now)),
        }
    }

    fn card(&self, st: &State, share: &ShareSettings, asker: &Asker, person: &Person) -> ShareCard {
        let network = match &asker.who {
            Who::Network(c) => Some(c.network().clone()),
            Who::Guest => None,
        };
        let kind = self.agents.agent(&share.agent).map(|a| a.kind);
        let left = self.left(st, share, person);
        let fingerprint = serde_json::json!([share.name, share.description, share.capability, share.state]).to_string();
        let version = u64::from_str_radix(&super::teams::keys::sha256_hex(fingerprint.as_bytes())[..12], 16).unwrap_or(1);
        ShareCard {
            owner: self.owner_identity(network.as_ref()),
            slug: share.slug.clone(),
            name: share.name.clone(),
            description: share.description.clone(),
            capability: share.capability,
            kind: kind.as_ref().map_or_else(String::new, |k| k.id.clone()),
            kind_name: kind.as_ref().map_or_else(String::new, |k| k.name.clone()),
            version,
            messages_per_day: Self::messages_per_day(share, person),
            left: left.messages_today,
        }
    }

    // ------------------------------------------------------------ owner: shares

    /// `share.create`: a draft (§5.5). The agent must exist.
    pub fn create_share(&self, share: ShareSettings) -> Result<ShareSettings> {
        if self.agents.agent(&share.agent).is_none() {
            return Err(PwError::not_found(format!("no agent {}", share.agent)));
        }
        let share = self.shares.create(self.normalized(share))?;
        let mut line = AuditLine::new(time::rfc3339(self.now()), "share.create", "ok");
        line.agent = Some(share.slug.clone());
        self.audit.write(&line);
        Ok(share)
    }

    pub fn share(&self, slug: &str) -> Option<ShareSettings> {
        self.shares.get(slug)
    }

    /// An account given by name (`--account Sharing`) stored by its id, so
    /// sessions find its folder.
    fn normalized(&self, mut share: ShareSettings) -> ShareSettings {
        if let (Some(key), Some(agent)) = (share.access.account.clone(), self.agents.agent(&share.agent)) {
            if key.eq_ignore_ascii_case(crate::accounts::DEFAULT) {
                // Explicitly the owner's own login (not the agent's account).
                share.access.account = Some(crate::accounts::DEFAULT.into());
            } else if let Some(acc) = self.agents.account(&agent.kind, &key) {
                share.access.account = Some(acc.id);
            }
        }
        share
    }

    /// What keeps `share` from going live (§6.3 "The agent account"):
    /// the owner's own login, the shared agent's own account, or Gemini
    /// CLI without an API key. Empty: nothing.
    pub fn live_blocks(&self, share: &ShareSettings) -> Vec<LiveBlock> {
        match self.agents.agent(&share.agent) {
            Some(agent) => super::live::live_blocks(share, &agent, &*self.agents),
            None => Vec::new(),
        }
    }

    /// How `share`'s sessions log in (the summary, the Share dialog).
    pub fn login(&self, share: &ShareSettings) -> super::live::Login {
        match self.agents.agent(&share.agent) {
            Some(agent) => super::live::login(share, &agent, &*self.agents),
            None => super::live::Login::Unknown,
        }
    }

    /// Refused with every reason, in one error.
    fn refuse_blocks(&self, share: &ShareSettings, blocks: &[LiveBlock]) -> Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }
        let text = blocks.iter().map(|b| b.message(&share.slug)).collect::<Vec<_>>().join(" ");
        Err(PwError::new(crate::error::ErrorCode::Conflict, text))
    }

    pub fn list_shares(&self) -> Vec<ShareSettings> {
        self.shares.list()
    }

    /// The diff `set_share` would apply (for the approval, §5.5).
    pub fn preview_share(&self, slug: &str, next: &ShareSettings) -> Result<ShareDiff> {
        self.shares.preview(slug, next)
    }

    /// `share.set`: apply and return the diff. Applies from the next
    /// message: sessions whose launch changed restart with the kind's
    /// resume, copies are rebuilt when the scope changed (§5.4). People no
    /// longer in the audience lose their queued and running turns at once.
    pub fn set_share(&self, slug: &str, next: ShareSettings) -> Result<ShareDiff> {
        let next = self.normalized(next);
        if let Some(current) = self.shares.get(slug).filter(|c| c.state != ShareState::Draft) {
            // A live share never moves onto a login it couldn't go live on.
            let mut probe = next.clone();
            probe.agent = current.agent.clone();
            probe.slug = current.slug.clone();
            let before = self.live_blocks(&current);
            let new: Vec<LiveBlock> = self.live_blocks(&probe).into_iter().filter(|b| !before.contains(b)).collect();
            self.refuse_blocks(&probe, &new)?;
        }
        let (d, share) = self.shares.set(slug, next)?;
        if d.0.is_empty() {
            return Ok(d);
        }
        let mut line = AuditLine::new(time::rfc3339(self.now()), "share.set", if d.widens() { "ok:widens" } else { "ok" });
        line.agent = Some(slug.to_string());
        line.detail = serde_json::to_value(&d).ok();
        self.audit.write(&line);
        let scope = d.0.iter().any(|l| l.field.starts_with("scope."));
        let launch = d.0.iter().any(|l| l.field.starts_with("access.") || l.field == "capability" || l.field == "offLimits");
        let mut actions = vec![Action::SharesChanged];
        {
            let mut st = self.lock();
            for conv in st.convs.values_mut().filter(|c| c.share == slug) {
                conv.rebuild |= scope;
                conv.relaunch |= scope || launch;
            }
            self.drop_outsiders(&mut st, &share, &mut actions);
            actions.extend(self.schedule(&mut st));
        }
        self.run(actions);
        Ok(d)
    }

    /// Callers no longer in the audience lose queued and running turns.
    fn drop_outsiders(&self, st: &mut State, share: &ShareSettings, actions: &mut Vec<Action>) {
        let ids: Vec<ConversationId> = st
            .convs
            .values()
            .filter(|c| c.share == share.slug && c.role == Role::Caller && !c.caller.guest)
            .filter(|c| {
                let person = self.people.person(&c.identity);
                let blocked = self.people.is_blocked(&person) || share.limits.overrides.iter().any(|(k, o)| o.blocked && person.is(k));
                let inside = c.proof.as_ref().is_some_and(|p| !audience_via(&share.audience, &person, p, self.teams.as_deref()).is_empty());
                blocked || !inside
            })
            .map(|c| c.id.clone())
            .collect();
        for id in ids {
            self.end_turns(st, &id, TurnStatus::Canceled, "no longer shared with you", actions);
        }
    }

    /// Cancel every unfinished turn of a conversation (`status`, with `why`).
    fn end_turns(&self, st: &mut State, id: &ConversationId, status: TurnStatus, why: &str, actions: &mut Vec<Action>) {
        let now = time::rfc3339(self.now());
        let Some(conv) = st.convs.get_mut(id) else { return };
        let mut ended = Vec::new();
        let active = conv.active();
        for (i, t) in conv.turns.iter_mut().enumerate() {
            if t.turn.status.is_final() {
                continue;
            }
            if Some(i) == active {
                actions.push(Action::Interrupt { agent: conv.agent.clone() });
                if t.turn.reply.is_none() {
                    t.turn.reply = self.host.partial_reply(&conv.agent);
                }
            }
            t.turn.status = status.clone();
            t.turn.status_message = Some(why.into());
            t.turn.done_at = Some(now.clone());
            ended.push(t.turn.clone());
        }
        if conv.proc == Proc::Working || conv.proc == Proc::Blocked {
            conv.proc = Proc::Ready;
        }
        actions.push(Action::Save(Box::new(conv.clone())));
        for t in ended {
            st.queue.retain(|q| q != &t.id);
            actions.push(Action::Changed(Box::new(t)));
        }
    }

    /// `share.go_live` (always asked of the owner first). Guest sessions are
    /// deleted unless `keep_guest`.
    pub fn go_live(&self, slug: &str, keep_guest: bool) -> Result<ShareSettings> {
        if let Some(draft) = self.shares.get(slug).filter(|s| s.state == ShareState::Draft) {
            self.refuse_blocks(&draft, &self.live_blocks(&draft))?;
        }
        let share = self.shares.go_live(slug)?;
        let mut line = AuditLine::new(time::rfc3339(self.now()), "share.go_live", "ok");
        line.agent = Some(slug.into());
        self.audit.write(&line);
        let mut actions = vec![Action::SharesChanged];
        if !keep_guest {
            let mut st = self.lock();
            let guests: Vec<ConversationId> = st.convs.values().filter(|c| c.share == slug && c.caller.guest).map(|c| c.id.clone()).collect();
            for id in guests {
                self.remove_conv(&mut st, &id, &mut actions);
            }
        }
        self.run(actions);
        Ok(share)
    }

    /// `share.pause`: queued messages are refused and running turns
    /// interrupted at once (§5.4).
    pub fn pause(&self, slug: &str) -> Result<ShareSettings> {
        let share = self.shares.pause(slug)?;
        self.audit_owner("share.pause", slug);
        let mut actions = vec![Action::SharesChanged];
        {
            let mut st = self.lock();
            let ids: Vec<ConversationId> = st.convs.values().filter(|c| c.share == slug && !c.caller.guest).map(|c| c.id.clone()).collect();
            for id in ids {
                self.end_turns(&mut st, &id, TurnStatus::Canceled, "the owner paused this agent", &mut actions);
            }
            actions.extend(self.schedule(&mut st));
        }
        self.run(actions);
        Ok(share)
    }

    pub fn resume(&self, slug: &str) -> Result<ShareSettings> {
        if let Some(paused) = self.shares.get(slug).filter(|s| s.state == ShareState::Paused) {
            self.refuse_blocks(&paused, &self.live_blocks(&paused))?;
        }
        let share = self.shares.resume(slug)?;
        self.audit_owner("share.resume", slug);
        self.run(vec![Action::SharesChanged]);
        Ok(share)
    }

    /// `share.stop`: stop sharing. Every conversation of the share ends and
    /// its session and copy are removed; the shared agent stays as it was.
    pub fn stop_share(&self, slug: &str) -> Result<()> {
        self.shares.stop(slug)?;
        self.audit_owner("share.stop", slug);
        let mut actions = vec![Action::SharesChanged];
        {
            let mut st = self.lock();
            let ids: Vec<ConversationId> = st.convs.values().filter(|c| c.share == slug).map(|c| c.id.clone()).collect();
            for id in ids {
                self.end_turns(&mut st, &id, TurnStatus::Canceled, "the owner stopped sharing this agent", &mut actions);
                self.remove_conv(&mut st, &id, &mut actions);
            }
            st.limited.remove(slug);
            actions.extend(self.schedule(&mut st));
        }
        actions.push(Action::Delete(self.paths.share_dir(slug)));
        self.run(actions);
        self.egress.forget(slug);
        Ok(())
    }

    fn audit_owner(&self, method: &str, slug: &str) {
        let mut line = AuditLine::new(time::rfc3339(self.now()), method, "ok");
        line.agent = Some(slug.into());
        self.audit.write(&line);
    }

    /// The owner's own text for the share's sessions (§6.5): written as is
    /// into the instruction files the kind reads, with the off-limits text.
    /// Soft: "not guaranteed". New and restarted sessions get it.
    pub fn set_instructions(&self, slug: &str, text: &str) -> Result<()> {
        if self.shares.get(slug).is_none() {
            return Err(PwError::not_found(format!("no share {slug}")));
        }
        let path = self.paths.base_dir(slug).join("instructions.md");
        super::store::write_atomic(&path, text.as_bytes())?;
        self.audit_owner("share.instructions", slug);
        let mut st = self.lock();
        for conv in st.convs.values_mut().filter(|c| c.share == slug) {
            conv.relaunch = true;
        }
        Ok(())
    }

    pub fn instructions(&self, slug: &str) -> String {
        std::fs::read_to_string(self.paths.base_dir(slug).join("instructions.md")).unwrap_or_default()
    }

    /// The owner's block list (§4.6): at once, on every share.
    pub fn block(&self, identity: &Identity) -> Result<()> {
        self.people.block(identity)?;
        let mut line = AuditLine::new(time::rfc3339(self.now()), "share.block", "ok");
        line.detail = Some(serde_json::json!(identity.key()));
        self.audit.write(&line);
        let person = self.people.person(identity);
        let mut actions = vec![Action::SharesChanged];
        {
            let mut st = self.lock();
            let ids: Vec<ConversationId> = st.convs.values().filter(|c| c.belongs_to(&person)).map(|c| c.id.clone()).collect();
            for id in ids {
                self.end_turns(&mut st, &id, TurnStatus::Canceled, "blocked", &mut actions);
            }
            actions.extend(self.schedule(&mut st));
        }
        self.run(actions);
        Ok(())
    }

    pub fn unblock(&self, identity: &Identity) -> Result<()> {
        self.people.unblock(identity)?;
        self.run(vec![Action::SharesChanged]);
        Ok(())
    }

    /// The network provider removed someone (§15.4): drop what we hold for them.
    pub fn on_revocation(&self, revocation: &Revocation) {
        let mut actions = Vec::new();
        {
            let mut st = self.lock();
            let ids: Vec<ConversationId> = st
                .convs
                .values()
                .filter(|c| match revocation {
                    Revocation::Identity { identity } => c.identity.is(identity),
                    Revocation::Device { device } => c.proof.as_ref().is_some_and(|p| p.device() == Some(device)),
                })
                .map(|c| c.id.clone())
                .collect();
            for id in ids {
                self.end_turns(&mut st, &id, TurnStatus::Canceled, "removed from the network", &mut actions);
            }
            actions.extend(self.schedule(&mut st));
        }
        self.run(actions);
    }

    /// Something for the owner (needs-you, no answer, limits).
    pub fn notices(&self) -> Receiver<Notice> {
        let (tx, rx) = mpsc::channel();
        self.lock().notice_subs.push(tx);
        rx
    }

    /// The owner's Usage view: every caller's use of every share on `day`
    /// (`2026-10-16`; `None`: today).
    pub fn usage(&self, day: Option<&str>) -> Vec<UsageRow> {
        let day = day.map_or_else(|| time::day(self.now()), String::from);
        self.lock().usage.as_ref().map(|u| u.day(&day)).unwrap_or_default()
    }

    /// Usage rows of the last `days` days, today included (the Usage
    /// view's 30 days).
    pub fn usage_days(&self, days: u32) -> Vec<UsageRow> {
        let now = self.now();
        let st = self.lock();
        let Some(u) = st.usage.as_ref() else { return Vec::new() };
        (0..days.max(1)).flat_map(|n| u.day(&time::day(now - chrono::Duration::days(i64::from(n))))).collect()
    }

    /// The share's agent shows its vendor limit: `Some(until)` (the reset
    /// time it showed, when it did).
    pub fn limited(&self, slug: &str) -> Option<Option<String>> {
        self.lock().limited.get(slug).map(|l| l.until.clone())
    }

    /// Whether engine agent `agent` is one of the shared sessions (its
    /// processes may not use Pitwall's socket at all, §8.5).
    pub fn is_session(&self, agent: &str) -> bool {
        self.lock().by_agent.contains_key(agent)
    }

    /// `share.sessions`: each caller's conversation with a share (owner sidebar).
    pub fn sessions(&self, slug: &str) -> Vec<SharedSessionView> {
        let st = self.lock();
        let day = time::day(self.now());
        st.convs
            .values()
            .filter(|c| c.share == slug && c.role == Role::Caller)
            .map(|c| {
                let person = self.people.person(&c.identity);
                let (messages, ms) = st.usage.as_ref().map_or((0, 0), |u| (u.messages_of(&day, slug, &person), u.agent_ms_of(&day, slug, &person)));
                let state = match c.proc {
                    Proc::Stopped if c.turns.is_empty() => SessionState::New,
                    Proc::Stopped => SessionState::Stopped,
                    Proc::Starting => SessionState::Starting,
                    Proc::Ready => SessionState::Ready,
                    Proc::Working => SessionState::Working,
                    Proc::Blocked => SessionState::Blocked,
                };
                SharedSessionView {
                    conversation: c.id.0.clone(),
                    share: c.share.clone(),
                    caller: c.caller.clone(),
                    state,
                    agent_id: c.agent.clone(),
                    turns: c.public_turns().count() as u32,
                    results: 0,
                    held: 0,
                    usage: SessionUsage { minutes_today: (ms / 60_000) as u32, messages_today: messages, tokens: None },
                    last_at: c.last_at.clone(),
                    flagged: c.flagged,
                }
            })
            .collect()
    }

    /// One conversation of share `slug`, turn by turn, as its owner sees
    /// it (the session pane's Messages, §11): what the caller wrote, what
    /// was typed, the reply and its source. Pitwall's own briefing turns
    /// are included, marked.
    pub fn owner_turns(&self, slug: &str, conversation: &ConversationId) -> Result<Vec<OwnerTurn>> {
        let st = self.lock();
        let conv = st.convs.get(conversation).filter(|c| c.share == slug).ok_or_else(|| PwError::not_found("no such conversation"))?;
        Ok(conv
            .turns
            .iter()
            .map(|t| OwnerTurn { turn: t.turn.clone(), text: t.text.clone(), delivered: t.delivered.clone(), internal: t.internal })
            .collect())
    }

    /// Who a conversation is with (`bob@example.com`), and its engine agent.
    pub fn caller_of(&self, conversation: &ConversationId) -> Option<(String, String)> {
        self.lock().convs.get(conversation).map(|c| (c.caller.login.clone(), c.agent.clone()))
    }

    /// Whether a conversation's session is flagged (a turn with no answer).
    pub fn flagged(&self, conversation: &ConversationId) -> bool {
        self.lock().convs.get(conversation).is_some_and(|c| c.flagged)
    }

    /// The text typed into the session for a turn (the owner view shows
    /// exactly what was typed, §8.3.1).
    pub fn delivered(&self, turn: &TurnId) -> Option<String> {
        self.lock().convs.get(&turn.conversation()).and_then(|c| c.turns.iter().find(|t| &t.turn.id == turn)).and_then(|t| t.delivered.clone())
    }

    /// What a conversation's session tried and its sandbox refused since
    /// `since` (the guest probes' "tried" lines, §5.4): reads, writes,
    /// programs, connections, as the OS reported them.
    pub fn sandbox_denials(&self, conversation: &ConversationId, since: std::time::SystemTime) -> Vec<String> {
        let denials = self.sandbox.denials(&session_tag(&conversation.0), since);
        crate::sandbox::tried_lines(&denials, &crate::paths::home())
    }

    /// The engine agent a conversation's session is.
    pub fn agent_of(&self, conversation: &ConversationId) -> Option<String> {
        self.lock().convs.get(conversation).map(|c| c.agent.clone())
    }

    // ------------------------------------------------------------ the queue

    fn running_count(st: &State, share: Option<&str>) -> u32 {
        st.convs.values().filter(|c| c.proc.running() && share.is_none_or(|s| c.share == s)).count() as u32
    }

    /// The least recently used running session with nothing to do (in
    /// `share` when given), to stop for a new one (§6.6). Kinds without
    /// resume stop too (their memory is lost; the caller is told).
    fn idle_victim(st: &State, share: Option<&str>) -> Option<ConversationId> {
        st.convs
            .values()
            .filter(|c| c.proc == Proc::Ready && c.active().is_none() && share.is_none_or(|s| c.share == s))
            .filter(|c| !st.queue.iter().any(|q| q.conversation() == c.id))
            .min_by_key(|c| c.last_used)
            .map(|c| c.id.clone())
    }

    /// Start what can start, in queue order. Turns that need a new session
    /// keep their share's order: once one waits for a slot, later ones of
    /// that share wait behind it.
    fn schedule(&self, st: &mut State) -> Vec<Action> {
        let mut actions = Vec::new();
        let max = self.settings().max_sessions.max(1);
        let mut waiting_shares: HashSet<String> = HashSet::new();
        let mut i = 0;
        while i < st.queue.len() {
            let id = st.queue[i].clone();
            let conv_id = id.conversation();
            let Some(conv) = st.convs.get(&conv_id) else {
                st.queue.remove(i);
                continue;
            };
            let Some(share) = self.shares.get(&conv.share) else {
                i += 1;
                continue;
            };
            if conv.busy() {
                i += 1;
                continue;
            }
            let needs_restart = conv.proc.running() && (conv.relaunch || conv.rebuild);
            if conv.proc.running() && !needs_restart {
                st.queue.remove(i);
                self.deliver(st, &conv_id, &id, &mut actions);
                continue;
            }
            if waiting_shares.contains(&share.slug) {
                i += 1;
                continue;
            }
            if !needs_restart {
                // A slot in the share, then on the machine.
                if Self::running_count(st, Some(&share.slug)) >= share.limits.sessions_running {
                    match Self::idle_victim(st, Some(&share.slug)) {
                        Some(v) => self.stop_session(st, &v, &mut actions),
                        None => {
                            waiting_shares.insert(share.slug.clone());
                            i += 1;
                            continue;
                        }
                    }
                }
                if Self::running_count(st, None) >= max {
                    match Self::idle_victim(st, None) {
                        Some(v) => self.stop_session(st, &v, &mut actions),
                        None => {
                            waiting_shares.insert(share.slug.clone());
                            i += 1;
                            continue;
                        }
                    }
                }
            }
            // A fork needs the share's base conversation first (§6.4).
            let start = match self.start_for(st, &conv_id, &share) {
                Ok(s) => s,
                // The base was just queued in front: start over from it.
                Err(true) => {
                    i = 0;
                    continue;
                }
                Err(false) => {
                    waiting_shares.insert(share.slug.clone());
                    i += 1;
                    continue;
                }
            };
            st.queue.remove(i);
            self.launch(st, &conv_id, &id, share, start, &mut actions);
        }
        self.positions(st, &mut actions);
        actions
    }

    /// How the session of `conv_id` starts: resume what it has, else fork
    /// the base (making it first: `Err(true)` when just queued, `Err(false)`
    /// while it is being made: wait), else fresh.
    fn start_for(&self, st: &mut State, conv_id: &ConversationId, share: &ShareSettings) -> std::result::Result<StartAs, bool> {
        let conv = &st.convs[conv_id];
        let Some(kind) = self.agents.agent(&share.agent).map(|a| a.kind) else { return Ok(StartAs::Fresh) };
        if let (Some(id), true) = (&conv.session_id, conv.has_conversation) {
            if can_resume(&kind) {
                return Ok(StartAs::Resume(id.clone()));
            }
        }
        let Start::Summary { approved_at, text } = &share.start else { return Ok(StartAs::Fresh) };
        if conv.role == Role::Base || !super::session::can_fork(&kind) {
            return Ok(StartAs::Fresh);
        }
        let base = st.convs.values().find(|c| c.role == Role::Base && c.share == share.slug && c.briefing.as_deref() == Some(approved_at.as_str()));
        match base {
            Some(b) if b.turns.iter().all(|t| t.turn.status.is_final()) => match (&b.session_id, b.turns.iter().any(|t| t.turn.status == TurnStatus::Done)) {
                (Some(sid), true) => Ok(StartAs::Fork(sid.clone())),
                // The base couldn't be made: brief this session itself.
                _ => Ok(StartAs::Fresh),
            },
            Some(_) => Err(false),
            None => {
                self.make_base(st, share, approved_at, text);
                Err(true)
            }
        }
    }

    /// Queue the share's base conversation: a fresh session whose first
    /// message is the approved summary, at the front of the queue.
    fn make_base(&self, st: &mut State, share: &ShareSettings, approved_at: &str, text: &str) {
        let now = time::rfc3339(self.now());
        let identity = Identity::new("local", "base");
        let mut conv = self.new_conv(&share.slug, Role::Base, &identity, PersonId(identity.key()), None, SessionCaller {
            login: "briefing".into(),
            agent: None,
            client: None,
            front: Front::A2a,
            guest: true,
            pitwall: true,
        });
        conv.briefing = Some(approved_at.to_string());
        conv.current = false;
        let id = TurnId::new(&conv.id, 1);
        conv.turns.push(TurnRec {
            turn: Turn {
                id: id.clone(),
                conversation: conv.id.clone(),
                share: share.slug.clone(),
                status: TurnStatus::Queued { position: 1 },
                status_message: None,
                reply: None,
                changes: None,
                left: None,
                at: now,
                done_at: None,
            },
            text: text.to_string(),
            delivered: None,
            started_at: None,
            internal: true,
        });
        st.by_agent.insert(conv.agent.clone(), conv.id.clone());
        st.convs.insert(conv.id.clone(), conv);
        st.queue.insert(0, id);
    }

    fn stop_session(&self, st: &mut State, id: &ConversationId, actions: &mut Vec<Action>) {
        let resumes = st
            .convs
            .get(id)
            .and_then(|c| self.shares.get(&c.share))
            .and_then(|s| self.agents.agent(&s.agent))
            .is_some_and(|a| can_resume(&a.kind));
        if let Some(conv) = st.convs.get_mut(id) {
            conv.proc = Proc::Stopped;
            conv.relaunch = false;
            if conv.session_id.is_none() || !resumes {
                // Its memory is gone with the process (§6.6).
                conv.lost = true;
            }
            actions.push(Action::Stop { agent: conv.agent.clone() });
        }
    }

    /// Start the session of `conv_id` for turn `id`.
    fn launch(&self, st: &mut State, conv_id: &ConversationId, id: &TurnId, share: ShareSettings, start: StartAs, actions: &mut Vec<Action>) {
        st.seq += 1;
        let seq = st.seq;
        let now = time::rfc3339(self.now());
        let conv = st.convs.get_mut(conv_id).expect("queued turns have conversations");
        if conv.proc.running() {
            // Settings changed: restart with the kind's resume.
            conv.stopping += 1;
            actions.push(Action::Stop { agent: conv.agent.clone() });
        }
        let lost = std::mem::take(&mut conv.lost) && conv.has_conversation && !matches!(start, StartAs::Resume(_));
        conv.proc = Proc::Starting;
        conv.last_used = seq;
        let make_copy = conv.copy.is_none() || conv.rebuild;
        if conv.rebuild && conv.copy.is_some() {
            conv.lines.push(format!("Files updated to {}", share.scope.git_ref));
        }
        conv.rebuild = false;
        conv.relaunch = false;
        let resumed = matches!(start, StartAs::Resume(_));
        // Summary start without a fork: the approved text is the first
        // message, sent like any message (§6.4).
        let brief = match (&share.start, &start, conv.role) {
            (Start::Summary { text, approved_at }, StartAs::Fresh, Role::Caller) if !conv.has_conversation => Some((text.clone(), approved_at.clone())),
            _ => None,
        };
        if let StartAs::Fork(_) = &start {
            if let Start::Summary { approved_at, .. } = &share.start {
                conv.briefing = Some(approved_at.clone());
            }
        }
        if !resumed && !matches!(start, StartAs::Fork(_)) && conv.has_conversation {
            conv.has_conversation = false;
        }
        if let Some((text, approved_at)) = brief {
            conv.briefing = Some(approved_at);
            let n = conv.turns.len() as u32 + 1;
            let internal = TurnId(format!("{}/b{n}", conv.id.0));
            conv.turns.push(TurnRec {
                turn: Turn {
                    id: internal,
                    conversation: conv.id.clone(),
                    share: conv.share.clone(),
                    status: TurnStatus::Working,
                    status_message: None,
                    reply: None,
                    changes: None,
                    left: None,
                    at: now.clone(),
                    done_at: None,
                },
                text,
                delivered: None,
                started_at: Some(now.clone()),
                internal: true,
            });
            // The caller's turn waits behind the briefing, first in line.
            st.queue.insert(0, id.clone());
        } else if let Some(t) = conv.turn_mut(id) {
            t.turn.status = TurnStatus::Working;
            t.turn.status_message = Some(if lost { "new conversation (the agent can't resume)" } else { "starting" }.into());
            t.started_at = Some(now.clone());
        }
        let conv = st.convs.get(conv_id).expect("still there").clone();
        if let Some(t) = conv.turns.iter().find(|t| &t.turn.id == id) {
            actions.push(Action::Changed(Box::new(t.turn.clone())));
        }
        actions.push(Action::Launch(Box::new(LaunchJob { conv, share, start, make_copy })));
    }

    /// Type turn `id` into its running session.
    fn deliver(&self, st: &mut State, conv_id: &ConversationId, id: &TurnId, actions: &mut Vec<Action>) {
        st.seq += 1;
        let seq = st.seq;
        let now = time::rfc3339(self.now());
        let Some(conv) = st.convs.get_mut(conv_id) else { return };
        let lines = std::mem::take(&mut conv.lines);
        let from = from_line(conv);
        conv.last_used = seq;
        conv.proc = Proc::Working;
        conv.has_conversation = true;
        let agent = conv.agent.clone();
        let Some(t) = conv.turn_mut(id) else { return };
        let mut text = String::new();
        if !t.internal {
            text.push_str(&from);
            text.push('\n');
            for l in &lines {
                text.push_str(l);
                text.push('\n');
            }
        }
        text.push_str(&t.text);
        t.delivered = Some(text.clone());
        t.turn.status = TurnStatus::Working;
        t.turn.status_message = None;
        if t.started_at.is_none() {
            t.started_at = Some(now);
        }
        let turn = t.turn.clone();
        actions.push(Action::Changed(Box::new(turn)));
        actions.push(Action::Save(Box::new(conv.clone())));
        actions.push(Action::Send { agent, text });
    }

    /// Queue positions per share ("2nd in line").
    fn positions(&self, st: &mut State, actions: &mut Vec<Action>) {
        let mut per_share: HashMap<String, u32> = HashMap::new();
        let queue = st.queue.clone();
        for id in queue {
            let conv_id = id.conversation();
            let Some(conv) = st.convs.get_mut(&conv_id) else { continue };
            // Starting, or being briefed with the approved summary first.
            let conv_starting = conv.proc == Proc::Starting || conv.active().is_some_and(|a| conv.turns[a].internal);
            let n = per_share.entry(conv.share.clone()).or_insert(0);
            *n += 1;
            let position = *n;
            let Some(t) = conv.turn_mut(&id) else { continue };
            if t.turn.status.is_final() {
                continue;
            }
            let status = TurnStatus::Queued { position };
            let message = Some(if conv_starting { "starting".to_string() } else { ordinal(position) });
            if t.turn.status != status || t.turn.status_message != message {
                t.turn.status = status;
                t.turn.status_message = message;
                actions.push(Action::Changed(Box::new(t.turn.clone())));
            }
        }
    }

    // ------------------------------------------------------------ running actions

    fn run(&self, actions: Vec<Action>) {
        if actions.is_empty() {
            return;
        }
        // Notifications go out at once; blocking work where it belongs.
        let (quick, slow): (Vec<Action>, Vec<Action>) = actions.into_iter().partition(|a| {
            matches!(a, Action::Changed(_) | Action::SharesChanged | Action::Notice(_) | Action::Audit(_) | Action::Save(_))
        });
        for a in quick {
            self.run_one(a);
        }
        if slow.is_empty() {
            return;
        }
        match (self.run, self.this.upgrade()) {
            (RunMode::Background, Some(me)) => {
                std::thread::spawn(move || {
                    for a in slow {
                        me.run_one(a);
                    }
                });
            }
            _ => {
                for a in slow {
                    self.run_one(a);
                }
            }
        }
    }

    fn run_one(&self, action: Action) {
        match action {
            Action::Changed(turn) => {
                let mut st = self.lock();
                let fin = turn.status.is_final();
                if let Some(list) = st.watchers.get_mut(&turn.id) {
                    list.retain(|tx| tx.send((*turn).clone()).is_ok());
                }
                if fin {
                    st.watchers.remove(&turn.id);
                }
            }
            Action::SharesChanged => self.lock().share_subs.retain(|tx| tx.send(()).is_ok()),
            Action::Notice(n) => self.lock().notice_subs.retain(|tx| tx.send(n.clone()).is_ok()),
            Action::Audit(line) => self.audit.write(&line),
            Action::Save(conv) => {
                let path = self.paths.session_dir(&conv.share, &conv.id.0).join("conversation.json");
                if let Err(e) = write_json(&path, &*conv) {
                    eprintln!("pitwall: saving conversation {}: {e}", conv.id.0);
                }
            }
            Action::Delete(path) => {
                let _ = std::fs::remove_dir_all(path);
            }
            Action::Send { agent, text } => {
                if let Err(e) = self.host.send(&agent, &text) {
                    self.fail_active(&agent, Failure::Crashed, &format!("couldn't reach the session: {e}"));
                }
            }
            Action::Interrupt { agent } => {
                let _ = self.host.interrupt(&agent);
            }
            Action::Stop { agent } => {
                let _ = self.host.stop(&agent);
            }
            Action::Remove { agent, folder } => {
                let _ = self.host.remove(&agent);
                let _ = std::fs::remove_dir_all(folder);
            }
            Action::Launch(job) => {
                let agent = job.conv.agent.clone();
                match self.prepare_and_launch(&job) {
                    Ok((session_id, copy)) => {
                        let mut st = self.lock();
                        if let Some(conv) = st.convs.get_mut(&job.conv.id) {
                            if session_id.is_some() {
                                conv.session_id = session_id;
                            }
                            if copy.is_some() {
                                conv.copy = copy;
                            }
                            let snapshot = conv.clone();
                            drop(st);
                            self.run_one(Action::Save(Box::new(snapshot)));
                        }
                    }
                    Err(e) => {
                        {
                            let mut st = self.lock();
                            if let Some(conv) = st.convs.get_mut(&job.conv.id) {
                                conv.proc = Proc::Stopped;
                            }
                            // A briefing turn first in line: the caller's turn is queued.
                            let queued: Vec<TurnId> = st.queue.iter().filter(|q| q.conversation() == job.conv.id).cloned().collect();
                            st.queue.retain(|q| q.conversation() != job.conv.id);
                            if let Some(conv) = st.convs.get_mut(&job.conv.id) {
                                for q in queued {
                                    if let Some(t) = conv.turn_mut(&q) {
                                        t.turn.status = TurnStatus::Working;
                                    }
                                }
                            }
                        }
                        eprintln!("pitwall: starting a shared session of {}: {e}", job.share.slug);
                        self.fail_all(&agent, Failure::Crashed, &format!("the session couldn't start: {}", e.message));
                    }
                }
            }
        }
    }

    /// Make the copy, write instructions and skills, build the sandbox
    /// policy, wrap the launch and start it. Returns the kind's
    /// conversation id when known, and the copy's base commit when made.
    fn prepare_and_launch(&self, job: &LaunchJob) -> Result<(Option<String>, Option<String>)> {
        let share = &job.share;
        let conv = &job.conv;
        let owner_agent = self.agents.agent(&share.agent).ok_or_else(|| PwError::not_found("the shared agent is gone"))?;
        // Guests (the owner trying a draft) may use any login; nobody else
        // reaches a session on the owner's own (§6.3).
        if !conv.caller.guest {
            self.refuse_blocks(share, &super::live::live_blocks(share, &owner_agent, &*self.agents))?;
        }
        let kind = owner_agent.kind.clone();
        let folder = self.paths.session_dir(&share.slug, &conv.id.0);
        let work = folder.join("work");
        let home = folder.join("home");
        let tmp = folder.join("tmp");
        for d in [&home, &tmp] {
            std::fs::create_dir_all(d).map_err(|e| PwError::other(format!("{}: {e}", d.display())))?;
        }
        let mut copy = None;
        if job.make_copy {
            if work.exists() {
                std::fs::remove_dir_all(&work).map_err(|e| PwError::other(format!("{}: {e}", work.display())))?;
            }
            std::fs::create_dir_all(&work).map_err(|e| PwError::other(format!("{}: {e}", work.display())))?;
            copy = Some(self.snapshots.make(&share.scope, &work)?.base_commit);
        }
        self.write_instructions(share, &kind, &folder)?;
        if let Some(skills) = &self.skills {
            if let Err(e) = skills.write(&share.agent, &kind, &folder) {
                eprintln!("pitwall: skills for a shared session of {}: {e}", share.slug);
            }
        }
        let account = share.access.account.clone().or(owner_agent.account.clone());
        let policy = self.policy(share, &kind, account.as_deref(), &folder)?;
        let hook = self.hooks.as_ref().map(|h| super::session::hook_command(&h.command, &conv.agent, &h.socket));
        let (mut argv, mut session_id, mut resumed) = kind_argv(&kind, &job.start, hook.as_deref());
        argv.extend(super::session::session_args(&kind, &work));
        let mut wrapped = self.sandbox.wrap(&policy, &argv)?;
        // Tier (b): nothing runs between turns; each turn is one sandboxed run.
        let mut turns = None;
        if let Some((first, next, id, res)) = super::session::turn_argv(&kind, &job.start, &work) {
            let first = self.sandbox.wrap(&policy, &first)?;
            let next = self.sandbox.wrap(&policy, &next)?;
            wrapped = first.clone();
            turns = Some(super::session::TurnArgs { first, next });
            session_id = Some(id);
            resumed = res;
        }
        let launch = HostLaunch {
            agent: conv.agent.clone(),
            name: format!("{} · {}", share.slug, conv.caller.agent.clone().unwrap_or_else(|| conv.caller.login.clone())),
            kind: kind.id.clone(),
            kind_name: kind.name.clone(),
            cwd: work,
            project: owner_agent.project.clone(),
            wrapped,
            shared: SharedSession {
                share: share.slug.clone(),
                agent: share.agent.clone(),
                conversation: conv.id.0.clone(),
                caller: conv.caller.agent.clone().unwrap_or_else(|| conv.caller.login.clone()),
            },
            session_id: session_id.clone(),
            resumed,
            turns,
        };
        self.host.launch(&launch)?;
        Ok((session_id, copy))
    }

    /// The owner's text, exactly as saved, into the instruction files the
    /// kind reads (§6.5). Nothing of Pitwall's own.
    fn write_instructions(&self, share: &ShareSettings, kind: &AgentKind, folder: &Path) -> Result<()> {
        let mut text = self.instructions(&share.slug).trim().to_string();
        if !share.off_limits.trim().is_empty() {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(share.off_limits.trim());
        }
        for file in share_kind(kind).instruction_files_or_default() {
            let path = folder.join(&file);
            if text.is_empty() {
                let _ = std::fs::remove_file(&path);
            } else {
                super::store::write_atomic(&path, format!("{text}\n").as_bytes())?;
            }
        }
        Ok(())
    }

    /// The OS sandbox's policy for one session (§6.3): the copy (read, and
    /// write from "Can change code"), the owner's instruction files, the
    /// agent's install from its kind data (`sandbox::kind::resolve`), the
    /// session's own `HOME` and temp, the agent account's folder, the
    /// share's egress proxy, the hook socket, and an environment built from
    /// nothing (`sandbox::env::build`). The backend adds the system's
    /// read-only parts itself.
    fn policy(&self, share: &ShareSettings, kind: &AgentKind, account: Option<&str>, folder: &Path) -> Result<SandboxPolicy> {
        use crate::sandbox::{env, kind as kinds};
        let sk = share_kind(kind);
        let work = folder.join("work");
        let home = folder.join("home");
        let tmp = folder.join("tmp");
        let owner_home = crate::paths::home();
        let owner_path = crate::shell::spawn_path().or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
        let command = kind.command.split_whitespace().next().unwrap_or_default();
        let program = kinds::which(&crate::paths::expand_tilde(command).to_string_lossy(), &owner_path);
        let install = kinds::resolve(kind, &owner_home, program.as_deref(), &owner_path);

        let mut read = vec![work.clone()];
        read.extend(sk.instruction_files_or_default().iter().map(|f| folder.join(f)));
        read.extend(install.read.iter().cloned());
        let mut write = vec![home.clone(), tmp.clone()];
        if share.capability != Capability::Read {
            write.push(work.clone());
        }
        // The agent account's folder: readable and writable (§6.3).
        let account_env = self.agents.account_env(kind, account).into_iter().next();
        if let Some((_, dir)) = &account_env {
            write.push(PathBuf::from(dir));
        }

        // A login folder no variable can move (Gemini CLI's `~/.gemini`) is
        // never mounted, whatever else the kind data says (§6.3).
        if let Some(login) = sk.login_dir.as_deref().map(crate::paths::expand_tilde) {
            read.retain(|p| !p.starts_with(&login));
            write.retain(|p| !p.starts_with(&login));
        }

        let mut path: Vec<PathBuf> = install.path.clone();
        path.extend(SYSTEM_PATH.iter().map(PathBuf::from).filter(|p| p.is_dir()));
        let mut hosts = install.hosts.clone();
        hosts.extend(share.access.network.allow.iter().cloned());
        let proxy_port = self.egress.port(share, &hosts)?;
        let mut pass: BTreeMap<String, String> = kind.update_off_env.iter().chain(sk.own_sandbox_off_env.iter()).map(|(k, v)| (k.clone(), v.clone())).collect();
        for name in &share.access.env {
            match env::check_passed(name) {
                Ok(()) => {
                    if let Some(v) = self.agents.env_value(name) {
                        pass.insert(name.clone(), v);
                    }
                }
                Err(why) => eprintln!("pitwall: {} isn't passed to {}'s sessions: {why}", name, share.slug),
            }
        }
        let env = env::build(&env::EnvSpec {
            home: home.clone(),
            tmp: tmp.clone(),
            path: path.clone(),
            proxy_port,
            account: account_env.map(|(k, v)| (k, PathBuf::from(v))),
            shell: SESSION_SHELLS.iter().map(PathBuf::from).find(|p| p.is_file()),
            lang: None,
            pass,
        })?;

        let search: Vec<PathBuf> = path;
        let resolve = |names: &[String]| -> Vec<PathBuf> { names.iter().flat_map(|n| resolve_program(n, &search)).collect() };
        let hook_programs: Vec<PathBuf> = self.hooks.iter().flat_map(|h| h.programs.iter().cloned()).collect();
        read.extend(hook_programs.iter().cloned());
        let exec = match super::shares::effective_programs(share) {
            Programs::Allow(mut names) => {
                // The agent itself and its runtimes, and the shell its hooks run in.
                names.extend(sk.programs.iter().cloned());
                if program.is_none() && !command.is_empty() {
                    names.push(command.to_string());
                }
                if self.hooks.is_some() {
                    names.push("sh".to_string());
                }
                let mut list = resolve(&names);
                list.extend(install.exec.iter().cloned());
                list.extend(hook_programs);
                list.sort();
                list.dedup();
                ExecRule::Allow(list)
            }
            Programs::Deny(names) => ExecRule::Deny(resolve(&names)),
        };
        let connect = self.hooks.iter().map(|h| LocalEndpoint::UnixSocket(h.socket.clone())).collect();
        let state_dir = self.paths.sandbox_state(&share.slug, folder.file_name().and_then(|n| n.to_str()).unwrap_or("session"));
        std::fs::create_dir_all(&state_dir).map_err(|e| PwError::other(format!("{}: {e}", state_dir.display())))?;
        let tag = Some(session_tag(folder.file_name().and_then(|n| n.to_str()).unwrap_or("session")));
        Ok(SandboxPolicy { read, write, exec, egress: Egress { proxy_port }, env, connect, state_dir, credential_store: sk.keychain, tag })
    }

    // ------------------------------------------------------------ events

    fn on_event(&self, agent: &str, event: SessionEvent) {
        let mut actions = Vec::new();
        {
            let mut st = self.lock();
            let Some(conv_id) = st.by_agent.get(agent).cloned() else { return };
            if !st.convs.contains_key(&conv_id) {
                return;
            }
            self.apply_event(&mut st, &conv_id, event, &mut actions);
            actions.extend(self.schedule(&mut st));
        }
        self.run(actions);
    }

    fn apply_event(&self, st: &mut State, conv_id: &ConversationId, event: SessionEvent, actions: &mut Vec<Action>) {
        let owner = self.settings().owner_name;
        let now_t = self.now();
        let now = time::rfc3339(now_t);
        let day = time::day(now_t);
        let share_slug = st.convs[conv_id].share.clone();
        match event {
            SessionEvent::SessionId(id) => {
                let conv = st.convs.get_mut(conv_id).expect("checked");
                conv.session_id = Some(id);
                actions.push(Action::Save(Box::new(conv.clone())));
            }
            SessionEvent::Ready => {
                let conv = st.convs.get_mut(conv_id).expect("checked");
                if conv.proc != Proc::Starting {
                    return;
                }
                conv.proc = Proc::Ready;
                // The turn that started it (or its briefing) goes in now.
                if let Some(i) = conv.active() {
                    let id = conv.turns[i].turn.id.clone();
                    if conv.turns[i].delivered.is_none() {
                        self.deliver(st, conv_id, &id, actions);
                    }
                }
            }
            SessionEvent::Working => {
                let conv = st.convs.get_mut(conv_id).expect("checked");
                if conv.proc.running() {
                    conv.proc = Proc::Working;
                }
                if let Some(i) = conv.active() {
                    let t = &mut conv.turns[i];
                    if t.turn.status == TurnStatus::WaitingForOwner {
                        t.turn.status = TurnStatus::Working;
                        t.turn.status_message = None;
                        actions.push(Action::Changed(Box::new(t.turn.clone())));
                    }
                }
            }
            SessionEvent::Blocked { detail } => {
                let conv = st.convs.get_mut(conv_id).expect("checked");
                if conv.proc.running() {
                    conv.proc = Proc::Blocked;
                }
                if let Some(i) = conv.active() {
                    let t = &mut conv.turns[i];
                    if t.turn.status != TurnStatus::WaitingForOwner {
                        t.turn.status = TurnStatus::WaitingForOwner;
                        t.turn.status_message = Some(format!("waiting for {owner}"));
                        actions.push(Action::Changed(Box::new(t.turn.clone())));
                        actions.push(Action::Notice(Notice::NeedsYou {
                            share: conv.share.clone(),
                            conversation: conv.id.clone(),
                            caller: conv.caller.login.clone(),
                            detail,
                        }));
                    }
                }
            }
            SessionEvent::TurnEnded(reply) => {
                let conv = st.convs.get_mut(conv_id).expect("checked");
                if conv.proc.running() {
                    conv.proc = Proc::Ready;
                }
                let Some(i) = conv.active() else { return };
                let person = self.people.person(&conv.identity);
                let t = &mut conv.turns[i];
                let ms = t.started_at.as_deref().and_then(time::parse).map_or(0, |s| (now_t - s).num_milliseconds().max(0) as u64);
                t.turn.done_at = Some(now.clone());
                t.turn.status_message = None;
                let internal = t.internal;
                match reply {
                    Some(r) => {
                        t.turn.reply = Some(r);
                        t.turn.status = TurnStatus::Done;
                        conv.flagged = false;
                    }
                    None if internal => t.turn.status = TurnStatus::Done,
                    None => {
                        t.turn.status = TurnStatus::Failed { reason: Failure::NoAnswer };
                        t.turn.status_message = Some("the agent's reply couldn't be read".into());
                        conv.flagged = true;
                        actions.push(Action::Notice(Notice::NoAnswer { share: conv.share.clone(), conversation: conv.id.clone() }));
                    }
                }
                let turn = conv.turns[i].turn.clone();
                conv.last_at = Some(now.clone());
                let role = conv.role;
                if role == Role::Base {
                    // The base is made: free its slot for the forks.
                    conv.proc = Proc::Stopped;
                    actions.push(Action::Stop { agent: conv.agent.clone() });
                }
                actions.push(Action::Save(Box::new(conv.clone())));
                if !internal {
                    actions.push(self.turn_audit(conv, &turn));
                    if let Some(u) = st.usage.as_mut() {
                        u.agent_time(&day, &share_slug, &person, ms);
                    }
                }
                actions.push(Action::Changed(Box::new(turn)));
            }
            SessionEvent::Limited { until } => {
                let ends = until.as_deref().and_then(|u| parse_until(u, now_t)).or_else(|| Some(now_t + chrono::Duration::from_std(LIMIT_WITHOUT_TIME).unwrap_or_default()));
                let fresh = !st.limited.contains_key(&share_slug);
                st.limited.insert(share_slug.clone(), Limit { until: until.clone(), ends });
                if fresh {
                    actions.push(Action::Notice(Notice::AgentLimit { share: share_slug.clone(), until: until.clone() }));
                }
                // Queued messages are told the same (§5.8).
                let queued: Vec<TurnId> = st.queue.iter().filter(|q| st.convs.get(&q.conversation()).is_some_and(|c| c.share == share_slug)).cloned().collect();
                let message = limit_sentence(&share_slug, until.as_deref());
                for q in queued {
                    st.queue.retain(|x| x != &q);
                    if let Some(conv) = st.convs.get_mut(&q.conversation()) {
                        if let Some(t) = conv.turn_mut(&q) {
                            t.turn.status = refusal(RefusalReason::AgentLimit, message.clone(), None, until.clone());
                            t.turn.status_message = Some(message.clone());
                            t.turn.done_at = Some(now.clone());
                            actions.push(Action::Changed(Box::new(t.turn.clone())));
                        }
                        actions.push(Action::Save(Box::new(conv.clone())));
                    }
                }
            }
            SessionEvent::LimitCleared => {
                st.limited.remove(&share_slug);
            }
            SessionEvent::Exited => {
                let conv = st.convs.get_mut(conv_id).expect("checked");
                if conv.stopping > 0 {
                    conv.stopping -= 1;
                    return;
                }
                if conv.proc == Proc::Stopped {
                    return;
                }
                conv.proc = Proc::Stopped;
                let agent = conv.agent.clone();
                if conv.active().is_some() {
                    let partial = self.host.partial_reply(&agent);
                    let conv = st.convs.get_mut(conv_id).expect("checked");
                    let i = conv.active().expect("checked");
                    let t = &mut conv.turns[i];
                    t.turn.status = TurnStatus::Failed { reason: Failure::Crashed };
                    t.turn.status_message = Some("the agent exited".into());
                    t.turn.done_at = Some(now.clone());
                    if t.turn.reply.is_none() {
                        t.turn.reply = partial;
                    }
                    let turn = t.turn.clone();
                    actions.push(self.turn_audit(conv, &turn));
                    actions.push(Action::Changed(Box::new(turn)));
                    actions.push(Action::Save(Box::new(conv.clone())));
                }
            }
        }
    }

    fn turn_audit(&self, conv: &Conv, turn: &Turn) -> Action {
        let outcome = match &turn.status {
            TurnStatus::Done => "done".to_string(),
            TurnStatus::Failed { reason } => format!("failed:{}", serde_json::to_value(reason).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()),
            TurnStatus::Canceled => "canceled".into(),
            TurnStatus::Refused(r) => format!("refused:{}", serde_json::to_value(r.reason).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()),
            other => format!("{other:?}").to_lowercase(),
        };
        let mut line = AuditLine::new(time::rfc3339(self.now()), "turn", outcome);
        line.agent = Some(conv.share.clone());
        line.login = conv.identity.key();
        line.peer = conv.proof.as_ref().map_or_else(|| "local".into(), |p| p.network().0.clone());
        line.node = conv.proof.as_ref().and_then(|p| p.device().map(|d| d.node.clone()));
        line.front = Some(match conv.caller.front {
            Front::A2a => "a2a".into(),
            Front::Mcp => "mcp".into(),
        });
        line.turn = Some(turn.id.0.clone());
        Action::Audit(Box::new(line))
    }

    /// The active turn of `agent`'s conversation fails.
    fn fail_active(&self, agent: &str, reason: Failure, why: &str) {
        let mut actions = Vec::new();
        {
            let mut st = self.lock();
            let Some(conv_id) = st.by_agent.get(agent).cloned() else { return };
            let now = time::rfc3339(self.now());
            let Some(conv) = st.convs.get_mut(&conv_id) else { return };
            if let Some(i) = conv.active() {
                let t = &mut conv.turns[i];
                t.turn.status = TurnStatus::Failed { reason };
                t.turn.status_message = Some(why.into());
                t.turn.done_at = Some(now);
                actions.push(Action::Changed(Box::new(t.turn.clone())));
                actions.push(Action::Save(Box::new(conv.clone())));
            }
            actions.extend(self.schedule(&mut st));
        }
        self.run(actions);
    }

    /// Every unfinished turn of `agent`'s conversation fails (it couldn't start).
    fn fail_all(&self, agent: &str, reason: Failure, why: &str) {
        let mut actions = Vec::new();
        {
            let mut st = self.lock();
            let Some(conv_id) = st.by_agent.get(agent).cloned() else { return };
            let now = time::rfc3339(self.now());
            if let Some(conv) = st.convs.get_mut(&conv_id) {
                for t in conv.turns.iter_mut().filter(|t| !t.turn.status.is_final() && !matches!(t.turn.status, TurnStatus::Queued { .. })) {
                    t.turn.status = TurnStatus::Failed { reason };
                    t.turn.status_message = Some(why.into());
                    t.turn.done_at = Some(now.clone());
                    actions.push(Action::Changed(Box::new(t.turn.clone())));
                }
                actions.push(Action::Save(Box::new(conv.clone())));
            }
            actions.extend(self.schedule(&mut st));
        }
        self.run(actions);
    }

    // ------------------------------------------------------------ time

    /// Timeouts, idle stops, vendor limits that ended, old conversations.
    /// The background ticker calls it every second; tests call it by hand.
    pub fn tick(&self) {
        let mut actions = Vec::new();
        {
            let mut st = self.lock();
            let now_t = self.now();
            let now = time::rfc3339(now_t);
            let day = time::day(now_t);
            st.limited.retain(|_, l| l.ends.is_none_or(|e| e > now_t));
            let ids: Vec<ConversationId> = st.convs.keys().cloned().collect();
            for id in ids {
                let Some(share) = st.convs.get(&id).and_then(|c| self.shares.get(&c.share)) else { continue };
                let timeout = chrono::Duration::minutes(i64::from(share.limits.turn_timeout_min));
                let conv = st.convs.get_mut(&id).expect("listed");
                // Turn timeout (§6.6): stopped, with whatever reply exists.
                if let Some(i) = conv.active() {
                    let started = conv.turns[i].started_at.as_deref().and_then(time::parse);
                    if started.is_some_and(|s| now_t - s >= timeout) {
                        let partial = self.host.partial_reply(&conv.agent);
                        let person = self.people.person(&conv.identity);
                        let t = &mut conv.turns[i];
                        t.turn.status = TurnStatus::Failed { reason: Failure::Timeout };
                        t.turn.status_message = Some(format!("stopped after {} min", share.limits.turn_timeout_min));
                        t.turn.done_at = Some(now.clone());
                        if t.turn.reply.is_none() {
                            t.turn.reply = partial;
                        }
                        let ms = started.map_or(0, |s| (now_t - s).num_milliseconds().max(0) as u64);
                        let turn = t.turn.clone();
                        let internal = t.internal;
                        conv.proc = Proc::Ready;
                        actions.push(Action::Interrupt { agent: conv.agent.clone() });
                        actions.push(Action::Save(Box::new(conv.clone())));
                        if !internal {
                            actions.push(self.turn_audit(conv, &turn));
                            if let Some(u) = st.usage.as_mut() {
                                u.agent_time(&day, &share.slug, &person, ms);
                            }
                        }
                        actions.push(Action::Changed(Box::new(turn)));
                        continue;
                    }
                }
                // Idle stop (§6.6): kinds that resume stop; the rest keep running.
                let idle = chrono::Duration::minutes(i64::from(share.limits.idle_stop_min));
                let last = conv.turns.iter().filter_map(|t| t.turn.done_at.as_deref()).filter_map(time::parse).max();
                let queued = st.queue.iter().any(|q| q.conversation() == id);
                let conv = st.convs.get_mut(&id).expect("listed");
                let resumes = self.agents.agent(&share.agent).is_some_and(|a| can_resume(&a.kind));
                if conv.proc == Proc::Ready && conv.active().is_none() && !queued && resumes && last.is_some_and(|l| now_t - l >= idle) {
                    conv.proc = Proc::Stopped;
                    actions.push(Action::Stop { agent: conv.agent.clone() });
                }
                // Conversations are kept `keepSessionsDays` after their last message.
                let keep = chrono::Duration::days(i64::from(share.limits.keep_sessions_days.max(1)));
                let last_at = conv.last_at.as_deref().or(Some(conv.created_at.as_str())).and_then(time::parse);
                if conv.active().is_none() && !queued && last_at.is_some_and(|l| now_t - l >= keep) {
                    self.remove_conv(&mut st, &id, &mut actions);
                }
            }
            actions.extend(self.schedule(&mut st));
        }
        self.run(actions);
    }

    // ------------------------------------------------------------ conversations

    fn new_conv(&self, share: &str, role: Role, identity: &Identity, person: PersonId, label: Option<String>, caller: SessionCaller) -> Conv {
        let id = ConversationId(format!("c_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]));
        Conv {
            id,
            share: share.into(),
            role,
            person: person.0,
            identity: identity.clone(),
            agent_label: label,
            caller,
            agent: uuid::Uuid::new_v4().to_string(),
            session_id: None,
            has_conversation: false,
            briefing: None,
            copy: None,
            current: true,
            created_at: time::rfc3339(self.now()),
            last_at: None,
            turns: Vec::new(),
            proc: Proc::Stopped,
            proof: None,
            last_used: 0,
            relaunch: false,
            rebuild: false,
            lines: Vec::new(),
            flagged: false,
            stopping: 0,
            lost: false,
        }
    }

    fn remove_conv(&self, st: &mut State, id: &ConversationId, actions: &mut Vec<Action>) {
        if let Some(conv) = st.convs.remove(id) {
            st.by_agent.remove(&conv.agent);
            st.queue.retain(|q| &q.conversation() != id);
            actions.push(Action::Remove { agent: conv.agent.clone(), folder: self.paths.session_dir(&conv.share, &conv.id.0) });
        }
    }

    /// The asker's conversation for this message: the one named (theirs
    /// only), a new one, or their current one with this share (created when
    /// there is none). Keyed by person and the asking agent's label (§6.1).
    fn conversation_for(&self, st: &mut State, asker: &Asker, person: &Person, ask: &Ask, share: &ShareSettings) -> Result<ConversationId> {
        if let Some(id) = &ask.conversation {
            let conv = st.convs.get(id).filter(|c| c.belongs_to(person) && c.share == share.slug).ok_or_else(|| PwError::not_found("no such conversation"))?;
            return Ok(conv.id.clone());
        }
        let label = asker.agent.clone();
        let mine = |c: &Conv| c.belongs_to(person) && c.share == share.slug && c.agent_label == label;
        if !ask.new_conversation {
            if let Some(c) = st.convs.values().find(|c| mine(c) && c.current) {
                return Ok(c.id.clone());
            }
        }
        for c in st.convs.values_mut().filter(|c| mine(c)) {
            c.current = false;
        }
        let (identity, guest, pitwall) = match &asker.who {
            Who::Network(c) => (c.identity().clone(), false, label.is_some() || asker.front == Front::A2a),
            Who::Guest => (person.identities[0].clone(), true, true),
        };
        let caller = SessionCaller { login: login(&identity), agent: label.clone(), client: asker.client.clone(), front: asker.front, guest, pitwall };
        let conv = self.new_conv(&share.slug, Role::Caller, &identity, person.id.clone(), label, caller);
        let id = conv.id.clone();
        st.by_agent.insert(conv.agent.clone(), id.clone());
        st.convs.insert(id.clone(), conv);
        Ok(id)
    }

    fn find_turn(&self, st: &State, asker: &Asker, turn: &TurnId) -> Result<Turn> {
        let person = self.person(&asker.who);
        st.convs
            .get(&turn.conversation())
            .filter(|c| c.belongs_to(&person))
            .and_then(|c| c.public_turns().find(|t| &t.turn.id == turn))
            .map(|t| t.turn.clone())
            .ok_or_else(|| PwError::not_found("no such turn"))
    }

    /// A message is refused when a limit says so (§5.8); `None`: go on.
    fn limit_refusal(&self, st: &State, share: &ShareSettings, person: &Person, guest: bool) -> Option<TurnStatus> {
        let now = self.now();
        let day = time::day(now);
        let midnight = time::rfc3339(self.clock.next_midnight(now));
        // The share's address as callers write it (`alice/api-expert`, §5.8).
        let owner = format!("{}/{}", super::turn::short_name(&self.owner_identity(None)), share.slug);
        if share.state == ShareState::Paused {
            return Some(refusal(RefusalReason::Paused, format!("{owner} is paused by its owner"), None, None));
        }
        let used_ms = st.usage.as_ref().map_or(0, |u| u.agent_ms(&day, &share.slug));
        if used_ms >= u64::from(share.limits.agent_minutes_per_day) * 60_000 {
            return Some(refusal(
                RefusalReason::ShareTotal,
                format!("{owner} has used its agent time for today; back at {}", time::clock_time(self.clock.next_midnight(now))),
                Some(midnight),
                None,
            ));
        }
        if let Some(l) = st.limited.get(&share.slug) {
            return Some(refusal(RefusalReason::AgentLimit, limit_sentence(&owner, l.until.as_deref()), None, l.until.clone()));
        }
        if !guest {
            let used = st.usage.as_ref().map_or(0, |u| u.messages_of(&day, &share.slug, person));
            if used >= Self::messages_per_day(share, person) {
                return Some(refusal(
                    RefusalReason::Quota,
                    format!("Daily limit reached for {owner}; resets at {}", time::clock_time(self.clock.next_midnight(now))),
                    Some(midnight),
                    None,
                ));
            }
        }
        None
    }

    /// The loop rule (§8.6), the same one the fronts apply before a message
    /// gets here ([`super::turn::chain_loop`]).
    fn loops(&self, asker: &Asker, share: &ShareSettings) -> bool {
        let network = match &asker.who {
            Who::Network(c) => Some(c.network().clone()),
            Who::Guest => None,
        };
        super::turn::chain_loop(&asker.chain, &share.slug, &self.owner_identity(network.as_ref())).is_some()
    }
}

/// The tag a conversation's sandbox puts on its denials (`pitwall-c_…`).
fn session_tag(conversation: &str) -> String {
    let id: String = conversation.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    format!("pitwall-{id}")
}

/// "From bob@example.com (agent client-fix on bob-mbp):" (§8.3.1).
fn from_line(conv: &Conv) -> String {
    let who = &conv.caller.login;
    if conv.caller.guest {
        return format!("From {who} (trying this share as a guest):");
    }
    match (&conv.caller.agent, &conv.caller.client, conv.caller.front) {
        (Some(agent), _, _) => match agent.split_once('@') {
            Some((name, device)) => format!("From {who} (agent {name} on {device}):"),
            None => format!("From {who} (agent {agent}):"),
        },
        (None, Some(client), Front::Mcp) => format!("From {who} ({client} over MCP):"),
        (None, None, Front::Mcp) => format!("From {who} (over MCP):"),
        _ => format!("From {who}:"),
    }
}

fn limit_sentence(share: &str, until: Option<&str>) -> String {
    match until {
        Some(u) => format!("{share} has reached its agent's usage limit; unavailable until {u}"),
        None => format!("{share} has reached its agent's usage limit; try again later"),
    }
}

/// `PATH` inside every session after the agent's own folders (the ones
/// that exist here).
const SYSTEM_PATH: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"];
/// `SHELL` inside a session: agents that run commands through it (Claude
/// Code's Bash tool) want bash or zsh.
const SESSION_SHELLS: &[&str] = &["/bin/zsh", "/bin/bash", "/bin/sh"];

/// A program name as absolute paths in `dirs` (an absolute path as is).
fn resolve_program(name: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
    if name.starts_with('/') {
        return vec![PathBuf::from(name)];
    }
    let found: Vec<PathBuf> = dirs.iter().map(|d| d.join(name)).filter(|p| p.is_file()).collect();
    if found.is_empty() {
        // Not installed here: keep the rule by name under the usual places.
        return vec![PathBuf::from("/usr/bin").join(name)];
    }
    found
}

impl Conversations for Collab {
    fn shares(&self, asker: &Asker) -> Vec<ShareCard> {
        let person = self.person(&asker.who);
        let st = self.lock();
        self.shares
            .list()
            .into_iter()
            .filter(|s| match asker.who {
                Who::Guest => s.state != ShareState::Paused,
                Who::Network(_) => s.state == ShareState::Live,
            })
            .filter(|s| self.visible(s, asker, &person))
            .map(|s| self.card(&st, &s, asker, &person))
            .collect()
    }

    fn ask(&self, asker: &Asker, ask: Ask) -> Result<Turn> {
        let person = self.person(&asker.who);
        let share = self.shares.get(&ask.share).filter(|s| self.visible(s, asker, &person)).ok_or_else(|| PwError::not_found("no such share"))?;
        if ask.my_changes.as_deref().is_some_and(|c| !c.trim().is_empty()) {
            return Err(PwError::unsupported("this share can't take changes (Read only)"));
        }
        if ask.message.trim().is_empty() {
            return Err(PwError::other("an empty message"));
        }
        let guest = matches!(asker.who, Who::Guest);
        let now_t = self.now();
        let now = time::rfc3339(now_t);
        let day = time::day(now_t);
        let mut actions = Vec::new();
        let turn = {
            let mut st = self.lock();
            let conv_id = self.conversation_for(&mut st, asker, &person, &ask, &share)?;
            let refused = if self.loops(asker, &share) {
                Some(refusal(RefusalReason::Loop, format!("{} already passed this message on", share.slug), None, None))
            } else {
                self.limit_refusal(&st, &share, &person, guest)
            };
            // A full queue is `busy` (§5.8).
            let refused = refused.or_else(|| {
                let conv = &st.convs[&conv_id];
                let now_ok = !conv.busy() && conv.proc.running();
                let queued = st.queue.iter().filter(|q| st.convs.get(&q.conversation()).is_some_and(|c| c.share == share.slug)).count() as u32;
                let max = self.settings().max_sessions.max(1);
                let share_free = Self::running_count(&st, Some(&share.slug)) < share.limits.sessions_running
                    || Self::idle_victim(&st, Some(&share.slug)).is_some();
                let machine_free = Self::running_count(&st, None) < max || Self::idle_victim(&st, None).is_some();
                let waits = !(now_ok || (share_free && machine_free && queued == 0 && !conv.busy()));
                (waits && queued >= share.limits.queue)
                    .then(|| refusal(RefusalReason::Busy, format!("{} is busy; try again in a few minutes", share.slug), None, None))
            });
            if refused.is_none() && !guest {
                if let Some(u) = st.usage.as_mut() {
                    u.message(&day, &share.slug, &person);
                }
            }
            let left = (!guest).then(|| self.left(&st, &share, &person));
            if let Who::Network(c) = &asker.who {
                if let Some(conv) = st.convs.get_mut(&conv_id) {
                    conv.proof = Some((**c).clone());
                }
            }
            let conv = st.convs.get_mut(&conv_id).expect("just made");
            let n = conv.public_turns().count() as u32 + 1;
            let id = TurnId::new(&conv.id, n);
            let status = refused.clone().unwrap_or(TurnStatus::Queued { position: 0 });
            let message = match &status {
                TurnStatus::Refused(r) => Some(r.message.clone()),
                _ => None,
            };
            let turn = Turn {
                id: id.clone(),
                conversation: conv.id.clone(),
                share: share.slug.clone(),
                status,
                status_message: message,
                reply: None,
                changes: None,
                left,
                at: now.clone(),
                done_at: refused.as_ref().map(|_| now.clone()),
            };
            conv.turns.push(TurnRec { turn: turn.clone(), text: ask.message.clone(), delivered: None, started_at: None, internal: false });
            conv.last_at = Some(now.clone());
            actions.push(Action::Save(Box::new(conv.clone())));
            let outcome = match &turn.status {
                TurnStatus::Refused(r) => format!("refused:{}", serde_json::to_value(r.reason).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()),
                _ => "ok".into(),
            };
            let mut line = self.audit_line(Some(asker), "ask", Some(&share.slug), &outcome);
            line.turn = Some(id.0.clone());
            actions.push(Action::Audit(Box::new(line)));
            if let TurnStatus::Refused(r) = &turn.status {
                if r.reason == RefusalReason::ShareTotal && st.told_total.insert((day.clone(), share.slug.clone())) {
                    actions.push(Action::Notice(Notice::ShareTotal { share: share.slug.clone() }));
                }
                drop(st);
                self.run(actions);
                return Ok(turn);
            }
            st.queue.push(id.clone());
            actions.extend(self.schedule(&mut st));
            st.convs[&conv_id].turns.iter().find(|t| t.turn.id == id).map(|t| t.turn.clone()).unwrap_or(turn)
        };
        self.run(actions);
        Ok(turn)
    }

    fn turn(&self, asker: &Asker, turn: &TurnId) -> Result<Turn> {
        let st = self.lock();
        self.find_turn(&st, asker, turn)
    }

    fn watch(&self, asker: &Asker, turn: &TurnId) -> Result<Receiver<Turn>> {
        let mut st = self.lock();
        let current = self.find_turn(&st, asker, turn)?;
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(current.clone());
        if !current.status.is_final() {
            st.watchers.entry(turn.clone()).or_default().push(tx);
        }
        Ok(rx)
    }

    fn turns(&self, asker: &Asker, conversation: Option<&ConversationId>) -> Result<Vec<Turn>> {
        let person = self.person(&asker.who);
        let st = self.lock();
        if let Some(id) = conversation {
            let conv = st.convs.get(id).filter(|c| c.belongs_to(&person)).ok_or_else(|| PwError::not_found("no such conversation"))?;
            return Ok(conv.public_turns().map(|t| t.turn.clone()).collect());
        }
        let mut out: Vec<Turn> = st.convs.values().filter(|c| c.belongs_to(&person)).flat_map(|c| c.public_turns().map(|t| t.turn.clone())).collect();
        out.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
        Ok(out)
    }

    fn cancel(&self, asker: &Asker, turn: &TurnId) -> Result<Turn> {
        let mut actions = Vec::new();
        let result = {
            let mut st = self.lock();
            let current = self.find_turn(&st, asker, turn)?;
            if current.status.is_final() {
                return Ok(current);
            }
            let now = time::rfc3339(self.now());
            let conv_id = turn.conversation();
            st.queue.retain(|q| q != turn);
            let conv = st.convs.get_mut(&conv_id).expect("found");
            let active = conv.active().map(|i| conv.turns[i].turn.id.clone());
            let agent = conv.agent.clone();
            if active.as_ref() == Some(turn) {
                actions.push(Action::Interrupt { agent: agent.clone() });
                let partial = self.host.partial_reply(&agent);
                let conv = st.convs.get_mut(&conv_id).expect("found");
                if conv.proc == Proc::Working || conv.proc == Proc::Blocked {
                    conv.proc = Proc::Ready;
                }
                let t = conv.turn_mut(turn).expect("found");
                if t.turn.reply.is_none() {
                    t.turn.reply = partial;
                }
            }
            let conv = st.convs.get_mut(&conv_id).expect("found");
            let t = conv.turn_mut(turn).expect("found");
            t.turn.status = TurnStatus::Canceled;
            t.turn.status_message = Some("canceled".into());
            t.turn.done_at = Some(now);
            let out = t.turn.clone();
            actions.push(Action::Changed(Box::new(out.clone())));
            actions.push(Action::Save(Box::new(conv.clone())));
            let mut line = self.audit_line(Some(asker), "cancel", Some(&conv.share.clone()), "ok");
            line.turn = Some(turn.0.clone());
            actions.push(Action::Audit(Box::new(line)));
            actions.extend(self.schedule(&mut st));
            out
        };
        self.run(actions);
        Ok(result)
    }

    fn patch(&self, asker: &Asker, turn: &TurnId) -> Result<String> {
        // Read only shares (C1a) never have changes; "Can change code" is C2.
        self.turn(asker, turn)?;
        Err(PwError::not_found("this turn has no changes"))
    }

    fn patch_part(&self, asker: &Asker, turn: &TurnId, _part: u32) -> Result<ChangesPart> {
        self.turn(asker, turn)?;
        Err(PwError::not_found("this turn has no changes"))
    }

    fn conversations(&self, asker: &Asker) -> Result<Vec<ConversationSummary>> {
        let person = self.person(&asker.who);
        let st = self.lock();
        Ok(st
            .convs
            .values()
            .filter(|c| c.belongs_to(&person))
            .map(|c| ConversationSummary {
                id: c.id.clone(),
                share: c.share.clone(),
                turns: c.public_turns().count() as u32,
                current: c.current,
                last_at: c.last_at.clone(),
            })
            .collect())
    }

    fn forget(&self, asker: &Asker, conversation: &ConversationId) -> Result<()> {
        let person = self.person(&asker.who);
        let mut actions = Vec::new();
        {
            let mut st = self.lock();
            let share = st.convs.get(conversation).filter(|c| c.belongs_to(&person)).map(|c| c.share.clone()).ok_or_else(|| PwError::not_found("no such conversation"))?;
            self.end_turns(&mut st, conversation, TurnStatus::Canceled, "conversation forgotten", &mut actions);
            self.remove_conv(&mut st, conversation, &mut actions);
            actions.retain(|a| !matches!(a, Action::Save(c) if &c.id == conversation));
            actions.push(Action::Audit(Box::new(self.audit_line(Some(asker), "forget", Some(&share), "ok"))));
            actions.extend(self.schedule(&mut st));
        }
        self.run(actions);
        Ok(())
    }

    fn shares_changed(&self) -> Receiver<()> {
        let (tx, rx) = mpsc::channel();
        self.lock().share_subs.push(tx);
        rx
    }
}

#[cfg(test)]
pub(crate) mod tests;
