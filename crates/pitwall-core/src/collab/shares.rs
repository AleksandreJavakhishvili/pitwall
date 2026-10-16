//! Share settings on the owner's machine (§5.4–5.7): `collab/shares/<slug>.json`,
//! one per shared agent. A share is created as a **draft** (visible only to
//! the owner and their guest sessions), goes live, pauses, resumes, and
//! stops (the file goes; the agent stays as it was, §5.1).
//!
//! Every change goes through [`diff`]: the exact lines that changed, each
//! marked `widens` per §5.5. Widening always asks the owner and can't be
//! remembered; narrowing may apply directly. Who asks is the daemon's
//! approval mechanism; this module only says which is which.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

use super::store::{read_json, valid_slug, write_json, CollabPaths};
use crate::error::{ErrorCode, PwError, Result};
use pitwall_proto::collab::{
    Capability, LimitOverride, Limits, Programs, ShareDiff, ShareDiffLine, ShareSettings, ShareState, Start,
};

/// The owner's shares, by slug.
pub struct ShareStore {
    paths: CollabPaths,
    shares: Mutex<BTreeMap<String, ShareSettings>>,
}

fn not_found(slug: &str) -> PwError {
    PwError::not_found(format!("no share {slug}"))
}

impl ShareStore {
    /// Load every `shares/*.json` (a damaged file is skipped and reported).
    pub fn open(paths: CollabPaths) -> ShareStore {
        let mut shares = BTreeMap::new();
        if let Ok(entries) = std::fs::read_dir(paths.shares_dir()) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                match read_json::<ShareSettings>(&path) {
                    Ok(Some(s)) if valid_slug(&s.slug) => {
                        shares.insert(s.slug.clone(), s);
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("pitwall: skipping share {}: {e}", path.display()),
                }
            }
        }
        ShareStore { paths, shares: Mutex::new(shares) }
    }

    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, ShareSettings>> {
        self.shares.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, share: &ShareSettings) -> Result<()> {
        write_json(&self.paths.share_file(&share.slug), share)
    }

    pub fn get(&self, slug: &str) -> Option<ShareSettings> {
        self.map().get(slug).cloned()
    }

    pub fn list(&self) -> Vec<ShareSettings> {
        self.map().values().cloned().collect()
    }

    /// The share of ordinary agent `agent`, if it is shared.
    pub fn of_agent(&self, agent: &str) -> Option<ShareSettings> {
        self.map().values().find(|s| s.agent == agent).cloned()
    }

    /// `share create` (§5.5): a draft with the given answers. Slugs are
    /// unique per owner; an agent is shared at most once.
    pub fn create(&self, mut share: ShareSettings) -> Result<ShareSettings> {
        check(&share)?;
        share.state = ShareState::Draft;
        let mut map = self.map();
        if map.contains_key(&share.slug) {
            return Err(PwError::new(ErrorCode::Conflict, format!("a share {} exists", share.slug)));
        }
        if map.values().any(|s| s.agent == share.agent) {
            return Err(PwError::new(ErrorCode::Conflict, "that agent is already shared"));
        }
        self.save(&share)?;
        map.insert(share.slug.clone(), share.clone());
        Ok(share)
    }

    /// What [`set`](Self::set) would change, without changing it: for the
    /// approval that shows the exact diff before a widening applies.
    pub fn preview(&self, slug: &str, next: &ShareSettings) -> Result<ShareDiff> {
        let current = self.get(slug).ok_or_else(|| not_found(slug))?;
        Ok(diff(&current, next))
    }

    /// `share set`: replace the settings (state, slug and agent stay) and
    /// return the diff applied. Applies from the next request (§5.4).
    pub fn set(&self, slug: &str, mut next: ShareSettings) -> Result<(ShareDiff, ShareSettings)> {
        let mut map = self.map();
        let current = map.get(slug).cloned().ok_or_else(|| not_found(slug))?;
        next.slug = current.slug.clone();
        next.agent = current.agent.clone();
        next.state = current.state;
        check(&next)?;
        let d = diff(&current, &next);
        if !d.0.is_empty() {
            self.save(&next)?;
            map.insert(slug.to_string(), next.clone());
        }
        Ok((d, next))
    }

    fn set_state(&self, slug: &str, from: &[ShareState], to: ShareState) -> Result<ShareSettings> {
        let mut map = self.map();
        let share = map.get_mut(slug).ok_or_else(|| not_found(slug))?;
        if share.state == to {
            return Ok(share.clone());
        }
        if !from.contains(&share.state) {
            return Err(PwError::new(ErrorCode::Conflict, format!("{slug} is {:?}", share.state).to_lowercase()));
        }
        let mut next = share.clone();
        next.state = to;
        self.save(&next)?;
        *share = next.clone();
        Ok(next)
    }

    /// Publish a draft (§5.4). Always asked of the owner first (the
    /// daemon's business). C1a hosts Read only shares; "Can change code"
    /// arrives in C2.
    pub fn go_live(&self, slug: &str) -> Result<ShareSettings> {
        let share = self.get(slug).ok_or_else(|| not_found(slug))?;
        if share.capability != Capability::Read {
            return Err(PwError::unsupported("only Read only shares can go live in this version"));
        }
        self.set_state(slug, &[ShareState::Draft], ShareState::Live)
    }

    pub fn pause(&self, slug: &str) -> Result<ShareSettings> {
        self.set_state(slug, &[ShareState::Live], ShareState::Paused)
    }

    pub fn resume(&self, slug: &str) -> Result<ShareSettings> {
        self.set_state(slug, &[ShareState::Paused], ShareState::Live)
    }

    /// Stop sharing: the settings go; the agent stays as it was (§5.1).
    pub fn stop(&self, slug: &str) -> Result<ShareSettings> {
        let share = self.map().remove(slug).ok_or_else(|| not_found(slug))?;
        let _ = std::fs::remove_file(self.paths.share_file(slug));
        Ok(share)
    }
}

/// What every share must be.
pub fn check(share: &ShareSettings) -> Result<()> {
    if !valid_slug(&share.slug) {
        return Err(PwError::other("a slug is 1 to 48 of a-z, 0-9 and -, not starting with -"));
    }
    if share.agent.trim().is_empty() {
        return Err(PwError::other("a share names the agent it shares"));
    }
    if share.name.trim().is_empty() {
        return Err(PwError::other("a share has a name"));
    }
    if share.scope.history {
        return Err(PwError::unsupported("git history in the copy isn't offered yet"));
    }
    if let Start::Summary { text, approved_at } = &share.start {
        if text.trim().is_empty() || approved_at.trim().is_empty() {
            return Err(PwError::other("a summary start needs the approved text"));
        }
    }
    let l = &share.limits;
    if l.sessions_running == 0 || l.turn_timeout_min == 0 {
        return Err(PwError::other("at least one conversation at once and a minute per turn"));
    }
    Ok(())
}

// ------------------------------------------------------------------ diff

fn v<T: Serialize + ?Sized>(x: &T) -> Value {
    serde_json::to_value(x).unwrap_or(Value::Null)
}

struct Lines(Vec<ShareDiffLine>);

impl Lines {
    fn push<T: Serialize + PartialEq + ?Sized>(&mut self, field: &str, from: &T, to: &T, widens: bool) {
        if from != to {
            self.0.push(ShareDiffLine { field: field.into(), from: v(from), to: v(to), widens });
        }
    }
}

/// Something in `to` that isn't in `from`.
fn added<T: PartialEq>(from: &[T], to: &[T]) -> bool {
    to.iter().any(|x| !from.contains(x))
}

fn capability_rank(c: Capability) -> u8 {
    match c {
        Capability::Read => 0,
        Capability::WriteApproval => 1,
        Capability::Write => 2,
    }
}

/// Paths `to` reaches that `from` didn't: none picked means the whole
/// repository (the widest).
fn more_paths(from: &[String], to: &[String]) -> bool {
    let norm = |p: &String| p.trim_start_matches("./").trim_end_matches('/').to_string();
    if from.is_empty() {
        return false;
    }
    if to.is_empty() {
        return true;
    }
    to.iter().map(norm).any(|t| !from.iter().map(norm).any(|f| f.is_empty() || t == f || t.starts_with(&format!("{f}/"))))
}

fn more_programs(from: &Option<Programs>, to: &Option<Programs>) -> bool {
    match (from, to) {
        (Some(Programs::Allow(a)), Some(Programs::Allow(b))) => added(a, b),
        (Some(Programs::Deny(a)), Some(Programs::Deny(b))) => added(b, a),
        (Some(Programs::Deny(_)), Some(Programs::Allow(_))) => false,
        _ => from != to,
    }
}

/// A person's daily messages (None: blocked) under `limits`.
fn allowance(limits: &Limits, o: Option<&LimitOverride>) -> Option<u32> {
    match o {
        Some(o) if o.blocked => None,
        Some(o) => Some(o.messages_per_day.unwrap_or(limits.per_caller.messages_per_day)),
        None => Some(limits.per_caller.messages_per_day),
    }
}

/// The exact changes from `a` to `b`, each marked `widens` per §5.5: more
/// people, teams, groups or networks; more paths or fewer exclusions; a
/// stronger capability; more network hosts, environment or programs; a new
/// base summary; higher limits; going live. Narrowing is the opposite.
/// Names, descriptions and the soft off-limits text never widen.
pub fn diff(a: &ShareSettings, b: &ShareSettings) -> ShareDiff {
    let mut d = Lines(Vec::new());
    d.push("name", &a.name, &b.name, false);
    d.push("description", &a.description, &b.description, false);
    d.push("state", &a.state, &b.state, b.state == ShareState::Live);
    d.push("capability", &a.capability, &b.capability, capability_rank(b.capability) > capability_rank(a.capability));
    d.push("audience.people", &a.audience.people, &b.audience.people, b.audience.people.iter().any(|p| !a.audience.people.iter().any(|q| q.is(p))));
    d.push("audience.teams", &a.audience.teams, &b.audience.teams, added(&a.audience.teams, &b.audience.teams));
    d.push("audience.groups", &a.audience.groups, &b.audience.groups, added(&a.audience.groups, &b.audience.groups));
    d.push("audience.networks", &a.audience.networks, &b.audience.networks, added(&a.audience.networks, &b.audience.networks));
    d.push("scope.repo", &a.scope.repo, &b.scope.repo, true);
    d.push("scope.ref", &a.scope.git_ref, &b.scope.git_ref, true);
    d.push("scope.paths", &a.scope.paths, &b.scope.paths, more_paths(&a.scope.paths, &b.scope.paths));
    d.push("scope.exclude", &a.scope.exclude, &b.scope.exclude, added(&b.scope.exclude, &a.scope.exclude));
    d.push("scope.uncommitted", &a.scope.uncommitted, &b.scope.uncommitted, b.scope.uncommitted);
    d.push("scope.history", &a.scope.history, &b.scope.history, b.scope.history);
    let new_summary = match (&a.start, &b.start) {
        (_, Start::Fresh) => false,
        (Start::Summary { text: x, .. }, Start::Summary { text: y, .. }) => x != y,
        (Start::Fresh, Start::Summary { .. }) => true,
    };
    d.push("start", &a.start, &b.start, new_summary);
    d.push("access.env", &a.access.env, &b.access.env, added(&a.access.env, &b.access.env));
    d.push("access.network.allow", &a.access.network.allow, &b.access.network.allow, added(&a.access.network.allow, &b.access.network.allow));
    d.push("access.account", &a.access.account, &b.access.account, true);
    d.push("access.programs", &a.access.programs, &b.access.programs, more_programs(&a.access.programs, &b.access.programs));
    d.push("offLimits", &a.off_limits, &b.off_limits, false);
    let (la, lb) = (&a.limits, &b.limits);
    d.push("limits.perCaller.messagesPerDay", &la.per_caller.messages_per_day, &lb.per_caller.messages_per_day, lb.per_caller.messages_per_day > la.per_caller.messages_per_day);
    d.push("limits.sessionsRunning", &la.sessions_running, &lb.sessions_running, lb.sessions_running > la.sessions_running);
    d.push("limits.queue", &la.queue, &lb.queue, lb.queue > la.queue);
    d.push("limits.turnTimeoutMin", &la.turn_timeout_min, &lb.turn_timeout_min, lb.turn_timeout_min > la.turn_timeout_min);
    d.push("limits.agentMinutesPerDay", &la.agent_minutes_per_day, &lb.agent_minutes_per_day, lb.agent_minutes_per_day > la.agent_minutes_per_day);
    d.push("limits.idleStopMin", &la.idle_stop_min, &lb.idle_stop_min, false);
    d.push("limits.keepSessionsDays", &la.keep_sessions_days, &lb.keep_sessions_days, false);
    let keys: std::collections::BTreeSet<&String> = la.overrides.keys().chain(lb.overrides.keys()).collect();
    for key in keys {
        let (oa, ob) = (la.overrides.get(key), lb.overrides.get(key));
        let widens = match (allowance(la, oa), allowance(lb, ob)) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => y > x,
            _ => false,
        };
        d.push(&format!("limits.overrides.{key}"), &oa, &ob, widens);
    }
    ShareDiff(d.0)
}
