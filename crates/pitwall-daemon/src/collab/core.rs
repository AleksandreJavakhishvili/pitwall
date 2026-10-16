//! The [`CollabBackend`] over the core's collaboration services: shares,
//! conversations and limits are `pitwall_core::collab::Collab`, teams its
//! `TeamStore`, the handler other Pitwalls and plain MCP clients reach is
//! the fronts' (`collab::front::Fronts`), asking another Pitwall is the
//! fronts' A2A client (`front::client::PitwallClient`), and the scan of a
//! scope is the sandbox's (`collab::sandboxed::scan_scope`).
//!
//! Networks come from the host (Tailscale in the app, Fake networks in
//! tests) and are made the first time collaboration is turned on ("Use
//! teams", or "Let people on `<network>` reach this machine"). The handler is
//! served on a network only while that network's switch is on.
//!
//! Who may call what was decided by the [`CollabService`](super::CollabService)
//! before a call gets here.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::{Duration, Instant, SystemTime};

use pitwall_core::collab::audit::AuditLog;
use pitwall_core::collab::engine_host::EngineHost;
use pitwall_core::collab::front::client::{turn_in, turn_of, Labels, Outgoing, PitwallClient};
use pitwall_core::collab::front::{Fronts, Options};
use pitwall_core::collab::sandboxed::{scan_scope, GitSnapshots, ProxyPool};
use pitwall_core::collab::store::CollabPaths;
use pitwall_core::collab::team::{PitwallInfo, TeamState};
use pitwall_core::collab::teams::keys::Ed25519;
use pitwall_core::collab::teams::{sync, ParsedInvite, TeamStore, TeamsDeps};
use pitwall_core::collab::turn::short_name;
use pitwall_core::collab::{
    Ask, Asker, Collab, CollabDeps, Conversations, ConversationId, HookWiring, RunMode, Turn, TurnStatus, Who,
};
use pitwall_core::error::{ErrorCode, PwError};
use pitwall_core::exec::LocalExec;
use pitwall_core::net::{Caller, DeviceAddr, Handler, InstanceKey, Network, PersonId, Probed, Prober, Request, Response, ServeHandle};
use pitwall_core::sandbox::SessionSandbox;
use pitwall_core::Shared;
use pitwall_proto::api::collab::{
    AskSend, CallerUsage, CollabSettings, DeviceView, InviteLink, JoinRequestView, MemberAdded, MemberView, NetworkRow, PersonView,
    ProbeResult, SensitiveFile, SetupLine, ShareUsage, TeamCreate, TeamDetail, TeamJoin, UsageCounts,
};
use pitwall_proto::collab::{
    AskFrom, AskStatus, AskView, DirectoryEntry, Front, Identity, RefusalReason, SessionState, ShareSettings, ShareState,
    SharedSessionView, TeamCard, TeamId, TeamRole,
};
use pitwall_proto::{code, ErrorBody};

use super::memory::{label_of, rfc3339, state_of};
use super::scan::{PatternScan, ScanResult, Scanner};
use super::{CollabBackend, Me};

/// How long a directory answer is reused before the devices are asked again.
const DIRECTORY_TTL: Duration = Duration::from_secs(30);
/// How often joins are polled and teams looked for while "Use teams" is on.
const JOIN_POLL: Duration = Duration::from_secs(30);
const DISCOVERY_EVERY: Duration = Duration::from_secs(5 * 60);
/// The directory asks devices that didn't answer again after this.
const RETRY_SILENT: Duration = Duration::from_secs(5 * 60);
/// `team.list` looks for teams again when the last look is older than this.
const DISCOVERY_TTL: Duration = Duration::from_secs(60);
/// `ask.send` without a timeout waits this long for the turn to settle.
const ASK_WAIT: Duration = Duration::from_secs(10 * 60);

/// Makes this Pitwall's networks (called once, when collaboration is first
/// turned on).
pub type NetworkFactory = Box<dyn Fn() -> Vec<Arc<dyn Network>> + Send + Sync>;

/// What [`CoreCollab`] is built from.
pub struct CoreDeps {
    /// Pitwall's data folder (`collab/` and the instance key live in it).
    pub paths: pitwall_core::paths::Paths,
    /// The engine shared sessions run in (as ordinary agents).
    pub engine: Shared,
    pub networks: NetworkFactory,
    /// The OS sandbox (`sandbox::session_sandbox()`).
    pub sandbox: Arc<dyn SessionSandbox>,
    /// How shared sessions' hooks reach Pitwall.
    pub hooks: Option<HookWiring>,
    /// Pitwall's version and this device's name, for `/info`.
    pub version: String,
    pub label: String,
    /// The owner's login while no network names them (`$USER`).
    pub user: String,
}

/// Switches and my own asks (`collab/local.json`, `collab/asks.json`).
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Local {
    #[serde(default)]
    settings: CollabSettings,
}

#[derive(Default)]
struct State {
    settings: CollabSettings,
    asks: Vec<AskView>,
    probes: HashMap<String, Vec<ProbeResult>>,
}

/// A shared agent I may use, and the device that answered for it.
#[derive(Clone)]
struct Found {
    entry: DirectoryEntry,
    addr: DeviceAddr,
}

/// Collaboration on the core.
pub struct CoreCollab {
    collab: Arc<Collab>,
    teams: Arc<TeamStore>,
    proxies: Arc<ProxyPool>,
    handler: Arc<dyn Handler>,
    make_networks: NetworkFactory,
    networks: RwLock<Option<Vec<Arc<dyn Network>>>>,
    serving: Mutex<HashMap<String, ServeHandle>>,
    state: Mutex<State>,
    directory: Mutex<Option<(Instant, Vec<Found>)>>,
    /// When teams were last looked for.
    discovered_at: Mutex<Option<Instant>>,
    /// When the directory last asked devices that hadn't answered.
    forced_at: Mutex<Option<Instant>>,
    prober: Prober,
    dir: PathBuf,
    user: String,
    this: Weak<CoreCollab>,
}

/// The fronts, with persons resolved by the core (linked identities are
/// one person, §15.7).
struct Served {
    fronts: Fronts,
    collab: Weak<Collab>,
}

impl Handler for Served {
    fn handle(&self, req: Request, caller: Caller) -> Response {
        self.fronts.handle(req, caller)
    }

    fn person_of(&self, identity: &Identity) -> PersonId {
        match self.collab.upgrade() {
            Some(c) => c.person_of(identity),
            None => PersonId::of(identity),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A core error as the socket's error body.
pub fn body(e: PwError) -> ErrorBody {
    let c = match e.code {
        ErrorCode::NotFound => code::NOT_FOUND,
        ErrorCode::Unsupported => code::UNSUPPORTED,
        ErrorCode::NotRunning => code::NOT_RUNNING,
        ErrorCode::Unreachable => code::OFFLINE,
        ErrorCode::Conflict => code::CONFLICT,
        ErrorCode::Denied => code::DENIED,
        ErrorCode::Other => code::OTHER,
    };
    ErrorBody::new(c, e.message)
}

/// A refusal's code for the CLI (`quota`, `busy`, …).
fn refusal_code(reason: RefusalReason) -> &'static str {
    match reason {
        RefusalReason::Quota => code::QUOTA,
        RefusalReason::Busy | RefusalReason::Paused => code::BUSY,
        RefusalReason::ShareTotal => code::SHARE_TOTAL,
        RefusalReason::AgentLimit => code::AGENT_LIMIT,
        RefusalReason::Loop => code::LOOP,
    }
}

/// How a person is shown: email, else subject.
fn login(identity: &Identity) -> String {
    identity.email.clone().unwrap_or_else(|| identity.subject.clone())
}

/// A turn as the caller's thread shows it.
pub fn ask_view(share: &str, text: &str, t: &Turn) -> AskView {
    let (status, message) = match &t.status {
        TurnStatus::Queued { .. } => (AskStatus::Queued, t.status_message.clone()),
        TurnStatus::Working => (AskStatus::Working, t.status_message.clone()),
        TurnStatus::WaitingForOwner | TurnStatus::ChangesHeld => (AskStatus::WaitingForOwner, t.status_message.clone()),
        TurnStatus::Done => (AskStatus::Done, t.status_message.clone()),
        TurnStatus::Failed { .. } => (AskStatus::Failed, t.status_message.clone()),
        TurnStatus::Canceled => (AskStatus::Canceled, t.status_message.clone()),
        TurnStatus::Refused(r) => (AskStatus::Refused, Some(r.message.clone())),
    };
    AskView {
        turn: t.id.0.clone(),
        conversation: t.conversation.0.clone(),
        share: share.to_string(),
        text: text.to_string(),
        from: AskFrom::Me,
        status,
        status_message: message,
        reply: t.reply.as_ref().map(|r| r.text.clone()),
        reply_source: t.reply.as_ref().map(|r| r.source),
        changes: None,
        at: t.at.clone(),
        done_at: t.done_at.clone(),
    }
}

impl CoreCollab {
    /// Open collaboration on the data folder: shares, conversations and
    /// teams as stored, the instance key (made on first use), shared
    /// sessions on the engine. Networks wait until collaboration is on.
    pub fn start(deps: CoreDeps) -> Result<Arc<CoreCollab>, String> {
        let CoreDeps { paths, engine, networks, sandbox, hooks, version, label, user } = deps;
        let cpaths = CollabPaths::of(&paths);
        let dir = cpaths.root().to_path_buf();
        let local: Local = std::fs::read_to_string(dir.join("local.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let asks: Vec<AskView> = std::fs::read_to_string(dir.join("asks.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let key = InstanceKey::load_or_create(paths.root()).map_err(|e| e.message)?;
        let clock = Arc::new(pitwall_core::collab::time::SystemTime);
        let teams = TeamStore::open(TeamsDeps {
            paths: cpaths.clone(),
            signer: Arc::new(key),
            check: Arc::new(Ed25519),
            clock: clock.clone(),
            audit: Arc::new(AuditLog::new(cpaths.audit_file())),
            version: version.clone(),
            label,
        });
        let host = EngineHost::new(engine);
        let proxies = Arc::new(ProxyPool::new());
        let collab = Collab::open(CollabDeps {
            paths: cpaths,
            clock,
            host: host.clone(),
            agents: host,
            sandbox,
            snapshots: Arc::new(GitSnapshots::new(Arc::new(LocalExec))),
            egress: proxies.clone(),
            skills: None,
            teams: Some(teams.clone()),
            hooks,
            settings: pitwall_core::collab::CollabSettings { max_sessions: local.settings.session_cap.max(1), owner_name: user.clone() },
            run: RunMode::Background,
        });
        collab.start();
        let me = Arc::new_cyclic(|this: &Weak<CoreCollab>| {
            let endpoints = this.clone();
            let fronts = Fronts::new(
                collab.clone(),
                teams.clone(),
                move |id| endpoints.upgrade()?.networks().into_iter().find(|n| &n.id() == id).map(|n| n.endpoint()),
                Options { version, ..Options::default() },
            );
            CoreCollab {
                handler: Arc::new(Served { fronts, collab: Arc::downgrade(&collab) }),
                collab,
                teams,
                proxies,
                make_networks: networks,
                networks: RwLock::new(None),
                serving: Mutex::default(),
                state: Mutex::new(State { settings: local.settings, asks, probes: HashMap::new() }),
                directory: Mutex::default(),
                discovered_at: Mutex::default(),
                forced_at: Mutex::default(),
                prober: Prober::default(),
                dir,
                user,
                this: this.clone(),
            }
        });
        me.apply();
        me.background();
        Ok(me)
    }

    /// The core's collaboration service (tests, the app's own views).
    pub fn collab(&self) -> &Arc<Collab> {
        &self.collab
    }

    pub fn teams_store(&self) -> &Arc<TeamStore> {
        &self.teams
    }

    /// The handler other Pitwalls and MCP clients reach (served on every
    /// network whose switch is on).
    pub fn handler(&self) -> Arc<dyn Handler> {
        self.handler.clone()
    }

    /// The networks, once collaboration was turned on.
    pub fn networks(&self) -> Vec<Arc<dyn Network>> {
        self.networks.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
    }

    fn settings_now(&self) -> CollabSettings {
        lock(&self.state).settings.clone()
    }

    fn on(s: &CollabSettings) -> bool {
        s.use_teams || s.reachable.values().any(|v| *v)
    }

    /// Make the networks (once), and hand them to the core and the teams.
    fn ensure_networks(self: &Arc<Self>) -> Vec<Arc<dyn Network>> {
        if let Some(n) = self.networks.read().unwrap_or_else(|e| e.into_inner()).clone() {
            return n;
        }
        let mut slot = self.networks.write().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = slot.clone() {
            return n;
        }
        let nets = (self.make_networks)();
        self.collab.set_networks(nets.clone());
        self.teams.set_networks(nets.clone());
        for net in &nets {
            // Someone the provider removed: what we hold for them ends (§15.4).
            let rx = net.revocations();
            let collab = Arc::downgrade(&self.collab);
            std::thread::spawn(move || {
                for r in rx {
                    match collab.upgrade() {
                        Some(c) => c.on_revocation(&r),
                        None => return,
                    }
                }
            });
        }
        *slot = Some(nets.clone());
        nets
    }

    /// Settings → Collaboration as they are now: the core's session cap,
    /// the networks, and serving where "reachable" is on.
    fn apply(self: &Arc<Self>) {
        let s = self.settings_now();
        let me = self.me();
        self.collab.set_settings(pitwall_core::collab::CollabSettings { max_sessions: s.session_cap.max(1), owner_name: me.name });
        let nets = if Self::on(&s) { self.ensure_networks() } else { self.networks() };
        let mut serving = lock(&self.serving);
        for net in nets {
            let id = net.id().0;
            let want = s.reachable.get(&id).copied().unwrap_or(false);
            if want && !serving.contains_key(&id) {
                match net.serve(self.handler.clone()) {
                    Ok(h) => {
                        serving.insert(id, h);
                    }
                    Err(e) => eprintln!("pitwall: serving collaboration on {id}: {e}"),
                }
            } else if !want {
                // Dropping the handle stops the listener.
                serving.remove(&id);
            }
        }
    }

    /// Joins polled and teams looked for in the background while "Use
    /// teams" is on.
    fn background(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            let mut last_discovery: Option<Instant> = None;
            loop {
                std::thread::sleep(JOIN_POLL);
                let Some(me) = weak.upgrade() else { return };
                if !me.settings_now().use_teams {
                    continue;
                }
                let _ = sync::poll_joins(&me.teams);
                if last_discovery.is_none_or(|t| t.elapsed() >= DISCOVERY_EVERY) {
                    last_discovery = Some(Instant::now());
                    let _ = sync::discover(&me.teams);
                }
            }
        });
    }

    fn save_local(&self, st: &State) {
        let _ = std::fs::create_dir_all(&self.dir);
        if let Ok(text) = serde_json::to_string_pretty(&Local { settings: st.settings.clone() }) {
            let _ = pitwall_core::collab::store::write_atomic(&self.dir.join("local.json"), text.as_bytes());
        }
        if let Ok(text) = serde_json::to_string_pretty(&st.asks) {
            let _ = pitwall_core::collab::store::write_atomic(&self.dir.join("asks.json"), text.as_bytes());
        }
    }

    fn my_identities(&self) -> Vec<Identity> {
        self.networks().iter().filter_map(|n| n.me().map(|(i, _)| i)).collect()
    }

    // ------------------------------------------------------------ directory

    /// Shared agents I may use: every Pitwall on my networks that answers
    /// `/info` (probed through `net::Prober`), asked for its directory.
    fn found(&self, force: bool) -> Vec<Found> {
        if !self.settings_now().use_teams {
            return Vec::new();
        }
        if !force {
            if let Some((at, list)) = lock(&self.directory).as_ref() {
                if at.elapsed() < DIRECTORY_TTL {
                    return list.clone();
                }
            }
        }
        // Devices that didn't answer are asked again every few minutes, not
        // after the Prober's day: a teammate may switch "reachable" on.
        let force = force || lock(&self.forced_at).is_none_or(|t| t.elapsed() >= RETRY_SILENT);
        if force {
            *lock(&self.forced_at) = Some(Instant::now());
        }
        let mut out: Vec<Found> = Vec::new();
        for net in self.networks() {
            let Ok(hints) = net.discover() else { continue };
            let answered: Vec<Probed<PitwallInfo>> = self.prober.probe(&*net, &hints, force);
            for p in answered {
                let Ok(http) = net.client(&p.hint.addr) else { continue };
                let Ok(list) = PitwallClient::new(http).directory() else { continue };
                out.extend(list.into_iter().map(|entry| Found { entry, addr: p.hint.addr.clone() }));
            }
        }
        // Agents seen before and not now: shown offline, with when.
        let previous = lock(&self.directory).take();
        if let Some((at, old)) = previous {
            let seen = rfc3339(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()).saturating_sub(at.elapsed().as_secs()));
            for mut f in old {
                if !out.iter().any(|n| n.entry.owner == f.entry.owner && n.entry.slug == f.entry.slug) {
                    if f.entry.online {
                        f.entry.last_seen = Some(seen.clone());
                    }
                    f.entry.online = false;
                    out.push(f);
                }
            }
        }
        *lock(&self.directory) = Some((Instant::now(), out.clone()));
        out
    }

    /// `alice/api-expert` among the agents I may use.
    fn find(&self, share: &str) -> Option<Found> {
        let (owner, slug) = share.split_once('/').unwrap_or(("", share));
        let hit = |list: Vec<Found>| {
            list.into_iter().find(|f| f.entry.slug == slug && (f.entry.owner == owner || f.entry.owner.split('@').next() == Some(owner)))
        };
        hit(self.found(false)).filter(|f| f.entry.online).or_else(|| hit(self.found(true)))
    }

    fn client_for(&self, addr: &DeviceAddr, agent: Option<&str>) -> Result<PitwallClient, ErrorBody> {
        let net = self
            .networks()
            .into_iter()
            .find(|n| n.id() == addr.network)
            .ok_or_else(|| ErrorBody::new(code::OFFLINE, format!("not on {}", addr.network.0)))?;
        let http = net.client(addr).map_err(body)?;
        Ok(PitwallClient::new(http).with_labels(Labels { agent: agent.map(String::from), chain: Vec::new() }))
    }

    /// Asks still running: their latest state from the owner's Pitwall.
    fn refresh_pending(&self) {
        let pending: Vec<AskView> = lock(&self.state)
            .asks
            .iter()
            .filter(|a| matches!(a.status, AskStatus::Queued | AskStatus::Working | AskStatus::WaitingForOwner))
            .cloned()
            .collect();
        if pending.is_empty() {
            return;
        }
        for a in pending {
            let Some(f) = self.find(&a.share) else { continue };
            let Ok(client) = self.client_for(&f.addr, None) else { continue };
            let Ok(task) = client.agent(&f.entry.slug).get(&pitwall_core::collab::TurnId(a.turn.clone())) else { continue };
            if let Some(t) = turn_of(&task) {
                let view = ask_view(&a.share, &a.text, &t);
                let mut st = lock(&self.state);
                if let Some(slot) = st.asks.iter_mut().find(|x| x.turn == a.turn) {
                    *slot = view;
                }
                self.save_local(&st);
            }
        }
    }

    // ------------------------------------------------------------ teams

    fn member_view(&self, state: &TeamState, m: &pitwall_core::collab::team::Member) -> MemberView {
        let first = m.identities.first().cloned().unwrap_or_else(|| Identity::new("unknown", &m.member.0));
        MemberView {
            person: self.collab.person_of(&first).0,
            login: login(&first),
            identities: m.identities.clone(),
            devices: m
                .devices
                .iter()
                .flat_map(|d| d.addr.iter())
                .map(|a| DeviceView { label: a.node.clone(), network: a.network.clone(), online: true })
                .collect(),
            admin: state.is_admin(&m.member),
            claude_only: m.devices.is_empty(),
        }
    }

    fn detail(&self, team: &TeamId) -> Option<TeamDetail> {
        let card = self.teams.cards().into_iter().find(|c| &c.team == team)?;
        let Some(state) = self.teams.state(team) else {
            return Some(TeamDetail { card, members: vec![], last_change_by: None, last_change_at: None, requests: vec![], setup: vec![], conflict: None });
        };
        let head = self.teams.chain(team).and_then(|c| c.last().cloned());
        let last_change_by = head.as_ref().and_then(|r| state.member(&r.by.member)).and_then(|m| m.identities.first()).map(login);
        let requests = if card.mine == Some(TeamRole::Admin) {
            self.teams
                .requests(team)
                .into_iter()
                .map(|r| {
                    let node = r.device.addr.first().map(|a| format!(" on {} ({})", a.node, a.network.0)).unwrap_or_default();
                    JoinRequestView {
                        person: self.collab.person_of(&r.identity).0,
                        login: login(&r.identity),
                        note: r.note,
                        from: format!("from {}{node}", login(&r.identity)),
                        at: r.at,
                    }
                })
                .collect()
        } else {
            vec![]
        };
        Some(TeamDetail {
            members: state.members.iter().map(|m| self.member_view(&state, m)).collect(),
            last_change_by,
            last_change_at: head.map(|r| r.at),
            requests,
            setup: self.setup_lines(),
            conflict: None,
            card,
        })
    }

    fn team_or_err(&self, team: &TeamId) -> Result<TeamDetail, ErrorBody> {
        self.detail(team).ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("you aren't in the team {}", team.0)))
    }
}

impl CollabBackend for CoreCollab {
    fn me(&self) -> Me {
        match self.my_identities().into_iter().next() {
            Some(i) => Me { name: short_name(&i), login: login(&i) },
            None => Me { login: self.user.clone(), name: self.user.clone() },
        }
    }

    fn is_shared_session(&self, agent_id: &str) -> bool {
        self.collab.is_session(agent_id)
    }

    fn settings(&self) -> CollabSettings {
        self.settings_now()
    }

    fn set_settings(&self, s: CollabSettings) -> Result<(), ErrorBody> {
        {
            let mut st = lock(&self.state);
            st.settings = s;
            self.save_local(&st);
        }
        *lock(&self.directory) = None;
        *lock(&self.discovered_at) = None;
        if let Some(me) = self.this.upgrade() {
            me.apply();
        }
        Ok(())
    }

    fn networks(&self) -> Vec<NetworkRow> {
        let reach = self.settings_now().reachable;
        CoreCollab::networks(self)
            .iter()
            .map(|n| {
                let id = n.id();
                NetworkRow {
                    reachable: reach.get(id.as_str()).copied().unwrap_or(false),
                    label: label_of(n),
                    state: state_of(&n.state()),
                    me: n.me().map(|(i, addr)| format!("{} on {}", login(&i), addr.node)),
                    endpoint: Some(n.endpoint().base_url).filter(|u| !u.is_empty()),
                    id,
                }
            })
            .collect()
    }

    fn setup_lines(&self) -> Vec<SetupLine> {
        let reach = self.settings_now().reachable;
        let name = self.me().name;
        CoreCollab::networks(self)
            .iter()
            .filter(|n| reach.get(n.id().as_str()).copied().unwrap_or(false))
            .filter(|n| !n.endpoint().base_url.is_empty())
            .map(|n| SetupLine {
                network: n.id(),
                owner: name.clone(),
                line: format!("claude mcp add --transport http {name} {}/mcp", n.endpoint().base_url.trim_end_matches('/')),
            })
            .collect()
    }

    // ------------------------------------------------------------ shares

    fn shares(&self) -> Vec<ShareSettings> {
        self.collab.list_shares()
    }

    fn save_share(&self, s: &ShareSettings, previous: Option<&str>) -> Result<(), ErrorBody> {
        let renamed = previous.filter(|p| *p != s.slug);
        if let Some(old) = renamed {
            // A share's address is what callers keep: only drafts move.
            let current = self.collab.share(old).ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no share {old}")))?;
            if current.state != ShareState::Draft {
                return Err(ErrorBody::new(code::CONFLICT, "a live share keeps its address: stop sharing and share it again to rename it"));
            }
            self.collab.stop_share(old).map_err(body)?;
        }
        let Some(current) = self.collab.share(&s.slug) else {
            let mut draft = s.clone();
            draft.state = ShareState::Draft;
            self.collab.create_share(draft).map_err(body)?;
            if s.state == ShareState::Live {
                self.collab.go_live(&s.slug, false).map_err(body)?;
            }
            return Ok(());
        };
        let mut next = s.clone();
        next.state = current.state;
        if next != current {
            self.collab.set_share(&s.slug, next).map_err(body)?;
        }
        match (current.state, s.state) {
            (a, b) if a == b => Ok(()),
            (ShareState::Draft, ShareState::Live) => self.collab.go_live(&s.slug, false).map(|_| ()).map_err(body),
            (ShareState::Live, ShareState::Paused) => self.collab.pause(&s.slug).map(|_| ()).map_err(body),
            (ShareState::Paused, ShareState::Live) => self.collab.resume(&s.slug).map(|_| ()).map_err(body),
            (_, ShareState::Draft) => Err(ErrorBody::new(code::CONFLICT, "a share that went live doesn't go back to a draft")),
            (ShareState::Draft, ShareState::Paused) => Err(ErrorBody::new(code::CONFLICT, "a draft goes live before it can pause")),
            _ => Ok(()),
        }
    }

    fn go_live(&self, s: &ShareSettings, keep_guest: bool) -> Result<(), ErrorBody> {
        let mut draft = s.clone();
        draft.state = self.collab.share(&s.slug).map_or(ShareState::Draft, |c| c.state);
        self.save_share(&draft, None)?;
        self.collab.go_live(&s.slug, keep_guest).map(|_| ()).map_err(body)
    }

    fn remove_share(&self, slug: &str) -> Result<(), ErrorBody> {
        lock(&self.state).probes.remove(slug);
        self.collab.stop_share(slug).map_err(body)
    }

    fn sessions(&self, slug: &str) -> Vec<SharedSessionView> {
        self.collab.sessions(slug)
    }

    fn usage(&self, slug: &str) -> ShareUsage {
        let Some(share) = self.collab.share(slug) else { return ShareUsage::default() };
        let today: Vec<_> = self.collab.usage(None).into_iter().filter(|r| r.share == slug).collect();
        let month: Vec<_> = self.collab.usage_days(30).into_iter().filter(|r| r.share == slug).collect();
        let blocked: Vec<String> = self.collab.people().blocked().iter().map(|i| i.key()).collect();
        let mut callers: BTreeMap<String, CallerUsage> = BTreeMap::new();
        for (rows, is_today) in [(&month, false), (&today, true)] {
            for r in rows.iter() {
                let c = callers.entry(r.person.clone()).or_insert_with(|| {
                    let o = share.limits.overrides.get(&r.person);
                    CallerUsage {
                        person: r.person.clone(),
                        login: r.person.split_once(':').map_or(r.person.clone(), |(_, s)| s.to_string()),
                        today: UsageCounts::default(),
                        days30: UsageCounts::default(),
                        messages_per_day: o.and_then(|o| o.messages_per_day).unwrap_or(share.limits.per_caller.messages_per_day),
                        blocked: o.is_some_and(|o| o.blocked) || blocked.contains(&r.person),
                    }
                });
                let counts = if is_today { &mut c.today } else { &mut c.days30 };
                counts.messages += r.messages;
                counts.minutes += (r.agent_ms / 60_000) as u32;
            }
        }
        let running = self
            .collab
            .sessions(slug)
            .iter()
            .filter(|s| matches!(s.state, SessionState::Starting | SessionState::Ready | SessionState::Working | SessionState::Blocked))
            .count() as u32;
        ShareUsage {
            agent_minutes_today: (today.iter().map(|r| r.agent_ms).sum::<u64>() / 60_000) as u32,
            running,
            limited_until: self.collab.limited(slug).map(|u| u.unwrap_or_else(|| "later".into())),
            callers: callers.into_values().collect(),
        }
    }

    fn probes(&self, slug: &str) -> Vec<ProbeResult> {
        lock(&self.state).probes.get(slug).cloned().unwrap_or_default()
    }

    fn guest(&self, slug: &str, probe: &str, message: &str) -> Result<ProbeResult, ErrorBody> {
        let share = self.collab.share(slug).ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no share {slug}")))?;
        let since = SystemTime::now();
        let asker = Asker { who: Who::Guest, agent: None, client: None, chain: Vec::new(), front: Front::A2a };
        // Each guest message in a conversation of its own: no history between probes.
        let ask = Ask { share: slug.into(), message: message.into(), new_conversation: true, ..Ask::default() };
        let turn = self.collab.ask(&asker, ask).map_err(body)?;
        let rx = self.collab.watch(&asker, &turn.id).map_err(body)?;
        let deadline = Instant::now() + Duration::from_secs(u64::from(share.limits.turn_timeout_min) * 60 + 60).min(Duration::from_secs(15 * 60));
        let mut last = turn;
        while !last.status.is_final() {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(t) => last = t,
                Err(_) => break,
            }
        }
        let mut tried = self.proxies.refused_since(slug, since);
        if !matches!(last.status, TurnStatus::Done) {
            tried.push(format!("the turn ended {:?}: {}", last.status, last.status_message.clone().unwrap_or_default()));
        }
        let secs = since.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let r = ProbeResult { probe: probe.into(), message: message.into(), reply: last.reply.map(|r| r.text), tried, at: rfc3339(secs) };
        lock(&self.state).probes.entry(slug.into()).or_default().push(r.clone());
        Ok(r)
    }

    fn blocked(&self) -> Vec<String> {
        self.collab.people().blocked().iter().map(|i| PersonId::of(i).0).collect()
    }

    fn set_blocked(&self, person: &str, blocked: bool) -> Result<(), ErrorBody> {
        let (provider, subject) = person.split_once(':').ok_or_else(|| ErrorBody::new(code::BAD_PARAMS, format!("{person}: a person id is provider:login")))?;
        let identity = Identity::new(provider, subject);
        if blocked {
            self.collab.block(&identity).map_err(body)
        } else {
            self.collab.unblock(&identity).map_err(body)
        }
    }

    fn scan(&self, scope: &pitwall_proto::collab::Scope) -> ScanResult {
        match scan_scope(&LocalExec, scope) {
            Ok(s) => ScanResult {
                sensitive: s.findings.into_iter().map(|f| SensitiveFile { path: f.path, why: f.reasons.join("; ") }).collect(),
                files: Some(s.files),
            },
            // Not a git checkout (a copy can't be made of it anyway): names only.
            Err(_) => PatternScan::default().scan(scope),
        }
    }

    // ------------------------------------------------------------ teams

    /// Mine, then the listed ones discovered on my networks: looked for
    /// again when the last look is older than a minute, and pending joins
    /// pulled once accepted.
    fn teams(&self) -> Vec<TeamCard> {
        if self.settings_now().use_teams {
            let stale = lock(&self.discovered_at).is_none_or(|t| t.elapsed() >= DISCOVERY_TTL);
            if stale {
                *lock(&self.discovered_at) = Some(Instant::now());
                let _ = sync::discover(&self.teams);
            }
            let _ = sync::poll_joins(&self.teams);
        }
        self.teams.cards()
    }

    fn team(&self, team: &TeamId) -> Option<TeamDetail> {
        self.detail(team)
    }

    fn create_team(&self, req: &TeamCreate) -> Result<TeamDetail, ErrorBody> {
        let id = self.teams.create(req.name.trim(), req.description.trim(), req.listed, req.join).map_err(body)?;
        self.team_or_err(&id)
    }

    fn join_team(&self, req: &TeamJoin) -> Result<TeamCard, ErrorBody> {
        let key = req.team.trim();
        if key.starts_with("pitwall://") {
            let invite = ParsedInvite::parse(key).ok_or_else(|| ErrorBody::new(code::BAD_PARAMS, "not an invite link"))?;
            // The link names the admin; any of their online devices redeems it.
            let mut last = ErrorBody::new(code::OFFLINE, format!("Can't reach {} right now: try again when they are online", invite.by));
            for net in CoreCollab::networks(self) {
                for hint in net.discover().unwrap_or_default().into_iter().filter(|h| h.online) {
                    if hint.owner.as_ref().is_none_or(|o| o.key() != invite.by) {
                        continue;
                    }
                    match sync::redeem(&self.teams, &hint.addr, key) {
                        Ok(team) => return self.teams.cards().into_iter().find(|c| c.team == team).ok_or_else(|| ErrorBody::new(code::OTHER, "joined, but the team isn't here yet")),
                        Err(e) => last = body(e),
                    }
                }
            }
            return Err(last);
        }
        let norm = |s: &str| s.replace('-', "").to_ascii_uppercase();
        let card = self
            .teams
            .cards()
            .into_iter()
            .filter(|c| c.mine.is_none())
            .find(|c| c.team.0 == key || norm(&c.fingerprint) == norm(key) || c.name.eq_ignore_ascii_case(key))
            .ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no listed team {key} on your networks (see `pitwall team list`)")))?;
        if card.join == pitwall_proto::collab::JoinPolicy::Closed {
            return Err(ErrorBody::new(code::DENIED, format!("{} is closed: ask an admin for an invite link", card.name)));
        }
        let seen: Vec<DeviceAddr> = self
            .teams
            .discovered()
            .into_iter()
            .find(|d| d.record.team.as_ref() == Some(&card.team) || pitwall_core::collab::teams::chain::team_id(&d.record) == card.team)
            .map(|d| d.seen_at)
            .unwrap_or_default();
        let mut last = ErrorBody::new(code::OFFLINE, format!("Can't reach a member of {} right now", card.name));
        for at in seen {
            match sync::request_join(&self.teams, &at, &card.team, &req.note) {
                Ok(()) => return Ok(self.teams.cards().into_iter().find(|c| c.team == card.team).unwrap_or(card)),
                Err(e) => last = body(e),
            }
        }
        Err(last)
    }

    fn leave_team(&self, team: &TeamId) -> Result<(), ErrorBody> {
        self.teams.leave(team).map_err(body)
    }

    fn approve(&self, team: &TeamId, person: &str, allow: bool) -> Result<TeamDetail, ErrorBody> {
        let request = self
            .teams
            .requests(team)
            .into_iter()
            .find(|r| PersonId::of(&r.identity).0 == person || r.identity.subject == person || r.identity.email.as_deref() == Some(person))
            .ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no request from {person}")))?;
        if allow {
            self.teams.approve(team, &request.identity).map_err(body)?;
        } else {
            self.teams.deny(team, &request.identity).map_err(body)?;
        }
        self.team_or_err(team)
    }

    fn invite(&self, team: &TeamId) -> Result<InviteLink, ErrorBody> {
        let link = self.teams.invite(team).map_err(body)?;
        Ok(InviteLink { link: link.url, expires_at: link.expires_at })
    }

    fn add_member(&self, team: &TeamId, who: &Identity) -> Result<MemberAdded, ErrorBody> {
        self.teams.add_member(team, who.clone()).map_err(body)?;
        let detail = self.team_or_err(team)?;
        let member = detail
            .members
            .iter()
            .find(|m| m.identities.iter().any(|i| i.is(who)))
            .cloned()
            .ok_or_else(|| ErrorBody::new(code::OTHER, "added, but not in the team record"))?;
        let setup = self.setup_lines();
        let note = if setup.is_empty() {
            "They can use shares open to this team once this machine is reachable (Settings → Collaboration). With Pitwall, they see the team themselves.".into()
        } else {
            "Send them the lines below: their Claude Code then sees the shared agents as tools. With Pitwall, they see the team themselves.".into()
        };
        Ok(MemberAdded { team: detail, member, setup, note })
    }

    // ------------------------------------------------------------ people and asking

    fn people(&self) -> Vec<PersonView> {
        let mine = self.my_identities();
        let blocked = self.blocked();
        let mut out: BTreeMap<String, PersonView> = BTreeMap::new();
        for team in self.teams.teams() {
            let Some(state) = self.teams.state(&team) else { continue };
            for m in &state.members {
                if m.identities.iter().any(|i| mine.iter().any(|me| me.is(i))) {
                    continue;
                }
                let view = self.member_view(&state, m);
                let p = out.entry(view.person.clone()).or_insert_with(|| PersonView {
                    person: view.person.clone(),
                    login: view.login.clone(),
                    identities: view.identities.clone(),
                    teams: vec![],
                    devices: view.devices.clone(),
                    shares: vec![],
                    blocked: blocked.contains(&view.person),
                    claude_only: view.claude_only,
                    role: Some(if view.admin { TeamRole::Admin } else { TeamRole::Member }),
                });
                p.teams.push(team.clone());
            }
        }
        for f in self.found(false) {
            let e = f.entry;
            match out.values_mut().find(|p| p.login == e.owner || p.identities.iter().any(|i| login(i) == e.owner)) {
                Some(p) => p.shares.push(e),
                None => {
                    let id = format!("owner:{}", e.owner);
                    out.entry(id.clone())
                        .or_insert_with(|| PersonView {
                            person: id,
                            login: e.owner.clone(),
                            identities: vec![],
                            teams: vec![],
                            devices: vec![DeviceView { label: f.addr.node.clone(), network: f.addr.network.clone(), online: e.online }],
                            shares: vec![],
                            blocked: false,
                            claude_only: false,
                            role: None,
                        })
                        .shares
                        .push(e);
                }
            }
        }
        let mut list: Vec<PersonView> = out.into_values().collect();
        list.sort_by(|a, b| a.login.cmp(&b.login));
        list
    }

    fn directory(&self) -> Vec<DirectoryEntry> {
        self.found(false).into_iter().map(|f| f.entry).collect()
    }

    fn ask(&self, req: &AskSend, agent: Option<&str>) -> Result<AskView, ErrorBody> {
        let Some(found) = self.find(&req.share) else {
            return Err(ErrorBody::new(code::NOT_FOUND, format!("no shared agent {} you may use (see `pitwall people list`)", req.share)));
        };
        if !found.entry.online {
            let since = found.entry.last_seen.as_deref().map(|s| format!(" (last seen {s})")).unwrap_or_default();
            return Err(ErrorBody::new(code::OFFLINE, format!("{} is offline{since}: try again later", found.entry.owner)));
        }
        let client = self.client_for(&found.addr, agent)?;
        let conversation = if req.new {
            None
        } else {
            lock(&self.state).asks.iter().rev().find(|a| a.share == req.share).map(|a| ConversationId(a.conversation.clone()))
        };
        let msg = Outgoing { text: req.message.clone(), conversation, new_conversation: req.new, ..Outgoing::default() };
        // The stream: heartbeats keep it open however long the turn takes.
        let events = client.agent(&found.entry.slug).stream(&msg).map_err(body)?;
        let (tx, rx) = std::sync::mpsc::channel::<Turn>();
        std::thread::spawn(move || {
            for ev in events {
                let Ok(ev) = ev else { return };
                let turn = ev.task.as_ref().and_then(turn_of).or_else(|| ev.status_update.as_ref().and_then(|u| turn_in(u.metadata.as_ref())));
                if let Some(t) = turn {
                    let fin = t.status.is_final();
                    if tx.send(t).is_err() || fin {
                        return;
                    }
                }
            }
        });
        let deadline = Instant::now() + req.timeout.map_or(ASK_WAIT, Duration::from_secs);
        // Until the turn settles, the owner's Pitwall goes away, or the
        // caller's timeout (the turn goes on: `ask.get` it later).
        let mut last: Option<Turn> = None;
        while let Ok(t) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            let fin = t.status.is_final();
            last = Some(t);
            if fin {
                break;
            }
        }
        let turn = last.ok_or_else(|| ErrorBody::new(code::OFFLINE, format!("{} didn't answer", found.entry.owner)))?;
        let view = ask_view(&req.share, &req.message, &turn);
        {
            let mut st = lock(&self.state);
            st.asks.retain(|a| a.turn != view.turn);
            st.asks.push(view.clone());
            self.save_local(&st);
        }
        if let TurnStatus::Refused(r) = &turn.status {
            return Err(ErrorBody::new(refusal_code(r.reason), r.message.clone()));
        }
        Ok(view)
    }

    fn asks(&self) -> Vec<AskView> {
        self.refresh_pending();
        lock(&self.state).asks.clone()
    }
}
