//! Review's dialogs (Tauri: `src/components/review/ReviewDialogs.tsx`,
//! `src/components/RemoveWorktreeDialog.tsx`, `src/components/Modal.tsx`):
//! Discard file, the editable prompt (Send comments, merge conflict),
//! Commit / Commit & merge, Remove worktree.

use gpui::{
    anchored, deferred, div, point, prelude::*, px, AnyElement, AppContext, Context, Div, Entity,
    FontWeight, Hsla, MouseButton, Subscription, Window,
};

use pitwall_core::review::{MergeResult, MergeStatus};
use pitwall_core::vcs::git::FileChange;
use pitwall_proto::Status;

use super::comments::Comments;
use super::model::{self, WtRef};
use super::ops::{self, CommitOutcome, CommitTarget};
use super::widgets::{self as w, Btn};
use super::{CloseDialog, ReviewView};
use crate::code_view::input::{TextField, TextFieldEvent};
use crate::theme::{Mode, Theme, RADIUS};

pub(super) enum Dialog {
    Discard {
        agent: String,
        agent_name: String,
        cwd: String,
        file: FileChange,
        scoped: bool,
        busy: bool,
        error: Option<String>,
    },
    Prompt {
        agent: String,
        name: String,
        running: bool,
        busy_status: bool,
        title: String,
        /// Shown above the text: an intro, or (conflict) an error.
        intro: Option<String>,
        conflict: Option<String>,
        field: Entity<TextField>,
        /// Clear the agent's comments once sent.
        clears_comments: bool,
        busy: bool,
        error: Option<String>,
        _sub: Subscription,
    },
    Commit {
        target: CommitTarget,
        /// A worktree's dialog: conflicts become a notice.
        for_worktree: bool,
        status: Option<Result<MergeStatus, String>>,
        message: Entity<TextField>,
        also_merge: bool,
        busy: bool,
        error: Option<String>,
        _sub: Subscription,
    },
    Remove {
        r: WtRef,
        busy: bool,
        error: Option<String>,
    },
}

impl ReviewView {
    pub(super) fn open_discard(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some((a, Some(f), task)) = self.agent_sel() else {
            return;
        };
        self.dialog = Some(Dialog::Discard {
            agent: a.id.clone(),
            agent_name: a.name.clone(),
            cwd: a.cwd_display.clone(),
            file: f.clone(),
            scoped: task.is_some(),
            busy: false,
            error: None,
        });
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn open_prompt(
        &mut self,
        title: String,
        intro: Option<String>,
        conflict: Option<String>,
        initial: String,
        clears_comments: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((a, _, _)) = self.agent_sel() else {
            return;
        };
        let (agent, name, running) = (a.id.clone(), a.name.clone(), a.running);
        let busy_status = matches!(a.status, Status::Working | Status::Blocked);
        let lines = initial.lines().count() + 1;
        let field = cx.new(|cx| {
            let mut f = TextField::multi("", lines.clamp(5, 16), cx).mono(true);
            f.set_text(initial, cx);
            f
        });
        let sub = cx.subscribe_in(&field, window, |this, field, e: &TextFieldEvent, _, cx| match e {
            TextFieldEvent::Submit => {
                let queue_first = matches!(&this.dialog, Some(Dialog::Prompt { running, busy_status, .. }) if !*running || *busy_status);
                if !field.read(cx).text().trim().is_empty() {
                    this.send_prompt(!queue_first, cx);
                }
            }
            TextFieldEvent::Changed => {
                let rows = field.read(cx).text().lines().count() + 1;
                field.update(cx, |f, cx| f.set_rows(rows.clamp(5, 16), cx));
                cx.notify();
            }
            _ => {}
        });
        field.update(cx, |f, _| f.focus(window));
        self.dialog = Some(Dialog::Prompt {
            agent,
            name,
            running,
            busy_status,
            title,
            intro,
            conflict,
            field,
            clears_comments,
            busy: false,
            error: None,
            _sub: sub,
        });
        cx.notify();
    }

    pub(super) fn open_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((a, _, _)) = self.agent_sel() else {
            return;
        };
        let (id, name) = (a.id.clone(), a.name.clone());
        let list = Comments::for_agent(cx, &id);
        if list.is_empty() {
            return;
        }
        self.open_prompt(
            format!("Send comments · {name}"),
            Some(format!(
                "Edit freely — this exact text is what {name} receives."
            )),
            None,
            model::compose_prompt(&list),
            true,
            window,
            cx,
        );
    }

    fn open_conflict(&mut self, r: MergeResult, window: &mut Window, cx: &mut Context<Self>) {
        self.open_prompt(
            "Merge conflict".into(),
            None,
            Some(r.message.clone()),
            model::conflict_prompt(r.branch.as_deref()),
            false,
            window,
            cx,
        );
    }

    fn open_commit(
        &mut self,
        target: CommitTarget,
        for_worktree: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = cx.new(|cx| TextField::multi("Describe the change", 3, cx));
        let sub = cx.subscribe_in(
            &message,
            window,
            |this, _, e: &TextFieldEvent, window, cx| match e {
                TextFieldEvent::Submit => this.run_commit(window, cx),
                _ => cx.notify(),
            },
        );
        message.update(cx, |f, _| f.focus(window));
        let t2 = target.clone();
        self.dialog = Some(Dialog::Commit {
            target,
            for_worktree,
            status: None,
            message,
            also_merge: true,
            busy: false,
            error: None,
            _sub: sub,
        });
        ops::spawn(
            cx,
            move |e| t2.status(e),
            |this: &mut Self, res, cx| {
                if let Some(Dialog::Commit { status, .. }) = this.dialog.as_mut() {
                    *status = Some(res);
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    pub(super) fn open_commit_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((a, _, _)) = self.agent_sel() else {
            return;
        };
        let target = CommitTarget::Agent {
            id: a.id.clone(),
            name: a.name.clone(),
            where_: a.cwd_display.clone(),
            project_display: a.project_display.clone(),
        };
        self.open_commit(target, false, window, cx);
    }

    pub(super) fn open_commit_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((r, _)) = self.wt_sel() else { return };
        let target = CommitTarget::Worktree {
            project: r.project_id.clone(),
            path: r.wt.path.clone(),
            name: r.wt.name.clone(),
            where_: r.wt.path_display.clone(),
            project_display: r.repo_display.clone(),
        };
        self.open_commit(target, true, window, cx);
    }

    pub(super) fn open_remove(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some((r, _)) = self.wt_sel() else { return };
        self.dialog = Some(Dialog::Remove {
            r,
            busy: false,
            error: None,
        });
        cx.notify();
    }

    pub(super) fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = None;
        window.focus(&self.list_focus);
        cx.notify();
    }

    // ── running them ───────────────────────────────────────────────────

    pub(super) fn run_discard(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Discard {
            agent, file, busy, ..
        }) = self.dialog.as_mut()
        else {
            return;
        };
        *busy = true;
        let (id, path) = (agent.clone(), file.path.clone());
        ops::spawn(
            cx,
            move |e| ops::discard_file(e, &id, &path).map(|_| path),
            |this: &mut Self, res, cx| match res {
                Ok(path) => {
                    this.dialog = None;
                    this.done(format!("Discarded {path}"), cx);
                }
                Err(e) => {
                    if let Some(Dialog::Discard { busy, error, .. }) = this.dialog.as_mut() {
                        *busy = false;
                        *error = Some(e);
                    }
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    /// Send now, or into Next up.
    fn send_prompt(&mut self, now: bool, cx: &mut Context<Self>) {
        let Some(Dialog::Prompt {
            agent, field, busy, ..
        }) = self.dialog.as_mut()
        else {
            return;
        };
        let text = field.read(cx).text().to_string();
        if text.trim().is_empty() {
            return;
        }
        *busy = true;
        let id = agent.clone();
        ops::spawn(
            cx,
            move |e| {
                if now {
                    ops::send_prompt(e, &id, text)
                } else {
                    ops::queue_add(e, &id, text)
                }
            },
            |this: &mut Self, res, cx| match res {
                Ok(()) => {
                    if let Some(Dialog::Prompt {
                        clears_comments,
                        agent,
                        ..
                    }) = this.dialog.take()
                    {
                        if clears_comments {
                            Comments::clear(cx, &agent);
                            this.notice = Some("Comments sent".into());
                        }
                    }
                    cx.notify();
                }
                Err(e) => {
                    if let Some(Dialog::Prompt { busy, error, .. }) = this.dialog.as_mut() {
                        *busy = false;
                        *error = Some(e);
                    }
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    pub(super) fn run_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        let Some(Dialog::Commit {
            target,
            status: Some(Ok(st)),
            message,
            also_merge,
            busy,
            error,
            ..
        }) = self.dialog.as_mut()
        else {
            return;
        };
        let msg = message.read(cx).text().to_string();
        let plan = model::plan(st, *also_merge);
        if *busy || !plan.can_go(&msg) {
            return;
        }
        *busy = true;
        *error = None;
        let (t, st) = (target.clone(), st.clone());
        ops::spawn(
            cx,
            move |e| ops::commit_and_merge(e, &t, &st, plan.need_commit, plan.will_merge, &msg),
            |this: &mut Self, out, cx| {
                let for_wt = matches!(
                    this.dialog,
                    Some(Dialog::Commit {
                        for_worktree: true,
                        ..
                    })
                );
                match out {
                    CommitOutcome::Done(msg) => {
                        this.dialog = None;
                        this.done(msg, cx);
                    }
                    CommitOutcome::Conflict(r) if for_wt => {
                        this.dialog = None;
                        this.done(r.message, cx);
                    }
                    CommitOutcome::Conflict(r) => {
                        this.dialog = None;
                        this.nonce += 1;
                        this.pending_conflict = Some(r);
                        cx.notify();
                    }
                    CommitOutcome::Failed(msg, st) => {
                        if let Some(Dialog::Commit {
                            busy,
                            error,
                            status,
                            ..
                        }) = this.dialog.as_mut()
                        {
                            *busy = false;
                            *error = Some(msg);
                            if let Some(st) = st {
                                *status = Some(Ok(st));
                            }
                        }
                        cx.notify();
                    }
                }
            },
        );
        cx.notify();
    }

    fn run_remove(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Remove { r, busy, .. }) = self.dialog.as_mut() else {
            return;
        };
        *busy = true;
        let (p, path, name) = (r.project_id.clone(), r.wt.path.clone(), r.wt.name.clone());
        ops::spawn(
            cx,
            move |e| ops::remove_worktree(e, &p, &path),
            move |this: &mut Self, res, cx| match res {
                Ok(()) => {
                    this.dialog = None;
                    this.sel = super::Sel::None;
                    this.done(format!("Removed worktree {name}"), cx);
                }
                Err(e) => {
                    if let Some(Dialog::Remove { busy, error, .. }) = this.dialog.as_mut() {
                        *busy = false;
                        *error = Some(e);
                    }
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    /// A conflict from the agent's Commit & merge opens the prompt (needs a
    /// window, so it happens at the next render).
    pub(super) fn take_pending_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = self.pending_conflict.take() {
            self.open_conflict(r, window, cx);
        }
    }

    // ── drawing ────────────────────────────────────────────────────────

    pub(super) fn render_dialog(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (title, width, body, foot): (String, f32, Div, Div) = match self
            .dialog
            .as_ref()
            .expect("dialog")
        {
            Dialog::Discard {
                agent_name,
                cwd,
                file,
                scoped,
                busy,
                error,
                ..
            } => {
                let text = if file.untracked {
                    format!("This file is new; it will be deleted from {cwd}.")
                } else {
                    format!("Restores the file to how it was when {agent_name} started (its base commit), in {cwd}.")
                };
                let text = if *scoped {
                    format!("{text} This undoes all of the agent's changes to this file, not only this task's.")
                } else {
                    text
                };
                let busy = *busy;
                (
                    "Discard file".into(),
                    480.,
                    body()
                        .child(w::mono().text_size(px(12.5)).child(file.path.clone()))
                        .child(muted(&text, t))
                        .child(hint_error("This can't be undone.", t))
                        .children(error.as_ref().map(|e| form_error(e, t))),
                    foot(t)
                        .child(
                            w::button("dlg-cancel", Btn::Ghost, true, t)
                                .child("Cancel")
                                .on_click(cx.listener(|v, _, w, cx| v.close_dialog(w, cx))),
                        )
                        .child(
                            w::button("dlg-discard", Btn::Danger, !busy, t)
                                .child(if busy { "Discarding…" } else { "Discard" })
                                .when(!busy, |d| {
                                    d.on_click(cx.listener(|v, _, _, cx| v.run_discard(cx)))
                                }),
                        ),
                )
            }
            Dialog::Prompt {
                name,
                running,
                busy_status,
                title,
                intro,
                conflict,
                field,
                busy,
                error,
                ..
            } => {
                let queue_first = !*running || *busy_status;
                let empty = field.read(cx).text().trim().is_empty();
                let enabled = !*busy && !empty;
                let note = if queue_first {
                    if *running {
                        format!(" {name} is busy, so it goes to Next up.")
                    } else {
                        format!(" {name} isn't running, so it goes to Next up.")
                    }
                } else {
                    String::new()
                };
                let focused = field.read(cx).is_focused(window);
                let mut f = foot(t).child(
                    w::button("dlg-cancel", Btn::Ghost, true, t)
                        .child("Cancel")
                        .on_click(cx.listener(|v, _, w, cx| v.close_dialog(w, cx))),
                );
                let now_btn = |id: &'static str, kind: Btn, label: &'static str| {
                    w::button(id, kind, enabled, t)
                        .child(label)
                        .when(enabled, |d| {
                            d.on_click(cx.listener(|v, _, _, cx| v.send_prompt(true, cx)))
                        })
                };
                let queue_btn = |id: &'static str, kind: Btn| {
                    w::button(id, kind, enabled, t)
                        .child("Add to Next up")
                        .when(enabled, |d| {
                            d.on_click(cx.listener(|v, _, _, cx| v.send_prompt(false, cx)))
                        })
                };
                if queue_first {
                    if *running {
                        f = f.child(now_btn("dlg-now", Btn::Ghost, "Send now anyway"));
                    }
                    f = f.child(queue_btn("dlg-queue", Btn::Primary));
                } else {
                    f = f.child(queue_btn("dlg-queue", Btn::Ghost)).child(now_btn(
                        "dlg-now",
                        Btn::Primary,
                        "Send now",
                    ));
                }
                (
                    title.clone(),
                    620.,
                    body()
                        .children(intro.as_ref().map(|i| muted(i, t)))
                        .children(conflict.as_ref().map(|c| form_error(c, t)))
                        .child(textarea(field.clone(), focused, true, t))
                        .child(
                            w::ui()
                                .text_size(px(12.))
                                .text_color(t.text_3)
                                .flex()
                                .flex_wrap()
                                .child("Sent to\u{a0}")
                                .child(div().font_weight(FontWeight::BOLD).child(name.clone()))
                                .child(format!("\u{a0}exactly as written above.{note}")),
                        )
                        .children(error.as_ref().map(|e| form_error(e, t))),
                    f,
                )
            }
            Dialog::Commit {
                target,
                status,
                message,
                also_merge,
                busy,
                error,
                ..
            } => {
                let st = status.as_ref().and_then(|s| s.as_ref().ok()).cloned();
                let title = format!(
                    "{} · {}",
                    if st.as_ref().is_some_and(|s| s.worktree) {
                        "Commit & merge"
                    } else {
                        "Commit"
                    },
                    target.title_name()
                );
                let msg = message.read(cx).text().to_string();
                let focused = message.read(cx).is_focused(window);
                let mut b = body();
                match status {
                    Some(Err(e)) => {
                        b = b.child(form_error(&format!("Couldn't read git status: {e}"), t))
                    }
                    None => b = b.child(hint("Reading git…", t)),
                    Some(Ok(_)) => {}
                }
                let plan = st.as_ref().map(|s| model::plan(s, *also_merge));
                if let (Some(st), Some(plan)) = (st.as_ref(), plan) {
                    if plan.need_commit {
                        let entries = if st.uncommitted == 1 {
                            "entry"
                        } else {
                            "entries"
                        };
                        b = b.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .child(w::label_text("Commit message", 12., FontWeight::SEMIBOLD, t.text_3))
                                .child(textarea(message.clone(), focused, false, t))
                                .child(
                                    w::ui()
                                        .text_size(px(12.))
                                        .text_color(t.text_3)
                                        .child(format!(
                                            "Runs git add -A && git commit in {}{} — {} uncommitted {entries}.{}",
                                            target.where_(),
                                            st.branch.as_ref().map(|b| format!(" on {b}")).unwrap_or_default(),
                                            st.uncommitted,
                                            if st.worktree {
                                                ""
                                            } else {
                                                " This is the main checkout: everything in it is committed, including changes not made by this agent."
                                            }
                                        )),
                                ),
                        );
                    } else {
                        b = b.child(muted(
                            &format!("Nothing uncommitted in {}.", target.where_()),
                            t,
                        ));
                    }
                    if st.worktree {
                        let branch = st.branch.clone().unwrap_or_default();
                        let into = st.target.clone().unwrap_or_else(|| "?".into());
                        let mut bx = div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .px(px(14.))
                            .py(px(12.))
                            .rounded(RADIUS)
                            .bg(t.surface_2)
                            .border_1()
                            .border_color(t.line_strong);
                        if plan.need_commit {
                            let can = plan.can_merge && !st.target_dirty;
                            let checked = *also_merge && can;
                            bx = bx.child(
                                w::ui()
                                    .id("dlg-merge")
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .text_size(px(13.))
                                    .when(can, |d| d.cursor_pointer())
                                    .when(!can, |d| d.opacity(0.45))
                                    .child(w::checkbox(checked, t.text, t))
                                    .child(format!("Then merge {branch} into {into}"))
                                    .when(can, |d| {
                                        d.on_click(cx.listener(|v, _, _, cx| {
                                            if let Some(Dialog::Commit { also_merge, .. }) =
                                                v.dialog.as_mut()
                                            {
                                                *also_merge = !*also_merge;
                                            }
                                            cx.notify();
                                        }))
                                    }),
                            );
                        } else {
                            let ahead = if st.ahead > 0 {
                                format!(
                                    " ({} commit{})",
                                    st.ahead,
                                    if st.ahead == 1 { "" } else { "s" }
                                )
                            } else {
                                " — nothing to merge".into()
                            };
                            bx = bx.child(
                                w::ui()
                                    .text_size(px(13.))
                                    .child(format!("Merge {branch} into {into}{ahead}")),
                            );
                        }
                        bx = bx.child(hint(
                            &format!(
                                "git merge --no-ff in the main checkout {}. On conflicts the merge is aborted and nothing changes.",
                                target.project_display()
                            ),
                            t,
                        ));
                        if st.target_dirty {
                            bx = bx.child(hint_error(
                                "The main checkout has uncommitted changes, so merging is refused. Commit or stash them first.",
                                t,
                            ));
                        }
                        if st.target.is_none() {
                            bx =
                                bx.child(hint_error("The main checkout is on a detached HEAD.", t));
                        }
                        if st.branch.is_none() {
                            bx = bx.child(hint_error(
                                "The worktree isn't on a branch (detached HEAD). Create a branch there to merge it.",
                                t,
                            ));
                        }
                        b = b.child(bx);
                    }
                    if let Some(e) = error {
                        b = b.child(form_error(e, t));
                    }
                }
                let enabled = !*busy && plan.is_some_and(|p| p.can_go(&msg));
                let label = if *busy {
                    "Working…"
                } else {
                    plan.map_or("…", |p| p.primary())
                };
                (
                    title,
                    540.,
                    b,
                    foot(t)
                        .child(
                            w::button("dlg-cancel", Btn::Ghost, true, t)
                                .child("Cancel")
                                .on_click(cx.listener(|v, _, w, cx| v.close_dialog(w, cx))),
                        )
                        .child(
                            w::button("dlg-commit", Btn::Primary, enabled, t)
                                .child(label)
                                .when(enabled, |d| {
                                    d.on_click(cx.listener(|v, _, w, cx| v.run_commit(w, cx)))
                                }),
                        ),
                )
            }
            Dialog::Remove { r, busy, error } => {
                let wt = &r.wt;
                let mut b =
                    body().child(w::mono().text_size(px(12.5)).child(wt.path_display.clone()));
                if wt.caps.remove {
                    let branch = match &wt.branch {
                        Some(br) => format!("Branch {br} is kept."),
                        None => {
                            "It is on a detached HEAD; commits not on a branch become hard to find."
                                .into()
                        }
                    };
                    b = b
                        .child(muted(
                            &format!(
                                "Runs git worktree remove (without --force): git refuses if it has uncommitted or untracked changes, and nothing is lost. {branch}"
                            ),
                            t,
                        ))
                        .child(form_error("The worktree folder will be deleted from disk.", t));
                } else {
                    let why = if wt.locked {
                        format!(
                            "It is locked{}. Pitwall doesn't remove locked worktrees.",
                            wt.lock_reason
                                .as_ref()
                                .map(|r| format!(" ({r})"))
                                .unwrap_or_default()
                        )
                    } else if wt.via == pitwall_proto::WorktreeVia::Own {
                        "An agent works there: remove the agent (with its worktree) instead.".into()
                    } else {
                        "This worktree can't be removed from Pitwall.".into()
                    };
                    b = b.child(hint_error(&why, t));
                }
                let b = b.children(error.as_ref().map(|e| form_error(e, t)));
                let busy = *busy;
                let mut f = foot(t).child(
                    w::button("dlg-cancel", Btn::Ghost, true, t)
                        .child("Cancel")
                        .on_click(cx.listener(|v, _, w, cx| v.close_dialog(w, cx))),
                );
                if wt.caps.remove {
                    f = f.child(
                        w::button("dlg-remove", Btn::Danger, !busy, t)
                            .child(if busy {
                                "Removing…"
                            } else {
                                "Remove worktree"
                            })
                            .when(!busy, |d| {
                                d.on_click(cx.listener(|v, _, _, cx| v.run_remove(cx)))
                            }),
                    );
                }
                (format!("Remove worktree {}", wt.name), 460., b, f)
            }
        };
        let backdrop: Hsla = if t.mode == Mode::Dark {
            gpui::rgba(0x0405_069e).into()
        } else {
            gpui::rgba(0x1e22_2847).into()
        };
        let size = window.viewport_size();
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .id("rv-backdrop")
                    .key_context("RvDialog")
                    .on_action(cx.listener(|v, _: &CloseDialog, w, cx| v.close_dialog(w, cx)))
                    .occlude()
                    .w(size.width)
                    .h(size.height)
                    .bg(backdrop)
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(24.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|v, _, w, cx| v.close_dialog(w, cx)),
                    )
                    .child(
                        w::ui()
                            .id("rv-modal")
                            .w(px(width))
                            .max_w_full()
                            .max_h_full()
                            .flex()
                            .flex_col()
                            .rounded(px(12.))
                            .bg(t.raised)
                            .border_1()
                            .border_color(t.line_strong)
                            .shadow_lg()
                            .overflow_hidden()
                            .text_size(px(13.))
                            .text_color(t.text)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .pt(px(14.))
                                    .pr(px(12.))
                                    .pb(px(6.))
                                    .pl(px(18.))
                                    .child(w::label(&title, t))
                                    .child(
                                        w::icon_button("dlg-x", false, t)
                                            .child("✕")
                                            .tooltip(|_, cx| {
                                                crate::code_view::tooltip_view("Close (esc)", cx)
                                            })
                                            .on_click(
                                                cx.listener(|v, _, w, cx| v.close_dialog(w, cx)),
                                            ),
                                    ),
                            )
                            .child(body)
                            .child(foot),
                    ),
            ),
        )
        .with_priority(20)
        .into_any_element()
    }
}

fn body() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .pt(px(8.))
        .px(px(18.))
        .pb(px(16.))
}

fn foot(t: &Theme) -> Div {
    div()
        .flex()
        .justify_end()
        .gap(px(8.))
        .px(px(18.))
        .py(px(12.))
        .border_t_1()
        .border_color(t.line)
        .bg(t.surface)
}

fn muted(text: &str, t: &Theme) -> Div {
    w::ui().text_color(t.text_2).child(text.to_string())
}

fn hint(text: &str, t: &Theme) -> Div {
    w::ui()
        .text_size(px(12.))
        .text_color(t.text_3)
        .child(text.to_string())
}

fn hint_error(text: &str, t: &Theme) -> Div {
    w::ui()
        .text_size(px(12.))
        .text_color(t.red)
        .child(text.to_string())
}

fn form_error(text: &str, t: &Theme) -> Div {
    w::ui()
        .text_size(px(12.))
        .text_color(t.red)
        .px(px(10.))
        .py(px(8.))
        .rounded(RADIUS)
        .bg(t.red_soft)
        .child(text.to_string())
}

/// `textarea` (`.input` look).
fn textarea(field: Entity<TextField>, focused: bool, mono: bool, t: &Theme) -> Div {
    div()
        .w_full()
        .bg(t.bg)
        .border_1()
        .border_color(if focused { t.text_3 } else { t.line_strong })
        .rounded(RADIUS)
        .px(px(9.))
        .py(px(7.))
        .text_size(px(if mono { 12.5 } else { 13. }))
        .line_height(px(19.))
        .child(field)
}
