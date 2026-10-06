//! Agents started by hand in a Pitwall terminal (docs/spec/terminals.md §2).
//!
//! A terminal is an agent of the shell kind. When the user types `claude`,
//! `codex`, … in it, the terminal's foreground process group shows that
//! agent; the pane then *is* that agent (kind, status rules, conversation)
//! until it exits, and the same Pitwall agent id and tile stay throughout.
//!
//! The agent is also remembered on the terminal's record
//! (`AgentRecord::inner_agent`), and kept after it exits: a restart of the
//! terminal starts the shell and continues that agent inside it
//! (lifecycle.rs `restart`). It is forgotten when another agent starts in the
//! terminal, or once the user has gone on using the shell without it: Enter
//! pressed in the shell after the agent exited, and `FORGET_AFTER_MS` passed
//! since it exited while the terminal kept running. Exiting the agent and
//! leaving the pane (or the terminal ending) keeps it.
//!
//! Polling is cheap: one batched `ps` for every terminal that is due, about
//! once a second while output flows and backing off to every few seconds
//! when a terminal is quiet. The registry lock is never held during `ps`,
//! `lsof` or transcript reads.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::{Agent, Engine, Shared};
use crate::kind::AgentKind;
use crate::model::{AgentRecord, InnerAgent, KindCaps, Source, Status};
use crate::onboarding::latest;
use crate::platform;
use crate::procs::{self, Matcher, Recognised};
use crate::provider::Locator;
use crate::term::TermHost;

/// The kind a plain terminal has (its record's kind never changes).
pub const TERMINAL_KIND: &str = "shell";

const POLL: Duration = Duration::from_millis(500);
/// Check at most this often while a terminal shows output…
pub const MIN_GAP_MS: u64 = 1_000;
/// …and at least this often when it's quiet.
pub const MAX_GAP_MS: u64 = 8_000;
/// How often (and how many times) to look for the conversation of an agent
/// whose arguments didn't name one (its transcript appears on first prompt).
const LOOKUP_EVERY_MS: u64 = 5_000;
const LOOKUPS: u8 = 36;
/// A transcript this much older than when the agent was noticed still counts.
const LOOKUP_SLACK_MS: u64 = MAX_GAP_MS + 5_000;
/// How long after the remembered agent exited a terminal whose shell is in
/// use (Enter pressed) forgets it and restarts as a plain shell again.
pub const FORGET_AFTER_MS: u64 = 3 * 60_000;

/// The agent running in a terminal's foreground right now.
#[derive(Debug, Clone, PartialEq)]
pub struct Inner {
    pub kind: String,
    pub kind_name: String,
    pub pid: u32,
    pub session_id: Option<String>,
    /// Unix ms when Pitwall noticed it.
    pub noticed_at: u64,
    /// Mono ms of the next conversation lookup, and how many are left.
    pub lookup_at: u64,
    pub lookups_left: u8,
}

/// When to look at a terminal's foreground next.
#[derive(Debug, Clone, PartialEq)]
pub struct Watch {
    pub last_at: u64,
    pub gap: u64,
    pub seen_seq: u64,
}

impl Default for Watch {
    fn default() -> Self {
        Watch { last_at: 0, gap: MIN_GAP_MS, seen_seq: u64::MAX }
    }
}

impl Watch {
    /// Whether to look now, given the terminal's output counter. New output
    /// makes it due after `MIN_GAP_MS` and resets the backoff; a quiet
    /// terminal is looked at after `gap`, which doubles up to `MAX_GAP_MS`.
    pub fn due(&mut self, seq: u64, now: u64) -> bool {
        let since = now.saturating_sub(self.last_at);
        let active = seq != self.seen_seq;
        if self.last_at != 0 && since < if active { MIN_GAP_MS } else { self.gap } {
            return false;
        }
        self.gap = if active { MIN_GAP_MS } else { (self.gap * 2).min(MAX_GAP_MS) };
        self.seen_seq = seq;
        self.last_at = now;
        true
    }
}

/// What a look at a terminal changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Same,
    /// An agent started in the foreground (or replaced another one).
    Entered,
    /// The agent exited; the terminal is a shell again (it still restarts
    /// as that agent).
    Left,
    /// The running agent's conversation became known.
    Learned,
    /// The shell has been used without the agent for a while: a restart
    /// starts a plain shell again.
    Forgot,
}

/// Remember conversation `id` for the agent with process `pid` on the
/// terminal's record. Returns whether the record changed.
pub(crate) fn remember_session(rec: &mut AgentRecord, pid: u32, id: &str) -> bool {
    match &mut rec.inner_agent {
        Some(m) if m.pid == Some(pid) && m.session_id.as_deref() != Some(id) => {
            m.session_id = Some(id.to_string());
            true
        }
        _ => false,
    }
}

/// The remembered agent exited `FORGET_AFTER_MS` ago and the user has used
/// the shell since.
fn forget_due(a: &Agent, unix_now: u64) -> bool {
    a.shell_used
        && a.rec.inner_agent.as_ref().and_then(|m| m.left_at).is_some_and(|t| unix_now.saturating_sub(t) >= FORGET_AFTER_MS)
}

/// Forget everything the previous foreground program told us about status.
fn reset_signals(a: &mut Agent, now: u64) {
    a.hook = None;
    a.screen_state = None;
    a.screen_detail = None;
    a.detect_seq = u64::MAX;
    a.turn_active = false;
    if !matches!(a.status, Status::Stopped | Status::Exited) {
        a.status = Status::Unknown;
        a.source = Source::Activity;
        a.detail = None;
        a.status_since = now;
    }
}

/// Fold what runs in a terminal's foreground (`None` = the shell itself)
/// into the agent. Pure bookkeeping, done under the registry lock.
/// `now`: mono ms; `unix_now`: unix ms.
pub fn apply(a: &mut Agent, seen: Option<(u32, Recognised)>, now: u64, unix_now: u64) -> Transition {
    if seen.is_none() && a.inner.is_none() && forget_due(a, unix_now) {
        a.rec.inner_agent = None;
        a.facts.inner = None;
        a.shell_used = false;
        return Transition::Forgot;
    }
    match (seen, &mut a.inner) {
        (None, None) => Transition::Same,
        (None, Some(_)) => {
            a.inner = None;
            a.hook_session = None;
            a.shell_used = false;
            if let Some(m) = &mut a.rec.inner_agent {
                m.left_at = Some(unix_now);
                m.pid = None;
            }
            reset_signals(a, now);
            Transition::Left
        }
        (Some((pid, r)), Some(inner)) if inner.pid == pid && inner.kind == r.kind => {
            match (&inner.session_id, r.session_id) {
                (None, Some(id)) => {
                    inner.session_id = Some(id.clone());
                    remember_session(&mut a.rec, pid, &id);
                    Transition::Learned
                }
                _ => Transition::Same,
            }
        }
        (Some((pid, r)), _) => {
            // A conversation id from a hook that arrived first counts too,
            // and so does the one remembered for this very process (Pitwall
            // restarted while it ran).
            let carried = a.rec.inner_agent.as_ref().filter(|m| m.kind == r.kind && m.pid == Some(pid)).and_then(|m| m.session_id.clone());
            let session_id = r.session_id.or_else(|| a.hook_session.clone()).or(carried);
            a.rec.inner_agent = Some(InnerAgent {
                kind: r.kind.clone(),
                kind_name: r.kind_name.clone(),
                session_id: session_id.clone(),
                pid: Some(pid),
                left_at: None,
            });
            a.inner = Some(Inner {
                kind: r.kind,
                kind_name: r.kind_name,
                pid,
                session_id,
                noticed_at: unix_now,
                lookup_at: now,
                lookups_left: LOOKUPS,
            });
            a.shell_used = false;
            reset_signals(a, now);
            Transition::Entered
        }
    }
}

/// [`apply`], then what the remembered agent's kind can do on the
/// terminal's provider (`kinds`: the current definitions).
pub fn observe(a: &mut Agent, seen: Option<(u32, Recognised)>, now: u64, unix_now: u64, kinds: &[AgentKind]) -> Transition {
    let t = apply(a, seen, now, unix_now);
    if t == Transition::Entered {
        let provider = a.facts.provider;
        a.facts.inner = a
            .rec
            .inner_agent
            .as_ref()
            .and_then(|m| kinds.iter().find(|k| k.id == m.kind))
            .map(|k| KindCaps::of(k, &provider));
    }
    t
}

/// Start the foreground poller.
pub fn start(core: Shared) {
    std::thread::Builder::new()
        .name("terminals".into())
        .spawn(move || {
            let mut matcher: Option<(u64, Matcher, Vec<AgentKind>)> = None;
            loop {
                poll(&core, &mut matcher);
                std::thread::sleep(POLL);
            }
        })
        .expect("spawn terminals poller");
}

/// Agent definitions change rarely; re-read them every 30 s.
const MATCHER_TTL_MS: u64 = 30_000;

struct Target {
    id: String,
    session: Arc<TermHost>,
    shell: u32,
}

struct Lookup {
    id: String,
    loc: Locator,
    kind: String,
    pid: u32,
    cwd: String,
    since: u64,
}

/// One round: look at the terminals that are due, then find conversations
/// of agents that don't have one yet.
fn poll(core: &Shared, matcher: &mut Option<(u64, Matcher, Vec<AgentKind>)>) {
    let now = core.now();
    // Only terminals whose shell is a process on this Mac: the process table
    // shows what runs in their foreground.
    let targets: Vec<Target> = core
        .agents()
        .iter_mut()
        .filter(|a| a.rec.kind == TERMINAL_KIND)
        .filter_map(|a| {
            let shell = a.local_pid()?;
            let session = a.host.clone()?;
            if session.ended() {
                return None;
            }
            let seq = session.output_seq.load(Ordering::Relaxed);
            a.fg.due(seq, now).then(|| Target { id: a.rec.id.clone(), session, shell })
        })
        .collect();
    let mut changed = false;
    if !targets.is_empty() {
        if matcher.as_ref().is_none_or(|(at, _, _)| now.saturating_sub(*at) >= MATCHER_TTL_MS) {
            let kinds = core.kinds().kinds();
            *matcher = Some((now, Matcher::new(&kinds), kinds));
        }
        let (_, m, kinds) = matcher.as_ref().expect("set above");
        if let Some(seen) = look(&targets, m) {
            let unix_now = crate::clock::unix_ms();
            let mut agents = core.agents();
            for (t, fg) in targets.iter().zip(seen) {
                let Some(a) = agents.iter_mut().find(|a| a.rec.id == t.id) else { continue };
                if !a.host.as_ref().is_some_and(|s| Arc::ptr_eq(s, &t.session)) {
                    continue;
                }
                changed |= observe(a, fg, now, unix_now, kinds) != Transition::Same;
            }
        }
    }
    let lookups = due_lookups(core, now);
    if !lookups.is_empty() {
        changed |= find_conversations(core, lookups);
    }
    if changed {
        // The remembered agent is part of the record.
        core.changed(true);
    }
}

/// What runs in each target's foreground; `None` when `ps` failed (nothing
/// is concluded from a failed look).
fn look(targets: &[Target], m: &Matcher) -> Option<Vec<Option<(u32, Recognised)>>> {
    let shells: Vec<u32> = targets.iter().map(|t| t.shell).collect();
    let groups = procs::parse_pid_numbers(&platform::terminal_groups(&shells)?);
    let mut pids: Vec<u32> = shells.clone();
    pids.extend(groups.values().filter(|g| **g > 0).map(|g| *g as u32));
    pids.sort_unstable();
    pids.dedup();
    let args = procs::parse_pid_args(&platform::process_args(&pids)?);
    Some(shells.iter().map(|s| procs::foreground_agent(*s, &groups, &args, m)).collect())
}

fn due_lookups(core: &Engine, now: u64) -> Vec<Lookup> {
    core.agents()
        .iter_mut()
        .filter_map(|a| {
            let cwd = a.rec.cwd.clone();
            let inner = a.inner.as_mut()?;
            if inner.session_id.is_some() || inner.lookups_left == 0 || now < inner.lookup_at {
                return None;
            }
            inner.lookups_left -= 1;
            inner.lookup_at = now + LOOKUP_EVERY_MS;
            Some(Lookup {
                id: a.rec.id.clone(),
                loc: a.rec.locator(),
                kind: inner.kind.clone(),
                pid: inner.pid,
                cwd,
                since: inner.noticed_at.saturating_sub(LOOKUP_SLACK_MS),
            })
        })
        .collect()
}

/// The newest conversation for each agent's folder (where the process really
/// is, as its provider sees it: the user may have `cd`ed since the terminal
/// opened). Unlocked I/O.
fn find_conversations(core: &Engine, lookups: Vec<Lookup>) -> bool {
    let home = crate::paths::home();
    let found: Vec<(Lookup, String)> = lookups
        .into_iter()
        .filter_map(|l| {
            let now_in = core
                .providers()
                .for_locator(&l.loc)
                .ok()
                .filter(|p| p.caps().process_cwd)
                .and_then(|p| p.process_cwd(&l.loc, Some(l.pid)).ok().flatten());
            let cwd = now_in.unwrap_or_else(|| l.cwd.clone());
            let s = latest::newest(&home, &l.kind, &cwd, Some(l.since))?;
            Some((l, s.session_id))
        })
        .collect();
    let mut changed = false;
    let mut agents = core.agents();
    for (l, session_id) in found {
        let Some(a) = agents.iter_mut().find(|a| a.rec.id == l.id) else { continue };
        let Some(inner) = a.inner.as_mut() else { continue };
        if inner.pid == l.pid && inner.session_id.is_none() {
            inner.session_id = Some(session_id.clone());
            remember_session(&mut a.rec, l.pid, &session_id);
            changed = true;
        }
    }
    changed
}

impl Engine {
    /// Conversations Pitwall agents have, including agents started by hand
    /// in a Pitwall terminal.
    pub fn owned_sessions(&self) -> HashSet<String> {
        self.agents()
            .iter()
            .flat_map(|a| {
                [
                    a.rec.session_id.clone(),
                    a.inner.as_ref().and_then(|i| i.session_id.clone()),
                    a.rec.inner_agent.as_ref().and_then(|i| i.session_id.clone()),
                ]
            })
            .flatten()
            .collect()
    }

    /// Processes whose descendants belong to Pitwall: this process and every
    /// agent's terminal (holders are detached from the app).
    pub fn owned_pids(&self) -> HashSet<u32> {
        let mut pids: HashSet<u32> = self
            .agents()
            .iter()
            .flat_map(|a| [a.local_pid(), a.inner.as_ref().map(|i| i.pid)])
            .flatten()
            .collect();
        pids.insert(std::process::id());
        pids
    }

    /// Agents whose process runs on this Mac: (agent id, name, pid of the
    /// process its holder runs). A process descending from one of these
    /// runs inside that agent (how the CLI server tells who is calling).
    pub fn agent_pids(&self) -> Vec<(String, String, u32)> {
        self.agents()
            .iter()
            .filter_map(|a| Some((a.rec.id.clone(), a.rec.name.clone(), a.local_pid()?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HookEffect, HookState};
    use crate::provider::ProviderCaps;
    use crate::testing::record;

    fn rec(kind: &str, sid: Option<&str>) -> Recognised {
        let name = if kind == "claude" { "Claude Code" } else { "Codex" };
        Recognised { kind: kind.into(), kind_name: name.into(), session_id: sid.map(String::from) }
    }

    fn terminal() -> Agent {
        let mut a = Agent::new(record("t", "/w/app"), None, 1);
        a.status = Status::Idle;
        a
    }

    #[test]
    fn a_terminal_becomes_the_agent_running_in_it_and_back() {
        let mut a = terminal();
        assert_eq!(apply(&mut a, None, 10, 1_000), Transition::Same);
        assert_eq!(a.view().kind, "shell");

        // `claude` typed in the shell.
        a.screen_state = Some(pitwall_detect::Detected::Idle);
        assert_eq!(apply(&mut a, Some((42, rec("claude", None))), 20, 2_000), Transition::Entered);
        let v = a.view();
        assert_eq!((v.kind.as_str(), v.kind_name.as_str(), v.terminal), ("claude", "Claude Code", true));
        assert_eq!((a.status, a.screen_state, a.detect_seq), (Status::Unknown, None, u64::MAX), "status starts over");
        assert_eq!(a.inner.as_ref().unwrap().noticed_at, 2_000);
        assert_eq!(a.rec.kind, "shell", "the record stays a terminal: a restart starts the shell");

        // Still running: nothing changes, an id from the args fills in.
        assert_eq!(apply(&mut a, Some((42, rec("claude", Some("s1")))), 30, 3_000), Transition::Learned);
        assert_eq!(apply(&mut a, Some((42, rec("claude", Some("s1")))), 35, 3_500), Transition::Same);
        assert_eq!(a.view().session_id.as_deref(), Some("s1"));

        // Exited: a shell again, same agent.
        a.hook = Some((HookState::Working, None));
        assert_eq!(apply(&mut a, None, 40, 4_000), Transition::Left);
        let v = a.view();
        assert_eq!((v.id.as_str(), v.kind.as_str(), v.kind_name.as_str()), ("t", "shell", "Shell"));
        assert_eq!((a.hook, a.status), (None, Status::Unknown));
        assert_eq!(v.session_id, None);
    }

    fn kinds() -> Vec<AgentKind> {
        crate::kind::load_kinds(std::path::Path::new("/nonexistent/pitwall-agents"))
    }

    /// The remembered agent as (kind, session id, pid, left at).
    type Remembered = (String, Option<String>, Option<u32>, Option<u64>);

    fn remembered(a: &Agent) -> Option<Remembered> {
        a.rec.inner_agent.as_ref().map(|m| (m.kind.clone(), m.session_id.clone(), m.pid, m.left_at))
    }

    #[test]
    fn the_agent_is_remembered_after_it_exits_until_the_shell_is_used_without_it() {
        let mut a = terminal();
        a.facts.provider = crate::testing::FakeProvider::local_like();
        let kinds = kinds();
        assert_eq!((a.view().restart_as, a.caps().resume), (None, false));

        assert_eq!(observe(&mut a, Some((42, rec("claude", None))), 1, 1_000, &kinds), Transition::Entered);
        assert_eq!(remembered(&a), Some(("claude".into(), None, Some(42), None)));
        assert_eq!(a.view().restart_as.as_deref(), Some("Claude Code"));
        assert!(!a.caps().resume, "no conversation known yet: a restart starts it fresh");
        observe(&mut a, Some((42, rec("claude", Some("s1")))), 2, 2_000, &kinds);
        assert_eq!(remembered(&a).unwrap().1.as_deref(), Some("s1"));
        assert!(a.caps().resume);

        // The agent exits: the pane is a shell, but still restarts as it.
        assert_eq!(apply(&mut a, None, 3, 10_000), Transition::Left);
        assert_eq!(a.view().kind, "shell");
        assert_eq!(remembered(&a), Some(("claude".into(), Some("s1".into()), None, Some(10_000))));
        assert!(a.caps().resume && a.view().restart_as.is_some());

        // Left at the prompt for a long time: still remembered.
        assert_eq!(apply(&mut a, None, 4, 10_000 + 10 * FORGET_AFTER_MS), Transition::Same);
        // The shell is used, but only just after the agent left.
        a.shell_used = true;
        assert_eq!(apply(&mut a, None, 5, 10_000 + FORGET_AFTER_MS - 1), Transition::Same);
        assert!(a.rec.inner_agent.is_some());
        // …and for a while: a plain shell again.
        assert_eq!(apply(&mut a, None, 6, 10_000 + FORGET_AFTER_MS), Transition::Forgot);
        assert_eq!((a.rec.inner_agent.clone(), a.facts.inner, a.shell_used), (None, None, false));
        assert_eq!((a.view().restart_as, a.caps().resume), (None, false));
    }

    #[test]
    fn starting_another_agent_replaces_the_remembered_one() {
        let mut a = terminal();
        a.facts.provider = crate::testing::FakeProvider::local_like();
        let kinds = kinds();
        observe(&mut a, Some((42, rec("claude", Some("s1")))), 1, 1, &kinds);
        apply(&mut a, None, 2, 2);
        a.shell_used = true;
        assert_eq!(observe(&mut a, Some((43, rec("codex", None))), 3, 3, &kinds), Transition::Entered);
        assert_eq!(remembered(&a), Some(("codex".into(), None, Some(43), None)));
        assert!(!a.shell_used);
        assert_eq!(a.view().restart_as.as_deref(), Some("Codex"));
    }

    #[test]
    fn the_same_process_keeps_its_conversation_across_a_pitwall_restart() {
        let mut a = terminal();
        apply(&mut a, Some((42, rec("claude", Some("s1")))), 1, 1);
        // Pitwall restarts: the record comes back from disk, the poller sees
        // the agent again (its arguments don't name the conversation).
        let mut again = Agent::new(a.rec.clone(), None, 1);
        assert_eq!(apply(&mut again, Some((42, rec("claude", None))), 2, 2), Transition::Entered);
        assert_eq!(again.view().session_id.as_deref(), Some("s1"));
        // A different process of the same kind is a different conversation.
        let mut other = Agent::new(a.rec.clone(), None, 1);
        apply(&mut other, Some((50, rec("claude", None))), 2, 2);
        assert_eq!((other.view().session_id, remembered(&other).unwrap().1), (None, None));
    }

    #[test]
    fn hooks_name_the_remembered_conversation() {
        let mut a = terminal();
        apply(&mut a, Some((7, rec("claude", None))), 1, 1);
        let persist = a.apply_hook(HookEffect { session_id: Some("h1".into()), ..Default::default() });
        assert!(persist, "the remembered agent is on the record");
        assert_eq!(remembered(&a).unwrap().1.as_deref(), Some("h1"));
        assert!(!a.apply_hook(HookEffect { session_id: Some("h1".into()), ..Default::default() }));
    }

    #[test]
    fn another_agent_replaces_the_first() {
        let mut a = terminal();
        apply(&mut a, Some((42, rec("claude", Some("s1")))), 1, 1);
        assert_eq!(apply(&mut a, Some((43, rec("codex", None))), 2, 2), Transition::Entered);
        let inner = a.inner.as_ref().unwrap();
        assert_eq!((inner.kind.as_str(), inner.pid, inner.session_id.as_deref()), ("codex", 43, None));
        // Same kind, new process (quit and started again): a new run too.
        assert_eq!(apply(&mut a, Some((44, rec("codex", None))), 3, 3), Transition::Entered);
    }

    #[test]
    fn hooks_in_a_terminal_name_the_inner_conversation() {
        let mut a = terminal();
        let start = |id: &str| HookEffect { session_id: Some(id.into()), state: Some(HookState::Idle), session_start: true, ..Default::default() };
        // SessionStart can arrive before the poller sees the process.
        a.apply_hook(start("h1"));
        assert_eq!(a.rec.session_id, None, "a terminal's own record has no conversation");
        apply(&mut a, Some((7, rec("codex", None))), 1, 1);
        assert_eq!(a.view().session_id.as_deref(), Some("h1"));
        a.apply_hook(start("h2"));
        assert_eq!(a.inner.as_ref().unwrap().session_id.as_deref(), Some("h2"));
        apply(&mut a, None, 2, 2);
        apply(&mut a, Some((8, rec("codex", None))), 3, 3);
        assert_eq!(a.view().session_id, None, "an exited agent's id isn't reused");
    }

    #[test]
    fn non_terminals_keep_hook_sessions_on_the_record() {
        let mut a = Agent::new(record("c", "/w"), None, 1);
        a.rec.kind = "claude".into();
        a.apply_hook(HookEffect { session_id: Some("s".into()), ..Default::default() });
        assert_eq!(a.rec.session_id.as_deref(), Some("s"));
    }

    #[test]
    fn polling_backs_off_while_quiet_and_speeds_up_on_output() {
        let mut w = Watch::default();
        assert!(w.due(5, 1_000), "first look right away");
        assert!(!w.due(5, 1_500));
        // Quiet: then 1 s, 2 s, 4 s, 8 s, 8 s…
        assert!(w.due(5, 2_000));
        assert!(!w.due(5, 3_999));
        assert!(w.due(5, 4_000));
        assert!(!w.due(5, 7_999));
        assert!(w.due(5, 8_000));
        assert!(!w.due(5, 15_999));
        assert!(w.due(5, 16_000));
        assert_eq!(w.gap, MAX_GAP_MS);
        assert!(!w.due(5, 23_999));
        assert!(w.due(5, 24_000));
        // Output flows: due one second after the last look, backoff reset.
        assert!(!w.due(6, 24_500));
        assert!(w.due(6, 25_000));
        assert_eq!(w.gap, MIN_GAP_MS);
        assert!(w.due(7, 26_000));
    }

    fn wait_until(f: impl Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !f() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        f()
    }

    fn shell_request(dir: &str) -> crate::model::CreateAgentRequest {
        serde_json::from_value(serde_json::json!({ "name": "t", "kind": "shell", "projectPath": dir })).unwrap()
    }

    #[test]
    fn a_terminal_restarts_as_the_agent_last_started_in_it() {
        use crate::engine::{input, lifecycle};
        let h = crate::testing::Harness::new(vec![]);
        let dir = h.dir.path().to_string_lossy().into_owned();
        let v = lifecycle::create(&h.engine, shell_request(&dir)).unwrap();
        assert_eq!(h.provider.launched()[0].command_line, "$SHELL");
        let kinds = kinds();
        h.engine.with(&v.id, |a| observe(a, Some((42, rec("claude", Some("s1")))), 1, 1_000, &kinds)).unwrap();
        h.engine.with(&v.id, |a| apply(a, None, 2, 2_000)).unwrap();
        // Back at the prompt; Enter there marks the shell as in use.
        input::write_input(&h.engine, &v.id, "\r").unwrap();
        assert!(h.engine.with(&v.id, |a| a.shell_used).unwrap());
        lifecycle::stop(&h.engine, &v.id).unwrap();
        assert!(wait_until(|| !h.engine.views()[0].running));
        let v = h.engine.views().remove(0);
        assert_eq!((v.kind.as_str(), v.restart_as.as_deref(), v.caps.resume), ("shell", Some("Claude Code"), true));

        // Restart: the shell runs `claude --resume s1` (hooks wired), then
        // itself again when Claude exits.
        let v = lifecycle::restart(&h.engine, &v.id, None).unwrap();
        let l = h.provider.launched().pop().unwrap();
        assert_eq!(l.kind, "shell");
        assert!(l.command_line.starts_with("claude --resume s1 --settings '"), "{}", l.command_line);
        assert!(l.command_line.ends_with("; exec $SHELL"), "{}", l.command_line);
        assert!(v.running);
        let r = h.engine.records().remove(0);
        let m = r.inner_agent.unwrap();
        assert_eq!((m.session_id.as_deref(), m.pid, m.left_at), (Some("s1"), None, None));
        assert!(!h.engine.with(&v.id, |a| a.shell_used).unwrap());
        assert_eq!((r.kind.as_str(), r.session_id), ("shell", None), "still a terminal");
        // Survives a save and reload (state.json v2 with an optional field).
        h.engine.save().unwrap();
        assert_eq!(crate::store::Store::load(&*h.store).agents[0].inner_agent.as_ref().and_then(|m| m.session_id.as_deref()), Some("s1"));
    }

    #[test]
    fn without_a_conversation_the_agent_starts_fresh_in_the_shell() {
        use crate::engine::lifecycle;
        let h = crate::testing::Harness::new(vec![]);
        let dir = h.dir.path().to_string_lossy().into_owned();
        let v = lifecycle::create(&h.engine, shell_request(&dir)).unwrap();
        let kinds = kinds();
        h.engine.with(&v.id, |a| observe(a, Some((42, rec("codex", None))), 1, 1_000, &kinds)).unwrap();
        let v = h.engine.views().remove(0);
        assert_eq!((v.restart_as.as_deref(), v.caps.resume), (Some("Codex"), false));
        lifecycle::restart(&h.engine, &v.id, None).unwrap();
        assert_eq!(h.provider.launched().pop().unwrap().command_line, "codex; exec $SHELL");

        // Claude gets its conversation id up front, which is remembered.
        h.engine.with(&v.id, |a| observe(a, Some((43, rec("claude", None))), 2, 2_000, &kinds)).unwrap();
        h.provider.set_caps(ProviderCaps { resume: false, ..crate::testing::FakeProvider::local_like() });
        lifecycle::restart(&h.engine, &v.id, None).unwrap();
        let line = h.provider.launched().pop().unwrap().command_line;
        let sid = h.engine.records()[0].inner_agent.as_ref().unwrap().session_id.clone().unwrap();
        assert!(line.starts_with(&format!("claude --session-id {sid} ")), "{line}");
    }

    #[test]
    fn providers_that_run_no_command_lines_restart_a_plain_shell() {
        use crate::engine::lifecycle;
        let h = crate::testing::Harness::new(vec![]);
        let dir = h.dir.path().to_string_lossy().into_owned();
        let v = lifecycle::create(&h.engine, shell_request(&dir)).unwrap();
        h.provider.set_caps(ProviderCaps { custom_command: false, ..crate::testing::FakeProvider::local_like() });
        lifecycle::restart(&h.engine, &v.id, None).unwrap(); // facts follow the new caps
        let kinds = kinds();
        h.engine.with(&v.id, |a| observe(a, Some((42, rec("claude", Some("s1")))), 1, 1_000, &kinds)).unwrap();
        let v = h.engine.views().remove(0);
        assert_eq!((v.restart_as, v.caps.resume), (None, false));
        lifecycle::restart(&h.engine, &v.id, None).unwrap();
        assert_eq!(h.provider.launched().pop().unwrap().command_line, "$SHELL");
    }

    #[test]
    fn owned_sessions_and_pids_include_agents_in_terminals() {
        let h = crate::testing::Harness::new(vec![record("t", "/w"), {
            let mut r = record("c", "/w");
            r.session_id = Some("rec-1".into());
            r
        }]);
        h.engine.with("t", |a| apply(a, Some((77, rec("claude", Some("in-1")))), 1, 1)).unwrap();
        let s = h.engine.owned_sessions();
        assert!(s.contains("rec-1") && s.contains("in-1"));
        let p = h.engine.owned_pids();
        assert!(p.contains(&77) && p.contains(&std::process::id()));
    }
}
