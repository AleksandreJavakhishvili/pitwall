//! Worktrees in the sidebar (React `Sidebar.tsx`, `AgentRow.tsx`,
//! `WorktreeRows.tsx`, `RemoveWorktreeDialog.tsx`, `src/lib/useWorktrees.ts`;
//! docs/spec/gpui inventory §2, §15, §18):
//!
//! - one worktree list per window, read only while some agent can have
//!   worktrees (`caps.worktrees`): on first use, every 30 s, 1 s after the
//!   agents' numbers settle, and forced (the engine's pace bypassed) when a
//!   list is expanded;
//! - under an agent with worktrees besides its own folder, a "2 worktrees"
//!   chip; under a project, "1 other worktree" for those of no agent; each
//!   expands into rows;
//! - a row: branch (or "<name> (detached)"), "locked" / "gone" chips, the
//!   diffstat (its changes read only while shown, every 15 s); a click opens
//!   Review on it, a right-click offers Review changes, Open terminal here
//!   and Remove worktree… by its caps;
//! - Remove worktree: `git worktree remove` without `--force`, after a
//!   confirmation that says what is kept.

use crate::kit::Ellipsis as _;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui::{
    div, prelude::*, px, AnyElement, Context, IntoElement, MouseButton, MouseDownEvent,
    SharedString, Task,
};

use pitwall_core::vcs::git::FileChange;
use pitwall_proto::{AgentView, ProjectWorktrees, WorktreeVia};

use crate::kit::{
    button, chevron, chip_tone, diffstat, hint, icon, tooltip, BtnKind, HoverText, Tone, MONO_FONT,
};
use crate::review::model::{find_worktree, other_worktrees, worktrees_by_agent, WtRef};
use crate::review::ops;
use crate::theme::{Theme, RADIUS_SM};

use super::menu::MenuItem;
use super::{MainScreen, Modal, Route, ScreenEvent};

/// A shown worktree's changes are read this often (`useWorktreeFiles`).
const FILES_EVERY: Duration = Duration::from_secs(15);

/// One shown worktree's changes.
#[derive(Default)]
struct Files {
    head: Option<String>,
    files: Option<Vec<FileChange>>,
    error: Option<String>,
    loading: bool,
}

/// The window's worktree list and what the sidebar shows of it.
#[derive(Default)]
pub struct WtState {
    pub projects: Vec<ProjectWorktrees>,
    /// Expanded lists: agent ids, and [`others_key`] of project groups.
    open: HashSet<String>,
    /// Changes of the rows shown, by worktree key.
    files: HashMap<String, Files>,
    files_poll: Option<Task<()>>,
}

/// Where a `.wt-chip` sits: under an agent row (30 px in), under a
/// project's agents (8 px), or in the Changes panel (at its edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WtChipAt {
    Agent,
    Others,
    Panel,
}

/// The collapse key of the Changes panel's list for an agent.
pub fn panel_key(agent: &str) -> String {
    format!("\u{0}panel:{agent}")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl WtState {
    /// Whether the list under `key` is expanded.
    pub fn is_open(&self, key: &str) -> bool {
        self.open.contains(key)
    }
}

/// The collapse key of a project group's other worktrees.
pub fn others_key(group: &str) -> String {
    format!("\u{0}wt:{group}")
}

/// "2 worktrees" (`countLabel`).
pub fn count_label(n: usize) -> String {
    format!("{n} worktree{}", if n == 1 { "" } else { "s" })
}

/// "1 other worktree" / "3 other worktrees".
pub fn others_label(n: usize) -> String {
    if n == 1 {
        "1 other worktree".into()
    } else {
        format!("{n} other worktrees")
    }
}

/// What the poller compares: a project changed when one of its agents'
/// numbers moved (`agentsSignature`).
pub fn agents_signature(agents: &[AgentView]) -> String {
    agents
        .iter()
        .filter(|a| a.caps.worktrees)
        .map(|a| {
            format!(
                "{}:{}:{}:{}:{}:{}:{:?}",
                a.id,
                a.cwd,
                a.added,
                a.removed,
                a.files_changed,
                a.branch.as_deref().unwrap_or(""),
                a.status
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// A row's tooltip: its folder, how it belongs to the agent, a read error.
pub fn row_tip(r: &WtRef, error: Option<&str>) -> String {
    let mut tip = r.wt.path_display.clone();
    if r.wt.via == WorktreeVia::Process {
        tip.push_str("\nA process of this agent works here");
    }
    if let Some(e) = error {
        tip.push('\n');
        tip.push_str(e);
    }
    tip
}

/// The row's name: its branch, or "<name> (detached)".
pub fn row_name(r: &WtRef) -> String {
    r.wt.branch
        .clone()
        .unwrap_or_else(|| format!("{} (detached)", r.wt.name))
}

/// The Remove worktree dialog's state.
pub struct RemoveWt {
    project_id: String,
    path: String,
    busy: bool,
}

impl MainScreen {
    /// The agents changed: the window's list follows them (it reads again
    /// once their numbers settle).
    pub(super) fn wt_follow(&mut self, cx: &mut Context<Self>) {
        if self.engine.is_none() {
            return;
        }
        let agents = self.agents(cx).to_vec();
        self.wt_list_entity.update(cx, |l, cx| l.follow(&agents, cx));
    }

    /// Read the window's list (`force`: now, bypassing the engine's pace).
    pub(super) fn wt_list(&mut self, force: bool, cx: &mut Context<Self>) {
        self.wt_list_entity.update(cx, |l, cx| {
            if force {
                l.force(None, cx)
            } else {
                l.list(cx)
            }
        });
    }

    /// The window's list changed: take it and read the shown rows' changes.
    pub(super) fn wt_taken(&mut self, cx: &mut Context<Self>) {
        let list = self.wt_list_entity.read(cx).projects.clone();
        if self.wt.projects != list {
            self.wt.projects = list;
            self.wt_load_files(false, cx);
        }
        cx.notify();
    }

    /// Demo only: the React mock's worktrees (`src/mockWorktrees.ts`) as if
    /// just read, with every list expanded when `open`.
    pub fn demo_worktrees(&mut self, open: bool, cx: &mut Context<Self>) {
        let agents = self.agents(cx).to_vec();
        let (list, files) = demo_list(&agents);
        self.wt.projects = list.clone();
        self.wt_list_entity.update(cx, |l, cx| l.set(list, cx));
        for (key, f) in files {
            self.wt.files.insert(
                key,
                Files {
                    files: Some(f),
                    ..Default::default()
                },
            );
        }
        if open {
            for g in self.sidebar_groups_for_wt(&agents) {
                self.wt.open.insert(others_key(&g.0));
                self.wt.open.extend(g.1);
            }
        }
        cx.notify();
    }

    /// Demo only: the Remove worktree dialog for the first removable one.
    pub fn demo_remove_worktree(&mut self, cx: &mut Context<Self>) {
        let first = self
            .wt
            .projects
            .iter()
            .flat_map(|p| p.worktrees.iter().map(move |w| (p, w)))
            .find(|(_, w)| w.caps.remove);
        if let Some((p, w)) = first {
            cx.emit(super::ScreenEvent::ModalOpened);
            self.modal = Some(Modal::RemoveWorktree(RemoveWt {
                project_id: p.id.clone(),
                path: w.path.clone(),
                busy: false,
            }));
            cx.notify();
        }
    }

    fn wt_toggle(&mut self, key: String, cx: &mut Context<Self>) {
        if self.wt.open.remove(&key) {
            cx.notify();
            return;
        }
        self.wt.open.insert(key);
        // Expanding lists them again now, and their rows read their files
        // anew (React mounts them fresh).
        self.wt_list(true, cx);
        self.wt_load_files(true, cx);
        cx.notify();
    }

    /// The worktrees shown now (in expanded lists of visible groups).
    fn wt_shown(&self, cx: &Context<Self>) -> Vec<WtRef> {
        let agents = self.agents(cx);
        let by_agent = worktrees_by_agent(&self.wt.projects);
        let mut out = Vec::new();
        for g in self.sidebar_groups_for_wt(agents) {
            if self.ui.collapsed.contains(&g.0) {
                continue;
            }
            for id in &g.1 {
                if self.wt.open.contains(id) {
                    out.extend(by_agent.get(id).cloned().unwrap_or_default());
                }
            }
            if self.wt.open.contains(&others_key(&g.0)) {
                let ids: Vec<&str> = g.1.iter().map(String::as_str).collect();
                out.extend(other_worktrees(&self.wt.projects, &ids));
            }
        }
        // The Changes panel's list of the selected agent.
        if let Some(a) = self.selected(cx) {
            if self.wt.open.contains(&panel_key(&a.id)) {
                out.extend(by_agent.get(&a.id).cloned().unwrap_or_default());
            }
        }
        out
    }

    fn sidebar_groups_for_wt(&self, agents: &[AgentView]) -> Vec<(String, Vec<String>)> {
        crate::agents::group_by_project(agents)
            .into_iter()
            .map(|g| (g.key, g.agents.into_iter().map(|a| a.id).collect()))
            .collect()
    }

    /// Read the changes of the rows shown: new ones, ones whose HEAD moved,
    /// or all of them (`all`, the 15 s poll).
    pub(super) fn wt_load_files(&mut self, all: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let shown = self.wt_shown(cx);
        if shown.is_empty() {
            self.wt.files_poll = None;
            return;
        }
        if self.wt.files_poll.is_none() {
            self.wt.files_poll = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(FILES_EVERY).await;
                // Paused while no window shows (`document.hidden`).
                crate::platform::visible::until_any_visible(cx).await;
                if this.update(cx, |s, cx| s.wt_load_files(true, cx)).is_err() {
                    break;
                }
            }));
        }
        for r in shown.into_iter().filter(|r| r.wt.caps.diff) {
            let key = r.key();
            let entry = self.wt.files.entry(key.clone()).or_default();
            let fresh = entry.files.is_some() && entry.head == r.wt.head;
            if entry.loading || (fresh && !all) {
                continue;
            }
            entry.loading = true;
            let head = r.wt.head.clone();
            let (e, project, path) = (engine.clone(), r.project_id.clone(), r.wt.path.clone());
            let task = cx
                .background_executor()
                .spawn(async move { ops::worktree_changes(&e, &project, &path) });
            cx.spawn(async move |this, cx| {
                let res = task.await;
                let _ = this.update(cx, |s, cx| {
                    let f = s.wt.files.entry(key).or_default();
                    f.loading = false;
                    f.head = head;
                    match res {
                        Ok(files) => {
                            if f.files.as_ref() != Some(&files) {
                                f.files = Some(files);
                                cx.notify();
                            }
                            if f.error.take().is_some() {
                                cx.notify();
                            }
                        }
                        Err(e) => {
                            f.error = Some(e);
                            cx.notify();
                        }
                    }
                });
            })
            .detach();
        }
    }

    /// Open Review on a worktree (the Review module answers
    /// [`ScreenEvent::ReviewWorktree`]).
    fn wt_review(&mut self, project_id: String, path: String, cx: &mut Context<Self>) {
        // Review opens on the worktree first, so the route change finds it
        // open instead of opening it on the agent.
        cx.emit(ScreenEvent::ReviewWorktree { project_id, path });
        self.set_route(Route::Review, cx);
    }

    fn wt_menu(
        &mut self,
        r: &WtRef,
        at: gpui::Point<gpui::Pixels>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let (p1, w1) = (r.project_id.clone(), r.wt.path.clone());
        let mut items = vec![MenuItem::new(
            "review",
            "Review changes",
            move |this, _, cx| this.wt_review(p1.clone(), w1.clone(), cx),
        )];
        if r.wt.caps.terminal {
            let path = r.wt.path.clone();
            items.push(MenuItem::new(
                "terminal",
                "Open terminal here",
                move |this, window, cx| this.open_terminal(path.clone(), window, cx),
            ));
        }
        if r.wt.caps.remove {
            let (p, w) = (r.project_id.clone(), r.wt.path.clone());
            items.push(MenuItem::new(
                "trash",
                "Remove worktree…",
                move |this, _, cx| {
                    cx.emit(super::ScreenEvent::ModalOpened);
                    this.modal = Some(Modal::RemoveWorktree(RemoveWt {
                        project_id: p.clone(),
                        path: w.clone(),
                        busy: false,
                    }));
                    cx.notify();
                },
            ));
        }
        self.open_menu(at, items, window, cx);
    }

    // ── drawing ──────────────────────────────────────────────────────────

    /// `.wt-chip`: the toggle of a worktree list. `others`: a project's
    /// other worktrees (8 px in) rather than an agent's (30 px in).
    fn wt_chip(
        &self,
        key: String,
        text: String,
        tip: &'static str,
        at: WtChipAt,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.wt.open.contains(&key);
        let id = SharedString::from(format!("wt-chip-{key}"));
        let group = SharedString::from(format!("wt-chip-g-{key}"));
        let (bg, fg) = (t.surface_3, t.text_2);
        div()
            .flex()
            .child(
                div()
                    .id(id)
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .mt(px(1.))
                    .mb(px(3.))
                    .ml(px(match at {
                        WtChipAt::Agent => 30.,
                        WtChipAt::Others => 8.,
                        WtChipAt::Panel => 0.,
                    }))
                    .pl(px(4.))
                    .pr(px(7.))
                    .py(px(2.))
                    .rounded(RADIUS_SM)
                    .text_size(px(11.))
                    .line_height(px(15.))
                    .text_color(t.text_3)
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .hover_text(fg, move |s| s.bg(bg))
                    .tooltip(tooltip(tip))
                    .on_click(cx.listener(move |this, _, _, cx| this.wt_toggle(key.clone(), cx)))
                    .child(chevron("chev", open, 10., t.text_4))
                    .child(icon("branch", 11., t.text_3).group_hover_text(group, fg, |s| s))
                    .child(text),
            )
            .into_any_element()
    }

    /// An agent's chip and, expanded, its rows (`AgentRow` `worktrees`).
    pub(super) fn wt_agent(
        &self,
        a: &AgentView,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let by_agent = worktrees_by_agent(&self.wt.projects);
        let Some(refs) = by_agent.get(&a.id).filter(|r| !r.is_empty()) else {
            return vec![];
        };
        let open = self.wt.open.contains(&a.id);
        let mut out = vec![self.wt_chip(
            a.id.clone(),
            count_label(refs.len()),
            if open {
                "Hide its worktrees"
            } else {
                "Show its worktrees"
            },
            WtChipAt::Agent,
            t,
            cx,
        )];
        if open {
            out.push(self.wt_rows(refs, 30., t, cx).into_any_element());
        }
        out
    }

    /// A project group's "N other worktrees" chip and rows.
    pub(super) fn wt_others(
        &self,
        group: &str,
        agents: &[AgentView],
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
        let others = other_worktrees(&self.wt.projects, &ids);
        if others.is_empty() {
            return vec![];
        }
        let key = others_key(group);
        let open = self.wt.open.contains(&key);
        // In the agent list (6 px in), then the chip's own 8 px.
        let mut out = vec![div()
            .mx(px(6.))
            .child(self.wt_chip(
                key,
                others_label(others.len()),
                "Worktrees of this project no agent works in",
                WtChipAt::Others,
                t,
                cx,
            ))
            .into_any_element()];
        if open {
            out.push(self.wt_rows(&others, 20., t, cx).into_any_element());
        }
        out
    }

    /// The Changes panel's `.panel-wts`: the agent's other worktrees,
    /// collapsed until asked for; open, a refresh control and the rows.
    pub(super) fn panel_worktrees(
        &self,
        a: &AgentView,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let refs = worktrees_by_agent(&self.wt.projects)
            .get(&a.id)
            .cloned()
            .filter(|r| !r.is_empty())?;
        let key = panel_key(&a.id);
        let open = self.wt.open.contains(&key);
        let list = self.wt_list_entity.read(cx);
        let busy = list.refreshing();
        let age = list
            .updated_at
            .map(|at| (now_ms() - at).max(0) as u64 / 1000);
        let chip = self.wt_chip(
            key,
            count_label(refs.len()),
            if open { "Hide its worktrees" } else { "Show its worktrees" },
            WtChipAt::Panel,
            t,
            cx,
        );
        Some(
            div()
                .mt(px(6.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(chip)
                        .when(open, |d| {
                            d.child(div().flex_1()).child(crate::kit::refresh_control(
                                "wt-panel-refresh",
                                busy,
                                Some(crate::kit::fresh_note(busy, age)),
                                t,
                                cx.listener(|this, _, _, cx| {
                                    this.wt_list(true, cx);
                                    this.wt_load_files(true, cx);
                                }),
                            ))
                        }),
                )
                .when(open, |d| d.child(self.wt_rows(&refs, 0., t, cx).mx(px(0.))))
                .into_any_element(),
        )
    }

    /// `.wt-list`: rows `inset` px in from the agent list's edge.
    fn wt_rows(&self, refs: &[WtRef], inset: f32, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .mx(px(6.))
            .pl(px(inset))
            .mb(px(4.))
            .flex()
            .flex_col()
            .children(refs.iter().map(|r| self.wt_row(r, t, cx)))
    }

    fn wt_row(&self, r: &WtRef, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let key = r.key();
        let f = self.wt.files.get(&key);
        let error = f.and_then(|f| f.error.as_deref());
        let totals = f.and_then(|f| f.files.as_ref()).map(|files| {
            files
                .iter()
                .fold((0, 0), |(a, d), x| (a + x.added, d + x.removed))
        });
        let (p, w) = (r.project_id.clone(), r.wt.path.clone());
        let menu_ref = r.clone();
        let bg = t.surface_3;
        div()
            .id(SharedString::from(format!("wt-row-{key}")))
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(6.))
            .pr(px(8.))
            .py(px(3.))
            .rounded(RADIUS_SM)
            .text_size(px(11.5))
            .line_height(px(16.))
            .text_color(t.text_2)
            .cursor_pointer()
            .hover_probed(move |s| s.bg(bg))
            .tooltip(tooltip(row_tip(r, error)))
            .on_click(cx.listener(move |this, _, _, cx| this.wt_review(p.clone(), w.clone(), cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.wt_menu(&menu_ref, e.position, window, cx)
                }),
            )
            .child(icon("branch", 12., t.text_2))
            // Wrapping text clamped to one line: gpui 0.2.2 keeps the first
            // (unbounded) measure of `nowrap` text, so `truncate()` never
            // ellipsizes in a flexible row.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .ellipsis()
                    .font_family(MONO_FONT)
                    .child(crate::kit::one_line(row_name(r))),
            )
            .when(r.wt.locked, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("wt-locked-{key}")))
                        .tooltip(tooltip(
                            r.wt.lock_reason.clone().unwrap_or_else(|| "locked".into()),
                        ))
                        .child(chip_tone("locked", Tone::Warn, t)),
                )
            })
            .when(r.wt.prunable, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("wt-gone-{key}")))
                        .tooltip(tooltip("Its folder is gone"))
                        .child(chip_tone("gone", Tone::Subtle, t)),
                )
            })
            .children(totals.map(|(a, d)| diffstat(a, d, t)))
            .into_any_element()
    }

    // ── Remove worktree ──────────────────────────────────────────────────

    fn remove_worktree_now(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::RemoveWorktree(r)) = self.modal.as_mut() else {
            return;
        };
        let (Some(engine), false) = (self.engine.clone(), r.busy) else {
            return;
        };
        r.busy = true;
        let (p, path) = (r.project_id.clone(), r.path.clone());
        let name = find_worktree(&self.wt.projects, &p, &path)
            .map(|w| w.wt.name)
            .unwrap_or_default();
        let task = cx
            .background_executor()
            .spawn(async move { ops::remove_worktree(&engine, &p, &path) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            let _ = this.update(cx, |s, cx| {
                s.wt_list(false, cx);
                match res {
                    Ok(()) => {
                        s.modal = None;
                        s.toast(
                            super::strip::Toast::info(&format!("Removed worktree {name}"), None),
                            cx,
                        );
                    }
                    Err(e) => {
                        if let Some(Modal::RemoveWorktree(r)) = s.modal.as_mut() {
                            r.busy = false;
                        }
                        s.failed(&format!("remove worktree {name}"), e, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_remove_worktree(
        &mut self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let Some(Modal::RemoveWorktree(r)) = &self.modal else {
            return None;
        };
        let busy = r.busy;
        let w = find_worktree(&self.wt.projects, &r.project_id, &r.path)?.wt;
        let ft = t.float();
        let mono = |s: String| div().font_family(MONO_FONT).child(s);
        let body = crate::kit::dialog::body()
            .line_height(px(18.))
            .child(mono(w.path_display.clone()))
            .map(|d| {
                if w.caps.remove {
                    let tail = match &w.branch {
                        Some(b) => format!("Branch {b} is kept."),
                        None => "It is on a detached HEAD; commits not on a branch become hard to find.".into(),
                    };
                    d.child(
                        div()
                            .text_color(ft.text_2)
                            .child(format!(
                                "Runs git worktree remove (without --force): git refuses if it has uncommitted or untracked changes, and nothing is lost. {tail}"
                            )),
                    )
                    .child(crate::kit::dialog::form_error(
                        "The worktree folder will be deleted from disk.",
                        &ft,
                    ))
                } else {
                    let why = if w.locked {
                        match &w.lock_reason {
                            Some(r) => format!("It is locked ({r}). Pitwall doesn't remove locked worktrees."),
                            None => "It is locked. Pitwall doesn't remove locked worktrees.".into(),
                        }
                    } else if w.via == WorktreeVia::Own {
                        "An agent works there: remove the agent (with its worktree) instead.".into()
                    } else {
                        "This worktree can't be removed from Pitwall.".into()
                    };
                    d.child(hint(why, &ft).text_color(ft.red))
                }
            });
        let entity = cx.entity();
        let foot = crate::kit::dialog::foot(&ft)
            .child(
                button("rmwt-cancel", BtnKind::Ghost, false, &ft)
                    .on_click(cx.listener(|s, _, window, cx| s.close_modal(window, cx)))
                    .child("Cancel"),
            )
            .when(w.caps.remove, |d| {
                d.child(
                    button("rmwt-go", BtnKind::Danger, busy, &ft)
                        .when(!busy, |b| {
                            b.on_click(cx.listener(|s, _, _, cx| s.remove_worktree_now(cx)))
                        })
                        .child(if busy {
                            "Removing…"
                        } else {
                            "Remove worktree"
                        }),
                )
            });
        Some(
            crate::kit::Modal::new("remove-worktree", 460., move |window, cx| {
                entity.update(cx, |s, cx| s.close_modal(window, cx))
            })
            .title(format!("Remove worktree {}", w.name))
            .motion(crate::theme::motion_on(cx))
            .render(t, div().flex().flex_col().min_h_0().child(crate::kit::dialog::scrolling("rmwt-body", body, t)).child(foot)),
        )
    }
}

/// The React mock's worktrees (`src/mockWorktrees.ts`) for these agents,
/// and the changes of each (by worktree key).
pub fn demo_list(agents: &[AgentView]) -> (Vec<ProjectWorktrees>, Vec<(String, Vec<FileChange>)>) {
    use pitwall_proto::{MachineView, WorktreeCaps, WorktreeView};
    let orders = "/Users/dev/code/orders-api";
    let f = |path: &str, added: u32, removed: u32, untracked: bool| FileChange {
        path: path.into(),
        added,
        removed,
        untracked,
        binary: false,
        status: None,
    };
    #[allow(clippy::type_complexity)]
    let wts: Vec<(
        &str,
        String,
        Option<&str>,
        Option<&str>,
        WorktreeVia,
        Option<&str>,
        Vec<FileChange>,
    )> = vec![
        (
            orders,
            "/Users/dev/.codex/worktrees/a1b2/orders-api".into(),
            Some("codex/api-fix"),
            Some("api-fix"),
            WorktreeVia::Own,
            None,
            vec![],
        ),
        (
            orders,
            format!("{orders}/.claude/worktrees/agent-a1f3"),
            Some("worktree-agent-a1f3"),
            Some("refactor"),
            WorktreeVia::ToolDir,
            None,
            vec![
                f("src/orders/handler.ts", 12, 4, false),
                f("src/orders/money.ts", 31, 0, true),
            ],
        ),
        (
            orders,
            format!("{orders}/.claude/worktrees/agent-77c2"),
            Some("worktree-agent-77c2"),
            Some("refactor"),
            WorktreeVia::ToolDir,
            Some("claude agent agent-77c2 (pid 4242)"),
            vec![f("docs/api/orders.md", 8, 2, false)],
        ),
        (
            orders,
            "/Users/dev/code/orders-api-hotfix".into(),
            Some("hotfix/rate-limit"),
            None,
            WorktreeVia::Other,
            None,
            vec![f("src/middleware/rateLimit.ts", 5, 1, false)],
        ),
        (
            "/Users/dev/code/checkout-web",
            "/Users/dev/code/checkout-web/.claude/worktrees/tests".into(),
            Some("worktree-tests"),
            Some("tests"),
            WorktreeVia::Own,
            None,
            vec![],
        ),
        (
            "/Users/dev/code/handbook",
            "/Users/dev/code/handbook/.claude/worktrees/docs".into(),
            Some("worktree-docs"),
            Some("docs"),
            WorktreeVia::Own,
            None,
            vec![],
        ),
    ];
    let tilde = |p: &str| p.replacen("/Users/dev", "~", 1);
    let mut repos: Vec<&str> = wts.iter().map(|w| w.0).collect();
    repos.dedup();
    let mut files = Vec::new();
    let list = repos
        .into_iter()
        .filter_map(|repo| {
            let members: Vec<&AgentView> = agents
                .iter()
                .filter(|a| a.caps.worktrees && a.project == repo)
                .collect();
            if members.is_empty() {
                return None;
            }
            let id = format!("local:this-mac:{repo}");
            let worktrees = wts
                .iter()
                .filter(|w| w.0 == repo)
                .map(|(_, path, branch, owner, via, locked, changes)| {
                    let owner = owner.and_then(|n| agents.iter().find(|a| a.name == n));
                    let via = if owner.is_some() {
                        *via
                    } else {
                        WorktreeVia::Other
                    };
                    files.push((crate::review::model::wt_key(&id, path), changes.clone()));
                    WorktreeView {
                        path: path.clone(),
                        path_display: tilde(path),
                        name: path.rsplit('/').next().unwrap_or(path).to_string(),
                        branch: branch.map(Into::into),
                        head: Some("4f2c9e1".into()),
                        locked: locked.is_some(),
                        lock_reason: locked.map(Into::into),
                        prunable: false,
                        agent_id: owner.map(|a| a.id.clone()),
                        via,
                        caps: WorktreeCaps {
                            diff: true,
                            commit: true,
                            merge: branch.is_some(),
                            remove: locked.is_none() && via != WorktreeVia::Own,
                            terminal: true,
                        },
                    }
                })
                .collect();
            Some(ProjectWorktrees {
                id,
                repo: repo.into(),
                repo_display: tilde(repo),
                branch: Some("main".into()),
                machine: MachineView {
                    provider: "local".into(),
                    id: "this-mac".into(),
                    label: "This Mac".into(),
                    can_create: true,
                },
                agent_ids: members.iter().map(|a| a.id.clone()).collect(),
                worktrees,
                error: None,
            })
        })
        .collect();
    (list, files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_the_react_app() {
        assert_eq!(count_label(1), "1 worktree");
        assert_eq!(count_label(2), "2 worktrees");
        assert_eq!(others_label(1), "1 other worktree");
        assert_eq!(others_label(3), "3 other worktrees");
        assert_eq!(others_key("g"), "\u{0}wt:g");
    }

    #[test]
    fn the_mock_gives_refactor_two_and_orders_api_one_other() {
        let agents = super::super::demo::agents();
        let (list, files) = demo_list(&agents);
        let by_agent = worktrees_by_agent(&list);
        let refactor = &by_agent["a-refactor"];
        assert_eq!(refactor.len(), 2);
        assert_eq!(row_name(&refactor[0]), "worktree-agent-77c2");
        assert!(refactor[0].wt.locked && !refactor[0].wt.caps.remove);
        // Agents' own folders are not listed under them.
        assert!(!by_agent.contains_key("a-api-fix"));
        let ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
        let others = other_worktrees(&list, &ids);
        assert_eq!(others.len(), 1);
        assert_eq!(row_name(&others[0]), "hotfix/rate-limit");
        assert!(others[0].wt.caps.remove);
        assert_eq!(files.len(), 6);
    }

    #[test]
    fn rows_name_detached_worktrees_and_say_why_they_are_listed() {
        let agents = super::super::demo::agents();
        let (list, _) = demo_list(&agents);
        let mut r = worktrees_by_agent(&list)["a-refactor"][1].clone();
        r.wt.branch = None;
        assert_eq!(row_name(&r), "agent-a1f3 (detached)");
        r.wt.via = WorktreeVia::Process;
        assert_eq!(
            row_tip(&r, Some("boom")),
            "~/code/orders-api/.claude/worktrees/agent-a1f3\nA process of this agent works here\nboom"
        );
    }

    #[test]
    fn the_signature_moves_with_the_numbers() {
        let mut agents = super::super::demo::agents();
        let a = agents_signature(&agents);
        agents[0].added += 1;
        assert_ne!(a, agents_signature(&agents));
    }
}
