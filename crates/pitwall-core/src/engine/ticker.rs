//! Background loops: the status ticker (~400ms) and the throttled
//! `AgentsChanged` emitter / state saver.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use pitwall_detect::{self as detect, Detection};

use super::status::{next_status, raw_state, ACTIVITY_WINDOW_MS};
use super::tasks::{self, Job};
use super::{worktree, Agent, Engine, Shared};
use crate::events::Event;
use crate::model::{Attention, Source, Status};
use crate::term::TermHost;
use crate::vcs::git::{FileChange, Git, NOT_A_REPO};

const TICK: Duration = Duration::from_millis(400);
const EMIT_GAP: Duration = Duration::from_millis(250);
const AUTO_SEND_SETTLE_MS: u64 = 800;
const AUTO_SEND_QUIET_INPUT_MS: u64 = 2500;
const AUTO_SEND_COOLDOWN_MS: u64 = 3000;
const GIT_EVERY_MS: u64 = 3000;
/// Git polling backs off (doubling) while refreshes find nothing new, up to this.
const GIT_MAX_EVERY_MS: u64 = 30_000;

pub fn start(core: Shared) {
    let c = core.clone();
    std::thread::Builder::new()
        .name("ticker".into())
        .spawn(move || {
            let mut badge = usize::MAX;
            loop {
                tick(&c, &mut badge);
                std::thread::sleep(TICK);
            }
        })
        .expect("spawn ticker");
    std::thread::Builder::new()
        .name("emitter".into())
        .spawn(move || loop {
            let (view, save) = core.wait_dirty();
            flush(&core, view, save);
            std::thread::sleep(EMIT_GAP);
        })
        .expect("spawn emitter");
}

/// Tell the host what changed and/or save the records.
fn flush(core: &Engine, view: bool, save: bool) {
    if view {
        core.emit(Event::AgentsChanged(core.views()));
    }
    if save {
        if let Err(e) = core.save() {
            eprintln!("pitwall: could not save state: {e}");
        }
    }
}

/// What the ticker read from an agent's terminal (without the registry lock).
#[derive(Debug, Clone, Default)]
struct Observation {
    exited: bool,
    /// Screen rules ran on new output (with the output seq they saw).
    detection: Option<(Detection, u64)>,
    recent_output: bool,
    /// Last user keystroke / resize (mono ms).
    last_input: u64,
}

struct Probe {
    id: String,
    session: Arc<TermHost>,
    obs: Observation,
}

fn probe(id: String, session: Arc<TermHost>, kind: &str, detect_seq: u64, now: u64) -> Probe {
    let exited = session.ended();
    let seq = session.output_seq.load(Ordering::Relaxed);
    let detection = (!exited && seq != detect_seq).then(|| {
        let (text, title) = session.screen_text();
        (detect::detect(kind, &text, title.as_deref()), seq)
    });
    let last = session.last_activity.load(Ordering::Relaxed);
    let obs = Observation {
        exited,
        detection,
        recent_output: last > 0 && now.saturating_sub(last) < ACTIVITY_WINDOW_MS,
        last_input: session.last_input.load(Ordering::Relaxed),
    };
    Probe { id, session, obs }
}

/// What folding one observation into an agent asks for.
#[derive(Debug, Default)]
struct Folded {
    changed: bool,
    persist: bool,
    alert: Option<Attention>,
    /// A queued prompt to send now (auto-send).
    send: Option<String>,
    task_jobs: Vec<Job>,
}

/// Update `a`'s status from `obs` at `now`; decide on attention and auto-send.
/// Pure bookkeeping (under the registry lock): no I/O.
fn fold(a: &mut Agent, obs: Observation, now: u64) -> Folded {
    let mut out = Folded::default();
    if let Some((d, seq)) = obs.detection {
        a.screen_state = d.state;
        a.screen_detail = d.detail;
        a.detect_seq = seq;
    }
    let prev = (a.status, a.source, a.detail.clone());
    if obs.exited {
        a.status = Status::Exited;
        a.detail = None;
    } else {
        let (raw, source) = raw_state(a.hook.as_ref().map(|h| h.0), a.screen_state, obs.recent_output);
        a.status = next_status(a.status, raw, &mut a.turn_active);
        a.source = source;
        a.detail = match source {
            Source::Hooks => a.hook.as_ref().and_then(|h| h.1.clone()),
            Source::Screen => a.screen_detail.clone(),
            Source::Activity => None,
        };
    }
    if (a.status, a.source, a.detail.clone()) != prev {
        out.changed = true;
    }
    if a.status != prev.0 {
        let ended = tasks::on_status(a, prev.0, a.status);
        // What the task changed shows up right away, even where git is
        // polled rarely (`ProviderCaps::git_poll_ms`).
        a.git_wanted |= ended.iter().any(|j| matches!(j, Job::End { .. }));
        // A new turn of work: poll at the normal pace again (git backoff).
        if a.status == Status::Working {
            a.git_every = 0;
        }
        out.task_jobs.extend(ended);
        a.status_since = now;
        let reason = match a.status {
            Status::Blocked => Some("blocked"),
            Status::Done => Some("done"),
            _ => None,
        };
        out.alert = reason.map(|reason| Attention {
            agent_id: a.rec.id.clone(),
            name: a.rec.name.clone(),
            reason,
            detail: a.detail.clone(),
        });
    }

    // Auto-send the next queued prompt once the agent has settled.
    if !obs.exited
        && a.rec.auto_send
        && !a.rec.queue.is_empty()
        && matches!(a.status, Status::Done | Status::Idle)
        && now.saturating_sub(a.status_since) >= AUTO_SEND_SETTLE_MS
        && now.saturating_sub(obs.last_input) >= AUTO_SEND_QUIET_INPUT_MS
        && now.saturating_sub(a.last_auto_send) >= AUTO_SEND_COOLDOWN_MS
    {
        let item = a.rec.queue.remove(0);
        a.note_sent(&item.text, now);
        out.task_jobs.extend(tasks::begin(a, Some(&item.text), true));
        out.send = Some(item.text);
        out.changed = true;
        out.persist = true;
    }
    out
}

pub(super) struct GitJob {
    id: String,
    cwd: String,
    base: Option<String>,
    seq: u64,
}

fn tick(core: &Shared, badge: &mut usize) {
    let now = core.now();
    // 1. Snapshot what to look at, then do the slow parts unlocked.
    // A dropped attachment (remote) isn't an exit: attach again first.
    let mut relinks = Vec::new();
    let targets: Vec<_> = core
        .agents()
        .iter_mut()
        .filter_map(|a| {
            let s = a.host.clone()?;
            if s.ended() && !s.eof_is_exit() {
                if !a.relinking {
                    a.relinking = true;
                    relinks.push(a.rec.id.clone());
                }
                return None;
            }
            Some((a.rec.id.clone(), s, a.kind().to_string(), a.detect_seq))
        })
        .collect();
    for id in relinks {
        let core = core.clone();
        std::thread::spawn(move || super::lifecycle::relink(&core, &id));
    }
    let probes: Vec<Probe> = targets
        .into_iter()
        .map(|(id, s, kind, seq)| probe(id, s, &kind, seq, now))
        .collect();

    // 2. Fold results in under the lock.
    let mut changed = false;
    let mut persist = false;
    let mut alerts = Vec::new();
    let mut sends = Vec::new();
    let jobs;
    let mut task_jobs = Vec::new();
    let blocked;
    let wt_probes;
    {
        let mut agents = core.agents();
        for p in probes {
            let Some(a) = agents.iter_mut().find(|a| a.rec.id == p.id) else { continue };
            if !a.host.as_ref().is_some_and(|s| Arc::ptr_eq(s, &p.session)) {
                continue;
            }
            let f = fold(a, p.obs, now);
            changed |= f.changed;
            persist |= f.persist;
            alerts.extend(f.alert);
            task_jobs.extend(f.task_jobs);
            if let Some(text) = f.send {
                sends.push((p.session.clone(), text));
            }
        }
        jobs = git_due(&mut agents, now);
        blocked = agents.iter().filter(|a| a.status == Status::Blocked).count();
        wt_probes = worktree::due(&mut agents, now);
    }

    // 3. Side effects, unlocked.
    if !task_jobs.is_empty() {
        persist = true;
        core.submit(task_jobs);
    }
    for (session, text) in sends {
        session.send_text(text);
    }
    for job in jobs {
        let core = core.clone();
        std::thread::spawn(move || {
            let _ = refresh_git(&core, job);
        });
    }
    for probe in wt_probes {
        let core = core.clone();
        std::thread::spawn(move || {
            let cwd = worktree::process_cwd(&core, &probe);
            let found = worktree::discover(&*core.exec_for(&probe.id), &probe, cwd);
            worktree::apply(&core, &probe.id, found);
        });
    }
    if changed || persist {
        core.changed(persist);
    }
    for alert in alerts {
        core.emit(Event::Attention(alert));
    }
    if blocked != *badge {
        *badge = blocked;
        core.emit(Event::BlockedCount(blocked));
    }
}

/// Git refresh: wanted, or every 3s while active / after new output. Where
/// the provider asks for slower polling (`ProviderCaps::git_poll_ms`), only
/// while the agent works, at most that often. Never for an agent outside a
/// git repository, or whose machine can't run git. Idle agents (no new
/// output, not working) are never polled; active ones whose refreshes keep
/// finding nothing new back off to `GIT_MAX_EVERY_MS` (or the provider's
/// interval, if slower) until one finds a change.
fn git_due(agents: &mut [Agent], now: u64) -> Vec<GitJob> {
    let mut jobs = Vec::new();
    for a in agents.iter_mut() {
        if a.git_inflight || a.git_repo == Some(false) || !a.facts.provider.exec {
            continue;
        }
        let seq = a
            .host
            .as_ref()
            .map(|s| s.output_seq.load(Ordering::Relaxed))
            .unwrap_or(a.git_seq);
        let slow = a.facts.provider.git_poll_ms > 0;
        let every = a.git_every.max(git_base_ms(a));
        let active = a.status == Status::Working || (!slow && seq != a.git_seq);
        let due = now.saturating_sub(a.git_at) >= every && active;
        if a.git_wanted || due {
            a.git_wanted = false;
            a.git_inflight = true;
            jobs.push(GitJob {
                id: a.rec.id.clone(),
                cwd: a.rec.cwd.clone(),
                base: a.rec.base_commit.clone(),
                seq,
            });
        }
    }
    jobs
}

/// Which agents the ticker would refresh now (tests elsewhere in the engine).
#[cfg(test)]
pub(super) fn git_due_for_test(core: &Engine) -> Vec<String> {
    let now = core.now();
    git_due(&mut core.agents(), now).into_iter().map(|j| j.id).collect()
}

/// The provider's git interval, or the default.
fn git_base_ms(a: &Agent) -> u64 {
    match a.facts.provider.git_poll_ms {
        0 => GIT_EVERY_MS,
        ms => u64::from(ms),
    }
}

/// The next polling interval after a refresh: back to `base` when it found a
/// change, else twice as long, up to `GIT_MAX_EVERY_MS` (never below `base`).
fn next_git_every(prev: u64, base: u64, differs: bool) -> u64 {
    if differs {
        base
    } else {
        (prev.max(base) * 2).min(GIT_MAX_EVERY_MS.max(base))
    }
}

/// One git refresh for an agent: its changes and branch, folded into its
/// numbers and polling pace. A refresh already running for the agent (the
/// ticker's or a forced one) is awaited instead of run again. Blocking.
pub(super) fn refresh_git(core: &Engine, job: GitJob) -> Result<Vec<FileChange>, String> {
    let id = job.id.clone();
    let out = core.git_flights.run(&id, || run_git(core, job));
    // Also when this call only waited for another one: whoever asked marked
    // the agent in flight, and must never leave it so.
    let _ = core.with(&id, |a| a.git_inflight = false);
    out
}

fn run_git(core: &Engine, job: GitJob) -> Result<Vec<FileChange>, String> {
    let exec = core.exec_for(&job.id);
    let git = Git::new(&*exec, &job.cwd);
    let (changes, branch) = git.changes_and_branch(job.base.as_deref());
    let now = core.now();
    let differs = core
        .with(&job.id, |a| {
            a.git_inflight = false;
            a.git_at = now;
            a.git_seq = job.seq;
            let before = (a.branch.clone(), a.added, a.removed, a.files_changed, a.git_repo);
            a.branch = branch;
            match &changes {
                Ok(files) => {
                    a.git_repo = Some(true);
                    a.added = files.iter().map(|f| f.added).sum();
                    a.removed = files.iter().map(|f| f.removed).sum();
                    a.files_changed = files.len() as u32;
                }
                Err(e) if e == NOT_A_REPO => a.git_repo = Some(false),
                Err(_) => {}
            }
            let differs = before != (a.branch.clone(), a.added, a.removed, a.files_changed, a.git_repo);
            a.git_every = next_git_every(a.git_every, git_base_ms(a), differs);
            differs
        })
        .unwrap_or(false);
    if differs {
        core.changed(false);
    }
    changes
}

/// Refresh agent `id`'s git numbers now, whatever the polling pace (back-off,
/// a slow provider's interval, idle, outside a repository last time). One
/// already running is awaited. Emits `AgentsChanged` (throttled) when the
/// numbers moved; returns the changes. Blocking.
pub(super) fn refresh_git_now(core: &Engine, id: &str) -> Result<Vec<FileChange>, String> {
    let job = core.with(id, |a| {
        if !a.facts.provider.exec {
            return Err("Pitwall can't run git on this agent's machine.".to_string());
        }
        a.git_inflight = true;
        let seq = a.host.as_ref().map(|s| s.output_seq.load(Ordering::Relaxed)).unwrap_or(a.git_seq);
        Ok(GitJob { id: a.rec.id.clone(), cwd: a.rec.cwd.clone(), base: a.rec.base_commit.clone(), seq })
    })??;
    refresh_git(core, job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HookState;
    use crate::model::QueueItem;
    use crate::testing::{record, Harness};

    fn agent() -> Agent {
        let mut a = Agent::new(record("a", "/tmp"), None, 1);
        a.status = Status::Unknown;
        a.facts.provider.exec = true;
        a
    }

    fn live(last_input: u64) -> Observation {
        Observation { last_input, ..Default::default() }
    }

    #[test]
    fn blocked_and_done_raise_attention_once() {
        let mut a = agent();
        a.hook = Some((HookState::Blocked, Some("Permission: Edit".into())));
        let f = fold(&mut a, live(0), 10_000);
        assert_eq!((a.status, a.source, a.status_since), (Status::Blocked, Source::Hooks, 10_000));
        let alert = f.alert.expect("attention");
        assert_eq!((alert.reason, alert.detail.as_deref()), ("blocked", Some("Permission: Edit")));
        assert!(f.changed);
        // Same state again: nothing new.
        let f = fold(&mut a, live(0), 10_400);
        assert!(f.alert.is_none() && !f.changed);

        a.hook = Some((HookState::Done, None));
        assert_eq!(fold(&mut a, live(0), 11_000).alert.map(|x| x.reason), Some("done"));
    }

    #[test]
    fn exit_wins_over_every_signal() {
        let mut a = agent();
        a.hook = Some((HookState::Working, None));
        let f = fold(&mut a, Observation { exited: true, ..Default::default() }, 5_000);
        assert_eq!(a.status, Status::Exited);
        assert!(f.changed && f.alert.is_none());
    }

    #[test]
    fn auto_send_waits_for_the_agent_and_the_user_to_settle() {
        let mut a = agent();
        a.rec.queue.push(QueueItem { id: "q1".into(), text: "next please".into() });
        a.rec.queue.push(QueueItem { id: "q2".into(), text: "and then".into() });
        a.hook = Some((HookState::Idle, None));
        // Turns idle at t=10s: not settled yet.
        assert!(fold(&mut a, live(0), 10_000).send.is_none());
        assert!(fold(&mut a, live(0), 10_000 + AUTO_SEND_SETTLE_MS - 1).send.is_none());
        // Settled, but the user typed a moment ago.
        let typed = 10_000 + AUTO_SEND_SETTLE_MS;
        assert!(fold(&mut a, live(typed), typed + 100).send.is_none());
        // Quiet long enough: the first item goes, verbatim, and is recorded.
        let t = typed + AUTO_SEND_QUIET_INPUT_MS;
        let f = fold(&mut a, live(typed), t);
        assert_eq!(f.send.as_deref(), Some("next please"));
        assert!(f.persist && !f.task_jobs.is_empty());
        assert_eq!(a.rec.last_sent.as_deref(), Some("next please"));
        assert_eq!(a.rec.queue.len(), 1);
        // Cooldown before the next one, even though the agent looks idle.
        assert!(fold(&mut a, live(typed), t + AUTO_SEND_COOLDOWN_MS - 1).send.is_none());
        a.rec.auto_send = false;
        assert!(fold(&mut a, live(typed), t + AUTO_SEND_COOLDOWN_MS * 10).send.is_none());
    }

    #[test]
    fn tick_reports_blocked_count_changes() {
        let h = Harness::new(vec![record("a", "/tmp"), record("b", "/tmp")]);
        let mut badge = usize::MAX;
        tick(&h.engine, &mut badge);
        tick(&h.engine, &mut badge);
        h.engine.with("b", |a| a.status = Status::Blocked).unwrap();
        tick(&h.engine, &mut badge);
        let counts: Vec<usize> = h
            .events
            .take()
            .into_iter()
            .filter_map(|e| match e {
                Event::BlockedCount(n) => Some(n),
                _ => None,
            })
            .collect();
        assert_eq!(counts, vec![0, 1], "sent on change only");
    }

    #[test]
    fn flush_emits_views_and_saves() {
        let h = Harness::new(vec![record("a", "/tmp")]);
        flush(&h.engine, true, false);
        flush(&h.engine, false, true);
        let events = h.events.take();
        assert!(matches!(&events[..], [Event::AgentsChanged(v)] if v.len() == 1 && v[0].id == "a"));
        assert_eq!(h.store.saves(), 1);
    }

    #[test]
    fn git_refresh_is_due_when_wanted_or_active() {
        let mut agents = vec![agent()];
        assert_eq!(git_due(&mut agents, 100).len(), 1, "wanted at start");
        assert!(git_due(&mut agents, 200).is_empty(), "in flight");
        agents[0].git_inflight = false;
        agents[0].git_at = 200;
        assert!(git_due(&mut agents, 200 + GIT_EVERY_MS).is_empty(), "idle and no new output");
        agents[0].status = Status::Working;
        assert_eq!(git_due(&mut agents, 200 + GIT_EVERY_MS).len(), 1);
        agents[0].git_inflight = false;
        agents[0].git_repo = Some(false);
        agents[0].git_wanted = true;
        assert!(git_due(&mut agents, 400 + GIT_EVERY_MS * 2).is_empty(), "outside a repo: never");
        agents[0].git_repo = None;
        agents[0].facts.provider.exec = false;
        assert!(git_due(&mut agents, 400 + GIT_EVERY_MS * 2).is_empty(), "no commands on that machine: never");
    }

    #[test]
    fn slow_providers_are_polled_less_and_only_while_working() {
        let mut agents = vec![agent()];
        agents[0].facts.provider.git_poll_ms = 15_000;
        assert_eq!(git_due(&mut agents, 100).len(), 1, "wanted at start");
        agents[0].git_inflight = false;
        agents[0].git_at = 100;
        agents[0].git_seq = 0;
        agents[0].host = None;
        agents[0].status = Status::Working;
        assert!(git_due(&mut agents, 100 + GIT_EVERY_MS).is_empty(), "not after the default interval");
        assert_eq!(git_due(&mut agents, 100 + 15_000).len(), 1);
        agents[0].git_inflight = false;
        agents[0].git_at = 20_000;
        agents[0].status = Status::Idle;
        agents[0].git_seq = 7; // new output alone doesn't count for a slow provider
        assert!(git_due(&mut agents, 20_000 + 60_000).is_empty(), "idle: never on its own");
        // A turn that ends asks for one refresh.
        agents[0].rec.tasks.push(super::tasks::Task {
            id: "t".into(),
            prompt: String::new(),
            started_at: 0,
            ended_at: None,
            start_tree: None,
            end_tree: None,
        });
        agents[0].status = Status::Working;
        agents[0].turn_active = true;
        agents[0].hook = Some((HookState::Done, None));
        let f = fold(&mut agents[0], live(0), 90_000);
        assert!(!f.task_jobs.is_empty() && agents[0].git_wanted);
        assert_eq!(git_due(&mut agents, 90_001).len(), 1);
    }

    #[test]
    fn git_polling_backs_off_while_nothing_changes() {
        assert_eq!(next_git_every(0, GIT_EVERY_MS, false), 6_000);
        assert_eq!(next_git_every(6_000, GIT_EVERY_MS, false), 12_000);
        assert_eq!(next_git_every(24_000, GIT_EVERY_MS, false), GIT_MAX_EVERY_MS);
        assert_eq!(next_git_every(GIT_MAX_EVERY_MS, GIT_EVERY_MS, true), GIT_EVERY_MS, "a change resets it");
        // A slow provider's own interval is the floor.
        assert_eq!(next_git_every(0, 45_000, false), 45_000);
        assert_eq!(next_git_every(0, 15_000, true), 15_000);

        // Backed off: new output alone isn't due before the longer interval.
        let mut agents = vec![agent()];
        agents[0].git_wanted = false;
        agents[0].git_at = 1_000;
        agents[0].git_seq = 1;
        agents[0].git_every = 12_000;
        agents[0].status = Status::Working;
        assert!(git_due(&mut agents, 1_000 + GIT_EVERY_MS).is_empty());
        assert_eq!(git_due(&mut agents, 1_000 + 12_000).len(), 1);
    }

    #[test]
    fn git_polling_stops_outside_a_repository() {
        let x = crate::testing::FakeExec::new();
        x.on_exit(&["git", "-C", "/tmp/w", "diff", ".."], 128, "", "fatal: not a git repository (or any parent)")
            .on_exit(&["git", "-C", "/tmp/w", "symbolic-ref", ".."], 128, "", "fatal: not a git repository");
        let h = Harness::with_exec(vec![record("a", "/tmp/w")], x.clone());
        let jobs = git_due(&mut h.engine.agents(), 10_000);
        assert_eq!(jobs.len(), 1, "wanted once at start");
        for job in jobs {
            let _ = refresh_git(&h.engine, job);
        }
        let v = &h.engine.views()[0];
        assert!(!v.caps.diff && !v.caps.review);
        h.engine.with("a", |a| a.git_wanted = true).unwrap();
        assert!(git_due(&mut h.engine.agents(), 99_000).is_empty());
    }
}
