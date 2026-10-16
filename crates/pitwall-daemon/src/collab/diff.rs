//! A share's settings, changed: [`apply`] a [`SharePatch`] and [`diff`] the
//! result against what was there, line by line, each line marked when it
//! **widens** access (collaboration/sharing.md §5.5). Widening from the CLI
//! or an agent always asks the owner and can't be remembered; narrowing
//! applies directly.
//!
//! Widening: more people, teams, groups or a whole network; more paths or
//! fewer exclusions; uncommitted changes or history in; a stronger
//! capability (Read only → Can change code, or approval removed); more
//! hosts or environment variables; a base summary added or changed; a
//! different repository, branch or account; more programs; higher limits;
//! a person let in again; going live.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use pitwall_proto::api::collab::SharePatch;
use pitwall_proto::collab::{Capability, LimitOverride, Programs, ShareDiff, ShareDiffLine, ShareSettings, ShareState, Start, TeamId};

/// Strength of a capability: approval removed is stronger than kept.
fn rank(c: Capability) -> u8 {
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

fn line(field: &str, from: Value, to: Value, widens: bool) -> ShareDiffLine {
    ShareDiffLine { field: field.into(), from, to, widens }
}

fn v<T: serde::Serialize>(x: &T) -> Value {
    serde_json::to_value(x).unwrap_or(Value::Null)
}

/// Something in `new` that isn't in `old`.
fn added<T: Ord + Clone>(old: &[T], new: &[T]) -> bool {
    let old: BTreeSet<&T> = old.iter().collect();
    new.iter().any(|x| !old.contains(x))
}

/// A list field: one line when it changed, widening when `grow_widens` and
/// something was added (or, when not, something was taken out).
fn list<T: Ord + Clone + serde::Serialize>(out: &mut Vec<ShareDiffLine>, field: &str, old: &[T], new: &[T], grow_widens: bool) {
    let (a, b): (BTreeSet<&T>, BTreeSet<&T>) = (old.iter().collect(), new.iter().collect());
    if a == b {
        return;
    }
    let widens = if grow_widens { added(old, new) } else { added(new, old) };
    out.push(line(field, v(&old), v(&new), widens));
}

fn number(out: &mut Vec<ShareDiffLine>, field: &str, old: u32, new: u32) {
    if old != new {
        out.push(line(field, json!(old), json!(new), new > old));
    }
}

/// Every path in the copy (`[]`: the whole repository) is it still covered?
fn paths_widen(old: &[String], new: &[String]) -> bool {
    let covered = |p: &String| old.iter().any(|o| p.starts_with(o.as_str()));
    match (old.is_empty(), new.is_empty()) {
        (true, _) => false,
        (false, true) => true,
        (false, false) => !new.iter().all(covered),
    }
}

/// More programs allowed: an allowlist that grew, a denylist that shrank,
/// a denylist instead of an allowlist, or the defaults dropped for a list
/// (compared with the capability's default: an allowlist at Read only).
fn programs_widen(old: &Option<Programs>, new: &Option<Programs>) -> bool {
    match (old, new) {
        (Some(Programs::Allow(a)), Some(Programs::Allow(b))) => added(a, b),
        (Some(Programs::Deny(a)), Some(Programs::Deny(b))) => added(b, a),
        (Some(Programs::Allow(_)), Some(Programs::Deny(_))) => true,
        (Some(Programs::Deny(_)), Some(Programs::Allow(_))) => false,
        (None, Some(Programs::Deny(_))) => true,
        (None, Some(Programs::Allow(_))) => false,
        (Some(_), None) => true,
        (None, None) => false,
    }
}

fn override_widens(old: Option<&LimitOverride>, new: Option<&LimitOverride>, base: u32) -> bool {
    let limit = |o: Option<&LimitOverride>| o.and_then(|o| o.messages_per_day).unwrap_or(base);
    let blocked = |o: Option<&LimitOverride>| o.is_some_and(|o| o.blocked);
    (blocked(old) && !blocked(new)) || (!blocked(new) && limit(new) > limit(old))
}

/// What changed from `old` to `new`, in a fixed order.
pub fn diff(old: &ShareSettings, new: &ShareSettings) -> ShareDiff {
    let mut out = Vec::new();
    if old.state != new.state {
        out.push(line("state", v(&old.state), v(&new.state), state_rank(new.state) > state_rank(old.state)));
    }
    for (field, a, b) in [("slug", &old.slug, &new.slug), ("name", &old.name, &new.name), ("description", &old.description, &new.description)] {
        if a != b {
            out.push(line(field, json!(a), json!(b), false));
        }
    }
    if old.capability != new.capability {
        out.push(line("capability", v(&old.capability), v(&new.capability), rank(new.capability) > rank(old.capability)));
    }
    list(&mut out, "audience.people", &old.audience.people, &new.audience.people, true);
    list(&mut out, "audience.teams", &old.audience.teams, &new.audience.teams, true);
    list(&mut out, "audience.groups", &old.audience.groups, &new.audience.groups, true);
    list(&mut out, "audience.networks", &old.audience.networks, &new.audience.networks, true);
    let (s, t) = (&old.scope, &new.scope);
    if s.repo != t.repo {
        out.push(line("scope.repo", json!(s.repo), json!(t.repo), true));
    }
    if s.git_ref != t.git_ref {
        out.push(line("scope.ref", json!(s.git_ref), json!(t.git_ref), true));
    }
    if s.paths.iter().collect::<BTreeSet<_>>() != t.paths.iter().collect::<BTreeSet<_>>() {
        out.push(line("scope.paths", v(&s.paths), v(&t.paths), paths_widen(&s.paths, &t.paths)));
    }
    list(&mut out, "scope.exclude", &s.exclude, &t.exclude, false);
    for (field, a, b) in [("scope.uncommitted", s.uncommitted, t.uncommitted), ("scope.history", s.history, t.history)] {
        if a != b {
            out.push(line(field, json!(a), json!(b), b));
        }
    }
    if old.start != new.start {
        // Fresh is the narrowest start; any summary (new or edited) widens.
        out.push(line("start", v(&old.start), v(&new.start), new.start != Start::Fresh));
    }
    list(&mut out, "access.env", &old.access.env, &new.access.env, true);
    list(&mut out, "access.network.allow", &old.access.network.allow, &new.access.network.allow, true);
    if old.access.account != new.access.account {
        out.push(line("access.account", v(&old.access.account), v(&new.access.account), true));
    }
    if old.access.programs != new.access.programs {
        out.push(line("access.programs", v(&old.access.programs), v(&new.access.programs), programs_widen(&old.access.programs, &new.access.programs)));
    }
    if old.off_limits != new.off_limits {
        // Soft instructions: never access, so never asked for.
        out.push(line("offLimits", json!(old.off_limits), json!(new.off_limits), false));
    }
    let (a, b) = (&old.limits, &new.limits);
    number(&mut out, "limits.perCaller.messagesPerDay", a.per_caller.messages_per_day, b.per_caller.messages_per_day);
    number(&mut out, "limits.sessionsRunning", a.sessions_running, b.sessions_running);
    number(&mut out, "limits.queue", a.queue, b.queue);
    number(&mut out, "limits.turnTimeoutMin", a.turn_timeout_min, b.turn_timeout_min);
    number(&mut out, "limits.agentMinutesPerDay", a.agent_minutes_per_day, b.agent_minutes_per_day);
    let people: BTreeSet<&String> = a.overrides.keys().chain(b.overrides.keys()).collect();
    for p in people {
        let (o, n) = (a.overrides.get(p), b.overrides.get(p));
        if o != n {
            let widens = override_widens(o, n, b.per_caller.messages_per_day);
            out.push(line(&format!("limits.overrides.{p}"), v(&o), v(&n), widens));
        }
    }
    for (field, x, y) in [("limits.idleStopMin", a.idle_stop_min, b.idle_stop_min), ("limits.keepSessionsDays", a.keep_sessions_days, b.keep_sessions_days)] {
        if x != y {
            out.push(line(field, json!(x), json!(y), false));
        }
    }
    ShareDiff(out)
}

/// One diff line as the approval dialog and the CLI show it: `▲` widens
/// (asked, amber in Pitwall), `▼` doesn't. Lists say what was added and
/// taken out: `▲ audience.people: + dave@example.com`.
pub fn describe(l: &ShareDiffLine) -> String {
    let mark = if l.widens { "▲" } else { "▼" };
    let show = |x: &Value| match x {
        Value::String(s) => s.clone(),
        Value::Null => "none".into(),
        Value::Object(o) if o.contains_key("subject") => o["subject"].as_str().unwrap_or_default().to_string(),
        Value::Object(o) if o.contains_key("network") && o.contains_key("id") => {
            format!("{} on {}", o["id"].as_str().unwrap_or_default(), o["network"].as_str().unwrap_or_default())
        }
        other => other.to_string(),
    };
    match (&l.from, &l.to) {
        (Value::Array(a), Value::Array(b)) => {
            let mut parts = Vec::new();
            for x in b.iter().filter(|x| !a.contains(x)) {
                parts.push(format!("+ {}", show(x)));
            }
            for x in a.iter().filter(|x| !b.contains(x)) {
                parts.push(format!("− {}", show(x)));
            }
            if parts.is_empty() {
                parts.push("reordered".into());
            }
            format!("{mark} {}: {}", l.field, parts.join(", "))
        }
        (from, to) => format!("{mark} {}: {} → {}", l.field, show(from), show(to)),
    }
}

/// Apply `p` to `old`. `team` resolves a team given by name, fingerprint
/// or id to its id (`Err`: a sentence saying it isn't known).
pub fn apply(old: &ShareSettings, p: &SharePatch, team: &dyn Fn(&str) -> Result<TeamId, String>) -> Result<ShareSettings, String> {
    let mut s = match &p.settings {
        Some(whole) => whole.clone(),
        None => old.clone(),
    };
    // What stays the owner's alone: the agent shared and the state (go
    // live, pause and stop are their own methods).
    s.agent = old.agent.clone();
    s.state = old.state;
    if let Some(x) = &p.slug {
        let slug = x.trim();
        if !valid_slug(slug) {
            return Err(format!("\"{slug}\" can't be a share's name in an address: use lower-case letters, digits and dashes"));
        }
        s.slug = slug.into();
    }
    if let Some(x) = &p.name {
        s.name = x.trim().into();
    }
    if let Some(x) = &p.description {
        s.description = x.trim().into();
    }
    if let Some(c) = p.capability {
        s.capability = c;
    }
    let a = &mut s.audience;
    for who in &p.add.people {
        if !a.people.iter().any(|x| x.is(who)) {
            a.people.push(who.clone());
        }
    }
    a.people.retain(|x| !p.remove.people.iter().any(|r| r.is(x)));
    for t in &p.add.teams {
        let id = team(t)?;
        if !a.teams.contains(&id) {
            a.teams.push(id);
        }
    }
    for t in &p.remove.teams {
        let id = team(t)?;
        a.teams.retain(|x| *x != id);
    }
    for g in &p.add.groups {
        if !a.groups.contains(g) {
            a.groups.push(g.clone());
        }
    }
    a.groups.retain(|x| !p.remove.groups.contains(x));
    for n in &p.add.networks {
        if !a.networks.contains(n) {
            a.networks.push(n.clone());
        }
    }
    a.networks.retain(|x| !p.remove.networks.contains(x));
    if let Some(paths) = &p.paths {
        s.scope.paths = paths.iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
    }
    for x in &p.exclude {
        if !s.scope.exclude.contains(x) {
            s.scope.exclude.push(x.clone());
        }
    }
    s.scope.exclude.retain(|x| !p.include.contains(x));
    if let Some(u) = p.uncommitted {
        s.scope.uncommitted = u;
    }
    if let Some(start) = &p.start {
        s.start = start.clone();
    }
    if let Some(env) = &p.env {
        s.access.env = env.clone();
    }
    if let Some(hosts) = &p.hosts {
        s.access.network.allow = hosts.clone();
    }
    if let Some(acc) = &p.account {
        s.access.account = if acc.is_empty() || acc == "default" { None } else { Some(acc.clone()) };
    }
    if let Some(pr) = &p.programs {
        s.access.programs = Some(pr.clone());
    }
    if let Some(o) = &p.off_limits {
        s.off_limits = o.trim().into();
    }
    let l = &mut s.limits;
    if let Some(n) = p.messages_per_day {
        l.per_caller.messages_per_day = n;
    }
    if let Some(n) = p.sessions_running {
        l.sessions_running = n.max(1);
    }
    if let Some(n) = p.queue {
        l.queue = n;
    }
    if let Some(n) = p.turn_timeout_min {
        l.turn_timeout_min = n.max(1);
    }
    if let Some(n) = p.agent_minutes_per_day {
        l.agent_minutes_per_day = n;
    }
    for (who, o) in &p.overrides {
        match o {
            Some(o) => {
                l.overrides.insert(who.clone(), o.clone());
            }
            None => {
                l.overrides.remove(who);
            }
        }
    }
    Ok(s)
}

/// `api-expert`: lower-case letters, digits and dashes, 1–40 long.
pub fn valid_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !s.starts_with('-')
}

/// A slug made from a name: `API expert` → `api-expert`.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').chars().take(40).collect::<String>();
    if out.is_empty() {
        "agent".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use pitwall_proto::api::collab::AudienceEdit;
    use pitwall_proto::collab::{Identity, NetworkId, Scope};

    use super::*;

    pub(crate) fn share() -> ShareSettings {
        ShareSettings {
            agent: "a1".into(),
            slug: "api-expert".into(),
            name: "API expert".into(),
            description: "Questions about the API".into(),
            state: ShareState::Live,
            capability: Capability::Read,
            audience: Default::default(),
            scope: Scope { repo: "~/code/api".into(), git_ref: "main".into(), paths: vec!["src/".into(), "docs/".into()], exclude: vec![".env".into()], ..Default::default() },
            start: Start::Fresh,
            access: Default::default(),
            off_limits: String::new(),
            limits: Default::default(),
        }
    }

    fn teams(t: &str) -> Result<TeamId, String> {
        match t {
            "platform" | "Platform" => Ok(TeamId("t_9f2c".into())),
            "payments" => Ok(TeamId("t_77aa".into())),
            other => Err(format!("no team \"{other}\"")),
        }
    }

    fn patch(f: impl FnOnce(&mut SharePatch)) -> SharePatch {
        let mut p = SharePatch::default();
        f(&mut p);
        p
    }

    #[test]
    fn adding_a_person_widens_with_the_exact_line() {
        let old = share();
        let dave = Identity::new("tailscale", "dave@example.com");
        let new = apply(&old, &patch(|p| p.add = AudienceEdit { people: vec![dave.clone()], ..Default::default() }), &teams).unwrap();
        let d = diff(&old, &new);
        assert_eq!(d.0.len(), 1);
        assert_eq!(d.0[0].field, "audience.people");
        assert!(d.widens());
        assert_eq!(describe(&d.0[0]), "▲ audience.people: + dave@example.com");
        // And taking them out again narrows.
        let back = apply(&new, &patch(|p| p.remove.people = vec![dave]), &teams).unwrap();
        let d = diff(&new, &back);
        assert!(!d.widens());
        assert_eq!(describe(&d.0[0]), "▼ audience.people: − dave@example.com");
    }

    #[test]
    fn widening_and_narrowing_by_field() {
        let old = share();
        let cases: Vec<(SharePatch, &str, bool)> = vec![
            (patch(|p| p.add.teams = vec!["payments".into()]), "audience.teams", true),
            (patch(|p| p.add.networks = vec![NetworkId::new("tailscale", "example.ts.net")]), "audience.networks", true),
            (patch(|p| p.capability = Some(Capability::WriteApproval)), "capability", true),
            (patch(|p| p.paths = Some(vec!["src/".into()])), "scope.paths", false),
            (patch(|p| p.paths = Some(vec!["src/".into(), "docs/".into(), "ops/".into()])), "scope.paths", true),
            (patch(|p| p.paths = Some(vec![])), "scope.paths", true),
            (patch(|p| p.paths = Some(vec!["src/api/".into()])), "scope.paths", false),
            (patch(|p| p.exclude = vec!["secrets/".into()]), "scope.exclude", false),
            (patch(|p| p.include = vec![".env".into()]), "scope.exclude", true),
            (patch(|p| p.uncommitted = Some(true)), "scope.uncommitted", true),
            (patch(|p| p.start = Some(Start::Summary { text: "We use JWTs.".into(), approved_at: "2026-10-16T09:00:00Z".into() })), "start", true),
            (patch(|p| p.env = Some(vec!["NPM_TOKEN".into()])), "access.env", true),
            (patch(|p| p.hosts = Some(vec!["registry.npmjs.org".into()])), "access.network.allow", true),
            (patch(|p| p.account = Some("work".into())), "access.account", true),
            (patch(|p| p.programs = Some(Programs::Deny(vec!["curl".into()]))), "access.programs", true),
            (patch(|p| p.programs = Some(Programs::Allow(vec!["git".into()]))), "access.programs", false),
            (patch(|p| p.off_limits = Some("Don't discuss pricing".into())), "offLimits", false),
            (patch(|p| p.messages_per_day = Some(50)), "limits.perCaller.messagesPerDay", true),
            (patch(|p| p.messages_per_day = Some(5)), "limits.perCaller.messagesPerDay", false),
            (patch(|p| p.sessions_running = Some(1)), "limits.sessionsRunning", false),
            (patch(|p| p.agent_minutes_per_day = Some(240)), "limits.agentMinutesPerDay", true),
            (patch(|p| p.name = Some("API".into())), "name", false),
        ];
        for (p, field, widens) in cases {
            let new = apply(&old, &p, &teams).unwrap();
            let d = diff(&old, &new);
            assert_eq!(d.0.len(), 1, "{field}: {d:?}");
            assert_eq!((d.0[0].field.as_str(), d.0[0].widens), (field, widens), "{field}");
        }
        // Capability order: approval removed widens, approval added narrows.
        let mut w = old.clone();
        w.capability = Capability::WriteApproval;
        let mut full = old.clone();
        full.capability = Capability::Write;
        assert!(diff(&w, &full).widens());
        assert!(!diff(&full, &w).widens());
        // Going live widens; pausing narrows.
        let mut draft = old.clone();
        draft.state = ShareState::Draft;
        assert!(diff(&draft, &old).widens());
        let mut paused = old.clone();
        paused.state = ShareState::Paused;
        assert!(!diff(&old, &paused).widens());
    }

    #[test]
    fn overrides_widen_when_they_let_more_through() {
        let old = share();
        let more = apply(&old, &patch(|p| {
            p.overrides.insert("tailscale:dave@example.com".into(), Some(LimitOverride { messages_per_day: Some(50), blocked: false }));
        }), &teams)
        .unwrap();
        assert!(diff(&old, &more).widens());
        let blocked = apply(&more, &patch(|p| {
            p.overrides.insert("tailscale:dave@example.com".into(), Some(LimitOverride { messages_per_day: None, blocked: true }));
        }), &teams)
        .unwrap();
        assert!(!diff(&more, &blocked).widens(), "blocking narrows");
        let unblocked = apply(&blocked, &patch(|p| {
            p.overrides.insert("tailscale:dave@example.com".into(), None);
        }), &teams)
        .unwrap();
        assert!(diff(&blocked, &unblocked).widens(), "letting someone in again widens");
    }

    #[test]
    fn a_patch_never_moves_the_agent_or_the_state_and_checks_names() {
        let old = share();
        let mut other = old.clone();
        other.agent = "zz".into();
        other.state = ShareState::Draft;
        let got = apply(&old, &patch(|p| p.settings = Some(other)), &teams).unwrap();
        assert_eq!((got.agent.as_str(), got.state), ("a1", ShareState::Live));
        assert!(apply(&old, &patch(|p| p.add.teams = vec!["nope".into()]), &teams).unwrap_err().contains("no team"));
        assert!(apply(&old, &patch(|p| p.slug = Some("Bad Slug".into())), &teams).is_err());
        assert_eq!(slugify("API expert!"), "api-expert");
        assert_eq!(slugify("  "), "agent");
        assert!(valid_slug("docs-2") && !valid_slug("-x") && !valid_slug(""));
        assert!(diff(&old, &old).0.is_empty());
    }
}
