//! Drawing Review, after `Review.tsx`, `FileTree.tsx`, `WorktreeSection.tsx`
//! and `review.css`: the bar, the agents → files list, the main pane
//! (header, diff, comment composer, footer actions) and the empty states.

use crate::kit::Ellipsis as _;
use crate::kit::HoverText as _;
use gpui::{
    anchored, deferred, div, point, prelude::*, px, relative, AnyElement, App, ClickEvent, Context,
    Corner, Div, Focusable, FontWeight, IntoElement, MouseButton, SharedString, Window,
};

use pitwall_proto::{AgentView, FileStatus};

use super::comments::Comments;
use super::model::{self, TreeRow, WtRef};
use super::{
    Back, CancelComment, DiffState, NextFile, OpenFile, PreviousFile, RefreshChanges, ReviewView,
    Sel,
};
use crate::kit::{self, BtnKind, MONO_FONT};
use crate::theme::{theme, Theme, RADIUS, RADIUS_LG, RADIUS_SM};

const LIST_W: f32 = 300.;

/// ⌘↵ / Ctrl+↵ (the text input's submit keys).
pub(super) const SUBMIT_KEYS: &str = if cfg!(target_os = "macos") {
    "⌘↵"
} else {
    "Ctrl+↵"
};

/// Every file row the list shows, in order: (owner key, path).
pub(super) fn list_file_rows(v: &ReviewView, cx: &App) -> Vec<(String, String)> {
    let _ = cx;
    let mut out = Vec::new();
    let mut tree_rows = |owner: &str, files: &[pitwall_core::vcs::git::FileChange]| {
        let tree = model::build_tree(files);
        let closed = |d: &str| v.closed_dirs.contains(&format!("{owner}\u{1}{d}"));
        for r in model::visible_rows(&tree, &closed) {
            if let TreeRow::File { path, .. } = r {
                out.push((owner.to_string(), path));
            }
        }
    };
    let by_agent = model::worktrees_by_agent(&v.projects);
    for g in &v.groups {
        for a in &g.agents {
            if v.collapsed.contains(&a.id) {
                continue;
            }
            if let Some(files) = v.files.get(&a.id) {
                tree_rows(&a.id, files);
            }
            for r in by_agent.get(&a.id).into_iter().flatten() {
                let key = r.key();
                if let (true, Some(Ok(files))) = (v.wt_open.contains(&key), v.wt_files.get(&key)) {
                    tree_rows(&key, files);
                }
            }
        }
        let ids: Vec<&str> = g.agents.iter().map(|a| a.id.as_str()).collect();
        for r in model::other_worktrees(&v.projects, &ids) {
            let key = r.key();
            if let (true, Some(Ok(files))) = (v.wt_open.contains(&key), v.wt_files.get(&key)) {
                tree_rows(&key, files);
            }
        }
    }
    out
}

fn hint(text: impl Into<SharedString>, t: &Theme) -> Div {
    kit::hint(text, t)
}

fn mono() -> Div {
    div().font_family(MONO_FONT)
}

fn spacer() -> Div {
    div().flex_1()
}

impl ReviewView {
    fn bar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let all = Self::all_projects(cx);
        let note: SharedString = if self.refreshing {
            "refreshing…".into()
        } else {
            self.updated_at
                .map(|at| model::updated_label(at, super::now_ms().max(at)))
                .unwrap_or_default()
                .into()
        };
        let seg = |id: &'static str, label: &'static str, on: bool, first: bool| {
            div()
                .id(id)
                .h(px(24.))
                .px(px(9.))
                .flex()
                .items_center()
                .cursor_pointer()
                .when(!first, |d| d.border_l_1().border_color(t.line_strong))
                .when(on, |d| d.bg(t.surface_3))
                .child(kit::label_t(label, if on { t.text } else { t.text_3 }, 11., 0.08))
        };
        div()
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(12.))
            .pr(px(8.))
            .border_b_1()
            .border_color(t.line)
            .child(kit::label("Review", t.text_3, 12.))
            .when(self.can_widen, |d| {
                d.child(
                    div()
                        .flex_none()
                        .max_w(px(180.))
                        .overflow_hidden()
                        .px(px(6.))
                        .pt(px(3.))
                        .pb(px(2.))
                        .rounded(px(3.))
                        .bg(t.surface_3)
                        .child(kit::label_fit(
                            if all { "all projects".to_string() } else { kit::project_name(&self.label).to_string() },
                            t.text_2,
                            11.,
                            26,
                        )),
                )
            })
            .child(
                hint(
                    "what your agents changed · click a line number to comment",
                    t,
                )
                .min_w_0()
                .ellipsis(),
            )
            .child(spacer())
            .when(self.can_widen, |d| {
                d.child(
                    div()
                        .id("rv-all")
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(5.))
                        .text_size(px(12.))
                        .text_color(t.text_2)
                        .cursor_pointer()
                        .child(kit::checkbox_box(all, false, 12., t))
                        .child("All projects")
                        .on_click(cx.listener(move |v, _, _, cx| v.set_all_projects(!all, cx))),
                )
            })
            .child(self.refresh_control(
                "rv-refresh",
                true,
                &note,
                t,
                cx.listener(|v, _, _, cx| v.refresh_all(cx)),
            ))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .border_1()
                    .border_color(t.line_strong)
                    .rounded(RADIUS)
                    .overflow_hidden()
                    .child(
                        seg("rv-sbs", "Side by side", self.side_by_side, true)
                            .on_click(cx.listener(|v, _, _, cx| v.set_side_by_side(true, cx))),
                    )
                    .child(
                        seg("rv-inline", "Inline", !self.side_by_side, false)
                            .on_click(cx.listener(|v, _, _, cx| v.set_side_by_side(false, cx))),
                    ),
            )
            .child(
                kit::button("rv-back", BtnKind::Small, false, t)
                    .child("Back")
                    .child(kit::kbd("esc", t))
                    .on_click(cx.listener(|v, _, _, cx| v.exit(cx))),
            )
    }

    /// "refreshing…" / "updated N s ago" and ↻.
    fn refresh_control(
        &self,
        id: &'static str,
        show_note: bool,
        note: &SharedString,
        t: &Theme,
        on: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Div {
        kit::refresh_control(id, self.refreshing, show_note.then(|| note.clone()), t, on)
    }

    // ── the list ────────────────────────────────────────────────────────

    fn list(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col: Vec<AnyElement> = Vec::new();
        if self.ordered.is_empty() {
            col.push(
                hint(
                    if self.hidden > 0 {
                        "No agents with changes to review."
                    } else {
                        "No agents yet."
                    },
                    t,
                )
                .p(px(16.))
                .into_any_element(),
            );
        }
        let by_agent = model::worktrees_by_agent(&self.projects);
        let list_focused = self.list_focus.is_focused(window);
        for (gi, g) in self.groups.iter().enumerate() {
            let mut group = div()
                .flex()
                .flex_col()
                .when(gi > 0, |d| d.border_t_1().border_color(t.line_strong));
            for (ai, a) in g.agents.iter().enumerate() {
                let wts = by_agent.get(&a.id).cloned().unwrap_or_default();
                group = group.child(self.agent_section(a, ai > 0, &wts, list_focused, t, cx));
            }
            let ids: Vec<&str> = g.agents.iter().map(|a| a.id.as_str()).collect();
            let others = model::other_worktrees(&self.projects, &ids);
            if !others.is_empty() {
                let mut o = div()
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_color(t.line)
                    .pt(px(4.))
                    .child(
                        div()
                            .pt(px(6.))
                            .pr(px(10.))
                            .pb(px(2.))
                            .pl(px(12.))
                            .child(kit::label(
                                format!("Other worktrees · {}", kit::project_name(&g.display)),
                                t.text_3,
                                12.,
                            )),
                    );
                for r in &others {
                    o = o.child(self.worktree_section(r, list_focused, t, cx));
                }
                group = group.child(o);
            }
            col.push(group.into_any_element());
        }
        if self.hidden > 0 && !self.ordered.is_empty() {
            let who = if self.hidden == 1 {
                "1 agent works".to_string()
            } else {
                format!("{} agents work", self.hidden)
            };
            col.push(
                hint(
                    format!("{who} outside a git repository — no changes to review."),
                    t,
                )
                .p(px(16.))
                .into_any_element(),
            );
        }
        let list = div()
            .id("rv-list")
            .track_scroll(&self.list_scroll)
            .key_context("ReviewList")
            .track_focus(&self.list_focus)
            .on_action(cx.listener(|v, _: &PreviousFile, _, cx| v.move_cursor(-1, cx)))
            .on_action(cx.listener(|v, _: &NextFile, _, cx| v.move_cursor(1, cx)))
            .on_action(cx.listener(|v, _: &OpenFile, window, cx| {
                if let Some((owner, path)) = v.cursor.clone() {
                    v.select_file(&owner, &path, window, cx);
                }
            }))
            .size_full()
            .overflow_y_scroll()
            .pt(px(4.))
            .pb(px(16.))
            .flex()
            .flex_col()
            .children(col);
        kit::vscroll_fill("rv-list-bar", &self.list_scroll, t, list)
            .w(px(LIST_W))
            .min_w(px(220.))
            .flex_none()
            .h_full()
            .border_r_1()
            .border_color(t.line)
            .bg(t.surface)
    }

    fn chevron_toggle(id: SharedString, open: bool, size: f32, t: &Theme) -> gpui::Stateful<Div> {
        div()
            .id(id)
            .w(px(18.))
            .h(px(22.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(kit::chevron("chev", open, size, t.text_4))
    }

    fn agent_section(
        &self,
        a: &AgentView,
        border: bool,
        wts: &[WtRef],
        list_focused: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let list = self.files.get(&a.id);
        let err = self.errors.get(&a.id);
        let open = !self.collapsed.contains(&a.id);
        let current = matches!(&self.sel, Sel::Agent { id, .. } if id == &a.id);
        let scoped = self.task_scope.get(&a.id).cloned().flatten().is_some();
        let comments = Comments::for_agent(cx, &a.id);
        let (added, removed) = list.map(|l| model::diffstat(l)).unwrap_or((0, 0));
        let id = a.id.clone();
        let group: SharedString = format!("rv-agent-name-{}", a.id).into();
        let head = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .pt(px(6.))
            .pr(px(10.))
            .pb(px(4.))
            .pl(px(6.))
            .min_w_0()
            .child(
                Self::chevron_toggle(format!("rv-tog-{}", a.id).into(), open, 12., t).on_click(
                    cx.listener({
                        let id = id.clone();
                        move |v, _, _, cx| {
                            if !v.collapsed.remove(&id) {
                                v.collapsed.insert(id.clone());
                            }
                            cx.notify();
                        }
                    }),
                ),
            )
            .child(
                div()
                    .id(group.clone())
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(4.))
                    .py(px(2.))
                    .rounded(RADIUS_SM)
                    .cursor_pointer()
                    .hover_probed(|s| s.bg(t.surface_3))
                    .tooltip({
                        let tip = format!("{} · {}", a.name, a.cwd_display);
                        move |_, cx| kit::tooltip_view(tip.clone(), cx)
                    })
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |v, _, _, cx| v.show_agent(&id, cx)
                    }))
                    .child(kit::glyph_play(format!("rv-list-{}", a.id), a.status, t, true))
                    .child(
                        kit::label_t(a.name.clone(), if current { t.text } else { t.text_2 }, 13., 0.1)
                            .font_weight(FontWeight::BOLD),
                    )
                    .when_some(a.branch.clone(), |d, b| {
                        d.child(
                            mono()
                                .min_w_0()
                                .ellipsis()
                                .text_size(px(10.5))
                                .text_color(t.text_4)
                                .child(crate::kit::one_line(b)),
                        )
                    }),
            )
            .when(list.is_some_and(|l| !l.is_empty()), |d| {
                d.child(kit::diffstat(added, removed, t))
            });
        let mut body = div().flex().flex_col();
        if open {
            if scoped {
                body = body.child(
                    div()
                        .ml(px(34.))
                        .mb(px(2.))
                        .child(kit::label_t("this task only", t.text_3, 10.5, 0.1)),
                );
            }
            if let Some(e) = err {
                let id = id.clone();
                body = body.child(
                    self.fresh_error(
                        e,
                        "",
                        t,
                        cx.listener(move |v, _, _, cx| v.refresh_agent(&id, cx)),
                    )
                    .pt(px(2.))
                    .pr(px(14.))
                    .pb(px(4.))
                    .pl(px(34.)),
                );
            } else if list.is_none() {
                body = body.child(
                    hint("Reading git…", t)
                        .pt(px(2.))
                        .pr(px(14.))
                        .pb(px(4.))
                        .pl(px(34.)),
                );
            } else if list.is_some_and(|l| l.is_empty()) {
                body = body.child(
                    hint(
                        if scoped {
                            "No changes in this task."
                        } else {
                            "No changes."
                        },
                        t,
                    )
                    .pt(px(2.))
                    .pr(px(14.))
                    .pb(px(4.))
                    .pl(px(34.)),
                );
            }
            if let Some(l) = list.filter(|l| !l.is_empty()) {
                let sel_path = match &self.sel {
                    Sel::Agent { id: s, path } if s == &a.id => path.clone(),
                    _ => None,
                };
                let counts = comments
                    .iter()
                    .fold(std::collections::HashMap::new(), |mut m, c| {
                        *m.entry(c.path.clone()).or_insert(0usize) += 1;
                        m
                    });
                body = body.child(self.file_tree(
                    &a.id,
                    l,
                    sel_path.as_deref(),
                    &counts,
                    list_focused,
                    t,
                    cx,
                ));
            }
            if !comments.is_empty() {
                let mut ul = div()
                    .flex()
                    .flex_col()
                    .mt(px(6.))
                    .mx(px(6.))
                    .ml(px(18.))
                    .pt(px(4.))
                    .border_t_1()
                    .border_dashed()
                    .border_color(t.line_strong);
                for c in comments {
                    let (agent, c2, cid) = (a.id.clone(), c.clone(), c.id);
                    let agent2 = a.id.clone();
                    ul = ul.child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(2.))
                            .child(
                                div()
                                    .id(SharedString::from(format!("rv-c-{}", c.id)))
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(1.))
                                    .px(px(8.))
                                    .py(px(4.))
                                    .rounded(RADIUS_SM)
                                    .cursor_pointer()
                                    .hover_probed(|s| s.bg(t.surface_3))
                                    .tooltip(|_, cx| {
                                        kit::tooltip_view("Show in diff".to_string(), cx)
                                    })
                                    .on_click(cx.listener(move |v, _, window, cx| {
                                        v.reveal_comment(&agent, &c2, window, cx)
                                    }))
                                    .child(
                                        mono().text_size(px(10.5)).text_color(t.text_3).child(
                                            format!("{}:{}", model::split_path(&c.path).1, c.line),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(t.text_2)
                                            .ellipsis()
                                            .child(crate::kit::one_line(c.text.clone())),
                                    ),
                            )
                            .child(
                                kit::icon_btn(
                                    SharedString::from(format!("rv-cx-{}", c.id)),
                                    "x",
                                    "Delete comment",
                                    true,
                                    t,
                                )
                                .on_click(cx.listener(
                                    move |_, _, _, cx| Comments::remove(cx, &agent2, cid),
                                )),
                            ),
                    );
                }
                body = body.child(ul);
            }
            if !wts.is_empty() {
                let mut box_ = div()
                    .flex()
                    .flex_col()
                    .ml(px(18.))
                    .border_l_1()
                    .border_color(t.line);
                for r in wts {
                    box_ = box_.child(self.worktree_section(r, list_focused, t, cx));
                }
                body = body.child(box_);
            }
        }
        div()
            .flex()
            .flex_col()
            .pt(px(2.))
            .pb(px(6.))
            .when(border, |d| d.border_t_1().border_color(t.line))
            .child(head)
            .child(body)
    }

    fn fresh_error(
        &self,
        e: &str,
        prefix: &str,
        t: &Theme,
        retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Div {
        let first: String = e.lines().next().unwrap_or("").chars().take(160).collect();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .child(
                div()
                    .min_w_0()
                    .ellipsis()
                    .text_color(t.red)
                    .child(crate::kit::one_line(format!("{prefix}{first}"))),
            )
            .child(
                div()
                    .id(SharedString::from(format!("retry-{prefix}{first}")))
                    .flex_none()
                    .text_color(t.text_2)
                    .cursor_pointer()
                    .hover_text(t.text, |s| s)
                    .child("Retry")
                    .on_click(retry),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn file_tree(
        &self,
        owner: &str,
        files: &[pitwall_core::vcs::git::FileChange],
        selected: Option<&str>,
        counts: &std::collections::HashMap<String, usize>,
        list_focused: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tree = model::build_tree(files);
        let closed = |d: &str| self.closed_dirs.contains(&format!("{owner}\u{1}{d}"));
        let rows = model::visible_rows(&tree, &closed);
        let mut out = div().flex().flex_col().pl(px(12.)).pr(px(6.));
        for r in rows {
            match r {
                TreeRow::Dir {
                    path,
                    name,
                    depth,
                    open,
                } => {
                    let key = format!("{owner}\u{1}{path}");
                    out = out.child(
                        mono()
                            .id(SharedString::from(format!("rv-d-{key}")))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .w_full()
                            .pl(px(6. + depth as f32 * 12.))
                            .pr(px(8.))
                            .py(px(3.))
                            .rounded(RADIUS_SM)
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .cursor_pointer()
                            .hover_probed(|s| s.bg(t.surface_3))
                            .tooltip({
                                let p = path.clone();
                                move |_, cx| kit::tooltip_view(p.clone(), cx)
                            })
                            .on_click(cx.listener(move |v, _, _, cx| {
                                if !v.closed_dirs.remove(&key) {
                                    v.closed_dirs.insert(key.clone());
                                }
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .w(px(10.))
                                    .flex_none()
                                    .child(kit::chevron("chev", open, 10., t.text_4)),
                            )
                            .child(kit::folder_icon(&path, open, 16.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .ellipsis()
                                    .text_color(t.text_2)
                                    .child(crate::kit::one_line(name)),
                            ),
                    );
                }
                TreeRow::File {
                    path,
                    name,
                    depth,
                    file,
                } => {
                    let is_sel = selected == Some(path.as_str());
                    let is_cursor = list_focused
                        && self
                            .cursor
                            .as_ref()
                            .is_some_and(|(o, p)| o == owner && p == &path);
                    let status = model::file_status(&file);
                    let n = counts.get(&path).copied().unwrap_or(0);
                    let (o, p) = (owner.to_string(), path.clone());
                    out = out.child(
                        mono()
                            .id(SharedString::from(format!("rv-f-{owner}\u{1}{path}")))
                            .relative()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .w_full()
                            .pl(px(6. + depth as f32 * 12.))
                            .pr(px(8.))
                            .py(px(3.))
                            .rounded(RADIUS_SM)
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .cursor_pointer()
                            .when(is_sel, |d| d.bg(t.surface_3))
                            .when(!is_sel, |d| d.hover_probed(|s| s.bg(t.surface_3)))
                            .when(is_cursor, |d| d.border_1().border_color(t.focus))
                            .tooltip({
                                let p = path.clone();
                                move |_, cx| kit::tooltip_view(p.clone(), cx)
                            })
                            .on_click(cx.listener(move |v, _, window, cx| {
                                v.select_file(&o, &p, window, cx)
                            }))
                            .when(is_sel, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .left_0()
                                        .top_0()
                                        .bottom_0()
                                        .w(px(2.))
                                        .bg(t.text_2),
                                )
                            })
                            .child(div().w(px(10.)).flex_none())
                            .child(kit::file_icon(&path, 16.).mr(px(-2.)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .ellipsis()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .when(status == FileStatus::D, |d| d.line_through())
                                    .child(crate::kit::one_line(name)),
                            )
                            .when(n > 0, |d| d.child(kit::count_badge(n, t)))
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .gap(px(8.))
                                    .ml_auto()
                                    .child(if file.binary {
                                        kit::chip_subtle("bin", t)
                                    } else {
                                        kit::diffstat_inline(file.added, file.removed, t)
                                    })
                                    .child(kit::status_letter(status, t)),
                            ),
                    );
                }
            }
        }
        out
    }

    fn worktree_section(
        &self,
        r: &WtRef,
        list_focused: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let key = r.key();
        let open = self.wt_open.contains(&key);
        let current = matches!(&self.sel, Sel::Worktree { key: k, .. } if k == &key);
        let files = self.wt_files.get(&key);
        let ok = files.and_then(|f| f.as_ref().ok());
        let (added, removed) = ok.map(|l| model::diffstat(l)).unwrap_or((0, 0));
        let wt = &r.wt;
        let tip = format!(
            "{}{}",
            wt.path_display,
            match (&wt.lock_reason, wt.locked) {
                (Some(why), _) => format!("\nLocked: {why}"),
                (None, true) => "\nLocked".into(),
                _ => String::new(),
            }
        );
        let head = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .pt(px(6.))
            .pr(px(10.))
            .pb(px(4.))
            .pl(px(6.))
            .min_w_0()
            .child(
                Self::chevron_toggle(format!("rv-wtog-{key}").into(), open, 12., t).on_click(
                    cx.listener({
                        let key = key.clone();
                        move |v, _, _, cx| v.toggle_worktree(&key, cx)
                    }),
                ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("rv-wt-{key}")))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(4.))
                    .py(px(2.))
                    .rounded(RADIUS_SM)
                    .cursor_pointer()
                    .hover_probed(|s| s.bg(t.surface_3))
                    .tooltip(move |_, cx| kit::tooltip_view(tip.clone(), cx))
                    .on_click(cx.listener({
                        let key = key.clone();
                        move |v, _, _, cx| v.show_worktree(&key, cx)
                    }))
                    .child(kit::icon("branch", 12., t.text_3))
                    .child(
                        kit::label_t(wt.name.clone(), if current { t.text } else { t.text_2 }, 13., 0.1)
                            .font_weight(FontWeight::BOLD),
                    )
                    .child(
                        mono()
                            .min_w_0()
                            .ellipsis()
                            .text_size(px(10.5))
                            .text_color(t.text_4)
                            .child(crate::kit::one_line(wt.branch.clone().unwrap_or_else(|| "detached".into()))),
                    )
                    .when(wt.locked, |d| {
                        d.child(kit::chip_tone("locked", kit::Tone::Warn, t).ml(px(4.)))
                    }),
            )
            .when(ok.is_some_and(|l| !l.is_empty()), |d| {
                d.child(kit::diffstat(added, removed, t))
            })
            .when(open, |d| {
                let key = key.clone();
                let note = SharedString::default();
                d.child(self.refresh_control(
                    "rv-wt-refresh",
                    false,
                    &note,
                    t,
                    cx.listener(move |v, _, _, cx| {
                        if let Some(r) = v.wt_ref(&key) {
                            v.load_worktrees(Some(Some(r.project_id.clone())), cx);
                        }
                        v.load_worktree_files(&key, cx);
                    }),
                ))
            });
        let mut body = div().flex().flex_col();
        if open {
            let pad = |d: Div| d.pt(px(2.)).pr(px(14.)).pb(px(4.)).pl(px(34.));
            if !wt.caps.diff {
                body = body.child(pad(hint("Its folder is gone.", t)));
            }
            match files {
                Some(Err(e)) => {
                    let key = key.clone();
                    body = body.child(pad(self.fresh_error(
                        e,
                        "",
                        t,
                        cx.listener(move |v, _, _, cx| v.load_worktree_files(&key, cx)),
                    )));
                }
                None if wt.caps.diff => body = body.child(pad(hint("Reading git…", t))),
                Some(Ok(l)) if l.is_empty() => {
                    body = body.child(pad(hint(
                        format!(
                            "No changes against {}",
                            r.target
                                .clone()
                                .unwrap_or_else(|| "the project's branch".into())
                        ),
                        t,
                    )))
                }
                Some(Ok(l)) => {
                    let sel_path = match &self.sel {
                        Sel::Worktree { key: k, path } if k == &key => path.clone(),
                        _ => None,
                    };
                    body = body.child(self.file_tree(
                        &key,
                        l,
                        sel_path.as_deref(),
                        &Default::default(),
                        list_focused,
                        t,
                        cx,
                    ));
                }
                None => {}
            }
        }
        div()
            .flex()
            .flex_col()
            .pt(px(2.))
            .pb(px(6.))
            .child(head)
            .child(body)
    }

    // ── the main pane ───────────────────────────────────────────────────

    fn file_head(f: &pitwall_core::vcs::git::FileChange, t: &Theme) -> Div {
        let (dir, base) = model::split_path(&f.path);
        let dir = dir.trim_end_matches('/').to_string();
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .min_w_0()
            .child(kit::file_icon(&f.path, 16.))
            .child(
                mono()
                    .min_w_0()
                    .ellipsis()
                    .text_size(px(12.))
                    .flex()
                    .gap(px(4.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(base.to_string()),
                    )
                    .child(div().text_color(t.text_3).ellipsis().child(crate::kit::one_line(dir))),
            )
            .child(kit::status_letter(model::file_status(f), t))
            .child(if f.binary {
                kit::chip_subtle("bin", t)
            } else {
                kit::diffstat(f.added, f.removed, t)
            })
    }

    fn head_row() -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(7.))
            .min_w_0()
    }

    fn empty(text: &str, t: &Theme) -> Div {
        div()
            .flex_1()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(kit::label_t(text.to_string(), t.text_4, 13., 0.12).font_weight(FontWeight::NORMAL))
    }

    fn diff_area(
        &self,
        empty_text: &str,
        has_file: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let colors_bg = t.term_bg;
        let mut area = div()
            .relative()
            .flex_1()
            .min_h_0()
            .bg(colors_bg)
            .flex()
            .flex_col();
        if !has_file {
            return area.child(Self::empty(empty_text, t));
        }
        area = match &self.diff {
            DiffState::Error(e) => area.child(
                div()
                    .p(px(16.))
                    .text_size(px(12.))
                    .text_color(t.red)
                    .child(format!("Couldn't load this file: {e}")),
            ),
            DiffState::Loading | DiffState::Idle => area.child(hint("Loading…", t).p(px(16.))),
            DiffState::Binary => {
                area.child(Self::empty("Binary or very large file — not shown", t))
            }
            DiffState::Empty => area.child(Self::empty("No content on either side", t)),
            DiffState::Shown => area.child(div().flex_1().min_h_0().child(self.code.clone())),
        };
        if let Some(c) = self.composer.as_ref() {
            let field = c.field.clone();
            area = area.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(14.))
                    .flex()
                    .justify_center()
                    .px(px(14.))
                    .child(
                        div()
                            .id("rv-composer")
                            .key_context("RvComposer")
                            .on_action(cx.listener(|v, _: &CancelComment, window, cx| {
                                v.composer = None;
                                window.focus(&v.code.focus_handle(cx));
                                cx.notify();
                            }))
                            .occlude()
                            .w(px(560.))
                            .max_w_full()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .pt(px(10.))
                            .px(px(12.))
                            .pb(px(12.))
                            .rounded(RADIUS_LG)
                            .bg(t.raised)
                            .border_1()
                            .border_color(t.line_strong)
                            .shadow_lg()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(kit::label("Comment", t.text_3, 12.))
                                    .child(
                                        mono()
                                            .min_w_0()
                                            .ellipsis()
                                            .text_size(px(11.5))
                                            .text_color(t.text_2)
                                            .child(crate::kit::one_line(format!("{}:{}", c.path, c.line))),
                                    )
                                    .child(spacer())
                                    .child(
                                        kit::icon_btn("rv-comp-x", "x", "Close (esc)", true, t)
                                            .on_click(cx.listener(|v, _, _, cx| {
                                                v.composer = None;
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(field)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(hint("Collected, not sent.", t))
                                    .child(kit::kbd(SUBMIT_KEYS, t))
                                    .child(hint("to add", t))
                                    .child(spacer())
                                    .child(
                                        kit::button("rv-comp-cancel", BtnKind::Ghost, false, t)
                                            .child("Cancel")
                                            .on_click(cx.listener(|v, _, _, cx| {
                                                v.composer = None;
                                                cx.notify();
                                            })),
                                    )
                                    .child({
                                        let can = !c.field.read(cx).text().trim().is_empty();
                                        kit::button("rv-comp-add", BtnKind::Primary, !can, t)
                                            .child("Add comment")
                                            .when(can, |d| {
                                                d.on_click(cx.listener(|v, _, window, cx| {
                                                    v.add_comment(window, cx)
                                                }))
                                            })
                                    }),
                            ),
                    ),
            );
        }
        area
    }

    fn task_select(
        &self,
        a: &AgentView,
        task: Option<&String>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let numbered: Vec<(usize, &pitwall_core::engine::Task)> = self
            .tasks
            .iter()
            .enumerate()
            .map(|(i, t)| (i + 1, t))
            .collect();
        let current = match task {
            None => format!("Since the agent started · {}", a.name),
            Some(id) => numbered
                .iter()
                .find(|(_, x)| &x.id == id)
                .map(|(n, x)| model::task_label(x, *n, &model::clock))
                .unwrap_or_default(),
        };
        // A native `<select>` is as wide as its widest option.
        let options: Vec<String> = std::iter::once(format!("Since the agent started · {}", a.name))
            .chain(numbered.iter().map(|(n, x)| {
                format!(
                    "{}{}",
                    model::task_label(x, *n, &model::clock),
                    if x.start_tree.is_some() { "" } else { " (snapshot pending)" }
                )
            }))
            .collect();
        let open = self.task_menu;
        let mut sel = div()
            .id("rv-scope")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            // `padding-right: 28px`, the chevron inside it.
            .gap(px(10.))
            .max_w(px(460.))
            .h(px(28.))
            .pl(px(9.))
            .pr(px(6.))
            .bg(t.bg)
            .border_1()
            .border_color(if open { t.text_3 } else { t.line_strong })
            .rounded(RADIUS)
            .text_size(px(12.))
            .cursor_pointer()
            .tooltip(|_, cx| {
                kit::tooltip_view("Everything since the agent started (committed or not), or one task (prompt). The Changes panel shows only uncommitted work, like git status.".to_string(), cx)
            })
            .on_click(cx.listener(|v, _, _, cx| {
                v.task_menu = !v.task_menu;
                cx.notify();
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(div().ellipsis().child(crate::kit::one_line(current)))
                    .child(
                        div()
                            .h_0()
                            .overflow_hidden()
                            .children(options.into_iter().map(|o| div().whitespace_nowrap().child(o))),
                    ),
            )
            .child(kit::icon("chevron-down", 12., t.text_3));
        if open {
            let id = a.id.clone();
            let mut menu = div()
                .id("rv-scope-menu")
                .occlude()
                .min_w(px(260.))
                .max_w(px(560.))
                .max_h(px(360.))
                .overflow_y_scroll()
                .py(px(4.))
                .bg(t.raised)
                .border_1()
                .border_color(t.line_strong)
                .rounded(RADIUS)
                .shadow_lg()
                .text_size(px(12.))
                .on_mouse_down_out(cx.listener(|v, _, _, cx| {
                    v.task_menu = false;
                    cx.notify();
                }));
            let item = |key: String, label: String, enabled: bool, chosen: bool| {
                div()
                    .id(SharedString::from(format!("rv-task-{key}")))
                    .mx(px(4.))
                    .px(px(8.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .rounded(RADIUS_SM)
                    .text_color(if enabled { t.text } else { t.text_4 })
                    .when(chosen, |d| d.bg(t.surface_3))
                    .when(enabled, |d| d.cursor_pointer().hover_probed(|s| s.bg(t.surface_3)))
                    .child(div().ellipsis().child(crate::kit::one_line(label)))
            };
            {
                let id = id.clone();
                menu = menu.child(
                    item(
                        "all".into(),
                        format!("Since the agent started · {}", a.name),
                        true,
                        task.is_none(),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |v, _, _, cx| v.set_task(&id, None, cx))),
                );
            }
            for (n, x) in numbered.iter().rev() {
                let enabled = x.start_tree.is_some();
                let label = format!(
                    "{}{}",
                    model::task_label(x, *n, &model::clock),
                    if enabled { "" } else { " (snapshot pending)" }
                );
                let (id, tid) = (id.clone(), x.id.clone());
                let chosen = task == Some(&x.id);
                menu = menu.child(
                    item(x.id.clone(), label, enabled, chosen)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when(enabled, |d| {
                            d.on_click(cx.listener(move |v, _, _, cx| {
                                v.set_task(&id, Some(tid.clone()), cx)
                            }))
                        }),
                );
            }
            sel = sel.child(
                deferred(
                    anchored()
                        .anchor(Corner::TopLeft)
                        .position(point(px(0.), px(30.)))
                        .position_mode(gpui::AnchoredPositionMode::Local)
                        .snap_to_window()
                        .child(menu),
                )
                .with_priority(5),
            );
        }
        sel
    }

    fn agent_main(
        &self,
        a: &AgentView,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (_, file, task) = self.agent_sel().expect("agent selected");
        let file = file.cloned();
        let comments = Comments::for_agent(cx, &a.id);
        let list = self.files.get(&a.id);
        let empty = if list.is_some_and(|l| l.is_empty()) {
            "Nothing to review here."
        } else {
            "Pick a file on the left."
        };
        let prompt = task.as_ref().map(|id| {
            self.tasks
                .iter()
                .find(|x| &x.id == id)
                .map(|x| x.prompt.clone())
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| "(typed in the terminal)".into())
        });
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                Self::head_row()
                    .border_b_1()
                    .border_color(t.line)
                    .child(self.task_select(a, task.as_ref(), t, cx))
                    .when_some(file.as_ref(), |d, f| d.child(Self::file_head(f, t))),
            )
            .when_some(prompt, |d, p| {
                d.child(
                    kit::vscroll(
                        "rv-task-prompt-bar",
                        &self.prompt_scroll,
                        t,
                        div()
                            .id("rv-task-prompt")
                            .track_scroll(&self.prompt_scroll)
                            .max_h(px(64.))
                            .overflow_y_scroll()
                            .pt(px(6.))
                            .px(px(12.))
                            .pb(px(7.))
                            .text_size(px(12.))
                            .text_color(t.text_2)
                            .tooltip(|_, cx| {
                                kit::tooltip_view("The prompt of this task, as sent".to_string(), cx)
                            })
                            .child(p),
                    )
                    .flex_none()
                    .max_h(px(64.))
                    .border_b_1()
                    .border_color(t.line)
                    .bg(t.surface),
                )
            })
            .child(self.diff_area(empty, file.is_some(), t, cx))
            .child(self.footer_agent(a, file.is_some(), comments.len(), t, cx))
    }

    fn footer_base(&self, t: &Theme) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .min_w_0()
            .border_t_1()
            .border_color(t.line)
            .bg(t.surface)
    }

    fn notice(&self, t: &Theme) -> Option<Div> {
        self.notice.as_ref().map(|n| {
            div()
                .min_w_0()
                .ellipsis()
                .text_size(px(12.))
                .text_color(t.text_3)
                .child(crate::kit::one_line(n.clone()))
        })
    }

    fn footer_agent(
        &self,
        a: &AgentView,
        has_file: bool,
        n_comments: usize,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.footer_base(t)
            .child(kit::glyph_play(format!("rv-foot-{}", a.id), a.status, t, true))
            .child(kit::label_t(a.name.clone(), t.text, 13., 0.1).font_weight(FontWeight::BOLD))
            .children(self.notice(t))
            .child(spacer())
            .child(
                kit::button("rv-discard", BtnKind::Ghost, !has_file, t)
                    .child("Discard file")
                    .tooltip(|_, cx| {
                        kit::tooltip_view("Restore this file from where the agent started".to_string(), cx)
                    })
                    .when(has_file, |d| {
                        d.on_click(cx.listener(|v, _, window, cx| v.open_discard(window, cx)))
                    }),
            )
            .child(
                kit::button("rv-send", BtnKind::Ghost, n_comments == 0, t)
                    .child("Send comments")
                    .when(n_comments > 0, |d| {
                        d.child(kit::count_badge(n_comments, t).ml(px(2.)))
                    })
                    .when(n_comments > 0, |d| {
                        d.on_click(cx.listener(|v, _, window, cx| v.open_send(window, cx)))
                    }),
            )
            .child(
                kit::button("rv-commit", BtnKind::Primary, false, t)
                    .child(if a.worktree {
                        "Commit & merge"
                    } else {
                        "Commit"
                    })
                    .on_click(cx.listener(|v, _, window, cx| v.open_commit_agent(window, cx))),
            )
    }

    fn worktree_main(
        &self,
        r: &WtRef,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (_, file) = self.wt_sel().expect("worktree selected");
        let file = file.cloned();
        let list = self.wt_files.get(&r.key());
        let empty = match list {
            Some(Ok(l)) if l.is_empty() => "Nothing to review here.",
            Some(_) => "Pick a file on the left.",
            None => "Reading git…",
        };
        let wt = &r.wt;
        let path = wt.path.clone();
        let tip = wt.path_display.clone();
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                Self::head_row()
                    .border_b_1()
                    .border_color(t.line)
                    .child(
                        div()
                            .id("rv-wt-scope")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(4.))
                            .max_w(relative(0.6))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(12.))
                            .text_color(t.text_2)
                            .tooltip(move |_, cx| kit::tooltip_view(tip.clone(), cx))
                            .child("Worktree")
                            .child(mono().child(wt.name.clone()))
                            .child(format!(
                                "· {} · against {}",
                                wt.branch.clone().unwrap_or_else(|| "detached".into()),
                                r.target.clone().unwrap_or_else(|| "its HEAD".into())
                            )),
                    )
                    .when_some(file.as_ref(), |d, f| d.child(Self::file_head(f, t))),
            )
            .child(self.diff_area(empty, file.is_some(), t, cx))
            .child(
                self.footer_base(t)
                    .child(kit::icon("branch", 12., t.text_2))
                    .child(kit::label_t(wt.name.clone(), t.text, 13., 0.1).font_weight(FontWeight::BOLD))
                    .when(wt.locked, |d| {
                        let why = wt.lock_reason.clone().unwrap_or_default();
                        d.child(
                            div()
                                .id("rv-wt-locked")
                                .child(kit::chip_tone("locked", kit::Tone::Warn, t))
                                .when(!why.is_empty(), |d| {
                                    d.tooltip(move |_, cx| kit::tooltip_view(why.clone(), cx))
                                }),
                        )
                    })
                    .children(self.notice(t))
                    .child(spacer())
                    .when(wt.caps.terminal, |d| {
                        d.child(
                            kit::button("rv-wt-term", BtnKind::Ghost, false, t)
                                .child("Open terminal")
                                .tooltip(|_, cx| {
                                    kit::tooltip_view("A shell in this worktree".to_string(), cx)
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(super::ReviewEvent::OpenTerminal(path.clone()))
                                })),
                        )
                    })
                    .when(wt.caps.remove, |d| {
                        d.child(
                            kit::button("rv-wt-remove", BtnKind::Ghost, false, t)
                                .child("Remove worktree…")
                                .tooltip(|_, cx| {
                                    kit::tooltip_view("git worktree remove (asks first)".to_string(), cx)
                                })
                                .on_click(
                                    cx.listener(|v, _, window, cx| v.open_remove(window, cx)),
                                ),
                        )
                    })
                    .when(wt.caps.commit, |d| {
                        d.child(
                            kit::button("rv-wt-commit", BtnKind::Primary, false, t)
                                .child(if wt.caps.merge {
                                    "Commit & merge"
                                } else {
                                    "Commit"
                                })
                                .on_click(cx.listener(|v, _, window, cx| {
                                    v.open_commit_worktree(window, cx)
                                })),
                        )
                    }),
            )
    }
}

impl Render for ReviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.take_pending_conflict(window, cx);
        let t = theme(cx).clone();
        let main: AnyElement = if let Some((r, _)) = self.wt_sel() {
            self.worktree_main(&r, &t, cx).into_any_element()
        } else if let Some((a, _, _)) = self.agent_sel() {
            let a = a.clone();
            self.agent_main(&a, &t, cx).into_any_element()
        } else {
            Self::empty(
                if self.ordered.is_empty() {
                    "Start an agent to review its changes."
                } else {
                    "Nothing selected."
                },
                &t,
            )
            .into_any_element()
        };
        let dialog = self
            .dialog
            .is_some()
            .then(|| self.render_dialog(&t, window, cx));
        div()
            .id("review")
            .key_context("Review")
            .font_family(kit::UI_FONT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|v, _: &Back, window, cx| {
                if v.dialog.is_some() {
                    // A dialog without a field has no focus of its own: Esc
                    // closes it from here (React: the modal's Esc).
                    v.close_dialog(window, cx);
                } else if v.composer.is_none() && !v.task_menu {
                    v.exit(cx)
                } else if v.task_menu {
                    v.task_menu = false;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|v, _: &RefreshChanges, _, cx| v.refresh_all(cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .text_size(px(13.))
            .child(self.bar(&t, cx))
            .when_some(self.fresh_error.clone(), |d, e| {
                d.child(
                    div()
                        .flex_none()
                        .px(px(12.))
                        .py(px(4.))
                        .border_b_1()
                        .border_color(t.line)
                        .child(self.fresh_error(
                            &e,
                            "Couldn't refresh: ",
                            &t,
                            cx.listener(|v, _, _, cx| v.refresh_all(cx)),
                        )),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.list(&t, window, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .child(main),
                    ),
            )
            .children(dialog)
    }
}
