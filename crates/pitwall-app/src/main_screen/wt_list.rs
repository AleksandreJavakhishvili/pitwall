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
}

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
    }
}
