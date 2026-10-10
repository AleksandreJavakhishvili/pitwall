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

use pitwall_core::vcs::git::FileChange;
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
            (None, Some(files)) => div()
                .id("file-list")
                .px(px(6.))
                .children(
                    files
                        .iter()
                        .enumerate()
                        .map(|(i, f)| self.file_row(i, a, f, t, cx)),
                )
                .into_any_element(),
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

    fn file_row(
        &self,
        i: usize,
        a: &AgentView,
        f: &FileChange,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (dir, base) = split_path(&f.path);
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
