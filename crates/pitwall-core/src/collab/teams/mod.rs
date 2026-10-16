//! Teams (§4): chains of signed records, kept by every member device and
//! replicated by pull (§4.3). [`TeamStore`] is this Pitwall's side of it:
//!
//! - the [`Teams`] service the `/pitwall/v1/…` endpoints call (info,
//!   discover, heads, records, announce, join, invite, requests);
//! - the owner's own actions as admin or member (create, add a member by
//!   identity, approve a join request, invite, remove, leave, set info);
//! - the client side: probing other Pitwalls for listed teams, asking to
//!   join, redeeming an invite and pulling records ([`sync`]).
//!
//! C1a: one admin per team, records already signed, chained and validated.
//! Several admins writing at once (the fork choice, §4.4), announce/push and
//! re-applying dropped ops are C1b: a conflicting record is refused at
//! [`chain::Accept::Conflict`], the seam where the fork choice goes.

pub mod chain;
pub mod keys;
pub mod sync;

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};

use serde::{Deserialize, Serialize};

use self::chain::{record_hash, Accept, Chain};
use self::keys::{key_fingerprint, sha256_hex, RecordSigner, SignatureCheck};
use super::audit::{AuditLine, AuditLog};
use super::store::{append_jsonl, read_json, read_jsonl, valid_id, write_json, CollabPaths};
use super::team::{
    DeviceKey, InviteRedeem, JoinPolicy, JoinRequest, Member, MemberId, PendingJoin, PitwallInfo, Signer, TeamDevice,
    TeamHead, TeamId, TeamOp, TeamRecord, TeamState,
};
use super::time::{self, WallClock};
use super::Teams;
use crate::error::{ErrorCode, PwError, Result};
use crate::net::{Caller, DeviceAddr, Identity, Network};
use pitwall_proto::collab::{TeamCard, TeamRole};

/// Join requests and invite codes are valid this long (§4.6).
pub const REQUEST_DAYS: i64 = 7;
/// Open join requests per person per team (§4.6).
pub const MAX_OPEN_REQUESTS: usize = 3;

fn not_found() -> PwError {
    PwError::not_found("no such team")
}

/// An invite code, kept as its hash (admin only, `invites.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    /// `sha256` of the code.
    pub hash: String,
    pub expires_at: String,
    #[serde(default)]
    pub used: bool,
}

/// What an admin hands out (§4.5): the link carries the trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    /// `pitwall://team/join?t=<team>&v=<version>&by=<admin>&k=<key fp>&c=<code>`.
    pub url: String,
    pub code: String,
    pub expires_at: String,
}

/// An invite link, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInvite {
    pub team: TeamId,
    pub version: u64,
    /// The admin who made it (`provider:subject`).
    pub by: String,
    pub key_fingerprint: String,
    pub code: String,
}

impl ParsedInvite {
    pub fn parse(url: &str) -> Option<ParsedInvite> {
        let query = url.strip_prefix("pitwall://team/join?")?;
        let mut fields = BTreeMap::new();
        for pair in query.split('&') {
            let (k, v) = pair.split_once('=')?;
            fields.insert(k, decode(v));
        }
        Some(ParsedInvite {
            team: TeamId(fields.get("t")?.clone()),
            version: fields.get("v")?.parse().ok()?,
            by: fields.get("by")?.clone(),
            key_fingerprint: fields.get("k")?.clone(),
            code: fields.get("c")?.clone(),
        })
    }
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'@' | b':' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A team seen on another Pitwall (`teams:discover`), cached in
/// `discovered.json`: its latest listed record and where it was seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovered {
    pub record: TeamRecord,
    pub seen_at: Vec<DeviceAddr>,
    pub last_seen: String,
}

/// A team this Pitwall asked to join and isn't in yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Joining {
    pub team: TeamId,
    /// Where to pull its records from once accepted.
    pub from: DeviceAddr,
    pub at: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Local {
    #[serde(default)]
    discovered: BTreeMap<TeamId, Discovered>,
    #[serde(default)]
    joining: BTreeMap<TeamId, Joining>,
}

#[derive(Default)]
struct Inner {
    chains: BTreeMap<TeamId, Chain>,
    requests: BTreeMap<TeamId, Vec<PendingJoin>>,
    invites: BTreeMap<TeamId, Vec<Invite>>,
    local: Local,
}

/// What a [`TeamStore`] is built from.
pub struct TeamsDeps {
    pub paths: CollabPaths,
    pub signer: Arc<dyn RecordSigner>,
    pub check: Arc<dyn SignatureCheck>,
    pub clock: Arc<dyn WallClock>,
    pub audit: Arc<AuditLog>,
    /// Pitwall's version, for `/info`.
    pub version: String,
    /// This device's name (`studio`), for `/info`.
    pub label: String,
}

/// This Pitwall's teams.
pub struct TeamStore {
    paths: CollabPaths,
    signer: Arc<dyn RecordSigner>,
    check: Arc<dyn SignatureCheck>,
    clock: Arc<dyn WallClock>,
    audit: Arc<AuditLog>,
    version: String,
    label: String,
    networks: RwLock<Vec<Arc<dyn Network>>>,
    inner: Mutex<Inner>,
    subscribers: Mutex<Vec<Sender<()>>>,
    /// Itself, for pulls in the background.
    this: Weak<TeamStore>,
}

impl TeamStore {
    /// Load every stored chain (each re-validated; a damaged one is skipped
    /// and reported), pending requests, invites and the discovery cache.
    pub fn open(deps: TeamsDeps) -> Arc<TeamStore> {
        let TeamsDeps { paths, signer, check, clock, audit, version, label } = deps;
        let mut inner = Inner::default();
        if let Ok(entries) = std::fs::read_dir(paths.teams_dir()) {
            for entry in entries.flatten() {
                let dir = entry.path();
                let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(String::from) else { continue };
                let records: Vec<TeamRecord> = read_jsonl(&dir.join("chain.jsonl"));
                if records.is_empty() {
                    continue;
                }
                match Chain::from_records(records, &*check) {
                    Ok(chain) if chain.id.0 == name => {
                        let id = chain.id.clone();
                        inner.requests.insert(id.clone(), read_json(&dir.join("requests.json")).ok().flatten().unwrap_or_default());
                        inner.invites.insert(id.clone(), read_json(&dir.join("invites.json")).ok().flatten().unwrap_or_default());
                        inner.chains.insert(id, chain);
                    }
                    Ok(_) => eprintln!("pitwall: team {name}: the chain names another team; skipped"),
                    Err(e) => eprintln!("pitwall: team {name}: {e}; skipped"),
                }
            }
        }
        inner.local = read_json(&paths.discovered_file()).ok().flatten().unwrap_or_default();
        Arc::new_cyclic(|this| TeamStore {
            this: this.clone(),
            paths,
            signer,
            check,
            clock,
            audit,
            version,
            label,
            networks: RwLock::default(),
            inner: Mutex::new(inner),
            subscribers: Mutex::default(),
        })
    }

    /// The networks this Pitwall is on (for `/info`, discovery and pulls).
    pub fn set_networks(&self, networks: Vec<Arc<dyn Network>>) {
        *self.networks.write().unwrap_or_else(|e| e.into_inner()) = networks;
    }

    pub(crate) fn networks(&self) -> Vec<Arc<dyn Network>> {
        self.networks.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn arc(&self) -> Option<Arc<TeamStore>> {
        self.this.upgrade()
    }

    /// A tick whenever membership changes (records accepted, requests
    /// arrive). Audiences naming teams depend on it.
    pub fn changes(&self) -> Receiver<()> {
        let (tx, rx) = mpsc::channel();
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner()).push(tx);
        rx
    }

    fn notify(&self) {
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner()).retain(|tx| tx.send(()).is_ok());
    }

    fn now(&self) -> String {
        time::rfc3339(self.clock.now())
    }

    fn audit(&self, method: &str, team: &TeamId, caller: Option<&Caller>, outcome: &str) {
        let mut line = AuditLine::new(self.now(), method, outcome);
        line.agent = Some(team.0.clone());
        if let Some(c) = caller {
            line.peer = c.network().0.clone();
            line.login = c.identity().key();
            line.node = c.device().map(|d| d.node.clone());
        }
        self.audit.write(&line);
    }

    /// This device's identities and addresses on every network it is on.
    fn me(&self) -> (Vec<Identity>, Vec<DeviceAddr>) {
        let mut ids: Vec<Identity> = Vec::new();
        let mut addrs = Vec::new();
        for n in self.networks() {
            if let Some((id, addr)) = n.me() {
                if !ids.iter().any(|i| i.is(&id)) {
                    ids.push(id);
                }
                addrs.push(addr);
            }
        }
        (ids, addrs)
    }

    pub(crate) fn my_device(&self) -> TeamDevice {
        TeamDevice { key: self.signer.key(), addr: self.me().1 }
    }

    /// My member entry in a team, by this device's key.
    fn my_member(&self, state: &TeamState) -> Option<MemberId> {
        chain::has_device(state, &self.signer.key())
    }

    fn save_chain_record(&self, team: &TeamId, record: &TeamRecord) -> Result<()> {
        append_jsonl(&self.paths.team_dir(&team.0).join("chain.jsonl"), record)
    }

    fn save_local(&self, inner: &Inner) {
        if let Err(e) = write_json(&self.paths.discovered_file(), &inner.local) {
            eprintln!("pitwall: {e}");
        }
    }

    fn save_requests(&self, team: &TeamId, inner: &Inner) {
        let list = inner.requests.get(team).cloned().unwrap_or_default();
        if let Err(e) = write_json(&self.paths.team_dir(&team.0).join("requests.json"), &list) {
            eprintln!("pitwall: {e}");
        }
    }

    fn save_invites(&self, team: &TeamId, inner: &Inner) {
        let list = inner.invites.get(team).cloned().unwrap_or_default();
        if let Err(e) = write_json(&self.paths.team_dir(&team.0).join("invites.json"), &list) {
            eprintln!("pitwall: {e}");
        }
    }

    // ------------------------------------------------------------ reads

    /// The current state of a team I'm in.
    pub fn state(&self, team: &TeamId) -> Option<TeamState> {
        self.inner().chains.get(team).map(|c| c.state().clone())
    }

    /// A team's whole chain, genesis first.
    pub fn chain(&self, team: &TeamId) -> Option<Vec<TeamRecord>> {
        self.inner().chains.get(team).map(|c| c.records.clone())
    }

    pub fn head(&self, team: &TeamId) -> Option<TeamHead> {
        self.inner().chains.get(team).map(head_of)
    }

    pub fn teams(&self) -> Vec<TeamId> {
        self.inner().chains.keys().cloned().collect()
    }

    /// Whether someone with `identities` (one person, linked) and `groups`
    /// is a member of `team` (§4.2; a bound team: its provider group, §4.8).
    pub fn is_member(&self, team: &TeamId, identities: &[Identity], groups: &[crate::net::GroupRef]) -> bool {
        let inner = self.inner();
        let Some(chain) = inner.chains.get(team) else { return false };
        let state = chain.state();
        match &state.binding {
            Some(b) => groups.iter().any(|g| g.network == b.network && g.id == b.group),
            None => identities.iter().any(|i| state.member_with(i).is_some()),
        }
    }

    /// Identities team records link to `identity` (§15.7): every identity of
    /// every member entry that lists it, across my teams.
    pub fn linked(&self, identity: &Identity) -> Vec<Identity> {
        let inner = self.inner();
        let mut out: Vec<Identity> = Vec::new();
        for chain in inner.chains.values() {
            if let Some(m) = chain.state().member_with(identity) {
                for i in &m.identities {
                    if !out.iter().any(|o| o.is(i)) {
                        out.push(i.clone());
                    }
                }
            }
        }
        out
    }

    /// Every identity group in my teams' records with more than one
    /// identity: the links team records make (§15.7).
    pub fn identity_links(&self) -> Vec<Vec<Identity>> {
        let inner = self.inner();
        inner
            .chains
            .values()
            .flat_map(|c| c.state().members.iter().filter(|m| m.identities.len() > 1).map(|m| m.identities.clone()))
            .collect()
    }

    fn card(&self, team: &TeamId, state: &TeamState, version: u64, mine: Option<TeamRole>) -> TeamCard {
        let admins = state
            .admins
            .iter()
            .filter_map(|a| state.member(a))
            .map(|m| m.identities.first().map_or(m.member.0.clone(), |i| i.email.clone().unwrap_or_else(|| i.subject.clone())))
            .collect();
        TeamCard {
            team: team.clone(),
            name: state.name.clone(),
            description: state.description.clone(),
            admins,
            members: state.members.len() as u32,
            version,
            listed: state.listed,
            join: state.join,
            fingerprint: chain::fingerprint(team),
            bound: state.binding.is_some(),
            mine,
        }
    }

    /// People → Teams: mine, the ones I asked to join, and listed teams seen
    /// elsewhere (`mine: None`).
    pub fn cards(&self) -> Vec<TeamCard> {
        let inner = self.inner();
        let mut out = Vec::new();
        for (id, chain) in &inner.chains {
            let state = chain.state();
            let mine = self.my_member(state).map(|m| if state.is_admin(&m) { TeamRole::Admin } else { TeamRole::Member });
            out.push(self.card(id, state, chain.version(), mine));
        }
        for (id, d) in &inner.local.discovered {
            if inner.chains.contains_key(id) {
                continue;
            }
            let mine = inner.local.joining.contains_key(id).then_some(TeamRole::Requested);
            out.push(self.card(id, &d.record.state, d.record.version, mine));
        }
        out
    }

    /// Pending join requests of a team I administer (for the owner's UI).
    pub fn requests(&self, team: &TeamId) -> Vec<PendingJoin> {
        let now = self.clock.now();
        self.inner()
            .requests
            .get(team)
            .map(|l| l.iter().filter(|r| !expired(&r.at, now)).cloned().collect())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------ writing records

    fn member_id_for(state: Option<&TeamState>, identity: &Identity) -> MemberId {
        let base = format!("p_{}", &sha256_hex(identity.key().as_bytes())[..10]);
        let taken = |id: &str| state.is_some_and(|s| s.member(&MemberId(id.into())).is_some() || s.removed.iter().any(|r| r.member.0 == id));
        if !taken(&base) {
            return MemberId(base);
        }
        (2..).map(|n| format!("{base}{n}")).find(|c| !taken(c)).map(MemberId).unwrap_or(MemberId(base))
    }

    /// Create a team (§4.5): its genesis record, with me as its only admin
    /// and member. Needs this device signed in to at least one network.
    pub fn create(&self, name: &str, description: &str, listed: bool, join: JoinPolicy) -> Result<TeamId> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(PwError::other("a team name is 1 to 80 characters"));
        }
        let (identities, addrs) = self.me();
        let first = identities.first().ok_or_else(|| PwError::unsupported("sign in to a network first"))?;
        let me = Self::member_id_for(None, first);
        let state = TeamState {
            name: name.into(),
            description: description.trim().into(),
            listed,
            join,
            admins: vec![me.clone()],
            members: vec![Member { member: me.clone(), identities, devices: vec![TeamDevice { key: self.signer.key(), addr: addrs }] }],
            removed: vec![],
            binding: None,
        };
        let mut genesis = TeamRecord {
            team: None,
            version: 1,
            prev: None,
            ops: vec![],
            state,
            by: Signer { member: me, key: self.signer.key() },
            at: self.now(),
            sig: String::new(),
            cosig: vec![],
        };
        genesis.sig = self.signer.sign(&chain::signing_bytes(&genesis));
        let chain = Chain::found(genesis.clone(), &*self.check)?;
        let id = chain.id.clone();
        self.save_chain_record(&id, &genesis)?;
        self.inner().chains.insert(id.clone(), chain);
        self.audit("team.create", &id, None, "ok");
        self.notify();
        Ok(id)
    }

    /// Write, sign, check and store the next record of `team` with `ops`.
    fn write(&self, team: &TeamId, ops: Vec<TeamOp>) -> Result<TeamRecord> {
        let mut inner = self.inner();
        let chain = inner.chains.get_mut(team).ok_or_else(not_found)?;
        let me = self.my_member(chain.state()).ok_or_else(|| PwError::new(ErrorCode::Denied, "this device isn't in the team"))?;
        let head = chain.head().clone();
        let state = chain::apply(&head.state, &ops, head.version + 1)?;
        let mut record = TeamRecord {
            team: Some(team.clone()),
            version: head.version + 1,
            prev: Some(record_hash(&head)),
            ops,
            state,
            by: Signer { member: me, key: self.signer.key() },
            at: self.now(),
            sig: String::new(),
            cosig: vec![],
        };
        record.sig = self.signer.sign(&chain::signing_bytes(&record));
        match chain.offer(record.clone(), &*self.check)? {
            Accept::Added => {}
            other => return Err(PwError::other(format!("couldn't add the record ({other:?})"))),
        }
        drop(inner);
        self.save_chain_record(team, &record)?;
        self.notify();
        Ok(record)
    }

    /// Add someone by identity (§4.7): works for people without Pitwall
    /// (no devices). Admins only.
    pub fn add_member(&self, team: &TeamId, identity: Identity) -> Result<TeamRecord> {
        let state = self.state(team).ok_or_else(not_found)?;
        let member = Self::member_id_for(Some(&state), &identity);
        let record = self.write(team, vec![TeamOp::AddMember { member, identities: vec![identity.clone()], devices: vec![] }])?;
        self.audit("team.add_member", team, None, &format!("ok:{}", identity.key()));
        Ok(record)
    }

    /// Approve a join request (§4.6): `add_member` with the identity the
    /// network proved and the device the request came from.
    pub fn approve(&self, team: &TeamId, identity: &Identity) -> Result<TeamRecord> {
        let request = self
            .requests(team)
            .into_iter()
            .find(|r| r.identity.is(identity))
            .ok_or_else(|| PwError::not_found("no such join request"))?;
        let state = self.state(team).ok_or_else(not_found)?;
        let member = Self::member_id_for(Some(&state), identity);
        let record = self.write(
            team,
            vec![TeamOp::AddMember { member, identities: vec![request.identity.clone()], devices: vec![request.device.clone()] }],
        )?;
        let mut inner = self.inner();
        if let Some(list) = inner.requests.get_mut(team) {
            list.retain(|r| !r.identity.is(identity));
        }
        self.save_requests(team, &inner);
        drop(inner);
        self.audit("team.approve", team, None, &format!("ok:{}", identity.key()));
        Ok(record)
    }

    pub fn deny(&self, team: &TeamId, identity: &Identity) -> Result<()> {
        let mut inner = self.inner();
        let list = inner.requests.get_mut(team).ok_or_else(not_found)?;
        let before = list.len();
        list.retain(|r| !r.identity.is(identity));
        if list.len() == before {
            return Err(PwError::not_found("no such join request"));
        }
        self.save_requests(team, &inner);
        drop(inner);
        self.audit("team.deny", team, None, &format!("ok:{}", identity.key()));
        Ok(())
    }

    /// Remove a member (admins), or leave (my own entry).
    pub fn remove_member(&self, team: &TeamId, member: &MemberId) -> Result<TeamRecord> {
        let record = self.write(team, vec![TeamOp::RemoveMember { member: member.clone() }])?;
        self.audit("team.remove_member", team, None, &format!("ok:{}", member.0));
        Ok(record)
    }

    /// Leave a team: my own `remove_member`.
    pub fn leave(&self, team: &TeamId) -> Result<()> {
        let state = self.state(team).ok_or_else(not_found)?;
        let me = self.my_member(&state).ok_or_else(not_found)?;
        self.remove_member(team, &me)?;
        Ok(())
    }

    /// Link another identity to a member (§15.7): `add_identity`, as admin.
    pub fn add_identity(&self, team: &TeamId, member: &MemberId, identity: Identity) -> Result<TeamRecord> {
        self.write(team, vec![TeamOp::AddIdentity { member: member.clone(), identity }])
    }

    pub fn set_info(&self, team: &TeamId, name: Option<String>, description: Option<String>, listed: Option<bool>) -> Result<TeamRecord> {
        self.write(team, vec![TeamOp::SetInfo { name, description, listed }])
    }

    pub fn set_join(&self, team: &TeamId, join: JoinPolicy) -> Result<TeamRecord> {
        self.write(team, vec![TeamOp::SetJoin { join }])
    }

    /// A one-time invite link (§4.5, §4.6), valid 7 days. Admins only.
    pub fn invite(&self, team: &TeamId) -> Result<InviteLink> {
        let state = self.state(team).ok_or_else(not_found)?;
        let me = self.my_member(&state).filter(|m| state.is_admin(m)).ok_or_else(|| PwError::new(ErrorCode::Denied, "only admins invite"))?;
        let by = state.member(&me).and_then(|m| m.identities.first()).map(Identity::key).unwrap_or_default();
        let code = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let expires_at = time::rfc3339(self.clock.now() + chrono::Duration::days(REQUEST_DAYS));
        let mut inner = self.inner();
        let version = inner.chains.get(team).map_or(1, Chain::version);
        inner.invites.entry(team.clone()).or_default().push(Invite { hash: sha256_hex(code.as_bytes()), expires_at: expires_at.clone(), used: false });
        self.save_invites(team, &inner);
        drop(inner);
        let url = format!(
            "pitwall://team/join?t={}&v={version}&by={}&k={}&c={code}",
            encode(&team.0),
            encode(&by),
            key_fingerprint(&self.signer.key())
        );
        self.audit("team.invite", team, None, "ok");
        Ok(InviteLink { url, code, expires_at })
    }

    // ------------------------------------------------------------ records from others

    /// Offer records of `team` (replication, or the chain a joiner pulls).
    /// A team I don't hold yet is founded from its genesis when `pinned`
    /// names it (§4.5: the id is pinned from discovery or the invite link).
    /// Returns how many were added.
    pub fn accept(&self, team: &TeamId, records: Vec<TeamRecord>, pinned: bool) -> Result<usize> {
        let mut inner = self.inner();
        let mut added = Vec::new();
        let mut records = records;
        records.sort_by_key(|r| r.version);
        if !inner.chains.contains_key(team) {
            if !pinned {
                return Err(not_found());
            }
            let mut it = records.into_iter();
            let genesis = it.next().ok_or_else(|| PwError::other("no records"))?;
            let chain = Chain::found(genesis.clone(), &*self.check)?;
            if &chain.id != team {
                return Err(PwError::new(ErrorCode::Denied, "those records found another team"));
            }
            added.push(genesis);
            inner.chains.insert(team.clone(), chain);
            records = it.collect();
        }
        let chain = inner.chains.get_mut(team).expect("just ensured");
        for record in records {
            match chain.offer(record.clone(), &*self.check)? {
                Accept::Added => added.push(record),
                Accept::Known => {}
                Accept::Conflict => return Err(PwError::new(ErrorCode::Conflict, "a conflicting record (several admins arrive in C1b)")),
                Accept::Gap => return Err(PwError::other("records are missing in between")),
            }
        }
        if !added.is_empty() && inner.local.joining.remove(team).is_some() {
            self.save_local(&inner);
        }
        drop(inner);
        for r in &added {
            self.save_chain_record(team, r)?;
        }
        if !added.is_empty() {
            self.notify();
        }
        Ok(added.len())
    }

    /// Whether `caller` is a member device of `team` (§8.4 "member devices"):
    /// one of a member's identities, from one of that member's devices.
    fn member_device(&self, team: &TeamId, caller: &Caller) -> bool {
        let inner = self.inner();
        let Some(chain) = inner.chains.get(team) else { return false };
        let Some(m) = chain.state().member_with(caller.identity()) else { return false };
        let Some(dev) = caller.device() else { return false };
        m.devices.iter().any(|d| d.addr.contains(dev))
    }

    fn is_admin_caller(&self, team: &TeamId, caller: &Caller) -> bool {
        let inner = self.inner();
        let Some(chain) = inner.chains.get(team) else { return false };
        let state = chain.state();
        state.member_with(caller.identity()).is_some_and(|m| state.is_admin(&m.member))
    }

    fn am_admin(&self, team: &TeamId) -> bool {
        let inner = self.inner();
        inner.chains.get(team).is_some_and(|c| self.my_member(c.state()).is_some_and(|m| c.state().is_admin(&m)))
    }

    /// Remember a team I asked to join, to pull it once accepted.
    pub(crate) fn note_joining(&self, team: &TeamId, from: DeviceAddr, record: Option<TeamRecord>) {
        let mut inner = self.inner();
        let at = self.now();
        inner.local.joining.insert(team.clone(), Joining { team: team.clone(), from: from.clone(), at: at.clone() });
        if let Some(record) = record {
            inner.local.discovered.entry(team.clone()).or_insert(Discovered { record, seen_at: vec![from], last_seen: at });
        }
        self.save_local(&inner);
    }

    pub(crate) fn joining(&self) -> Vec<Joining> {
        self.inner().local.joining.values().cloned().collect()
    }

    /// Cache listed teams seen on another Pitwall (§3); the highest valid
    /// version wins.
    pub(crate) fn note_discovered(&self, from: &DeviceAddr, records: Vec<TeamRecord>) -> usize {
        let mut inner = self.inner();
        let now = self.now();
        let mut n = 0;
        for record in records {
            let Some(team) = record.team.clone().or_else(|| record.is_genesis().then(|| chain::team_id(&record))) else { continue };
            if !record.state.listed || sync::check_listed(&record, &*self.check).is_err() {
                continue;
            }
            n += 1;
            let entry = inner.local.discovered.entry(team).or_insert_with(|| Discovered { record: record.clone(), seen_at: vec![], last_seen: now.clone() });
            if record.version > entry.record.version {
                entry.record = record;
            }
            if !entry.seen_at.contains(from) {
                entry.seen_at.push(from.clone());
            }
            entry.last_seen = now.clone();
        }
        self.save_local(&inner);
        n
    }

    pub fn discovered(&self) -> Vec<Discovered> {
        self.inner().local.discovered.values().cloned().collect()
    }
}

fn head_of(chain: &Chain) -> TeamHead {
    TeamHead { team: chain.id.clone(), version: chain.version(), hash: record_hash(chain.head()) }
}

fn expired(at: &str, now: chrono::DateTime<chrono::FixedOffset>) -> bool {
    time::parse(at).is_none_or(|t| now - t > chrono::Duration::days(REQUEST_DAYS))
}

impl Teams for TeamStore {
    fn info(&self, caller: &Caller) -> PitwallInfo {
        let networks = self.networks();
        let person = networks
            .iter()
            .filter(|n| &n.id() == caller.network())
            .chain(networks.iter())
            .find_map(|n| n.me().map(|(i, _)| i))
            .unwrap_or_else(|| Identity::new("unknown", ""));
        let inner = self.inner();
        let listed_teams = inner
            .chains
            .iter()
            .filter(|(_, c)| c.state().listed && self.my_member(c.state()).is_some())
            .map(|(id, _)| id.clone())
            .collect();
        PitwallInfo { version: self.version.clone(), person, label: self.label.clone(), listed_teams, a2a: true, mcp: "/mcp".into() }
    }

    fn listed(&self, _caller: &Caller) -> Vec<TeamRecord> {
        let inner = self.inner();
        inner
            .chains
            .values()
            .filter(|c| c.state().listed && self.my_member(c.state()).is_some())
            .map(|c| c.head().clone())
            .collect()
    }

    fn heads(&self, caller: &Caller, theirs: &[TeamHead]) -> Result<Vec<TeamHead>> {
        let teams = self.teams();
        let ours: Vec<TeamHead> = teams.iter().filter(|t| self.member_device(t, caller)).filter_map(|t| self.head(t)).collect();
        // The side behind pulls (§4.3): when theirs is ahead, from them.
        let behind: Vec<TeamId> = theirs
            .iter()
            .filter(|h| ours.iter().any(|o| o.team == h.team && o.version < h.version))
            .map(|h| h.team.clone())
            .collect();
        if let (false, Some(from)) = (behind.is_empty(), caller.device().cloned()) {
            sync::pull_later(self, from, behind);
        }
        Ok(ours)
    }

    fn records(&self, caller: &Caller, team: &TeamId, after: u64) -> Result<Vec<TeamRecord>> {
        if !self.member_device(team, caller) {
            return Err(not_found());
        }
        Ok(self.inner().chains.get(team).map(|c| c.after(after)).unwrap_or_default())
    }

    fn announce(&self, caller: &Caller, head: TeamHead) -> Result<()> {
        if !self.member_device(&head.team, caller) {
            return Err(not_found());
        }
        let ours = self.head(&head.team).map_or(0, |h| h.version);
        if head.version > ours {
            if let Some(from) = caller.device().cloned() {
                sync::pull_later(self, from, vec![head.team]);
            }
        }
        Ok(())
    }

    fn request_join(&self, caller: &Caller, team: &TeamId, request: JoinRequest) -> Result<()> {
        let state = self.state(team).ok_or_else(not_found)?;
        if self.my_member(&state).is_none() {
            return Err(not_found());
        }
        if state.join != JoinPolicy::Request || state.binding.is_some() {
            self.audit("team.join", team, Some(caller), "refused:closed");
            return Err(PwError::new(ErrorCode::Denied, "this team doesn't take join requests"));
        }
        if state.member_with(caller.identity()).is_some() {
            return Err(PwError::new(ErrorCode::Conflict, "already a member"));
        }
        let from = caller.device().cloned().ok_or_else(|| PwError::new(ErrorCode::Denied, "join requests come from a Pitwall device"))?;
        if state.members.iter().any(|m| m.devices.iter().any(|d| d.key == request.device.key)) {
            return Err(PwError::new(ErrorCode::Conflict, "that device is already in the team"));
        }
        let now = self.clock.now();
        let mut inner = self.inner();
        let list = inner.requests.entry(team.clone()).or_default();
        list.retain(|r| !expired(&r.at, now));
        if list.iter().filter(|r| r.identity.is(caller.identity())).count() >= MAX_OPEN_REQUESTS {
            drop(inner);
            self.audit("team.join", team, Some(caller), "refused:too_many");
            return Err(PwError::new(ErrorCode::Conflict, "at most 3 open requests per person"));
        }
        // The key from the request, bound to the address it came from (§4.6).
        let device = TeamDevice { key: request.device.key, addr: vec![from] };
        list.retain(|r| !r.identity.is(caller.identity()) || r.device.key != device.key);
        list.push(PendingJoin { team: team.clone(), identity: caller.identity().clone(), device, note: request.note.chars().take(500).collect(), at: time::rfc3339(now) });
        self.save_requests(team, &inner);
        drop(inner);
        self.audit("team.join", team, Some(caller), "ok:requested");
        self.notify();
        Ok(())
    }

    fn redeem_invite(&self, caller: &Caller, team: &TeamId, redeem: InviteRedeem) -> Result<()> {
        if !self.am_admin(team) {
            return Err(not_found());
        }
        let from = caller.device().cloned().ok_or_else(not_found)?;
        let hash = sha256_hex(redeem.code.as_bytes());
        let now = self.clock.now();
        {
            let mut inner = self.inner();
            let invite = inner
                .invites
                .get_mut(team)
                .and_then(|l| l.iter_mut().find(|i| i.hash == hash && !i.used && time::parse(&i.expires_at).is_some_and(|e| e > now)));
            let Some(invite) = invite else {
                drop(inner);
                self.audit("team.redeem", team, Some(caller), "refused:bad_code");
                return Err(not_found());
            };
            invite.used = true;
            self.save_invites(team, &inner);
        }
        let state = self.state(team).ok_or_else(not_found)?;
        if state.member_with(caller.identity()).is_some() {
            return Ok(());
        }
        let member = Self::member_id_for(Some(&state), caller.identity());
        let device = TeamDevice { key: redeem.device.key, addr: vec![from] };
        self.write(team, vec![TeamOp::AddMember { member, identities: vec![caller.identity().clone()], devices: vec![device] }])?;
        self.audit("team.redeem", team, Some(caller), "ok");
        Ok(())
    }

    fn join_requests(&self, caller: &Caller, team: &TeamId) -> Result<Vec<PendingJoin>> {
        if !self.is_admin_caller(team, caller) {
            return Err(not_found());
        }
        Ok(self.requests(team))
    }
}

/// `t_…` ids are made by hashing; anything else isn't a folder name.
pub fn valid_team_id(team: &TeamId) -> bool {
    team.0.starts_with("t_") && valid_id(&team.0)
}

/// A device key's member in a team's state.
pub fn member_of_key(state: &TeamState, key: &DeviceKey) -> Option<MemberId> {
    chain::has_device(state, key)
}

#[cfg(test)]
mod tests;
