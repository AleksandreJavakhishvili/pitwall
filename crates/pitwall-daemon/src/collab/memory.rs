//! A [`CollabBackend`] in memory: the test double for the permission
//! matrix, and the fallback when the core's (`super::core::CoreCollab`)
//! can't open. It keeps share settings in
//! `collab/shares/<slug>.json` and the switches in `collab/local.json`
//! when given a folder; teams, people and conversations live in memory.
//!
//! What it can't do itself it takes through seams, so the other tracks
//! plug in without touching the methods:
//! - [`AskClient`]: send a message to a teammate's shared agent (the A2A
//!   client of the fronts track);
//! - [`GuestRunner`]: one guest message through the real pipeline (copy,
//!   sandbox, session: the core and sandbox tracks);
//! - [`Network`]s: Settings → Networks rows and the setup lines.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use pitwall_core::net::{NetState, Network};
use pitwall_proto::api::collab::{
    AskSend, CollabSettings, InviteLink, MemberAdded, MemberView, NetworkRow, PersonView, ProbeResult, SetupLine, ShareUsage, TeamCreate,
    TeamDetail, TeamJoin,
};
use pitwall_proto::collab::{
    AskFrom, AskStatus, AskView, DirectoryEntry, Identity, PersonId, ShareSettings, SharedSessionView, TeamCard, TeamId, TeamRole,
};
use pitwall_proto::{code, ErrorBody};

use super::{CollabBackend, Me};

/// A test rule for [`CollabBackend::live_check`].
pub type LiveRule = Arc<dyn Fn(&ShareSettings) -> super::LiveCheck + Send + Sync>;

/// Sends one message to another Pitwall's shared agent and waits for the
/// turn (the fronts track's A2A client). Refusals and failures come back
/// as errors with the CLI's codes (`quota`, `busy`, `share_total`,
/// `agent_limit`, `loop`, `offline`, `no_answer`).
pub trait AskClient: Send + Sync {
    fn send(&self, to: &DirectoryEntry, req: &AskSend, conversation: Option<&str>, agent: Option<&str>) -> Result<AskView, ErrorBody>;
}

/// Runs one guest message against a draft through the same pipeline as a
/// real caller (copy, sandbox, clean environment): the reply, and what the
/// agent tried that was refused.
pub trait GuestRunner: Send + Sync {
    fn run(&self, share: &ShareSettings, message: &str) -> Result<(Option<String>, Vec<String>), ErrorBody>;
}

#[derive(Default)]
struct Team {
    detail: Option<TeamDetail>,
    card: Option<TeamCard>,
}

#[derive(Default)]
struct State {
    settings: CollabSettings,
    shares: BTreeMap<String, ShareSettings>,
    sessions: HashMap<String, Vec<SharedSessionView>>,
    usage: HashMap<String, ShareUsage>,
    probes: HashMap<String, Vec<ProbeResult>>,
    blocked: BTreeSet<String>,
    teams: Vec<Team>,
    directory: Vec<DirectoryEntry>,
    asks: Vec<AskView>,
    shared_agents: HashSet<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Local {
    #[serde(default)]
    settings: CollabSettings,
    #[serde(default)]
    blocked: BTreeSet<String>,
}

pub struct MemoryCollab {
    me: Me,
    dir: Option<PathBuf>,
    state: Mutex<State>,
    networks: Vec<Arc<dyn Network>>,
    ask_client: Option<Arc<dyn AskClient>>,
    guest: Option<Arc<dyn GuestRunner>>,
    live: Option<LiveRule>,
    counter: std::sync::atomic::AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> String {
    // RFC 3339 in UTC from the system clock (no date crate in the daemon).
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    rfc3339(secs)
}

/// Seconds since 1970 as `2026-10-16T09:12:00Z`.
pub fn rfc3339(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    // Civil from days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `t_9f2c…` → `9F2-C1A`.
pub fn fingerprint(team: &TeamId) -> String {
    let hex: String = team.as_str().trim_start_matches("t_").chars().filter(|c| c.is_ascii_hexdigit()).take(6).collect::<String>().to_ascii_uppercase();
    if hex.len() < 6 {
        return hex;
    }
    format!("{}-{}", &hex[..3], &hex[3..])
}

pub(crate) fn label_of(net: &Arc<dyn Network>) -> String {
    match net.id().provider() {
        "tailscale" => "Tailscale".into(),
        "cloud" => "Pitwall Cloud".into(),
        other => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

pub(crate) fn state_of(s: &NetState) -> String {
    match s {
        NetState::Off => "off".into(),
        NetState::SignedOut => "signed_out".into(),
        NetState::Connecting => "connecting".into(),
        NetState::Ready => "ready".into(),
        NetState::Error { message } => format!("error: {message}"),
    }
}

impl MemoryCollab {
    /// `dir`: where share settings and the switches are kept (`<data>/collab`);
    /// `None`: memory only.
    pub fn new(me: Me, dir: Option<PathBuf>) -> MemoryCollab {
        let mut state = State::default();
        if let Some(d) = &dir {
            if let Ok(text) = std::fs::read_to_string(d.join("local.json")) {
                if let Ok(local) = serde_json::from_str::<Local>(&text) {
                    state.settings = local.settings;
                    state.blocked = local.blocked;
                }
            }
            if let Ok(rd) = std::fs::read_dir(d.join("shares")) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) != Some("json") {
                        continue;
                    }
                    if let Some(s) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<ShareSettings>(&t).ok()) {
                        state.shares.insert(s.slug.clone(), s);
                    }
                }
            }
        }
        MemoryCollab {
            me,
            dir,
            state: Mutex::new(state),
            networks: Vec::new(),
            ask_client: None,
            guest: None,
            live: None,
            counter: std::sync::atomic::AtomicU64::new(1),
        }
    }

    pub fn with_networks(mut self, networks: Vec<Arc<dyn Network>>) -> MemoryCollab {
        self.networks = networks;
        self
    }

    pub fn with_ask_client(mut self, client: Arc<dyn AskClient>) -> MemoryCollab {
        self.ask_client = Some(client);
        self
    }

    pub fn with_guest_runner(mut self, runner: Arc<dyn GuestRunner>) -> MemoryCollab {
        self.guest = Some(runner);
        self
    }

    /// Tests: what keeps a share from going live (the core's rule in Pitwall).
    pub fn with_live_rule(mut self, rule: LiveRule) -> MemoryCollab {
        self.live = Some(rule);
        self
    }

    /// Tests and demos: a teammate's shared agent I may use.
    pub fn add_directory_entry(&self, e: DirectoryEntry) {
        lock(&self.state).directory.push(e);
    }

    /// Tests and demos: a team I'm in, or one discovered (`detail: None`).
    pub fn add_team(&self, card: TeamCard, detail: Option<TeamDetail>) {
        lock(&self.state).teams.push(Team { card: Some(card), detail });
    }

    /// Tests and demos: a caller's conversation with one of my shares.
    pub fn add_session(&self, slug: &str, s: SharedSessionView) {
        let mut st = lock(&self.state);
        st.shared_agents.insert(s.agent_id.clone());
        st.sessions.entry(slug.into()).or_default().push(s);
    }

    pub fn set_usage(&self, slug: &str, u: ShareUsage) {
        lock(&self.state).usage.insert(slug.into(), u);
    }

    /// An engine agent that is a shared session (the core sets
    /// `AgentRecord.shared`; here, tests).
    pub fn mark_shared_session(&self, agent_id: &str) {
        lock(&self.state).shared_agents.insert(agent_id.into());
    }

    fn next(&self) -> u64 {
        self.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    fn persist_local(&self, st: &State) {
        let Some(d) = &self.dir else { return };
        let local = Local { settings: st.settings.clone(), blocked: st.blocked.clone() };
        let _ = std::fs::create_dir_all(d);
        if let Ok(text) = serde_json::to_string_pretty(&local) {
            let _ = write_atomic(&d.join("local.json"), &text);
        }
    }

    fn me_member(&self) -> MemberView {
        let id = self.my_identity();
        MemberView {
            person: PersonId::of(&id).0,
            login: self.me.login.clone(),
            identities: vec![id],
            devices: self
                .networks
                .iter()
                .filter_map(|n| n.me().map(|(_, addr)| pitwall_proto::api::collab::DeviceView { label: addr.node.clone(), network: addr.network, online: true }))
                .collect(),
            admin: true,
            claude_only: false,
        }
    }

    fn my_identity(&self) -> Identity {
        self.networks
            .iter()
            .find_map(|n| n.me().map(|(i, _)| i))
            .unwrap_or_else(|| Identity::new("local", &self.me.login))
    }

    fn team_mut<'a>(st: &'a mut State, team: &TeamId) -> Result<&'a mut TeamDetail, ErrorBody> {
        st.teams
            .iter_mut()
            .find_map(|t| t.detail.as_mut().filter(|d| d.card.team == *team))
            .ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("you aren't in the team {}", team.as_str())))
    }

    fn admin_of(d: &TeamDetail) -> Result<(), ErrorBody> {
        if d.card.mine == Some(TeamRole::Admin) {
            Ok(())
        } else {
            Err(ErrorBody::new(code::DENIED, format!("only an admin of {} can do this", d.card.name)))
        }
    }
}

fn write_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

impl CollabBackend for MemoryCollab {
    fn me(&self) -> Me {
        self.me.clone()
    }

    fn is_shared_session(&self, agent_id: &str) -> bool {
        lock(&self.state).shared_agents.contains(agent_id)
    }

    fn live_check(&self, s: &ShareSettings) -> super::LiveCheck {
        self.live.as_ref().map(|rule| rule(s)).unwrap_or_default()
    }

    fn settings(&self) -> CollabSettings {
        lock(&self.state).settings.clone()
    }

    fn set_settings(&self, s: CollabSettings) -> Result<(), ErrorBody> {
        let mut st = lock(&self.state);
        st.settings = s;
        self.persist_local(&st);
        Ok(())
    }

    fn networks(&self) -> Vec<NetworkRow> {
        let reach = lock(&self.state).settings.reachable.clone();
        self.networks
            .iter()
            .map(|n| {
                let id = n.id();
                NetworkRow {
                    reachable: reach.get(id.as_str()).copied().unwrap_or(false),
                    label: label_of(n),
                    state: state_of(&n.state()),
                    me: n.me().map(|(i, addr)| format!("{} on {}", i.email.unwrap_or(i.subject), addr.node)),
                    endpoint: Some(n.endpoint().base_url),
                    id,
                }
            })
            .collect()
    }

    fn setup_lines(&self) -> Vec<SetupLine> {
        let reach = lock(&self.state).settings.reachable.clone();
        self.networks
            .iter()
            .filter(|n| reach.get(n.id().as_str()).copied().unwrap_or(false))
            .map(|n| SetupLine {
                network: n.id(),
                owner: self.me.name.clone(),
                line: format!("claude mcp add --transport http {} {}/mcp", self.me.name, n.endpoint().base_url.trim_end_matches('/')),
            })
            .collect()
    }

    fn shares(&self) -> Vec<ShareSettings> {
        lock(&self.state).shares.values().cloned().collect()
    }

    fn save_share(&self, s: &ShareSettings, previous: Option<&str>) -> Result<(), ErrorBody> {
        let mut st = lock(&self.state);
        if let Some(old) = previous {
            st.shares.remove(old);
            if let Some(d) = &self.dir {
                let _ = std::fs::remove_file(d.join("shares").join(format!("{old}.json")));
            }
        }
        st.shares.insert(s.slug.clone(), s.clone());
        if let Some(d) = &self.dir {
            let dir = d.join("shares");
            std::fs::create_dir_all(&dir).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))?;
            let text = serde_json::to_string_pretty(s).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))?;
            write_atomic(&dir.join(format!("{}.json", s.slug)), &text).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))?;
        }
        Ok(())
    }

    fn remove_share(&self, slug: &str) -> Result<(), ErrorBody> {
        let mut st = lock(&self.state);
        st.shares.remove(slug);
        if let Some(gone) = st.sessions.remove(slug) {
            for s in gone {
                st.shared_agents.remove(&s.agent_id);
            }
        }
        st.probes.remove(slug);
        if let Some(d) = &self.dir {
            let _ = std::fs::remove_file(d.join("shares").join(format!("{slug}.json")));
        }
        Ok(())
    }

    fn sessions(&self, slug: &str) -> Vec<SharedSessionView> {
        lock(&self.state).sessions.get(slug).cloned().unwrap_or_default()
    }

    fn usage(&self, slug: &str) -> ShareUsage {
        let st = lock(&self.state);
        let mut u = st.usage.get(slug).cloned().unwrap_or_default();
        u.running = st.sessions.get(slug).map_or(0, |v| v.iter().filter(|s| matches!(s.state, pitwall_proto::collab::SessionState::Working)).count() as u32);
        u
    }

    fn probes(&self, slug: &str) -> Vec<ProbeResult> {
        lock(&self.state).probes.get(slug).cloned().unwrap_or_default()
    }

    fn guest(&self, slug: &str, probe: &str, message: &str) -> Result<ProbeResult, ErrorBody> {
        let share = lock(&self.state).shares.get(slug).cloned().ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no share {slug}")))?;
        let runner = self.guest.clone().ok_or_else(|| {
            ErrorBody::new(code::UNSUPPORTED, "Guest sessions need the sandboxed session pipeline, which this Pitwall doesn't have yet")
        })?;
        let (reply, tried) = runner.run(&share, message)?;
        let r = ProbeResult { probe: probe.into(), message: message.into(), reply, tried, at: now() };
        lock(&self.state).probes.entry(slug.into()).or_default().push(r.clone());
        Ok(r)
    }

    fn blocked(&self) -> Vec<String> {
        lock(&self.state).blocked.iter().cloned().collect()
    }

    fn set_blocked(&self, person: &str, blocked: bool) -> Result<(), ErrorBody> {
        let mut st = lock(&self.state);
        if blocked {
            st.blocked.insert(person.into());
        } else {
            st.blocked.remove(person);
        }
        self.persist_local(&st);
        Ok(())
    }

    fn teams(&self) -> Vec<TeamCard> {
        let st = lock(&self.state);
        let mut mine: Vec<TeamCard> = st.teams.iter().filter_map(|t| t.detail.as_ref().map(|d| d.card.clone())).collect();
        let found = st.teams.iter().filter(|t| t.detail.is_none()).filter_map(|t| t.card.clone());
        mine.extend(found);
        mine
    }

    fn team(&self, team: &TeamId) -> Option<TeamDetail> {
        let st = lock(&self.state);
        st.teams.iter().find_map(|t| t.detail.as_ref().filter(|d| d.card.team == *team).cloned()).or_else(|| {
            st.teams.iter().find_map(|t| {
                t.card.as_ref().filter(|c| c.team == *team).map(|c| TeamDetail {
                    card: c.clone(),
                    members: vec![],
                    last_change_by: None,
                    last_change_at: None,
                    requests: vec![],
                    setup: vec![],
                    conflict: None,
                })
            })
        })
    }

    fn create_team(&self, req: &TeamCreate) -> Result<TeamDetail, ErrorBody> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (req.name.as_str(), self.me.login.as_str(), now(), self.next()).hash(&mut h);
        let mut h2 = std::collections::hash_map::DefaultHasher::new();
        (h.finish(), "genesis").hash(&mut h2);
        let id = TeamId(format!("t_{:016x}{:016x}", h.finish(), h2.finish()));
        let card = TeamCard {
            team: id.clone(),
            name: req.name.trim().into(),
            description: req.description.trim().into(),
            admins: vec![self.me.login.clone()],
            members: 1,
            version: 1,
            listed: req.listed,
            join: req.join,
            fingerprint: fingerprint(&id),
            bound: false,
            mine: Some(TeamRole::Admin),
        };
        let detail = TeamDetail {
            card,
            members: vec![self.me_member()],
            last_change_by: Some(self.me.login.clone()),
            last_change_at: Some(now()),
            requests: vec![],
            setup: vec![],
            conflict: None,
        };
        lock(&self.state).teams.push(Team { card: None, detail: Some(detail.clone()) });
        Ok(detail)
    }

    fn join_team(&self, req: &TeamJoin) -> Result<TeamCard, ErrorBody> {
        let mut st = lock(&self.state);
        let key = req.team.trim();
        let found = st.teams.iter_mut().find(|t| {
            t.detail.is_none() && t.card.as_ref().is_some_and(|c| c.team.as_str() == key || c.fingerprint == key || c.name.eq_ignore_ascii_case(key))
        });
        let Some(t) = found else {
            return Err(ErrorBody::new(
                code::OFFLINE,
                format!("Can't reach a member of {key} right now (joining by link or request needs the team's devices online)"),
            ));
        };
        let card = t.card.as_mut().expect("found by card");
        if card.join == pitwall_proto::collab::JoinPolicy::Closed {
            return Err(ErrorBody::new(code::DENIED, format!("{} is closed: ask an admin for an invite link", card.name)));
        }
        card.mine = Some(TeamRole::Requested);
        Ok(card.clone())
    }

    fn leave_team(&self, team: &TeamId) -> Result<(), ErrorBody> {
        let mut st = lock(&self.state);
        if let Some(d) = st.teams.iter().find_map(|t| t.detail.as_ref().filter(|d| d.card.team == *team)) {
            if d.card.mine == Some(TeamRole::Admin) && d.card.admins.len() == 1 && d.members.len() > 1 {
                return Err(ErrorBody::new(code::CONFLICT, format!("You are the last admin of {}: make someone else an admin first", d.card.name)));
            }
        }
        let before = st.teams.len();
        st.teams.retain(|t| t.detail.as_ref().is_none_or(|d| d.card.team != *team));
        for t in st.teams.iter_mut() {
            if let Some(c) = t.card.as_mut().filter(|c| c.team == *team) {
                c.mine = None;
            }
        }
        if st.teams.len() == before && !st.teams.iter().any(|t| t.card.as_ref().is_some_and(|c| c.team == *team)) {
            return Err(ErrorBody::new(code::NOT_FOUND, format!("you aren't in the team {}", team.as_str())));
        }
        Ok(())
    }

    fn approve(&self, team: &TeamId, person: &str, allow: bool) -> Result<TeamDetail, ErrorBody> {
        let mut st = lock(&self.state);
        let d = Self::team_mut(&mut st, team)?;
        Self::admin_of(d)?;
        let i = d
            .requests
            .iter()
            .position(|r| r.person == person || r.login.eq_ignore_ascii_case(person))
            .ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no request from {person}")))?;
        let r = d.requests.remove(i);
        if allow {
            d.members.push(MemberView { person: r.person, login: r.login, identities: vec![], devices: vec![], admin: false, claude_only: false });
            d.card.members = d.members.len() as u32;
            d.card.version += 1;
            d.last_change_by = Some(self.me.login.clone());
            d.last_change_at = Some(now());
        }
        Ok(d.clone())
    }

    fn invite(&self, team: &TeamId) -> Result<InviteLink, ErrorBody> {
        let mut st = lock(&self.state);
        let d = Self::team_mut(&mut st, team)?;
        Self::admin_of(d)?;
        let code = format!("{:012x}", {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (team.as_str(), now(), self.next()).hash(&mut h);
            h.finish()
        });
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Ok(InviteLink {
            link: format!("pitwall://team/join?t={}&v={}&by={}&c={code}", team.as_str(), d.card.version, self.me.login),
            expires_at: rfc3339(secs + 7 * 86_400),
        })
    }

    fn add_member(&self, team: &TeamId, who: &Identity) -> Result<MemberAdded, ErrorBody> {
        let setup = self.setup_lines();
        let mut st = lock(&self.state);
        let d = Self::team_mut(&mut st, team)?;
        Self::admin_of(d)?;
        let person = PersonId::of(who).0;
        if d.members.iter().any(|m| m.person == person) {
            return Err(ErrorBody::new(code::CONFLICT, format!("{} is already in {}", who.subject, d.card.name)));
        }
        let member = MemberView { person, login: who.email.clone().unwrap_or_else(|| who.subject.clone()), identities: vec![who.clone()], devices: vec![], admin: false, claude_only: true };
        d.members.push(member.clone());
        d.card.members = d.members.len() as u32;
        d.card.version += 1;
        d.last_change_by = Some(self.me.login.clone());
        d.last_change_at = Some(now());
        d.setup = setup.clone();
        let note = if setup.is_empty() {
            "They can use shares open to this team once this machine is reachable (Settings → Collaboration). With Pitwall, they see the team themselves.".into()
        } else {
            "Send them the lines below: their Claude Code then sees the shared agents as tools. With Pitwall, they see the team themselves.".into()
        };
        Ok(MemberAdded { team: d.clone(), member, setup, note })
    }

    fn people(&self) -> Vec<PersonView> {
        let st = lock(&self.state);
        let me = PersonId::of(&self.my_identity()).0;
        let mut out: BTreeMap<String, PersonView> = BTreeMap::new();
        for d in st.teams.iter().filter_map(|t| t.detail.as_ref()) {
            for m in d.members.iter().filter(|m| m.person != me) {
                let p = out.entry(m.person.clone()).or_insert_with(|| PersonView {
                    person: m.person.clone(),
                    login: m.login.clone(),
                    identities: m.identities.clone(),
                    teams: vec![],
                    devices: m.devices.clone(),
                    shares: vec![],
                    blocked: st.blocked.contains(&m.person),
                    claude_only: m.claude_only,
                    role: Some(if m.admin { TeamRole::Admin } else { TeamRole::Member }),
                });
                p.teams.push(d.card.team.clone());
            }
        }
        for e in &st.directory {
            if let Some(p) = out.values_mut().find(|p| p.login == e.owner) {
                p.shares.push(e.clone());
            } else {
                let id = PersonId(format!("owner:{}", e.owner));
                out.entry(id.0.clone()).or_insert_with(|| PersonView {
                    person: id.0.clone(),
                    login: e.owner.clone(),
                    identities: vec![],
                    teams: vec![],
                    devices: vec![],
                    shares: vec![],
                    blocked: false,
                    claude_only: false,
                    role: None,
                })
                .shares
                .push(e.clone());
            }
        }
        let mut list: Vec<PersonView> = out.into_values().collect();
        list.sort_by(|a, b| a.login.cmp(&b.login));
        list
    }

    fn directory(&self) -> Vec<DirectoryEntry> {
        lock(&self.state).directory.clone()
    }

    fn ask(&self, req: &AskSend, agent: Option<&str>) -> Result<AskView, ErrorBody> {
        let (owner, slug) = req.share.split_once('/').unwrap_or(("", req.share.as_str()));
        let entry = {
            let st = lock(&self.state);
            st.directory
                .iter()
                .find(|e| e.slug == slug && (e.owner == owner || e.owner.split('@').next() == Some(owner)))
                .cloned()
        };
        let Some(entry) = entry else {
            return Err(ErrorBody::new(code::NOT_FOUND, format!("no shared agent {} you may use (see `pitwall people list`)", req.share)));
        };
        if !entry.online {
            let since = entry.last_seen.as_deref().map(|s| format!(" (last seen {s})")).unwrap_or_default();
            return Err(ErrorBody::new(code::OFFLINE, format!("{owner} is offline{since}: try again later")));
        }
        let client = self.ask_client.clone().ok_or_else(|| {
            ErrorBody::new(code::UNSUPPORTED, "This Pitwall can't send messages to other Pitwalls yet (the A2A client isn't built in)")
        })?;
        let conversation = if req.new {
            None
        } else {
            lock(&self.state).asks.iter().rev().find(|a| a.share == req.share).map(|a| a.conversation.clone())
        };
        let view = client.send(&entry, req, conversation.as_deref(), agent)?;
        let mut st = lock(&self.state);
        st.asks.retain(|a| a.turn != view.turn);
        st.asks.push(view.clone());
        Ok(view)
    }

    fn asks(&self) -> Vec<AskView> {
        lock(&self.state).asks.clone()
    }
}

/// An ask view for a message just sent and not answered yet.
pub fn pending_ask(share: &str, conversation: &str, n: u32, text: &str) -> AskView {
    AskView {
        turn: format!("{conversation}/{n}"),
        conversation: conversation.into(),
        share: share.into(),
        text: text.into(),
        from: AskFrom::Me,
        status: AskStatus::Queued,
        status_message: None,
        reply: None,
        reply_source: None,
        changes: None,
        at: now(),
        done_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_fingerprints() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_792_142_720), "2026-10-16T09:25:20Z");
        assert_eq!(fingerprint(&TeamId("t_9f2c1a77".into())), "9F2-C1A");
    }

    #[test]
    fn shares_and_switches_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("pw-mem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let me = Me { login: "alice@example.com".into(), name: "alice".into() };
        let m = MemoryCollab::new(me.clone(), Some(dir.clone()));
        let s: ShareSettings = serde_json::from_value(serde_json::json!({
            "agent": "a1", "slug": "docs", "name": "Docs", "description": "", "scope": {"repo": "~/d", "ref": "main"}
        }))
        .unwrap();
        m.save_share(&s, None).unwrap();
        m.set_settings(CollabSettings { use_teams: true, ..Default::default() }).unwrap();
        m.set_blocked("tailscale:eve@example.com", true).unwrap();
        let again = MemoryCollab::new(me, Some(dir.clone()));
        assert_eq!(again.shares(), vec![s]);
        assert!(again.settings().use_teams);
        assert_eq!(again.blocked(), ["tailscale:eve@example.com"]);
        again.remove_share("docs").unwrap();
        assert!(!dir.join("shares/docs.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
