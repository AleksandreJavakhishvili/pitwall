//! Agents named after their conversation (docs/spec/agents-more.md): the
//! title the agent itself records (`onboarding::latest::session_title`),
//! read again only on the events that can change it (a prompt, a finished
//! turn, a new conversation id) and never by polling. Pitwall only reads
//! the agent's files; it never types anything into the agent.

use std::path::PathBuf;

use super::{Agent, Engine};
use crate::model::NameSource;
use crate::onboarding::latest;

/// One title to read (unlocked).
#[derive(Debug, Clone)]
pub(super) struct Job {
    id: String,
    kind: String,
    cwd: String,
    session_id: String,
    account: Option<String>,
}

/// Agents whose title is wanted and can be read here: agents (not
/// terminals) on this Mac with a known conversation. Clears the wish.
pub(super) fn due(agents: &mut [Agent]) -> Vec<Job> {
    agents
        .iter_mut()
        .filter_map(|a| {
            // Where it runs isn't known yet: later.
            if a.facts_pending || !std::mem::take(&mut a.title_wanted) || a.is_terminal() || !a.facts.provider.local_process {
                return None;
            }
            Some(Job {
                id: a.rec.id.clone(),
                kind: a.rec.kind.clone(),
                cwd: a.rec.cwd.clone(),
                session_id: a.rec.session_id.clone()?,
                account: a.rec.account.clone(),
            })
        })
        .collect()
}

/// The kind's own folder for that login (`~/.claude`, an account's).
fn root(core: &Engine, kind: &str, account: Option<&str>) -> Option<PathBuf> {
    let def = core.kinds().find(kind)?;
    let list = account.map(|_| crate::accounts::Accounts::load(core.paths()));
    let acc = account.and_then(|a| list.as_ref()?.get(a));
    crate::accounts::dir_of(&def, acc)
}

/// Read each job's title and apply it. Blocking file reads: off the lock.
pub(super) fn run(core: &Engine, jobs: Vec<Job>) {
    for j in jobs {
        let title = root(core, &j.kind, j.account.as_deref())
            .and_then(|r| latest::session_title(&r, &j.kind, &j.cwd, &j.session_id));
        if let Some(title) = title {
            apply(core, &j.id, &j.session_id, title);
        }
    }
}

/// Remember `title` for agent `id` (still in conversation `session_id`);
/// an `Auto` name takes it.
pub(super) fn apply(core: &Engine, id: &str, session_id: &str, title: String) {
    let changed = core.with(id, |a| {
        if a.rec.session_id.as_deref() != Some(session_id) {
            return false;
        }
        let mut changed = a.rec.session_title.as_ref() != Some(&title);
        if a.rec.name_auto() && a.rec.name != title {
            a.rec.name = title.clone();
            changed = true;
        }
        a.rec.session_title = Some(title);
        changed
    });
    if changed == Ok(true) {
        core.changed(true);
    }
}

impl Engine {
    /// Follow the conversation's title again (`pitwall agent rename --auto`,
    /// "Use session title"): the name becomes the title when one is known
    /// (and is read again now); until then it stays as it is.
    pub fn use_session_title(&self, id: &str) -> Result<crate::model::AgentView, String> {
        let view = self.with(id, |a| {
            a.rec.name_source = Some(NameSource::Auto);
            if let Some(t) = a.rec.session_title.clone() {
                a.rec.name = t;
            }
            a.title_wanted = true;
            a.view()
        })?;
        self.changed(true);
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HookEffect, HookState};
    use crate::model::AgentRecord;
    use crate::testing::{record, Harness};

    fn claude(id: &str, name: &str, source: Option<NameSource>) -> AgentRecord {
        let mut r = record(id, "/w/app");
        (r.kind, r.kind_name, r.name, r.name_source) = ("claude".into(), "Claude Code".into(), name.into(), source);
        r.session_id = Some("s1".into());
        r
    }

    fn named(h: &Harness, id: &str) -> (String, bool, Option<String>) {
        let v = h.engine.views().into_iter().find(|v| v.id == id).unwrap();
        (v.name, v.name_auto, v.session_title)
    }

    #[test]
    fn older_records_keep_the_names_people_chose() {
        let mut engineer = claude("e", "Claude Code", None);
        engineer.engineer = true;
        let h = Harness::new(vec![claude("a", "Claude Code", None), claude("b", "api", None), engineer]);
        assert!(named(&h, "a").1, "still its kind's name: follows its title");
        assert!(!named(&h, "b").1, "a chosen name is kept");
        assert!(!named(&h, "e").1, "the Race Engineer keeps its name");
        // Settled once: following the title doesn't make it look chosen.
        apply(&h.engine, "a", "s1", "Fix login".into());
        apply(&h.engine, "a", "s1", "Fix login, then SSO".into());
        assert_eq!(named(&h, "a"), ("Fix login, then SSO".into(), true, Some("Fix login, then SSO".into())));
        let rec = h.engine.records().into_iter().find(|r| r.id == "a").unwrap();
        assert_eq!(rec.name_source, Some(NameSource::Auto));
    }

    #[test]
    fn auto_names_follow_the_title_until_renamed() {
        let h = Harness::new(vec![claude("a", "Claude Code", Some(NameSource::Auto)), claude("b", "api", Some(NameSource::Manual))]);
        apply(&h.engine, "a", "s1", "Fix login".into());
        apply(&h.engine, "b", "s1", "Fix login".into());
        assert_eq!(named(&h, "a"), ("Fix login".into(), true, Some("Fix login".into())));
        assert_eq!(named(&h, "b"), ("api".into(), false, Some("Fix login".into())), "known, not used");
        apply(&h.engine, "a", "s0", "Another conversation".into());
        assert_eq!(named(&h, "a").0, "Fix login", "not its conversation any more");

        // Renaming pins the name; the title is still kept up to date.
        h.engine.rename("a", "mine").unwrap();
        apply(&h.engine, "a", "s1", "Fix login and SSO".into());
        assert_eq!(named(&h, "a"), ("mine".into(), false, Some("Fix login and SSO".into())));
        // Back to the title.
        let v = h.engine.use_session_title("a").unwrap();
        assert_eq!((v.name.as_str(), v.name_auto), ("Fix login and SSO", true));
        // No title yet: the name stays until there is one.
        let v = h.engine.use_session_title("b").unwrap();
        assert_eq!((v.name.as_str(), v.name_auto), ("Fix login", true));
    }

    #[test]
    fn titles_are_read_on_events_never_polled() {
        let mut a = Agent::new(claude("a", "Claude Code", None), None, 1);
        assert!(a.title_wanted, "once at load: it may have changed meanwhile");
        let wanted = |a: &mut Agent, e: HookEffect| {
            a.title_wanted = false;
            a.apply_hook(e);
            a.title_wanted
        };
        assert!(!wanted(&mut a, HookEffect { state: Some(HookState::Working), ..Default::default() }));
        assert!(wanted(&mut a, HookEffect { prompt_submitted: true, ..Default::default() }));
        assert!(wanted(&mut a, HookEffect { state: Some(HookState::Done), ..Default::default() }));
        a.rec.session_title = Some("old".into());
        assert!(wanted(&mut a, HookEffect { session_id: Some("s2".into()), ..Default::default() }));
        assert_eq!(a.rec.session_title, None, "a new conversation has its own");

        // Only for agents on this Mac whose machine is known, not terminals.
        a.facts.provider.local_process = true;
        let mut t = Agent::new(record("t", "/w"), None, 1);
        (t.rec.session_id, t.title_wanted, t.facts.provider.local_process) = (Some("x".into()), true, true);
        let mut agents = vec![a, t];
        agents[0].facts_pending = true;
        assert!(due(&mut agents).is_empty() && agents[0].title_wanted, "kept for later");
        agents[0].facts_pending = false;
        let jobs = due(&mut agents);
        assert_eq!(jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["a"]);
        assert!(due(&mut agents).is_empty(), "read once per wish");
    }
}
