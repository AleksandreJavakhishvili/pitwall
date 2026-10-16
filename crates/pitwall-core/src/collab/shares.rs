//! Share settings on the owner's machine (§5.4–5.7): `collab/shares/<slug>.json`,
//! one per shared agent. A share is created as a **draft** (visible only to
//! the owner and their guest sessions), goes live, pauses, resumes, and
//! stops (the file goes; the agent stays as it was, §5.1).
//!
//! Every change goes through [`diff`]: the exact lines that changed, each
//! marked `widens` per §5.5. Widening always asks the owner and can't be
//! remembered; narrowing may apply directly. Who asks is the daemon's
//! approval mechanism; this module only says which is which.

use std::collections::{BTreeMap, BTreeSet};
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
//
// The one share diff: the core applies it (`set_share`, `preview_share`)
// and the daemon's approvals show its lines (`pitwall_daemon::collab::diff`
// re-exports it), so what the owner approves is what changes.

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

    /// A list compared as a set (order never matters): one line when its
    /// members changed, widening when `grow_widens` and something was added
    /// (or, when not, when something was taken out).
    fn set<T: Serialize + Ord>(&mut self, field: &str, from: &[T], to: &[T], grow_widens: bool) {
        let (a, b): (BTreeSet<&T>, BTreeSet<&T>) = (from.iter().collect(), to.iter().collect());
        if a == b {
            return;
        }
        let widens = if grow_widens { b.iter().any(|x| !a.contains(x)) } else { a.iter().any(|x| !b.contains(x)) };
        self.0.push(ShareDiffLine { field: field.into(), from: v(from), to: v(to), widens });
    }

    fn number(&mut self, field: &str, from: u32, to: u32) {
        self.push(field, &from, &to, to > from);
    }
}

fn capability_rank(c: Capability) -> u8 {
    match c {
        Capability::Read => 0,
        Capability::WriteApproval => 1,
        Capability::Write => 2,
    }
}

fn state_rank(s: ShareState) -> u8 {
    match s {
        ShareState::Draft | ShareState::Paused => 0,
        ShareState::Live => 1,
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

/// "Programs it may run" as it applies: the share's own list, else the
/// capability's default (an allowlist at Read only, a denylist at Can
/// change code; `sandbox::kind`).
pub fn effective_programs(share: &ShareSettings) -> Programs {
    share.access.programs.clone().unwrap_or_else(|| default_programs(share.capability))
}

/// The capability's default "Programs it may run" (§6.3; §14 question 16).
pub fn default_programs(capability: Capability) -> Programs {
    let names = |l: &[&str]| l.iter().map(|s| s.to_string()).collect();
    match capability {
        Capability::Read => Programs::Allow(names(crate::sandbox::kind::DEFAULT_ALLOW)),
        _ => Programs::Deny(names(crate::sandbox::kind::DEFAULT_DENY)),
    }
}

/// More programs allowed: an allowlist that grew, a denylist that shrank,
/// or a denylist instead of an allowlist.
fn more_programs(from: &Programs, to: &Programs) -> bool {
    let added = |a: &[String], b: &[String]| b.iter().any(|x| !a.contains(x));
    match (from, to) {
        (Programs::Allow(a), Programs::Allow(b)) => added(a, b),
        (Programs::Deny(a), Programs::Deny(b)) => added(b, a),
        (Programs::Allow(_), Programs::Deny(_)) => true,
        (Programs::Deny(_), Programs::Allow(_)) => false,
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

/// The exact changes from `a` to `b`, in a fixed order, each marked
/// `widens` per §5.5: going live; a stronger capability; more people,
/// teams, groups or networks; another repository, ref or account; more
/// paths or fewer exclusions; uncommitted changes or history in; a new
/// base summary; more environment, hosts or programs; higher limits; a
/// person let in again. Narrowing is the opposite. Names, descriptions and
/// the soft off-limits text never widen. Lists compare as sets.
pub fn diff(a: &ShareSettings, b: &ShareSettings) -> ShareDiff {
    let mut d = Lines(Vec::new());
    d.push("state", &a.state, &b.state, state_rank(b.state) > state_rank(a.state));
    d.push("slug", &a.slug, &b.slug, false);
    d.push("name", &a.name, &b.name, false);
    d.push("description", &a.description, &b.description, false);
    d.push("capability", &a.capability, &b.capability, capability_rank(b.capability) > capability_rank(a.capability));
    {
        // People match on any of their identities (a login given with or
        // without its email is the same person).
        let (pa, pb): (BTreeSet<_>, BTreeSet<_>) = (a.audience.people.iter().collect(), b.audience.people.iter().collect());
        if pa != pb {
            let widens = b.audience.people.iter().any(|p| !a.audience.people.iter().any(|q| q.is(p)));
            d.0.push(ShareDiffLine { field: "audience.people".into(), from: v(&a.audience.people), to: v(&b.audience.people), widens });
        }
    }
    d.set("audience.teams", &a.audience.teams, &b.audience.teams, true);
    d.set("audience.groups", &a.audience.groups, &b.audience.groups, true);
    d.set("audience.networks", &a.audience.networks, &b.audience.networks, true);
    d.push("scope.repo", &a.scope.repo, &b.scope.repo, true);
    d.push("scope.ref", &a.scope.git_ref, &b.scope.git_ref, true);
    if a.scope.paths.iter().collect::<BTreeSet<_>>() != b.scope.paths.iter().collect::<BTreeSet<_>>() {
        let widens = more_paths(&a.scope.paths, &b.scope.paths);
        d.0.push(ShareDiffLine { field: "scope.paths".into(), from: v(&a.scope.paths), to: v(&b.scope.paths), widens });
    }
    d.set("scope.exclude", &a.scope.exclude, &b.scope.exclude, false);
    d.push("scope.uncommitted", &a.scope.uncommitted, &b.scope.uncommitted, b.scope.uncommitted);
    d.push("scope.history", &a.scope.history, &b.scope.history, b.scope.history);
    let new_summary = match (&a.start, &b.start) {
        (_, Start::Fresh) => false,
        (Start::Summary { text: x, .. }, Start::Summary { text: y, .. }) => x != y,
        (Start::Fresh, Start::Summary { .. }) => true,
    };
    d.push("start", &a.start, &b.start, new_summary);
    d.set("access.env", &a.access.env, &b.access.env, true);
    d.set("access.network.allow", &a.access.network.allow, &b.access.network.allow, true);
    d.push("access.account", &a.access.account, &b.access.account, true);
    d.push("access.programs", &a.access.programs, &b.access.programs, more_programs(&effective_programs(a), &effective_programs(b)));
    d.push("offLimits", &a.off_limits, &b.off_limits, false);
    let (la, lb) = (&a.limits, &b.limits);
    d.number("limits.perCaller.messagesPerDay", la.per_caller.messages_per_day, lb.per_caller.messages_per_day);
    d.number("limits.sessionsRunning", la.sessions_running, lb.sessions_running);
    d.number("limits.queue", la.queue, lb.queue);
    d.number("limits.turnTimeoutMin", la.turn_timeout_min, lb.turn_timeout_min);
    d.number("limits.agentMinutesPerDay", la.agent_minutes_per_day, lb.agent_minutes_per_day);
    let keys: BTreeSet<&String> = la.overrides.keys().chain(lb.overrides.keys()).collect();
    for key in keys {
        let (oa, ob) = (la.overrides.get(key), lb.overrides.get(key));
        let widens = match (allowance(la, oa), allowance(lb, ob)) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => y > x,
            _ => false,
        };
        d.push(&format!("limits.overrides.{key}"), &oa, &ob, widens);
    }
    d.push("limits.idleStopMin", &la.idle_stop_min, &lb.idle_stop_min, false);
    d.push("limits.keepSessionsDays", &la.keep_sessions_days, &lb.keep_sessions_days, false);
    ShareDiff(d.0)
}
