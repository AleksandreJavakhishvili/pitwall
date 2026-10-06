//! Agents started by hand in a Pitwall terminal (docs/spec/terminals.md §2).
//!
//! A terminal is an agent of the shell kind. When the user types `claude`,
//! `codex`, … in it, the terminal's foreground process group shows that
//! agent; the pane then *is* that agent (kind, status rules, conversation)
//! until it exits, and the same Pitwall agent id and tile stay throughout.
//!
//! Polling is cheap: one batched `ps` for every terminal that is due, about
//! once a second while output flows and backing off to every few seconds
//! when a terminal is quiet. The registry lock is never held during `ps`,
//! `lsof` or transcript reads.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::{Agent, Engine, Shared};
use crate::model::{Source, Status};
use crate::onboarding::latest;
use crate::platform;
use crate::procs::{self, Matcher, Recognised};
use crate::session::Session;

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
const LSOF_TIMEOUT: Duration = Duration::from_secs(2);

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
    /// The agent exited; the terminal is a shell again.
    Left,
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
    match (seen, &mut a.inner) {
        (None, None) => Transition::Same,
        (None, Some(_)) => {
            a.inner = None;
            a.hook_session = None;
            reset_signals(a, now);
            Transition::Left
        }
        (Some((pid, r)), Some(inner)) if inner.pid == pid && inner.kind == r.kind => {
            if inner.session_id.is_none() {
                inner.session_id = r.session_id;
            }
            Transition::Same
        }
        (Some((pid, r)), _) => {
            // A conversation id from a hook that arrived first counts too.
            let session_id = r.session_id.or_else(|| a.hook_session.clone());
            a.inner = Some(Inner {
                kind: r.kind,
                kind_name: r.kind_name,
                pid,
                session_id,
                noticed_at: unix_now,
                lookup_at: now,
                lookups_left: LOOKUPS,
            });
            reset_signals(a, now);
            Transition::Entered
        }
    }
}

/// Start the foreground poller.
pub fn start(core: Shared) {
    std::thread::Builder::new()
        .name("terminals".into())
        .spawn(move || {
            let mut matcher: Option<(u64, Matcher)> = None;
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
    session: Arc<Session>,
    shell: u32,
}

struct Lookup {
    id: String,
    kind: String,
    pid: u32,
    cwd: String,
    since: u64,
}

/// One round: look at the terminals that are due, then find conversations
/// of agents that don't have one yet.
fn poll(core: &Shared, matcher: &mut Option<(u64, Matcher)>) {
    let now = core.now();
    let targets: Vec<Target> = core
        .agents()
        .iter_mut()
        .filter(|a| a.rec.kind == TERMINAL_KIND)
        .filter_map(|a| {
            let session = a.session.clone()?;
            let shell = session.pid?;
            if session.exited.load(Ordering::Relaxed) {
                return None;
            }
            let seq = session.output_seq.load(Ordering::Relaxed);
            a.term.due(seq, now).then(|| Target { id: a.rec.id.clone(), session, shell })
        })
        .collect();
    let mut changed = false;
    if !targets.is_empty() {
        if matcher.as_ref().is_none_or(|(at, _)| now.saturating_sub(*at) >= MATCHER_TTL_MS) {
            *matcher = Some((now, Matcher::new(&core.kinds().kinds())));
        }
        let m = &matcher.as_ref().expect("set above").1;
        if let Some(seen) = look(&targets, m) {
            let unix_now = crate::clock::unix_ms();
            let mut agents = core.agents();
            for (t, fg) in targets.iter().zip(seen) {
                let Some(a) = agents.iter_mut().find(|a| a.rec.id == t.id) else { continue };
                if !a.session.as_ref().is_some_and(|s| Arc::ptr_eq(s, &t.session)) {
                    continue;
                }
                changed |= apply(a, fg, now, unix_now) != Transition::Same;
            }
        }
    }
    let lookups = due_lookups(core, now);
    if !lookups.is_empty() {
        changed |= find_conversations(core, lookups);
    }
    if changed {
        core.changed(false);
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
                kind: inner.kind.clone(),
                pid: inner.pid,
                cwd,
                since: inner.noticed_at.saturating_sub(LOOKUP_SLACK_MS),
            })
        })
        .collect()
}

/// The newest conversation for each agent's folder (where the process really
/// is: the user may have `cd`ed since the terminal opened). Unlocked I/O.
fn find_conversations(core: &Engine, lookups: Vec<Lookup>) -> bool {
    let pids: Vec<u32> = lookups.iter().map(|l| l.pid).collect();
    let cwds: HashMap<u32, String> = platform::process_cwds(&pids, LSOF_TIMEOUT)
        .map(|v| v.into_iter().collect())
        .unwrap_or_default();
    let home = crate::paths::home();
    let found: Vec<(Lookup, String)> = lookups
        .into_iter()
        .filter_map(|l| {
            let cwd = cwds.get(&l.pid).cloned().unwrap_or_else(|| l.cwd.clone());
            let s = latest::newest(&home, &l.kind, &cwd, Some(l.since))?;
            Some((l, s.session_id))
        })
        .collect();
    let mut changed = false;
    let mut agents = core.agents();
    for (l, session_id) in found {
        let Some(inner) = agents.iter_mut().find(|a| a.rec.id == l.id).and_then(|a| a.inner.as_mut()) else { continue };
        if inner.pid == l.pid && inner.session_id.is_none() {
            inner.session_id = Some(session_id);
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
            .flat_map(|a| [a.rec.session_id.clone(), a.inner.as_ref().and_then(|i| i.session_id.clone())])
            .flatten()
            .collect()
    }

    /// Processes whose descendants belong to Pitwall: this process and every
    /// agent's terminal (holders are detached from the app).
    pub fn owned_pids(&self) -> HashSet<u32> {
        let mut pids: HashSet<u32> = self
            .agents()
            .iter()
            .flat_map(|a| [a.session.as_ref().and_then(|s| s.pid), a.inner.as_ref().map(|i| i.pid)])
            .flatten()
            .collect();
        pids.insert(std::process::id());
        pids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HookEffect, HookState};
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
        assert_eq!(apply(&mut a, Some((42, rec("claude", Some("s1")))), 30, 3_000), Transition::Same);
        assert_eq!(a.view().session_id.as_deref(), Some("s1"));

        // Exited: a shell again, same agent.
        a.hook = Some((HookState::Working, None));
        assert_eq!(apply(&mut a, None, 40, 4_000), Transition::Left);
        let v = a.view();
        assert_eq!((v.id.as_str(), v.kind.as_str(), v.kind_name.as_str()), ("t", "shell", "Shell"));
        assert_eq!((a.hook, a.status), (None, Status::Unknown));
        assert_eq!(v.session_id, None);
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
