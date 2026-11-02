//! The window's one worktree list (React `src/lib/useWorktrees.ts`),
//! shared by the sidebar, the Changes panel and Review: read on first use,
//! every 30 s and a second after the agents' numbers settle while some
//! agent can have worktrees; forced (the engine's pace bypassed) by ↻,
//! ⌘⇧R and expanding a list. Each reader observes the entity.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, Global, Task, Window, WindowId};

use pitwall_core::Shared;
use pitwall_proto::{AgentView, ProjectWorktrees};

use crate::review::ops;

/// The list is asked for this often while some agent can have worktrees
/// (`LIST_EVERY_MS`).
const LIST_EVERY: Duration = Duration::from_secs(30);
/// After agents' numbers move, ask once things settle (`SETTLE_MS`).
const SETTLE: Duration = Duration::from_secs(1);

/// One window's list.
pub struct WorktreeList {
    engine: Option<Shared>,
    pub projects: Vec<ProjectWorktrees>,
    /// A forced read is running (the ↻ spins, "refreshing…").
    forcing: usize,
    /// When it was last read (ms since the epoch: "updated 12 s ago").
    pub updated_at: Option<i64>,
    /// The agents' numbers last seen (`agentsSignature`).
    sig: String,
    inflight: bool,
    again: bool,
    poll: Option<Task<()>>,
    settle: Option<Task<()>>,
    /// Whether each worktree has anything to review, by `wt_key`: (its
    /// HEAD when read, has changes, when read). Read for every listed
    /// worktree, again when its HEAD moves or after `WORK_EVERY_MS`, so the
    /// sidebar and Review fold the same ones.
    work: HashMap<String, (Option<String>, bool, i64)>,
    checking: bool,
}

/// A worktree's changes are read again at most this often unless its HEAD moves.
const WORK_EVERY_MS: i64 = 60_000;

#[derive(Default)]
struct Lists(HashMap<WindowId, Entity<WorktreeList>>);

impl Global for Lists {}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl WorktreeList {
    /// The list of `window` (made on first use).
    pub fn of(window: &Window, cx: &mut App) -> Entity<WorktreeList> {
        let id = window.window_handle().window_id();
        let open: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
        let lists = cx.default_global::<Lists>();
        lists.0.retain(|w, _| open.contains(w) || *w == id);
        if let Some(l) = lists.0.get(&id) {
            return l.clone();
        }
        let engine = cx
            .try_global::<super::EngineHandle>()
            .map(|e| e.0.clone());
        let list = cx.new(|_| WorktreeList {
            engine,
            projects: Vec::new(),
            forcing: 0,
            updated_at: None,
            sig: String::new(),
            inflight: false,
            again: false,
            poll: None,
            settle: None,
            work: HashMap::new(),
            checking: false,
        });
        cx.default_global::<Lists>().0.insert(id, list.clone());
        list
    }

    /// A forced read is running.
    pub fn refreshing(&self) -> bool {
        self.forcing > 0
    }

    /// Keep reading while some of `agents` can have worktrees; read again
    /// a second after their numbers move.
    pub fn follow(&mut self, agents: &[AgentView], cx: &mut Context<Self>) {
        if !agents.iter().any(|a| a.caps.worktrees) {
            self.poll = None;
            self.settle = None;
            self.sig.clear();
            if !self.projects.is_empty() {
                self.projects.clear();
                cx.notify();
            }
            return;
        }
        if self.poll.is_none() {
            self.poll = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(LIST_EVERY).await;
                // Paused while no window shows (`document.hidden`).
                crate::platform::visible::until_any_visible(cx).await;
                if this.update(cx, |l, cx| l.list(cx)).is_err() {
                    break;
                }
            }));
        }
        let sig = super::worktrees::agents_signature(agents);
        if sig == self.sig {
            return;
        }
        let first = self.sig.is_empty();
        self.sig = sig;
        let wait = if first { Duration::ZERO } else { SETTLE };
        self.settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |l, cx| l.list(cx));
        }));
    }

    /// Read the list at the engine's pace (asked while one runs: once more
    /// after it).
    pub fn list(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        if self.inflight {
            self.again = true;
            return;
        }
        self.inflight = true;
        let task = cx
            .background_executor()
            .spawn(async move { ops::list_worktrees(&engine) });
        cx.spawn(async move |this, cx| {
            let list = task.await;
            let _ = this.update(cx, |l, cx| {
                l.inflight = false;
                l.take(list, cx);
                if std::mem::take(&mut l.again) {
                    l.list(cx);
                }
            });
        })
        .detach();
    }

    /// List `project` (every project when `None`) now, bypassing the
    /// engine's pace: ↻, ⌘⇧R, a list expanded.
    pub fn force(&mut self, project: Option<String>, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        self.forcing += 1;
        cx.notify();
        let task = cx
            .background_executor()
            .spawn(async move { ops::refresh_worktrees(&engine, project.as_deref()) });
        cx.spawn(async move |this, cx| {
            let list = task.await;
            let _ = this.update(cx, |l, cx| {
                l.forcing -= 1;
                l.take(list, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Demo and tests: a list as if just read.
    pub fn set(&mut self, projects: Vec<ProjectWorktrees>, cx: &mut Context<Self>) {
        self.take(projects, cx);
    }

    fn take(&mut self, list: Vec<ProjectWorktrees>, cx: &mut Context<Self>) {
        self.updated_at = Some(now_ms());
        if self.projects != list {
            self.projects = list;
            cx.notify();
        }
        self.check_work(cx);
    }

    /// Has anything to review (`None`: not read yet, or its folder is gone).
    pub fn has_work(&self, key: &str) -> Option<bool> {
        self.work.get(key).map(|w| w.1)
    }

    /// Demo and tests: what a worktree has, as if read.
    pub fn set_work(&mut self, key: String, has: bool, cx: &mut Context<Self>) {
        self.work.insert(key, (None, has, i64::MAX / 2));
        cx.notify();
    }

    /// Read the changes of listed worktrees that are new, moved, or stale.
    fn check_work(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else { return };
        if self.checking {
            return;
        }
        let now = now_ms();
        let mut keys = std::collections::HashSet::new();
        let mut due = Vec::new();
        for p in &self.projects {
            for w in &p.worktrees {
                let key = crate::review::model::wt_key(&p.id, &w.path);
                keys.insert(key.clone());
                let fresh = self.work.get(&key).is_some_and(|(head, _, at)| *head == w.head && now - at < WORK_EVERY_MS);
                if w.caps.diff && !fresh {
                    due.push((key, p.id.clone(), w.path.clone(), w.head.clone()));
                }
            }
        }
        // Gone from the list: forget.
        self.work.retain(|k, _| keys.contains(k));
        if due.is_empty() {
            return;
        }
        self.checking = true;
        let task = cx.background_executor().spawn(async move {
            due.into_iter()
                .filter_map(|(key, p, path, head)| {
                    let has = !ops::worktree_changes(&engine, &p, &path).ok()?.is_empty();
                    Some((key, head, has))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let got = task.await;
            let _ = this.update(cx, |l, cx| {
                l.checking = false;
                let at = now_ms();
                for (key, head, has) in got {
                    l.work.insert(key, (head, has, at));
                }
                cx.notify();
            });
        })
        .detach();
    }
}
