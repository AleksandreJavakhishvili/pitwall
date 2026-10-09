//! The Review screen (Tauri: `src/components/review/Review.tsx`,
//! docs/spec/review.md): what the agents changed, per agent since it started
//! (its base commit) or per task, and their worktrees; a side-by-side or
//! inline diff of one file ([`crate::code_view`]) where clicking the margin
//! collects a comment; and the actions: discard a file, send the comments
//! as a prompt, commit (and merge a worktree's branch), remove a worktree.
//!
//! The host shows a [`ReviewView`] in the main area while Review is on
//! (⌘R toggles it) and drops it when it closes; draft comments outlive it
//! for the app session ([`comments::Comments`]).

pub mod comments;
mod dialogs;
mod mount;
pub mod model;
pub mod ops;
mod render;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui::{
    actions, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    Subscription, Task, Window,
};

use pitwall_core::engine::Task as AgentTask;
use pitwall_core::vcs::git::FileChange;
use pitwall_proto::{AgentView, ProjectWorktrees};

use crate::agents::{group_by_project, AgentStore, ProjectGroup};
use crate::kit::{InputEvent, TextInput};
use crate::code_view::{CodeView, CodeViewEvent, Layout, Side};
use comments::Comments;
use dialogs::Dialog;
use model::{Space, WtRef};

pub use mount::{mount, review_space, ReviewRoute};
pub use ops::ReviewEngine;

actions!(
    review,
    [
        /// Esc: leave Review.
        Back,
        /// ⌘⇧R: read every agent's changes and worktrees again now.
        RefreshChanges,
        /// ↑ in the file list.
        PreviousFile,
        /// ↓ in the file list.
        NextFile,
        /// Enter in the file list: open the focused file.
        OpenFile,
        /// Esc in the comment composer.
        CancelComment,
        /// Esc in a dialog.
        CloseDialog,
    ]
);

/// Key bindings (Review's own and the code view's; ⌘R, which opens and
/// closes Review, is the main screen's).
pub fn register(cx: &mut App) {
    crate::code_view::register(cx);
    let mac = cfg!(target_os = "macos");
    cx.bind_keys([
        KeyBinding::new(
            if mac {
                "cmd-shift-r"
            } else {
                "ctrl-shift-alt-r"
            },
            RefreshChanges,
            Some("Review"),
        ),
        KeyBinding::new("escape", Back, Some("Review")),
        KeyBinding::new("up", PreviousFile, Some("ReviewList")),
        KeyBinding::new("down", NextFile, Some("ReviewList")),
        KeyBinding::new("enter", OpenFile, Some("ReviewList")),
        KeyBinding::new("escape", CancelComment, Some("RvComposer")),
        KeyBinding::new("escape", CloseDialog, Some("RvDialog")),
    ]);
}

/// What Review is asked to show first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Focus {
    /// An agent (and one of its files).
    Agent { id: String, path: Option<String> },
    /// One worktree of a project.
    Worktree { project_id: String, path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewEvent {
    /// Back / Esc.
    Exit,
    /// "Open terminal" on a worktree: a shell in that folder (the host
    /// starts it).
    OpenTerminal(String),
}

impl EventEmitter<ReviewEvent> for ReviewView {}

/// What the main pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sel {
    None,
    /// An agent; `path`: its file shown.
    Agent {
        id: String,
        path: Option<String>,
    },
    /// A worktree (by key); `path`: its file shown.
    Worktree {
        key: String,
        path: Option<String>,
    },
}

/// What the diff area shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DiffState {
    Idle,
    Loading,
    Shown,
    Binary,
    Empty,
    Error(String),
}

struct Composer {
    /// 1-based line of the modified file.
    line: usize,
    path: String,
    field: Entity<TextInput>,
    _sub: Subscription,
}

pub struct ReviewView {
    store: Entity<AgentStore>,
    focus: FocusHandle,
    list_focus: FocusHandle,
    list_scroll: gpui::ScrollHandle,
    prompt_scroll: gpui::ScrollHandle,
    space: Space,
    focused_agent: Option<String>,
    extra: Vec<String>,
    /// Listed agents (in scope, changes readable), in sidebar order.
    ordered: Vec<AgentView>,
    groups: Vec<ProjectGroup>,
    /// In scope but outside a git repository.
    hidden: usize,
    can_widen: bool,
    label: String,
    files: HashMap<String, Vec<FileChange>>,
    errors: HashMap<String, String>,
    /// Totals each agent's files were read at (refetch when they move).
    sigs: HashMap<String, String>,
    task_scope: HashMap<String, Option<String>>,
    tasks: Vec<AgentTask>,
    tasks_for: Option<String>,
    projects: Vec<ProjectWorktrees>,
    /// The window's worktree list, shared with the sidebar and Changes.
    wt_list: Entity<crate::main_screen::wt_list::WorktreeList>,
    wt_open: HashSet<String>,
    wt_files: HashMap<String, Result<Vec<FileChange>, String>>,
    sel: Sel,
    pending_pick: Option<String>,
    collapsed: HashSet<String>,
    closed_dirs: HashSet<String>,
    /// The row ↑/↓ moved to: (owner key, path).
    cursor: Option<(String, String)>,
    code: Entity<CodeView>,
    diff: DiffState,
    diff_key: Option<String>,
    side_by_side: bool,
    refreshing: bool,
    updated_at: Option<u64>,
    fresh_error: Option<String>,
    nonce: u64,
    notice: Option<String>,
    composer: Option<Composer>,
    dialog: Option<Dialog>,
    task_menu: bool,
    /// A merge conflict to show as a prompt at the next render.
    pending_conflict: Option<pitwall_core::review::MergeResult>,
    _subs: Vec<Subscription>,
    _poll: Option<Task<()>>,
}

impl Focusable for ReviewView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// The totals that mean an agent's files may have changed.
fn agent_sig(a: &AgentView, task: Option<&String>) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        a.added,
        a.removed,
        a.files_changed,
        a.current_task_id.as_deref().unwrap_or(""),
        task.map_or("", |t| t.as_str())
    )
}

impl ReviewView {
    /// Review over `store`'s agents, opened on `focus` (or the default
    /// selection), in `space` with `focused_agent` the one focused there.
    pub fn new(
        store: Entity<AgentStore>,
        space: Space,
        focused_agent: Option<String>,
        focus: Option<Focus>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let code = cx.new(CodeView::new);
        let side_by_side = ops::load_side_by_side(cx);
        code.update(cx, |c, cx| {
            c.set_layout(
                if side_by_side {
                    Layout::Split
                } else {
                    Layout::Unified
                },
                cx,
            )
        });
        let wt_list = crate::main_screen::wt_list::WorktreeList::of(window, cx);
        let projects = wt_list.read(cx).projects.clone();
        let subs = vec![
            cx.observe(&store, |this, _, cx| this.agents_changed(cx)),
            cx.observe(&wt_list, |this, _, cx| this.worktrees_taken(cx)),
            cx.subscribe_in(&code, window, |this, _, e: &CodeViewEvent, window, cx| {
                if let CodeViewEvent::Comment { line } = e {
                    this.start_comment(*line + 1, window, cx);
                }
            }),
            cx.observe_global::<Comments>(|this, cx| {
                this.push_comments(cx);
                cx.notify();
            }),
        ];
        let mut v = ReviewView {
            store,
            focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            list_scroll: gpui::ScrollHandle::new(),
            prompt_scroll: gpui::ScrollHandle::new(),
            space,
            focused_agent,
            extra: Vec::new(),
            ordered: Vec::new(),
            groups: Vec::new(),
            hidden: 0,
            can_widen: false,
            label: String::new(),
            files: HashMap::new(),
            errors: HashMap::new(),
            sigs: HashMap::new(),
            task_scope: HashMap::new(),
            tasks: Vec::new(),
            tasks_for: None,
            projects,
            wt_list,
            wt_open: HashSet::new(),
            wt_files: HashMap::new(),
            sel: Sel::None,
            pending_pick: None,
            collapsed: HashSet::new(),
            closed_dirs: HashSet::new(),
            cursor: None,
            code,
            diff: DiffState::Idle,
            diff_key: None,
            side_by_side,
            refreshing: false,
            updated_at: None,
            fresh_error: None,
            nonce: 0,
            notice: None,
            composer: None,
            dialog: None,
            task_menu: false,
            pending_conflict: None,
            _subs: subs,
            _poll: None,
        };
        v.show(focus, cx);
        v.agents_changed(cx);
        // Opening Review reads everything now.
        v.refresh_all(cx);
        v.start_polling(cx);
        window.focus(&v.list_focus);
        v
    }

    /// Show `focus` (as opening Review on it does).
    pub fn show(&mut self, focus: Option<Focus>, cx: &mut Context<Self>) {
        match focus {
            Some(Focus::Agent { id, path }) => {
                self.collapsed.remove(&id);
                self.extra = vec![id.clone()];
                match path {
                    Some(p) => self.sel = Sel::Agent { id, path: Some(p) },
                    None => {
                        self.pending_pick = Some(id.clone());
                        self.sel = Sel::Agent { id, path: None };
                    }
                }
            }
            Some(Focus::Worktree { project_id, path }) => {
                let key = model::wt_key(&project_id, &path);
                self.extra = self
                    .projects
                    .iter()
                    .find(|p| p.id == project_id)
                    .map(|p| p.agent_ids.clone())
                    .unwrap_or_default();
                self.wt_open.insert(key.clone());
                self.sel = Sel::Worktree { key, path: None };
            }
            None => {}
        }
        cx.notify();
    }

    /// The space and focused agent of the window (they change while open).
    pub fn set_context(
        &mut self,
        space: Space,
        focused_agent: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.space != space || self.focused_agent != focused_agent {
            self.space = space;
            self.focused_agent = focused_agent;
            self.agents_changed(cx);
        }
    }

    fn all_projects(cx: &App) -> bool {
        cx.try_global::<AllProjects>().is_some_and(|a| a.0)
    }

    fn set_all_projects(&mut self, on: bool, cx: &mut Context<Self>) {
        cx.set_global(AllProjects(on));
        self.agents_changed(cx);
    }

    // ── data ────────────────────────────────────────────────────────────

    /// Agents moved: scope again, and read the files of those whose totals
    /// changed.
    fn agents_changed(&mut self, cx: &mut Context<Self>) {
        let agents = self.store.read(cx).agents.clone();
        let focused = self
            .focused_agent
            .as_ref()
            .and_then(|id| agents.iter().find(|a| &a.id == id));
        let s = model::scope(
            &agents,
            &self.space,
            Self::all_projects(cx),
            &self.extra,
            focused,
        );
        let reviewable: Vec<AgentView> =
            s.agents.iter().filter(|a| a.caps.review).cloned().collect();
        self.hidden = s.agents.len() - reviewable.len();
        self.can_widen = s.can_widen;
        self.label = s.label;
        self.groups = group_by_project(&reviewable);
        self.ordered = self.groups.iter().flat_map(|g| g.agents.clone()).collect();
        // A removed agent, or one no longer listed.
        let listed = |id: &str| self.ordered.iter().any(|a| a.id == id);
        if let Sel::Agent { id, .. } = &self.sel {
            if !listed(id) {
                self.sel = Sel::None;
            }
        }
        let stale: Vec<String> = self
            .ordered
            .iter()
            .filter(|a| {
                self.sigs.get(&a.id)
                    != Some(&agent_sig(
                        a,
                        self.task_scope.get(&a.id).and_then(|t| t.as_ref()),
                    ))
            })
            .map(|a| a.id.clone())
            .collect();
        for id in stale {
            self.load_files(&id, cx);
        }
        // The shown agent's tasks follow its current task.
        self.load_tasks(cx);
        self.pick_default(cx);
        self.sync_diff(cx);
        cx.notify();
    }

    fn load_files(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(a) = self.ordered.iter().find(|a| a.id == id) else {
            return;
        };
        let task = self.task_scope.get(id).cloned().flatten();
        self.sigs
            .insert(id.to_string(), agent_sig(a, task.as_ref()));
        let (id2, task2) = (id.to_string(), task.clone());
        ops::spawn(cx, move |e| ops::task_changes(e, &id2, task2.as_deref()), {
            let id = id.to_string();
            move |this: &mut Self, res, cx| {
                // A newer scope was picked meanwhile: drop this one.
                if this.task_scope.get(&id).cloned().flatten() != task {
                    return;
                }
                match res {
                    Ok(files) => {
                        this.errors.remove(&id);
                        if this.files.get(&id) != Some(&files) {
                            this.files.insert(id.clone(), files);
                        }
                    }
                    Err(e) => {
                        this.errors.insert(id.clone(), e);
                    }
                }
                this.pick_default(cx);
                this.fix_selection();
                this.sync_diff(cx);
                cx.notify();
            }
        });
    }

    fn load_tasks(&mut self, cx: &mut Context<Self>) {
        let Sel::Agent { id, .. } = &self.sel else {
            self.tasks.clear();
            self.tasks_for = None;
            return;
        };
        let id = id.clone();
        let cur = self
            .ordered
            .iter()
            .find(|a| a.id == id)
            .and_then(|a| a.current_task_id.clone());
        let key = format!("{id}:{}:{}", cur.unwrap_or_default(), self.nonce);
        if self.tasks_for.as_deref() == Some(key.as_str()) {
            return;
        }
        self.tasks_for = Some(key.clone());
        let id2 = id.clone();
        ops::spawn(
            cx,
            move |e| ops::list_tasks(e, &id2),
            move |this: &mut Self, res, cx| {
                if this.tasks_for.as_deref() == Some(key.as_str()) {
                    this.tasks = res.unwrap_or_default();
                    cx.notify();
                }
            },
        );
    }

    /// Read the window's worktree list (`force`: now, for one project or
    /// all); it is shared with the sidebar and the Changes panel.
    fn load_worktrees(&mut self, force: Option<Option<String>>, cx: &mut Context<Self>) {
        self.wt_list.update(cx, |l, cx| match force {
            Some(p) => l.force(p, cx),
            None => l.list(cx),
        });
    }

    /// The window's list changed.
    fn worktrees_taken(&mut self, cx: &mut Context<Self>) {
        let projects = self.wt_list.read(cx).projects.clone();
        if self.projects != projects {
            self.projects = projects;
            // A shown worktree that left the list: nothing.
            if let Sel::Worktree { key, .. } = &self.sel {
                if !self.projects.is_empty() && self.wt_ref(key).is_none() {
                    self.sel = Sel::None;
                }
            }
            self.load_open_worktrees(cx);
        }
        cx.notify();
    }

    /// Read the files of every open or shown worktree.
    fn load_open_worktrees(&mut self, cx: &mut Context<Self>) {
        let mut keys: HashSet<String> = self.wt_open.clone();
        if let Sel::Worktree { key, .. } = &self.sel {
            keys.insert(key.clone());
        }
        for key in keys {
            self.load_worktree_files(&key, cx);
        }
    }

    fn load_worktree_files(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(r) = self.wt_ref(key) else { return };
        if !r.wt.caps.diff {
            return;
        }
        let (p, path, key) = (r.project_id.clone(), r.wt.path.clone(), key.to_string());
        ops::spawn(
            cx,
            move |e| ops::worktree_changes(e, &p, &path),
            move |this: &mut Self, res, cx| {
                if this.wt_files.get(&key) != Some(&res) {
                    this.wt_files.insert(key.clone(), res);
                    this.fix_selection();
                    this.sync_diff(cx);
                    cx.notify();
                }
            },
        );
    }

    fn wt_ref(&self, key: &str) -> Option<WtRef> {
        self.projects
            .iter()
            .flat_map(|p| p.worktrees.iter().map(move |w| (p, w)))
            .find(|(p, w)| model::wt_key(&p.id, &w.path) == key)
            .and_then(|(p, w)| model::find_worktree(&self.projects, &p.id, &w.path))
    }

    /// ↻ / ⌘⇧R / opening: every agent's changes and the worktrees, now.
    fn refresh_all(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        // The worktrees too: the window's list, forced.
        self.load_worktrees(Some(None), cx);
        let agents: Vec<(String, String)> = self
            .ordered
            .iter()
            .map(|a| (a.id.clone(), a.name.clone()))
            .collect();
        ops::spawn(
            cx,
            move |e| {
                let mut failed = Vec::new();
                for (id, name) in &agents {
                    if let Err(err) = ops::refresh_changes(e, id) {
                        failed.push(format!("{name}: {err}"));
                    }
                }
                failed
            },
            |this: &mut Self, failed, cx| {
                this.refreshing = false;
                this.fresh_error = match failed.len() {
                    0 => None,
                    1 => Some(failed[0].clone()),
                    n => Some(format!("{} (and {} more)", failed[0], n - 1)),
                };
                if this.fresh_error.is_none() {
                    this.updated_at = Some(now_ms());
                }
                this.reload_everything(cx);
            },
        );
        cx.notify();
    }

    /// Refresh only one agent (picking it), then read its list again.
    fn refresh_agent(&mut self, id: &str, cx: &mut Context<Self>) {
        let id2 = id.to_string();
        self.refreshing = true;
        ops::spawn(cx, move |e| ops::refresh_changes(e, &id2), {
            let id = id.to_string();
            move |this: &mut Self, res, cx| {
                this.refreshing = false;
                match res {
                    Ok(_) => {
                        this.updated_at = Some(now_ms());
                        this.fresh_error = None;
                    }
                    Err(e) => this.fresh_error = Some(e),
                }
                this.nonce += 1;
                this.load_files(&id, cx);
                this.sync_diff(cx);
                cx.notify();
            }
        });
    }

    fn reload_everything(&mut self, cx: &mut Context<Self>) {
        self.nonce += 1;
        let ids: Vec<String> = self.ordered.iter().map(|a| a.id.clone()).collect();
        for id in ids {
            self.load_files(&id, cx);
        }
        self.load_open_worktrees(cx);
        self.load_tasks(cx);
        self.sync_diff(cx);
        cx.notify();
    }

    /// Files every 5 s, worktree files every 10 s, the worktree list every
    /// 30 s (as the React app polls them while visible).
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self._poll = Some(cx.spawn(async move |this, cx| {
            let mut tick: u64 = 0;
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                // Paused while no window shows (`document.hidden`).
                crate::platform::visible::until_any_visible(cx).await;
                tick += 1;
                let alive = this.update(cx, |v, cx| {
                    let ids: Vec<String> = v.ordered.iter().map(|a| a.id.clone()).collect();
                    for id in ids {
                        v.load_files(&id, cx);
                    }
                    if tick.is_multiple_of(2) {
                        v.load_open_worktrees(cx);
                    }
                    if tick.is_multiple_of(6) {
                        v.load_worktrees(None, cx);
                    }
                    // "updated N s ago" moves on.
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
        self.load_worktrees(None, cx);
    }

    // ── selection ───────────────────────────────────────────────────────

    /// The agent it was opened for (its first file once read), else the
    /// first agent with changes.
    fn pick_default(&mut self, _cx: &mut Context<Self>) {
        match &self.sel {
            Sel::None => {
                // In list order: wait for an agent's files before passing it.
                for a in &self.ordered {
                    match self.files.get(&a.id) {
                        None if !self.errors.contains_key(&a.id) => return,
                        Some(f) if !f.is_empty() => {
                            let first = model::tree_order(f).first().map(|f| f.path.clone());
                            self.sel = Sel::Agent {
                                id: a.id.clone(),
                                path: first,
                            };
                            return;
                        }
                        _ => {}
                    }
                }
            }
            Sel::Agent { id, path: None } if self.pending_pick.as_deref() == Some(id.as_str()) => {
                if let Some(list) = self.files.get(id).filter(|l| !l.is_empty()) {
                    let first = model::tree_order(list).first().map(|f| f.path.clone());
                    self.pending_pick = None;
                    self.sel = Sel::Agent {
                        id: id.clone(),
                        path: first,
                    };
                }
            }
            _ => {}
        }
    }

    /// The selected file went away (discarded, merged, scope changed): a
    /// neighbour (the first in tree order).
    fn fix_selection(&mut self) {
        match &self.sel {
            Sel::Agent { id, path: Some(p) } => {
                if let Some(list) = self.files.get(id) {
                    if !list.iter().any(|f| &f.path == p) {
                        let next = model::tree_order(list).first().map(|f| f.path.clone());
                        self.sel = Sel::Agent {
                            id: id.clone(),
                            path: next,
                        };
                    }
                }
            }
            Sel::Worktree { key, path } => {
                if let Some(Ok(list)) = self.wt_files.get(key) {
                    if path
                        .as_ref()
                        .is_none_or(|p| !list.iter().any(|f| &f.path == p))
                    {
                        let first = model::tree_order(list).first().map(|f| f.path.clone());
                        if first != *path {
                            self.sel = Sel::Worktree {
                                key: key.clone(),
                                path: first,
                            };
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn select_file(
        &mut self,
        owner: &str,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed_owner = match &self.sel {
            Sel::Agent { id, .. } => id != owner,
            Sel::Worktree { key, .. } => key != owner,
            Sel::None => true,
        };
        if self.is_worktree_key(owner) {
            self.sel = Sel::Worktree {
                key: owner.to_string(),
                path: Some(path.to_string()),
            };
        } else {
            self.sel = Sel::Agent {
                id: owner.to_string(),
                path: Some(path.to_string()),
            };
        }
        self.cursor = Some((owner.to_string(), path.to_string()));
        self.composer = None;
        if changed_owner {
            self.picked(cx);
        }
        self.load_tasks(cx);
        self.sync_diff(cx);
        window.focus(&self.list_focus);
        cx.notify();
    }

    fn is_worktree_key(&self, k: &str) -> bool {
        k.contains('\u{0}')
    }

    /// Picking an agent or worktree refreshes just that one.
    fn picked(&mut self, cx: &mut Context<Self>) {
        match self.sel.clone() {
            Sel::Agent { id, .. } => self.refresh_agent(&id, cx),
            Sel::Worktree { key, .. } => {
                if let Some(r) = self.wt_ref(&key) {
                    self.load_worktrees(Some(Some(r.project_id.clone())), cx);
                    self.load_worktree_files(&key, cx);
                }
            }
            Sel::None => {}
        }
    }

    fn show_agent(&mut self, id: &str, cx: &mut Context<Self>) {
        let first = self
            .files
            .get(id)
            .and_then(|l| model::tree_order(l).first().map(|f| f.path.clone()));
        let changed = !matches!(&self.sel, Sel::Agent { id: cur, .. } if cur == id);
        self.sel = Sel::Agent {
            id: id.to_string(),
            path: first,
        };
        self.composer = None;
        if changed {
            self.picked(cx);
        }
        self.load_tasks(cx);
        self.sync_diff(cx);
        cx.notify();
    }

    fn show_worktree(&mut self, key: &str, cx: &mut Context<Self>) {
        let first = match self.wt_files.get(key) {
            Some(Ok(l)) => model::tree_order(l).first().map(|f| f.path.clone()),
            _ => None,
        };
        let changed = !matches!(&self.sel, Sel::Worktree { key: cur, .. } if cur == key);
        self.sel = Sel::Worktree {
            key: key.to_string(),
            path: first,
        };
        self.wt_open.insert(key.to_string());
        self.composer = None;
        if changed {
            self.picked(cx);
        }
        self.load_worktree_files(key, cx);
        self.sync_diff(cx);
        cx.notify();
    }

    fn toggle_worktree(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.wt_open.remove(key) {
            self.wt_open.insert(key.to_string());
            // Expanding lists its project's worktrees again now.
            if let Some(r) = self.wt_ref(key) {
                self.load_worktrees(Some(Some(r.project_id.clone())), cx);
            }
            self.load_worktree_files(key, cx);
        }
        cx.notify();
    }

    fn set_task(&mut self, agent: &str, task: Option<String>, cx: &mut Context<Self>) {
        self.task_scope.insert(agent.to_string(), task);
        self.files.remove(agent);
        self.task_menu = false;
        self.load_files(agent, cx);
        self.sync_diff(cx);
        cx.notify();
    }

    fn set_side_by_side(&mut self, on: bool, cx: &mut Context<Self>) {
        self.side_by_side = on;
        ops::save_side_by_side(cx, on);
        self.code.update(cx, |c, cx| {
            c.set_layout(if on { Layout::Split } else { Layout::Unified }, cx)
        });
        cx.notify();
    }

    /// The selected agent, file and task (agent selection only).
    fn agent_sel(&self) -> Option<(&AgentView, Option<&FileChange>, Option<String>)> {
        let Sel::Agent { id, path } = &self.sel else {
            return None;
        };
        let a = self.ordered.iter().find(|a| &a.id == id)?;
        let file = path.as_ref().and_then(|p| {
            self.files
                .get(id)
                .and_then(|l| l.iter().find(|f| &f.path == p))
        });
        Some((a, file, self.task_scope.get(id).cloned().flatten()))
    }

    fn wt_sel(&self) -> Option<(WtRef, Option<&FileChange>)> {
        let Sel::Worktree { key, path } = &self.sel else {
            return None;
        };
        let r = self.wt_ref(key)?;
        let file = match (path, self.wt_files.get(key)) {
            (Some(p), Some(Ok(l))) => l.iter().find(|f| &f.path == p),
            _ => None,
        };
        Some((r, file))
    }

    // ── the diff ────────────────────────────────────────────────────────

    /// Load the shown file's two versions when what is shown changed.
    fn sync_diff(&mut self, cx: &mut Context<Self>) {
        enum Load {
            Agent(String, String, Option<String>),
            Wt(String, String, String),
        }
        let (key, load) = if let Some((a, Some(f), task)) = self.agent_sel() {
            (
                format!(
                    "a:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                    a.id,
                    f.path,
                    task.clone().unwrap_or_default(),
                    a.added,
                    a.removed,
                    a.files_changed,
                    self.nonce,
                    f.added,
                    f.removed
                ),
                Load::Agent(a.id.clone(), f.path.clone(), task),
            )
        } else if let Some((r, Some(f))) = self.wt_sel() {
            (
                format!(
                    "w:{}:{}:{}:{}:{}:{}",
                    r.key(),
                    f.path,
                    r.wt.head.clone().unwrap_or_default(),
                    self.nonce,
                    f.added,
                    f.removed
                ),
                Load::Wt(r.project_id.clone(), r.wt.path.clone(), f.path.clone()),
            )
        } else {
            if self.diff_key.take().is_some() {
                self.diff = DiffState::Idle;
                self.code.update(cx, |c, cx| c.clear(cx));
            }
            return;
        };
        if self.diff_key.as_deref() == Some(key.as_str()) {
            return;
        }
        // A different file (not a refresh of the same one): show "Loading…".
        let same_file = self
            .diff_key
            .as_ref()
            .is_some_and(|k| k.split(':').take(3).eq(key.split(':').take(3)));
        if !same_file {
            self.diff = DiffState::Loading;
        }
        self.diff_key = Some(key.clone());
        let path = match &load {
            Load::Agent(_, p, _) | Load::Wt(_, _, p) => p.clone(),
        };
        ops::spawn(
            cx,
            move |e| match load {
                Load::Agent(id, p, task) => ops::file_versions(e, &id, &p, task.as_deref()),
                Load::Wt(proj, wt, p) => ops::worktree_file_versions(e, &proj, &wt, &p),
            },
            move |this: &mut Self, res, cx| {
                if this.diff_key.as_deref() != Some(key.as_str()) {
                    return;
                }
                match res {
                    Err(e) => this.diff = DiffState::Error(e),
                    Ok(v) if v.binary => this.diff = DiffState::Binary,
                    Ok(v) if v.original.is_none() && v.modified.is_none() => {
                        this.diff = DiffState::Empty
                    }
                    Ok(v) => {
                        this.diff = DiffState::Shown;
                        let commentable = matches!(this.sel, Sel::Agent { .. });
                        this.code.update(cx, |c, cx| {
                            c.set_commentable(commentable, cx);
                            c.set_diff(
                                &path,
                                v.original.unwrap_or_default(),
                                v.modified.unwrap_or_default(),
                                cx,
                            );
                        });
                        this.push_comments(cx);
                    }
                }
                cx.notify();
            },
        );
    }

    /// The shown file's comments, into the diff's margin.
    fn push_comments(&mut self, cx: &mut Context<Self>) {
        let list: Vec<(usize, String)> = match self.agent_sel() {
            Some((a, Some(f), _)) => Comments::for_agent(cx, &a.id)
                .into_iter()
                .filter(|c| c.path == f.path)
                .map(|c| (c.line.saturating_sub(1), c.text))
                .collect(),
            _ => Vec::new(),
        };
        self.code.update(cx, |c, cx| c.set_comments(list, cx));
    }

    fn start_comment(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, Some(f), _)) = self.agent_sel() else {
            return;
        };
        let path = f.path.clone();
        let field = cx.new(|cx| TextInput::new(cx, "", "What should change here?").multiline(3));
        let sub = cx.subscribe_in(&field, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::Submit => this.add_comment(window, cx),
            InputEvent::Cancel => this.cancel_comment(window, cx),
            _ => cx.notify(),
        });
        field.update(cx, |f, _| f.focus(window));
        self.composer = Some(Composer {
            line,
            path,
            field,
            _sub: sub,
        });
        cx.notify();
    }

    fn add_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.composer.as_ref() else {
            return;
        };
        let text = c.field.read(cx).text().to_string();
        if text.trim().is_empty() {
            return;
        }
        let Some((a, _, _)) = self.agent_sel() else {
            return;
        };
        let id = a.id.clone();
        Comments::add(cx, &id, c.path.clone(), c.line, text);
        self.composer = None;
        window.focus(&self.code.focus_handle(cx));
        cx.notify();
    }

    fn cancel_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer = None;
        window.focus(&self.code.focus_handle(cx));
        cx.notify();
    }

    fn reveal_comment(
        &mut self,
        agent: &str,
        c: &model::Comment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_file(agent, &c.path, window, cx);
        let line = c.line.saturating_sub(1);
        self.code
            .update(cx, |v, cx| v.reveal(Side::New, line, None, cx));
    }

    // ── keyboard in the list ────────────────────────────────────────────

    /// Every file row shown in the list, in order: (owner key, path).
    fn file_rows(&self, cx: &App) -> Vec<(String, String)> {
        render::list_file_rows(self, cx)
    }

    fn move_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.file_rows(cx);
        if rows.is_empty() {
            return;
        }
        let current = self.cursor.clone().or_else(|| match &self.sel {
            Sel::Agent { id, path: Some(p) } => Some((id.clone(), p.clone())),
            Sel::Worktree { key, path: Some(p) } => Some((key.clone(), p.clone())),
            _ => None,
        });
        let i = current.and_then(|c| rows.iter().position(|r| *r == c));
        let next = match i {
            None => 0,
            Some(i) => (i as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
        };
        self.cursor = Some(rows[next].clone());
        cx.notify();
    }

    fn exit(&mut self, cx: &mut Context<Self>) {
        cx.emit(ReviewEvent::Exit);
    }

    /// After an action: a notice in the footer, and everything read again.
    fn done(&mut self, msg: String, cx: &mut Context<Self>) {
        self.notice = Some(msg);
        self.load_worktrees(Some(None), cx);
        self.reload_everything(cx);
    }
}

/// Review's "All projects" toggle (kept for the app session).
#[derive(Default)]
struct AllProjects(bool);

impl gpui::Global for AllProjects {}

#[cfg(test)]
mod tests;

/// Where a window keeps its Review while it is on: the host calls
/// [`ReviewSlot::toggle`] for ⌘R and draws [`ReviewSlot::view`] in its main
/// area instead of the terminals.
#[derive(Default)]
pub struct ReviewSlot {
    view: Option<Entity<ReviewView>>,
    _sub: Option<Subscription>,
}

impl ReviewSlot {
    pub fn view(&self) -> Option<&Entity<ReviewView>> {
        self.view.as_ref()
    }

    /// ⌘R: close Review, or open it on `focused` (when it can be reviewed).
    pub fn toggle<V: 'static>(
        &mut self,
        store: &Entity<AgentStore>,
        space: Space,
        focused: Option<String>,
        slot: fn(&mut V) -> &mut ReviewSlot,
        window: &mut Window,
        cx: &mut Context<V>,
    ) {
        if self.view.take().is_some() {
            self._sub = None;
            cx.notify();
            return;
        }
        let reviewable = focused
            .as_ref()
            .and_then(|id| store.read(cx).agent(id))
            .is_some_and(|a| a.caps.review);
        let focus = reviewable.then(|| Focus::Agent {
            id: focused.clone().unwrap_or_default(),
            path: None,
        });
        self.open(store, space, focused, focus, slot, window, cx);
    }

    /// Open Review (or show `focus` in the open one).
    #[allow(clippy::too_many_arguments)]
    pub fn open<V: 'static>(
        &mut self,
        store: &Entity<AgentStore>,
        space: Space,
        focused: Option<String>,
        focus: Option<Focus>,
        slot: fn(&mut V) -> &mut ReviewSlot,
        window: &mut Window,
        cx: &mut Context<V>,
    ) {
        if let Some(v) = &self.view {
            v.update(cx, |v, cx| {
                v.set_context(space, focused, cx);
                v.show(focus, cx);
            });
            return;
        }
        let store = store.clone();
        let view = cx.new(|cx| ReviewView::new(store, space, focused, focus, window, cx));
        self._sub = Some(cx.subscribe_in(
            &view,
            window,
            move |this, _, e: &ReviewEvent, window, cx| match e {
                ReviewEvent::Exit => {
                    let s = slot(this);
                    s.view = None;
                    s._sub = None;
                    // Its focus went with it: the host's view takes the
                    // keyboard back when it draws.
                    let _ = window;
                    cx.notify();
                }
                // The host starts it (`mount`).
                ReviewEvent::OpenTerminal(_) => {}
            },
        ));
        self.view = Some(view);
        cx.notify();
    }

    /// Close Review (its draft comments stay for the session).
    pub fn close(&mut self) {
        self.view = None;
        self._sub = None;
    }

    /// The window's space or focused agent changed.
    pub fn set_context(&self, space: Space, focused: Option<String>, cx: &mut App) {
        if let Some(v) = &self.view {
            v.update(cx, |v, cx| v.set_context(space, focused, cx));
        }
    }

    /// The window's focused agent changed (the sidebar).
    pub fn set_focused(&self, focused: Option<String>, cx: &mut App) {
        if let Some(v) = &self.view {
            v.update(cx, |v, cx| {
                let space = v.space.clone();
                v.set_context(space, focused, cx)
            });
        }
    }
}
