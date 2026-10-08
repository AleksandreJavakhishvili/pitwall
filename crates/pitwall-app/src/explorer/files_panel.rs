//! The right panel's Files tab (a port of `FilesPanel.tsx`): the focused
//! agent's folder, read-only. Head: tabs slot, ↻ with "updated … ago", ⤢
//! (open the viewer); the Where line; "Go to file ⌘P", "Search ⇧⌘F" and
//! "Ignored"; then the tree. The right panel (main-screen work) mounts this
//! view in its Changes / Files section.

use crate::kit::Ellipsis as _;
use std::time::Duration;

use crate::kit::HoverText;
use gpui::{
    div, prelude::*, px, ClipboardItem, Context, Entity, EventEmitter, SharedString,
    Subscription, Task, Window,
};

use super::tree::{now_ms, TreeModel};
use super::tree_view::{OpenFile, TreeView};
use super::widgets::refresh_control;
use crate::kit::{checkbox, icon, icon_btn, kbd, keys, tooltip_view, MONO_FONT};
use crate::agents::AgentStore;
use crate::theme::{self, RADIUS_SM};

pub enum FilesPanelEvent {
    /// Open the viewer (on a file, or on Search).
    OpenViewer {
        path: Option<String>,
        search: bool,
    },
    QuickOpen,
    SetIgnored(bool),
}

pub struct FilesPanel {
    agent_id: String,
    store: Entity<AgentStore>,
    tree: Entity<TreeModel>,
    tree_view: Entity<TreeView>,
    copied: bool,
    copied_task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<FilesPanelEvent> for FilesPanel {}

impl FilesPanel {
    pub fn new(
        agent_id: String,
        store: Entity<AgentStore>,
        tree: Entity<TreeModel>,
        cx: &mut Context<Self>,
    ) -> FilesPanel {
        let tree_view = cx.new(|cx| TreeView::new(tree.clone(), cx));
        let subs = vec![
            cx.subscribe(&tree_view, |_, _, e: &OpenFile, cx| {
                cx.emit(FilesPanelEvent::OpenViewer {
                    path: Some(e.0.clone()),
                    search: false,
                })
            }),
            cx.observe(&tree, |_, _, cx| cx.notify()),
            cx.observe(&store, |_, _, cx| cx.notify()),
        ];
        FilesPanel {
            agent_id,
            store,
            tree,
            tree_view,
            copied: false,
            copied_task: None,
            _subs: subs,
        }
    }

    fn copy_cwd(&mut self, cwd: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(cwd));
        self.copied = true;
        self.copied_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1200))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.copied = false;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for FilesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let Some(a) = self.store.read(cx).agent(&self.agent_id).cloned() else {
            return div().into_any_element();
        };
        let tree = self.tree.read(cx);
        let (refreshing, updated, ignored) = (
            tree.state().refreshing,
            tree.state().updated_at,
            tree.ignored(),
        );
        let tool = |id: &'static str,
                    glyph: &'static str,
                    label: &'static str,
                    key: String,
                    tip: String| {
            let glyph = icon(glyph, 12., t.text_3);
            let hover = t.surface_3;
            let text = t.text;
            div()
                .id(id)
                .h(px(22.))
                .px(px(6.))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.))
                .rounded(RADIUS_SM)
                .border_1()
                .border_color(t.line_strong)
                .text_size(px(11.5))
                .text_color(t.text_2)
                .cursor_pointer()
                .hover_text(text, move |s| s.bg(hover))
                .tooltip(move |_, cx| tooltip_view(tip.clone(), cx))
                .child(glyph)
                .child(label)
                .child(kbd(&key, &t).text_size(px(9.5)))
        };
        let cwd = a.cwd.clone();
        let where_tip: SharedString = format!("{} — click to copy", a.cwd).into();
        let branch_tip: SharedString = if a.worktree {
            "Works in its own git worktree".into()
        } else {
            "Current branch".into()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .h(px(34.))
                    .flex_none()
                    .pl(px(14.))
                    .pr(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    // The right panel's Changes / Files tabs go here.
                    .child(div().flex_1())
                    .child(refresh_control(
                        &t,
                        "ex-panel-refresh",
                        refreshing,
                        updated,
                        now_ms(),
                        cx.listener(|this, _, _, cx| {
                            this.tree.update(cx, |t, cx| t.refresh(false, cx))
                        }),
                    ))
                    .child(
                        icon_btn("ex-panel-expand", "expand", "", true, &t)
.on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(FilesPanelEvent::OpenViewer {
                                    path: None,
                                    search: false,
                                })
                            }))
                        .tooltip(|_, cx| tooltip_view("Open the file viewer", cx)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .mx(px(14.))
                    .mb(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .font_family(MONO_FONT)
                    .text_size(px(11.))
                    .text_color(t.text_3)
                    .child(
                        div()
                            .id("ex-where-branch")
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .tooltip(move |_, cx| tooltip_view(branch_tip.clone(), cx))
                            .child(icon("branch", 11., t.text_3))
                            .child(a.branch.clone().unwrap_or_else(|| "detached HEAD".into()))
                            .when(a.worktree, |d| {
                                d.child(
                                    div()
                                        .px(px(4.))
                                        .rounded(px(3.))
                                        .border_1()
                                        .border_color(t.line_strong)
                                        .text_size(px(9.5))
                                        .child("worktree"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id("ex-where-path")
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .cursor_pointer()
                            .hover_text(t.text_2, |s| s)
                            .tooltip(move |_, cx| tooltip_view(where_tip.clone(), cx))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.copy_cwd(cwd.clone(), cx)),
                            )
                            .child(icon("folder", 11., t.text_3))
                            .child(div().min_w_0().ellipsis().child(crate::kit::one_line(
                                if self.copied {
                                    "Copied".to_string()
                                } else {
                                    a.cwd_display.clone()
                                },
                            ))),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .ml(px(14.))
                    .mr(px(10.))
                    .mb(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        tool(
                            "ex-tool-goto",
                            "file",
                            "Go to file",
                            keys("⌘P", false),
                            keys("Go to file (⌘P)", false),
                        )
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(FilesPanelEvent::QuickOpen))),
                    )
                    .child(
                        tool(
                            "ex-tool-search",
                            "search",
                            "Search",
                            keys("⌘⇧F", false),
                            keys("Search in files (⌘⇧F)", false),
                        )
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(FilesPanelEvent::OpenViewer {
                                path: None,
                                search: true,
                            })
                        })),
                    )
                    .child(div().flex_1())
                    .child(
                        checkbox("ex-panel-ignored", ignored, "Ignored", &t, cx.listener(move |_, _, _, cx| {
                                cx.emit(FilesPanelEvent::SetIgnored(!ignored))
                            }))
                        .tooltip(|_, cx| {
                            tooltip_view("Also list what .gitignore leaves out (dimmed)", cx)
                        }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.tree_view.clone()),
            )
            .into_any_element()
    }
}
