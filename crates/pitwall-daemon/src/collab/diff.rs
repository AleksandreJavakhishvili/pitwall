//! A share's settings, changed: [`apply`] a [`SharePatch`] and [`diff`] the
//! result against what was there, line by line, each line marked when it
//! **widens** access (collaboration/sharing.md §5.5). The diff is the
//! core's (`pitwall_core::collab::shares::diff`), the same one `Collab`
//! applies. Widening from the CLI or an agent always asks the owner and
//! can't be remembered; narrowing applies directly.
//!
//! Widening: more people, teams, groups or a whole network; more paths or
//! fewer exclusions; uncommitted changes or history in; a stronger
//! capability (Read only → Can change code, or approval removed); more
//! hosts or environment variables; a base summary added or changed; a
//! different repository, branch or account; more programs; higher limits;
//! a person let in again; going live.

use pitwall_proto::api::collab::SharePatch;
use pitwall_proto::collab::{ShareSettings, TeamId};

/// What changed from `old` to `new`, line by line: the core's diff, the one
/// the core applies, so what an approval shows is what changes.
pub use pitwall_core::collab::shares::diff;

pub use pitwall_proto::api::collab::describe;

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
    use pitwall_proto::collab::{Capability, Identity, LimitOverride, NetworkId, Programs, Scope, ShareState, Start};

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
