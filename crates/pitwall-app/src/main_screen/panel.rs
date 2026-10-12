//! The right panel's Changes (React `RightPanel.tsx` header, `Changes.tsx`,
//! `Freshness.tsx`): count, refresh control, Open in Review, diffstat; the
//! where line (branch or "detached HEAD", worktree tag, click the folder to
//! copy its path); the file list with +/−, "bin" and the status letter, each
//! opening the diff dialog. Read when the panel opens for an agent (forced),
//! when its totals move, every 5 s while shown, and on ↻ / ⌘⇧R.
//!
//! Next up, Last sent and the Files tab are other modules' (inventory §10).

use crate::kit::Ellipsis as _;
use std::time::{Duration, Instant};

use crate::kit::HoverText;
use gpui::{
    div, prelude::*, px, ClipboardItem, Context, FontWeight, IntoElement, SharedString, Task,
    Window,
};

use pitwall_core::engine::changes::Stage;
use pitwall_core::vcs::git::{FileChange, Part, Parts};
use pitwall_proto::AgentView;

use crate::kit::{icon, MONO_FONT};
use super::model::{split_path, CHANGES_POLL};
use crate::kit::keys;
use crate::kit::{diffstat, icon_btn, label, label_t, tooltip};
use super::{MainScreen, Route};
use crate::theme::{Theme, RIGHT_W};

#[derive(Default)]
pub struct ChangesState {
    agent: Option<String>,
    pub files: Option<Vec<FileChange>>,
    pub error: Option<String>,
    refreshing: bool,
    updated: Option<Instant>,
    /// Totals last seen (`added:removed:filesChanged`).
    sig: (u32, u32, u32),
    load: Option<Task<()>>,
    poll: Option<Task<()>>,
    copied: Option<Instant>,
}

impl ChangesState {
    /// Demo only: these files for this agent, read just now.
    pub fn agent_for_demo(&mut self, id: &str, files: Vec<FileChange>) {
        self.agent = Some(id.to_string());
        self.files = Some(files);
        self.error = None;
        self.refreshing = false;
        self.updated = Some(Instant::now());
        self.poll = None;
        self.load = None;
    }
}


impl MainScreen {
    /// Tests: show these changes for agent `id` (as if just read).
    #[cfg(test)]
    pub(crate) fn set_changes_for_test(
        &mut self,
        id: &str,
        files: Vec<FileChange>,
        cx: &mut Context<Self>,
    ) {
        self.changes.agent_for_demo(id, files);
        cx.notify();
    }

    /// The focused agent changed, or its totals moved: (re)load its changes.
    pub(super) fn changes_follow(&mut self, cx: &mut Context<Self>) {
        let sel = self.selected(cx);
        let id = sel.as_ref().map(|a| a.id.clone());
        if id != self.changes.agent {
            self.changes = ChangesState {
                agent: id.clone(),
                ..Default::default()
            };
            let Some(a) = sel else { return };
            self.changes.sig = (a.added, a.removed, a.files_changed);
            if a.caps.diff {
                self.load_changes(true, cx);
                self.start_poll(cx);
            }
            return;
        }
        if let Some(a) = sel {
            let sig = (a.added, a.removed, a.files_changed);
            if sig != self.changes.sig {
                self.changes.sig = sig;
                if a.caps.diff {
                    self.load_changes(false, cx);
                }
            }
        }
    }

    fn start_poll(&mut self, cx: &mut Context<Self>) {
        self.changes.poll = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(CHANGES_POLL).await;
            // Paused while no window shows (`document.hidden`).
            crate::platform::visible::until_any_visible(cx).await;
            let alive = this.update(cx, |s, cx| {
                // Only while the panel shows them (and the window is up).
                if s.route == Route::Space && s.changes.load.is_none() {
                    s.load_changes(false, cx);
                }
            });
            if alive.is_err() {
                break;
            }
        }));
    }

    /// Read the focused agent's changes; `force` bypasses the engine's pace.
    pub(super) fn load_changes(&mut self, force: bool, cx: &mut Context<Self>) {
        let (Some(engine), Some(id)) = (self.engine.clone(), self.changes.agent.clone()) else {
            return;
        };
        // Polls are silent: only a forced read shows "refreshing…".
        if force {
            self.changes.refreshing = true;
        }
        let agent = id.clone();
        let task = cx.background_executor().spawn(async move {
            if force {
                pitwall_core::engine::changes::refresh(&engine, &agent)
            } else {
                pitwall_core::engine::changes::changes(&engine, &agent)
            }
        });
        self.changes.load = Some(cx.spawn(async move |this, cx| {
            let res = task.await;
            let _ = this.update(cx, |s, cx| {
                if s.changes.agent.as_deref() != Some(id.as_str()) {
                    return;
                }
                s.changes.refreshing = false;
                s.changes.load = None;
                match res {
                    Ok(files) => {
                        s.changes.files = Some(files);
                        s.changes.error = None;
                        s.changes.updated = Some(Instant::now());
                    }
                    Err(e) => {
                        s.note_access_error(&e, cx);
                        s.changes.error = Some(e);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn render_panel(
        &mut self,
        a: &AgentView,
        drawer: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let top = section(super::PanelSlot::Top, a, window, cx);
        let changes = self.changes_section(a, t, window, cx).into_any_element();
        let bottom = section(super::PanelSlot::Bottom, a, window, cx);
        let head = div()
                    .h(px(32.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(14.))
                    .pr(px(8.))
                    .border_b_1()
                    .border_color(t.line)
                    .child(crate::kit::glyph_play(format!("panel-{}", a.id), a.status, t, true))
                    .child(div().flex_1().min_w_0().overflow_hidden().child(
                        label_t(a.name.clone(), t.text, 13., 0.1).font_weight(FontWeight::BOLD),
                    ))
                    .when(drawer, |d| {
                        d.child(
                            icon_btn("close-details", "x", "Close details", true, t).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.drawer_open = false;
                                    cx.notify();
                                }),
                            ),
                        )
                    });
        // `.right { overflow-y: auto }`: the whole panel scrolls.
        crate::kit::scroll_area("right-panel-area", t, move |h| {
            div()
                .id("right-panel")
                .track_scroll(h)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .child(head)
                .children(top)
                .child(changes)
                .children(bottom)
                .into_any_element()
        })
        .fill()
        .w(RIGHT_W)
        .h_full()
        .flex_none()
        .bg(t.surface)
        // In a floating glass slab: its rounded shape (GPUI clips square).
        .when(crate::theme::glass_regions(cx), |d| d.rounded(px(crate::kit::PANEL_RADIUS)))
        .border_l_1()
        .border_color(t.line)
    }

    /// Show Files (or Changes); remembered in `ui.json` for every window.
    fn set_files_tab(&mut self, on: bool, cx: &mut Context<Self>) {
        self.update_ui(
            |mut s| {
                s.right_files = on;
                s
            },
            cx,
        );
    }

    /// Changes / Files tabs (`PanelTabs`), when a Files panel is registered.
    fn panel_tabs(&self, a: &AgentView, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = |id: &'static str, text: &'static str, on: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(22.))
                .px(px(6.))
                .rounded(crate::theme::RADIUS_SM)
                .cursor_pointer()
                .when(on, |d| d.bg(t.surface_3))
                .child(label(text, if on { t.text } else { t.text_4 }, 12.))
        };
        let changed = if a.caps.diff { a.files_changed } else { 0 };
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .ml(px(-6.))
            .child(
                tab("tab-changes", "Changes", !self.ui.right_files)
                    .when(changed > 0, |d| {
                        d.child(
                            div()
                                .font_family(MONO_FONT)
                                .text_size(px(11.))
                                .text_color(t.text_3)
                                .child(changed.to_string()),
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.set_files_tab(false, cx))),
            )
            .child(
                tab("tab-files", "Files", self.ui.right_files)
                    .on_click(cx.listener(|this, _, _, cx| this.set_files_tab(true, cx))),
            )
    }

    fn changes_section(
        &mut self,
        a: &AgentView,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hint = |text: String| {
            div()
                .px(px(14.))
                .py(px(4.))
                .text_size(px(12.))
                .text_color(t.text_3)
                .child(text)
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(14.))
            .pr(px(10.))
            .pt(px(12.))
            .pb(px(6.));
        let files_hook = a.caps.explorer && cx.has_global::<super::FilesPanel>();
        let head = if files_hook {
            head.child(self.panel_tabs(a, t, cx))
        } else {
            head.child(crate::kit::label("Changes", t.text_3, 12.))
        };
        if self.ui.right_files && files_hook {
            // The Files panel draws its own head row (refresh, open the
            // viewer); the tabs sit at its start, as `FilesPanel`'s `tabs`.
            let build = cx.global::<super::FilesPanel>().0.clone();
            let view = build(a, window, cx);
            // `.panel-files` (flex: 1 1 0; min-height: 240px): the tree
            // scrolls inside, so Last sent stays in reach.
            return div()
                .relative()
                .flex_1()
                .min_h(px(240.))
                .flex()
                .flex_col()
                .child(view)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h(px(34.))
                        .pl(px(14.))
                        .flex()
                        .items_center()
                        .child(self.panel_tabs(a, t, cx)),
                );
        }
        if !a.caps.diff {
            return div().flex_grow().flex_shrink_0().flex().flex_col().child(head).child(hint(
                "Not a git repository — Pitwall can't track changes in this folder.".into(),
            ));
        }
        let c = &self.changes;
        let count = c.files.as_ref().map(|f| f.len()).unwrap_or(0);
        let note = crate::kit::fresh_note(c.refreshing, c.updated.map(|u| u.elapsed().as_secs()));
        let busy = c.refreshing;
        let head = head
            .when(count > 0 && !files_hook, |d| {
                d.child(
                    div()
                        .font_family(MONO_FONT)
                        .text_size(px(11.))
                        .text_color(t.text_3)
                        .child(count.to_string()),
                )
            })
            .child(div().flex_1())
            // With tabs the note is left out (React `note={!tabs}`).
            .child(crate::kit::refresh_control(
                "refresh-changes",
                busy,
                (!files_hook).then_some(note),
                t,
                cx.listener(|this, _, _, cx| this.load_changes(true, cx)),
            ))
            .when(a.caps.review, |d| {
                d.child(
                    icon_btn(
                        "open-review",
                        "review",
                        keys("Open in Review (⌘R)", false),
                        true,
                        t,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.set_route(Route::Review, cx))),
                )
            })
            .child(diffstat(a.added, a.removed, t));

        let body = match (&c.error, &c.files) {
            (Some(e), _) if e.contains("not-a-git-repo") => {
                hint("Not a git repository — Pitwall can't track changes in this folder.".into())
                    .into_any_element()
            }
            (Some(e), _) => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(14.))
                .py(px(4.))
                .text_size(px(12.))
                .text_color(t.red)
                .child(div().min_w_0().ellipsis().child(crate::kit::one_line(format!(
                    "Couldn't read changes: {}",
                    e.lines().next().unwrap_or("")
                ))))
                .child(
                    crate::kit::small_btn("retry-changes", "Retry", t)
                        .on_click(cx.listener(|this, _, _, cx| this.load_changes(true, cx))),
                )
                .into_any_element(),
            (None, None) => hint("Reading git…".into()).into_any_element(),
            (None, Some(f)) if f.is_empty() => {
                hint("No changes since the agent started.".into()).into_any_element()
            }
            (None, Some(files)) => {
                // Repositories inside the agent's folder: a header for each,
                // their files named from it.
                let groups = change_groups(files);
                let grouped = groups.iter().any(|(repo, _)| repo.is_some());
                let mut rows: Vec<gpui::AnyElement> = Vec::new();
                // (Each child's file as its row shows it; headers none: for
                // the rows' bounds.)
                let mut slots: Vec<Option<FileChange>> = Vec::new();
                let mut k = 0;
                for (n, (repo, idx)) in groups.iter().enumerate() {
                    let key = (a.id.clone(), repo.clone().unwrap_or_default());
                    let open = !self.folded_repos.contains(&key);
                    if grouped {
                        let branch = idx.first().and_then(|&i| files[i].branch.clone());
                        rows.push(self.repo_header(n, repo.as_deref(), branch, a, idx.len(), open, t, cx));
                        slots.push(None);
                        if !open {
                            continue;
                        }
                    }
                    let mine: Vec<&FileChange> = idx.iter().map(|&i| &files[i]).collect();
                    for (stage, part) in stage_groups(&mine) {
                        if let Some(stage) = stage {
                            rows.push(stage_header(stage, part.len(), grouped, t));
                            slots.push(None);
                        }
                        for f in part {
                            let shown = repo
                                .as_ref()
                                .and_then(|r| f.path.strip_prefix(r.as_str())?.strip_prefix('/'))
                                .unwrap_or(&f.path)
                                .to_string();
                            let row = self.file_row(k, a, &f, &shown, t, cx);
                            k += 1;
                            // Under a section header: indented, as in VS Code.
                            rows.push(if grouped {
                                div().pl(px(14.)).child(row).into_any_element()
                            } else {
                                row.into_any_element()
                            });
                            slots.push(Some(f));
                        }
                    }
                }
                div()
                    .on_children_prepainted({
                        let (rows, agent) = (self.file_rows.clone(), a.clone());
                        move |bounds, _, _| {
                            *rows.borrow_mut() = bounds
                                .into_iter()
                                .zip(&slots)
                                .filter_map(|(b, s)| s.clone().map(|f| (b, agent.clone(), f)))
                                .collect();
                        }
                    })
                    .id("file-list")
                    .px(px(6.))
                    .children(rows)
                    .into_any_element()
            }
        };
        // The agent's other worktrees follow the files (`AgentWorktrees`).
        let wts = self.panel_worktrees(a, t, cx);
        let body = match wts {
            Some(w) => div()
                .flex()
                .flex_col()
                .child(body)
                .child(w)
                .into_any_element(),
            None => body,
        };
        // `.panel-grow` (flex: 1 0 auto): a long list grows the panel,
        // which scrolls as a whole (`.right { overflow-y: auto }`).
        div()
            .flex_grow()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .child(head)
            .child(self.where_line(a, t, cx))
            .child(body)
    }

    fn where_line(&self, a: &AgentView, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let copied = self
            .changes
            .copied
            .is_some_and(|c| c.elapsed() < Duration::from_millis(1200));
        let cwd = a.cwd.clone();
        let path_tip: SharedString = if copied {
            "Copied".into()
        } else {
            format!("{} — click to copy", a.cwd).into()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(14.))
            .pb(px(6.))
            .text_size(px(11.))
            .text_color(t.text_2)
            .child(
                // `.where-branch.mono` (0.94em); the path below is in the
                // UI face (`.where-path { font: inherit }`).
                div()
                    .id("where-branch")
                    .font_family(crate::kit::MONO_FONT)
                    .text_size(px(11. * 0.94))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .min_w_0()
                    .tooltip(tooltip(if a.worktree {
                            "Works in its own git worktree"
                        } else {
                            "Current branch"
                        }
                        ))
                    .child(icon("branch", 11., t.text_2))
                    .child(
                        div()
                            .min_w_0()
                            .ellipsis()
                            .child(crate::kit::one_line(a.branch.clone().unwrap_or_else(|| "detached HEAD".into()))),
                    )
                    .when(a.worktree, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .px(px(5.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(t.line_strong)
                                .font_family(crate::kit::UI_FONT)
                                .text_size(px(10.))
                                .child("worktree"),
                        )
                    }),
            )
            .child(
                div()
                    .id("where-path")
                    .group("where-path")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .min_w_0()
                    .cursor_pointer()
                    .tooltip(tooltip(path_tip))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(cwd.clone()));
                        this.changes.copied = Some(Instant::now());
                        cx.notify();
                        // Back to the path after a moment.
                        cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(Duration::from_millis(1250))
                                .await;
                            let _ = this.update(cx, |_, cx| cx.notify());
                        })
                        .detach();
                    }))
                    .child(icon("folder", 11., t.text_2))
                    .child(
                        div()
                            .min_w_0()
                            .ellipsis()
                            .group_hover_text("where-path", t.text, |s| s.underline())
                            .child(crate::kit::one_line(if copied {
                                "Copied".to_string()
                            } else {
                                a.cwd_display.clone()
                            })),
                    ),
            )
    }

    /// A repository's section header in the Changes list (VS Code's
    /// Source Control): chevron, name (the agent's own: its folder's name),
    /// branch, the count of its changed files; a click folds it.
    #[allow(clippy::too_many_arguments)]
    fn repo_header(
        &self,
        n: usize,
        repo: Option<&str>,
        branch: Option<String>,
        a: &AgentView,
        count: usize,
        open: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let name = match repo {
            Some(r) => r.to_string(),
            None => a
                .cwd_display
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .to_string(),
        };
        // The agent's own: its branch; a nested one's as read with its files.
        let branch = match repo {
            None => a.branch.clone(),
            Some(_) => branch.map(|b| if b == "HEAD" { "detached HEAD".into() } else { b }),
        };
        let key = (a.id.clone(), repo.unwrap_or_default().to_string());
        div()
            .id(("repo-section", n))
            .debug_selector(move || format!("repo-section-{n}"))
            .when(n > 0, |d| d.mt(px(4.)).border_t_1().border_color(t.line))
            .pt(px(if n > 0 { 4. } else { 0. }))
            .child(
                div()
                    .id(("repo-header", n))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(24.))
                    .px(px(6.))
                    .rounded(crate::theme::RADIUS_SM)
                    .cursor_pointer()
                    .hover_probed(|s| s.bg(t.surface_3))
                    .tooltip(tooltip(if open { "Collapse" } else { "Expand" }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.folded_repos.remove(&key) {
                            this.folded_repos.insert(key.clone());
                        }
                        cx.notify();
                    }))
                    .child(crate::kit::chevron(("repo-chev", n), open, 12., t.text_3))
                    .child(icon("branch", 12., t.text_3))
                    .child(
                        div()
                            .min_w_0()
                            .ellipsis()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(crate::kit::one_line(name)),
                    )
                    .when_some(branch, |d, b| {
                        d.child(
                            div()
                                .min_w_0()
                                .flex_shrink()
                                .ellipsis()
                                .font_family(MONO_FONT)
                                .text_size(px(11.))
                                .text_color(t.text_3)
                                .child(crate::kit::one_line(b)),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .px(px(6.))
                            .h(px(16.))
                            .flex()
                            .items_center()
                            .rounded_full()
                            .bg(t.surface_3)
                            .font_family(MONO_FONT)
                            .text_size(px(10.5))
                            .text_color(t.text_2)
                            .child(count.to_string()),
                    ),
            )
            .into_any_element()
    }

    fn file_row(
        &self,
        i: usize,
        a: &AgentView,
        f: &FileChange,
        shown: &str,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (dir, base) = split_path(shown);
        let status = crate::kit::file_status(f);
        let (agent, file) = (a.clone(), f.clone());
        div()
            .id(("file-row", i))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(5.))
            .rounded(crate::theme::RADIUS_SM)
            .font_family(MONO_FONT)
            .text_size(px(12.))
            .line_height(px(17.))
            .cursor_pointer()
            .hover_probed(|s| s.bg(t.surface_3))
            .tooltip(tooltip(f.path.clone()))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_diff(agent.clone(), file.clone(), window, cx)
            }))
            .child(div().mr(px(-2.)).child(crate::kit::file_icon(&f.path, 16.)))
            .child(
                div()
                    // (Not on the row: its hover probe names it.)
                    .debug_selector(move || format!("file-name-{i}"))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_shrink()
                            .min_w_0()
                            .ellipsis()
                            .text_color(t.text_3)
                            .child(crate::kit::one_line(dir.to_string())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .when(status == pitwall_proto::FileStatus::D, |d| {
                                d.line_through().text_decoration_color(t.text_3)
                            })
                            .child(base.to_string()),
                    ),
            )
            .child(file_stats(f, t))
            .child(
                div()
                    .id(("letter", i))
                    .flex_none()
                    .tooltip(tooltip(crate::kit::status_title(status)))
                    .child(crate::kit::status_letter(status, t)),
            )
    }
}

/// VS Code's "Staged Changes" and "Changes" of one repository's files:
/// each part as its own row (its counts and letter; `parts` saying which,
/// for its diff). Nothing staged: the files as they are, no headers.
fn stage_groups(files: &[&FileChange]) -> Vec<(Option<Stage>, Vec<FileChange>)> {
    let staged = files.iter().any(|f| f.parts.as_ref().is_some_and(|p| p.staged.is_some()));
    if !staged {
        let whole = files.iter().map(|f| FileChange { parts: None, ..(*f).clone() }).collect();
        return vec![(None, whole)];
    }
    let part = |f: &FileChange, stage: Stage| {
        let parts = f.parts.as_ref();
        let p = match stage {
            Stage::Staged => parts.and_then(|p| p.staged.clone()),
            // (No parts read: all of it counts as not staged.)
            Stage::Unstaged => match parts {
                Some(p) => p.unstaged.clone(),
                None => Some(Part { added: f.added, removed: f.removed, status: f.status }),
            },
        }?;
        let one = match stage {
            Stage::Staged => Parts { staged: Some(p.clone()), unstaged: None },
            Stage::Unstaged => Parts { staged: None, unstaged: Some(p.clone()) },
        };
        Some(FileChange {
            added: p.added,
            removed: p.removed,
            status: p.status.or(f.status),
            untracked: f.untracked && stage == Stage::Unstaged,
            parts: Some(one),
            ..f.clone()
        })
    };
    [Stage::Staged, Stage::Unstaged]
        .into_iter()
        .map(|stage| (Some(stage), files.iter().filter_map(|f| part(f, stage)).collect::<Vec<_>>()))
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

/// Which part of a file a row (and its diff) is: staged, not, or both.
pub(crate) fn stage_of(f: &FileChange) -> Option<Stage> {
    match f.parts.as_ref()? {
        Parts { staged: Some(_), unstaged: None } => Some(Stage::Staged),
        Parts { staged: None, unstaged: Some(_) } => Some(Stage::Unstaged),
        _ => None,
    }
}

/// "STAGED CHANGES 2" / "CHANGES 5" over a repository's two parts.
fn stage_header(stage: Stage, count: usize, indent: bool, t: &Theme) -> gpui::AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .when(indent, |d| d.pl(px(22.)))
        .when(!indent, |d| d.pl(px(8.)))
        .pr(px(8.))
        .pt(px(6.))
        .pb(px(2.))
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.text_3)
        .child(match stage {
            Stage::Staged => "STAGED CHANGES",
            Stage::Unstaged => "CHANGES",
        })
        .child(div().flex_1())
        .child(div().font_family(MONO_FONT).child(count.to_string()))
        .into_any_element()
}

/// The changed files by repository: the agent's own first, then those
/// inside its folder by name; indexes into `files`.
fn change_groups(files: &[FileChange]) -> Vec<(Option<String>, Vec<usize>)> {
    let mut groups: Vec<(Option<String>, Vec<usize>)> = Vec::new();
    for (i, f) in files.iter().enumerate() {
        match groups.iter_mut().find(|(r, _)| *r == f.repo) {
            Some((_, idx)) => idx.push(i),
            None => groups.push((f.repo.clone(), vec![i])),
        }
    }
    groups.sort_by(|(a, _), (b, _)| a.cmp(b));
    groups
}

/// Another module's panel section (Next up, Last sent), if registered.
fn section(
    slot: super::PanelSlot,
    a: &AgentView,
    window: &mut Window,
    cx: &mut gpui::App,
) -> Option<gpui::AnyView> {
    let build = cx
        .try_global::<super::PanelSections>()?
        .0
        .get(&slot)?
        .clone();
    Some(build(a, window, cx))
}

/// +/− (or "bin") of one file (`FileStats`).
pub fn file_stats(f: &FileChange, t: &Theme) -> impl IntoElement {
    let d = div()
        .flex()
        .flex_none()
        .gap(px(4.))
        .text_size(px(11.))
        .font_family(crate::kit::MONO_FONT);
    if f.binary {
        return d.child(crate::kit::chip("bin", t));
    }
    d.when(f.added > 0, |d| {
        d.child(div().text_color(t.green).child(format!("+{}", f.added)))
    })
    .when(f.removed > 0, |d| {
        d.child(div().text_color(t.red).child(format!("−{}", f.removed)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file staged and edited again is a row in each part, with that
    /// part's counts; nothing staged anywhere: plain rows, no headers.
    #[test]
    fn staged_and_not_staged_rows() {
        let part = |a: u32, r: u32| Some(Part { added: a, removed: r, status: None });
        let f = |path: &str, staged, unstaged| FileChange {
            path: path.into(),
            added: 9,
            parts: Some(Parts { staged, unstaged }),
            ..FileChange::default()
        };
        let both = f("a.rs", part(1, 0), part(2, 1));
        let only = f("b.rs", None, part(3, 0));
        let groups = stage_groups(&[&both, &only]);
        type Row<'a> = (&'a str, u32, u32, Option<Stage>);
        let got: Vec<(Option<Stage>, Vec<Row>)> = groups
            .iter()
            .map(|(s, rows)| {
                (*s, rows.iter().map(|r| (r.path.as_str(), r.added, r.removed, stage_of(r))).collect())
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (Some(Stage::Staged), vec![("a.rs", 1, 0, Some(Stage::Staged))]),
                (
                    Some(Stage::Unstaged),
                    vec![("a.rs", 2, 1, Some(Stage::Unstaged)), ("b.rs", 3, 0, Some(Stage::Unstaged))]
                ),
            ]
        );
        let plain = stage_groups(&[&only]);
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].0, None, "no headers");
        assert_eq!((plain[0].1[0].added, stage_of(&plain[0].1[0])), (9, None), "the whole file");
    }

    #[test]
    fn changes_group_by_repository_the_agents_own_first() {
        let f = |path: &str, repo: Option<&str>| FileChange {
            path: path.into(),
            repo: repo.map(String::from),
            ..FileChange::default()
        };
        let files = [
            f("upstreams/b/x.rs", Some("upstreams/b")),
            f("a/y.rs", Some("a")),
            f("main.rs", None),
            f("upstreams/b/z.rs", Some("upstreams/b")),
        ];
        assert_eq!(
            change_groups(&files),
            vec![
                (None, vec![2]),
                (Some("a".to_string()), vec![1]),
                (Some("upstreams/b".to_string()), vec![0, 3]),
            ]
        );
    }
}
