//! Background loops: the status ticker (~400ms) and the throttled
//! `agents-changed` emitter / state saver.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tauri::Emitter;

use pitwall_detect::{self as detect, Detection};
use crate::model::{mono_ms, Attention, Source, Status};
use crate::registry::{note_sent, Shared};
use crate::session::Session;
use crate::status::{next_status, raw_state, ACTIVITY_WINDOW_MS};
use crate::{attention, git, persist};

const TICK: Duration = Duration::from_millis(400);
const EMIT_GAP: Duration = Duration::from_millis(250);
const AUTO_SEND_SETTLE_MS: u64 = 800;
const AUTO_SEND_QUIET_INPUT_MS: u64 = 2500;
const AUTO_SEND_COOLDOWN_MS: u64 = 3000;
const GIT_EVERY_MS: u64 = 3000;

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
            if view {
                let _ = core.app.emit("agents-changed", core.views());
            }
            if save {
                if let Err(e) = persist::save(core.records()) {
                    eprintln!("pitwall: could not save state: {e}");
                }
            }
            std::thread::sleep(EMIT_GAP);
        })
        .expect("spawn emitter");
}

struct Probe {
    id: String,
    session: Arc<Session>,
    exited: bool,
    detection: Option<(Detection, u64)>,
    recent_output: bool,
}

fn probe(id: String, session: Arc<Session>, kind: &str, detect_seq: u64, now: u64) -> Probe {
    let exited = session.poll_exit();
    let seq = session.output_seq.load(Ordering::Relaxed);
    let detection = (!exited && seq != detect_seq).then(|| {
        let (text, title) = {
            let screen = session.screen.lock().unwrap_or_else(|e| e.into_inner());
            (screen.text(), screen.title())
        };
        (detect::detect(kind, &text, title.as_deref()), seq)
    });
    let last = session.last_activity.load(Ordering::Relaxed);
    Probe {
        id,
        exited,
        detection,
        recent_output: last > 0 && now.saturating_sub(last) < ACTIVITY_WINDOW_MS,
        session,
    }
}

struct GitJob {
    id: String,
    cwd: String,
    base: Option<String>,
    seq: u64,
}

fn tick(core: &Shared, badge: &mut usize) {
    let now = mono_ms();
    // 1. Snapshot what to look at, then do the slow parts unlocked.
    let targets: Vec<_> = core
        .agents()
        .iter()
        .filter_map(|a| {
            let s = a.session.clone()?;
            Some((a.rec.id.clone(), s, a.rec.kind.clone(), a.detect_seq))
        })
        .collect();
    let probes: Vec<Probe> = targets
        .into_iter()
        .map(|(id, s, kind, seq)| probe(id, s, &kind, seq, now))
        .collect();

    // 2. Fold results in under the lock.
    let mut changed = false;
    let mut persist = false;
    let mut alerts = Vec::new();
    let mut sends = Vec::new();
    let mut jobs = Vec::new();
    let mut task_jobs = Vec::new();
    let blocked;
    let wt_probes;
    {
        let mut agents = core.agents();
        for p in probes {
            let Some(a) = agents.iter_mut().find(|a| a.rec.id == p.id) else { continue };
            if !a.session.as_ref().is_some_and(|s| Arc::ptr_eq(s, &p.session)) {
                continue;
            }
            if let Some((d, seq)) = p.detection {
                a.screen_state = d.state;
                a.screen_detail = d.detail;
                a.detect_seq = seq;
            }
            let prev = (a.status, a.source, a.detail.clone());
            if p.exited {
                a.status = Status::Exited;
                a.detail = None;
            } else {
                let (raw, source) = raw_state(a.hook.as_ref().map(|h| h.0), a.screen_state, p.recent_output);
                a.status = next_status(a.status, raw, &mut a.turn_active);
                a.source = source;
                a.detail = match source {
                    Source::Hooks => a.hook.as_ref().and_then(|h| h.1.clone()),
                    Source::Screen => a.screen_detail.clone(),
                    Source::Activity => None,
                };
            }
            if (a.status, a.source, a.detail.clone()) != prev {
                changed = true;
            }
            if a.status != prev.0 {
                task_jobs.extend(crate::tasks::on_status(a, prev.0, a.status));
                a.status_since = now;
                let reason = match a.status {
                    Status::Blocked => Some("blocked"),
                    Status::Done => Some("done"),
                    _ => None,
                };
                if let Some(reason) = reason {
                    alerts.push(Attention {
                        agent_id: a.rec.id.clone(),
                        name: a.rec.name.clone(),
                        reason,
                        detail: a.detail.clone(),
                    });
                }
            }

            // Auto-send the next queued prompt once the agent has settled.
            let last_input = p.session.last_input.load(Ordering::Relaxed);
            if !p.exited
                && a.rec.auto_send
                && !a.rec.queue.is_empty()
                && matches!(a.status, Status::Done | Status::Idle)
                && now.saturating_sub(a.status_since) >= AUTO_SEND_SETTLE_MS
                && now.saturating_sub(last_input) >= AUTO_SEND_QUIET_INPUT_MS
                && now.saturating_sub(a.last_auto_send) >= AUTO_SEND_COOLDOWN_MS
            {
                let item = a.rec.queue.remove(0);
                note_sent(a, &item.text);
                task_jobs.extend(crate::tasks::begin(a, Some(&item.text), true));
                sends.push((p.session.clone(), item.text));
                changed = true;
                persist = true;
            }
        }

        // Git refresh: wanted, or every 3s while active / after new output.
        for a in agents.iter_mut() {
            if a.git_inflight {
                continue;
            }
            let seq = a
                .session
                .as_ref()
                .map(|s| s.output_seq.load(Ordering::Relaxed))
                .unwrap_or(a.git_seq);
            let due = now.saturating_sub(a.git_at) >= GIT_EVERY_MS
                && (a.status == Status::Working || seq != a.git_seq);
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
        blocked = agents.iter().filter(|a| a.status == Status::Blocked).count();
        wt_probes = crate::worktree::due(&mut agents, now);
    }

    // 3. Side effects, unlocked.
    if !task_jobs.is_empty() {
        persist = true;
        crate::tasks::submit(task_jobs);
    }
    for (session, text) in sends {
        session.send_text(text);
    }
    for job in jobs {
        let core = core.clone();
        std::thread::spawn(move || refresh_git(&core, job));
    }
    for probe in wt_probes {
        let core = core.clone();
        std::thread::spawn(move || {
            let found = crate::worktree::discover(&probe);
            crate::worktree::apply(&core, &probe.id, found);
        });
    }
    if changed || persist {
        core.changed(persist);
    }
    attention::raise(&core.app, alerts);
    if blocked != *badge {
        *badge = blocked;
        attention::set_badge(&core.app, blocked);
    }
}

fn refresh_git(core: &Shared, job: GitJob) {
    let changes = git::changes(&job.cwd, job.base.as_deref());
    let branch = git::current_branch(&job.cwd);
    let differs = core
        .with(&job.id, |a| {
            a.git_inflight = false;
            a.git_at = mono_ms();
            a.git_seq = job.seq;
            let before = (a.branch.clone(), a.added, a.removed, a.files_changed);
            a.branch = branch;
            if let Ok(files) = changes {
                a.added = files.iter().map(|f| f.added).sum();
                a.removed = files.iter().map(|f| f.removed).sum();
                a.files_changed = files.len() as u32;
            }
            before != (a.branch.clone(), a.added, a.removed, a.files_changed)
        })
        .unwrap_or(false);
    if differs {
        core.changed(false);
    }
}
