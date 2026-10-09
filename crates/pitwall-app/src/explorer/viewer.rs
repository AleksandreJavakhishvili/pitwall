//! The read-only file viewer (a port of `Viewer.tsx`): the whole main area,
//! like Review. Left: the agent's tree (same open folders as the Files tab)
//! or Search; centre: tabs of open files, a header (icon, path, change
//! letter, size, "Copy path", "Show diff") and the text. Binary files say
//! so; files over 2 MiB offer "Load anyway" (up to 10 MiB); an open file
//! that changed on disk offers "Reload". Nothing here can change a file.

use crate::kit::Ellipsis as _;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::kit::HoverText;
use gpui::{
    actions, div, prelude::*, px, AnyElement, App, ClipboardItem, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, KeyBinding, MouseButton, SharedString, Subscription, Task,
    Window,
};

use pitwall_core::explorer::{ContentKind, FileView};
use pitwall_proto::{AgentView, FileStatus};

use super::code_view::{code_view, Target};
use crate::kit::file_icons::file_icon;
use super::logic::{absolute_path, size_label, split_path};
use super::search::{Blurred, OpenHit, SearchPane};
use super::source::{Res, Source, LARGE_CAP, TEXT_CAP};
use super::tree::{now_ms, TreeModel};
use super::tree_view::{OpenFile, TreeView};
use super::widgets::{refresh_control, status_mark};
use crate::kit::{
    button, checkbox, icon, kbd, keys, small_btn, tooltip_view, BtnKind, MONO_FONT,
};
use crate::agents::AgentStore;
use crate::theme::{self, Theme, RADIUS, RADIUS_SM};

actions!(explorer_viewer, [Back, NextTab, PrevTab, PickTab]);

pub const CONTEXT: &str = "ExplorerViewer";
/// A focused file tab (`.ex-tab`, `tabIndex=0`).
const TAB_CONTEXT: &str = "ExplorerTab";

pub fn bindings() -> Vec<KeyBinding> {
    let tab = Some(TAB_CONTEXT);
    vec![
        KeyBinding::new("escape", Back, Some(CONTEXT)),
        KeyBinding::new("tab", NextTab, Some(CONTEXT)),
        KeyBinding::new("shift-tab", PrevTab, Some(CONTEXT)),
        // Tab / Shift-Tab walk the file tabs; Enter or Space shows one.
        KeyBinding::new("tab", NextTab, tab),
        KeyBinding::new("shift-tab", PrevTab, tab),
        KeyBinding::new("enter", PickTab, tab),
        KeyBinding::new("space", PickTab, tab),
    ]
}

pub enum ViewerEvent {
    Exit,
    QuickOpen,
    ShowDiff(String),
    /// A file was opened (quick open lists recent files first).
    Opened(String),
    SetIgnored(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Files,
    Search,
}

/// A file as read for the viewer (its text shared with the code view).
#[derive(Clone, PartialEq)]
struct Loaded {
    size: u64,
    kind: ContentKind,
    text: Option<SharedString>,
}

impl From<FileView> for Loaded {
    fn from(v: FileView) -> Loaded {
        Loaded {
            size: v.size,
            kind: v.kind,
            text: v.text.map(SharedString::from),
        }
    }
}

#[derive(Default)]
struct Doc {
    view: Option<Loaded>,
    error: Option<String>,
    loading: bool,
    /// Read again and different: what's on disk now (shown on "Reload").
    newer: Option<Loaded>,
}

pub struct Viewer {
    source: Arc<dyn Source>,
    store: Entity<AgentStore>,
    agent_id: String,
    tree: Entity<TreeModel>,
    tree_view: Entity<TreeView>,
    search: Entity<SearchPane>,
    pane: Pane,
    tabs: Vec<String>,
    active: Option<String>,
    docs: HashMap<String, Doc>,
    target: Option<Target>,
    nonce: u64,
    copied: bool,
    copied_task: Option<Task<()>>,
    focus: FocusHandle,
    /// Each file tab's keyboard focus (by path).
    tab_focus: HashMap<String, FocusHandle>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<ViewerEvent> for Viewer {}

impl Focusable for Viewer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Viewer {
    pub fn new(
        source: Arc<dyn Source>,
        store: Entity<AgentStore>,
        agent_id: String,
        tree: Entity<TreeModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Viewer {
        let tree_view = cx.new(|cx| TreeView::new(tree.clone(), cx));
        let search = cx.new(|cx| SearchPane::new(source.clone(), agent_id.clone(), cx));
        let subs = vec![
            cx.subscribe(&tree_view, |this, _, e: &OpenFile, cx| {
                this.open(e.0.clone(), None, cx)
            }),
            cx.subscribe(&search, |this, _, hit: &OpenHit, cx| {
                this.open(
                    hit.path.clone(),
                    Some((hit.line, Some((hit.from, hit.to)))),
                    cx,
                )
            }),
            cx.subscribe_in(&search, window, |this, _, _: &Blurred, window, _| {
                window.focus(&this.focus)
            }),
            cx.observe(&tree, |_, _, cx| cx.notify()),
            cx.observe(&store, |_, _, cx| cx.notify()),
        ];
        Viewer {
            source,
            store,
            agent_id,
            tree,
            tree_view,
            search,
            pane: Pane::Files,
            tabs: Vec::new(),
            active: None,
            docs: HashMap::new(),
            target: None,
            nonce: 0,
            copied: false,
            copied_task: None,
            focus: cx.focus_handle(),
            tab_focus: HashMap::new(),
            _subs: subs,
        }
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn tabs(&self) -> &[String] {
        &self.tabs
    }

    /// Shown (again): focus it, or the search field.
    pub fn show(&mut self, pane: Option<Pane>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = pane {
            self.pane = p;
        }
        if self.pane == Pane::Search && pane == Some(Pane::Search) {
            self.search.update(cx, |s, cx| s.focus_query(window, cx));
        } else {
            window.focus(&self.focus);
        }
        cx.notify();
    }

    /// Leaving: a running search stops; closed-over documents are dropped
    /// (tabs stay for the window's session).
    pub fn hidden(&mut self, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.cancel(cx));
        let active = self.active.clone();
        self.docs.retain(|p, _| Some(p) == active.as_ref());
    }

    /// Open `path` in a tab (at `line`, with `char` columns selected).
    pub fn open(
        &mut self,
        path: String,
        at: Option<(u32, Option<(usize, usize)>)>,
        cx: &mut Context<Self>,
    ) {
        if !self.tabs.contains(&path) {
            self.tabs.push(path.clone());
        }
        self.active = Some(path.clone());
        self.nonce += 1;
        self.target = at.map(|(line, cols)| Target {
            line,
            cols,
            nonce: self.nonce,
        });
        self.tree.update(cx, |t, cx| t.reveal(&path, cx));
        self.tree_view
            .update(cx, |v, cx| v.set_selected(Some(path.clone()), cx));
        if !self.docs.contains_key(&path) {
            self.read(&path, false, false, cx);
        }
        cx.emit(ViewerEvent::Opened(path));
        cx.notify();
    }

    fn select(&mut self, path: String, cx: &mut Context<Self>) {
        self.target = None;
        self.tree_view
            .update(cx, |v, cx| v.set_selected(Some(path.clone()), cx));
        if !self.docs.contains_key(&path) {
            self.read(&path, false, false, cx);
        }
        self.active = Some(path);
        cx.notify();
    }

    /// Close a tab (its document is dropped).
    pub fn close_tab(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(i) = self.tabs.iter().position(|p| p == path) else {
            return;
        };
        self.tabs.remove(i);
        // Closed files drop their documents.
        self.docs.remove(path);
        if self.active.as_deref() == Some(path) {
            let next = self
                .tabs
                .get(i.min(self.tabs.len().saturating_sub(1)))
                .cloned();
            match next {
                Some(p) => self.select(p, cx),
                None => {
                    self.active = None;
                    self.tree_view.update(cx, |v, cx| v.set_selected(None, cx));
                }
            }
        }
        cx.notify();
    }

    /// Read `path` (`large`: "Load anyway"; `quiet`: keep what is shown and
    /// offer the newer text when it differs).
    fn read(&mut self, path: &str, large: bool, quiet: bool, cx: &mut Context<Self>) {
        if !quiet {
            let d = self.docs.entry(path.to_string()).or_default();
            d.error = None;
            d.loading = true;
            d.newer = None;
        }
        let (source, agent, p) = (self.source.clone(), self.agent_id.clone(), path.to_string());
        let load = cx
            .background_executor()
            .spawn(async move { source.read_file(&agent, &p, large) });
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            let res: Res<FileView> = load.await;
            let _ = this.update(cx, |this, cx| {
                this.read_done(path, quiet, res, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn read_done(&mut self, path: String, quiet: bool, res: Res<FileView>, cx: &mut Context<Self>) {
        // Closed meanwhile: dropped.
        if !self.tabs.contains(&path) {
            return;
        }
        let d = self.docs.entry(path).or_default();
        match res {
            Ok(v) => {
                let v = Loaded::from(v);
                if quiet && d.view.is_some() {
                    if d.view.as_ref() != Some(&v) {
                        d.newer = Some(v);
                    }
                } else {
                    *d = Doc {
                        view: Some(v),
                        ..Doc::default()
                    };
                }
            }
            Err(e) => {
                if !(quiet && d.view.is_some()) {
                    *d = Doc {
                        error: Some(e),
                        ..Doc::default()
                    };
                }
            }
        }
        cx.notify();
    }

    /// ↻, ⌘⇧R, the agent's changes moving: the shown file read again
    /// quietly (a file loaded anyway stays loaded that way).
    pub fn recheck(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else {
            return;
        };
        let Some(shown) = self.docs.get(&active).and_then(|d| d.view.clone()) else {
            return;
        };
        let large = shown.kind == ContentKind::Text && shown.size > TEXT_CAP;
        self.read(&active, large, true, cx);
    }

    pub fn search_for(&mut self, query: &str, cx: &mut Context<Self>) {
        self.pane = Pane::Search;
        self.search.update(cx, |s, cx| s.set_query(query, cx));
        cx.notify();
    }

    /// "Load anyway": a file over 2 MiB read up to 10 MiB.
    pub fn load_anyway(&mut self, path: &str, cx: &mut Context<Self>) {
        self.read(path, true, false, cx);
    }

    /// The open file changed on disk since it was shown.
    pub fn is_stale(&self) -> bool {
        self.active
            .as_ref()
            .and_then(|p| self.docs.get(p))
            .is_some_and(|d| d.newer.is_some())
    }

    #[cfg(test)]
    pub(crate) fn doc_kind(&self, path: &str) -> Option<ContentKind> {
        self.docs.get(path)?.view.as_ref().map(|v| v.kind)
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else {
            return;
        };
        if let Some(d) = self.docs.get_mut(&active) {
            if let Some(n) = d.newer.take() {
                d.view = Some(n);
            }
        }
        cx.notify();
    }

    fn copy_path(&mut self, cwd: &str, cx: &mut Context<Self>) {
        let Some(active) = &self.active else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(absolute_path(cwd, active)));
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

    fn back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ViewerEvent::Exit);
    }

    fn agent(&self, cx: &App) -> Option<AgentView> {
        self.store.read(cx).agent(&self.agent_id).cloned()
    }

    fn bar(&self, t: &Theme, a: Option<&AgentView>, cx: &mut Context<Self>) -> impl IntoElement {
        let tree = self.tree.read(cx).state();
        let (refreshing, updated) = (tree.refreshing, tree.updated_at);
        let cwd_tip: SharedString = a.map(|a| a.cwd.clone()).unwrap_or_default().into();
        div()
            .id("ex-viewer-bar")
            .h(px(32.))
            .flex_none()
            .pl(px(12.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(t.line)
            .child(crate::kit::label("Files", t.text_3, 12.))
            .when_some(a, |d, a| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(7.))
                        // `<StatusGlyph size="sm">`.
                        .child(crate::kit::glyph(a.status, t, true))
                        .child(crate::kit::label(a.name.clone(), t.text_3, 12.).text_color(t.text)),
                )
                .child(
                    div()
                        .id("ex-viewer-hint")
                        .min_w_0()
                        .ellipsis()
                        .text_size(px(12.))
                        .text_color(t.text_3)
                        .tooltip(move |_, cx| tooltip_view(cwd_tip.clone(), cx))
                        .child(crate::kit::one_line(format!("read-only · {}", a.cwd_display))),
                )
            })
            .child(div().flex_1())
            .child(refresh_control(
                t,
                "ex-viewer-refresh",
                refreshing,
                updated,
                now_ms(),
                cx.listener(|this, _, _, cx| {
                    this.tree.update(cx, |t, cx| t.refresh(false, cx));
                    this.recheck(cx);
                }),
            ))
            .child(
                small_btn("ex-viewer-goto", "Go to file", t).child(kbd(&keys("⌘P", false), t))
.on_click(cx.listener(|_, _, _, cx| cx.emit(ViewerEvent::QuickOpen)))
                .tooltip(|_, cx| {
                    tooltip_view(keys("Go to file (⌘P)", false), cx)
                }),
            )
            .child(small_btn("ex-viewer-back", "Back", t).child(kbd("esc", t))
.on_click(cx.listener(|_, _, _, cx| cx.emit(ViewerEvent::Exit))))
    }

    fn side(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let ignored = self.tree.read(cx).ignored();
        let seg = |id: &'static str,
                   text: &'static str,
                   on: bool,
                   pane: Pane,
                   first: bool,
                   cx: &mut Context<Self>| {
            let hover = t.text_2;
            div()
                .id(id)
                .h(px(24.))
                .px(px(9.))
                .flex()
                .items_center()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .cursor_pointer()
                .text_color(if on { t.text } else { t.text_3 })
                .when(on, |d| d.bg(t.surface_3))
                .when(!first, |d| d.border_l_1().border_color(t.line_strong))
                .when(!on, |d| d.hover_text(hover, move |s| s))
                .child(text)
                .on_click(cx.listener(move |this, _, window, cx| this.show(Some(pane), window, cx)))
        };
        div()
            .w(px(300.))
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .border_r_1()
            .border_color(t.line)
            .bg(t.surface)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(12.))
                    .pr(px(10.))
                    .pt(px(8.))
                    .pb(px(6.))
                    .child(
                        div()
                            .flex()
                            .rounded(RADIUS)
                            .border_1()
                            .border_color(t.line_strong)
                            .overflow_hidden()
                            .child(seg(
                                "ex-seg-files",
                                "EXPLORER",
                                self.pane == Pane::Files,
                                Pane::Files,
                                true,
                                cx,
                            ))
                            .child(
                                seg(
                                    "ex-seg-search",
                                    "SEARCH",
                                    self.pane == Pane::Search,
                                    Pane::Search,
                                    false,
                                    cx,
                                )
                                .tooltip(|_, cx| {
                                    tooltip_view(keys("Search in files (⌘⇧F)", false)
                                        , cx)
                                }),
                            ),
                    )
                    .child(div().flex_1())
                    .when(self.pane == Pane::Files, |d| {
                        d.child(
                            checkbox("ex-viewer-ignored", ignored, "Ignored", t, cx.listener(move |_, _, _, cx| {
                                    cx.emit(ViewerEvent::SetIgnored(!ignored))
                                }))
                            .tooltip(|_, cx| {
                                tooltip_view("Also list what .gitignore leaves out (dimmed)", cx)
                            }),
                        )
                    }),
            )
            .child(match self.pane {
                Pane::Files => self.tree_view.clone().into_any_element(),
                Pane::Search => self.search.clone().into_any_element(),
            })
    }

    fn tabs_row(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree = self.tree.read(cx).state();
        let tabs: Vec<AnyElement> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let selected = self.active.as_deref() == Some(p.as_str());
                let status = tree.find_entry(p).and_then(|e| e.status);
                let green = matches!(status, Some(FileStatus::A | FileStatus::U));
                let (p1, p2, p3) = (p.clone(), p.clone(), p.clone());
                let title: SharedString = p.clone().into();
                let hover = t.text_2;
                let focus = self.tab_focus.get(p).cloned().unwrap_or_else(|| cx.focus_handle());
                let ring = focus.is_focused(window);
                let p4 = p.clone();
                let close_hover = t.surface_3;
                let close_text = t.text;
                div()
                    .id(("ex-tab", i))
                    .group("ex-tab")
                    .relative()
                    .h(px(32.))
                    .max_w(px(240.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(10.))
                    .pr(px(4.))
                    .border_r_1()
                    .border_color(t.line)
                    .font_family(MONO_FONT)
                    .text_size(px(12.))
                    .cursor_pointer()
                    .text_color(if selected { t.text } else { t.text_3 })
                    .when(selected, |d| {
                        d.bg(t.term_bg).child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .h(px(1.))
                                .bg(t.text_2),
                        )
                    })
                    .when(!selected, |d| d.hover_text(hover, move |s| s))
                    // Keyboard focus: a 1 px ring inside (`.ex-tab:focus-visible`);
                    // a click shows the file without taking the keys.
                    .key_context(TAB_CONTEXT)
                    .track_focus(&focus)
                    .when(ring, |d| d.child(div().absolute().inset_0().border_1().border_color(t.focus)))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_action(|_: &NextTab, window, _| window.focus_next())
                    .on_action(|_: &PrevTab, window, _| window.focus_prev())
                    .on_action(cx.listener(move |this, _: &PickTab, _, cx| this.select(p4.clone(), cx)))
                    .tooltip(move |_, cx| tooltip_view(title.clone(), cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(p1.clone(), cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, _, cx| this.close_tab(&p2, cx)),
                    )
                    .child(file_icon(p, 14.))
                    .child(
                        div()
                            .min_w_0()
                            .ellipsis()
                            .when(green, |d| d.text_color(t.green))
                            .child(crate::kit::one_line(split_path(p).1.to_string())),
                    )
                    .child(
                        div()
                            .id(("ex-tab-close", i))
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(RADIUS_SM)
                            .when(!selected, |d| {
                                d.invisible().group_hover_probed("ex-tab", |s| s.visible())
                            })
                            .hover_text(close_text, move |s| s.bg(close_hover))
                            .tooltip(|_, cx| tooltip_view("Close", cx))
                            .child(icon("x", 11., t.text_3))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(&p3, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id("ex-tabs")
            .h(px(32.))
            .flex_none()
            .flex()
            .overflow_x_scroll()
            .bg(t.surface)
            .border_b_1()
            .border_color(t.line)
            .children(tabs)
    }

    fn head(
        &self,
        t: &Theme,
        a: Option<&AgentView>,
        active: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let status = self
            .tree
            .read(cx)
            .state()
            .find_entry(active)
            .and_then(|e| e.status);
        let (dir, base) = split_path(active);
        let size = self
            .docs
            .get(active)
            .and_then(|d| d.view.as_ref())
            .map(|v| size_label(v.size));
        let cwd = a.map(|a| a.cwd.clone()).unwrap_or_default();
        let abs: SharedString = absolute_path(&cwd, active).into();
        let path_tip: SharedString = active.to_string().into();
        let can_diff = status.is_some() && a.is_some_and(|a| a.caps.review);
        let diff_path = active.to_string();
        div()
            .flex_none()
            .px(px(12.))
            .py(px(7.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(t.line)
            .child(file_icon(active, 16.))
            .child(
                div()
                    .id("ex-head-path")
                    .flex()
                    .items_baseline()
                    .gap(px(6.))
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .font_family(MONO_FONT)
                    .text_size(px(12.))
                    .tooltip(move |_, cx| tooltip_view(path_tip.clone(), cx))
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(base.to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_color(t.text_3)
                            .ellipsis()
                            .child(crate::kit::one_line(dir.to_string())),
                    ),
            )
            .when_some(status, |d, s| d.child(status_mark(t, "ex-head-status", s)))
            .when_some(size, |d, s| {
                d.child(
                    div()
                        .flex_none()
                        .font_family(MONO_FONT)
                        .text_size(px(11.))
                        .text_color(t.text_3)
                        .child(s),
                )
            })
            .child(div().flex_1())
            .child(
                button("ex-copy-path", BtnKind::GhostSm, false, t).gap(px(6.)).child(icon("copy", 13., t.text_2)).child(if self.copied { "Copied" } else { "Copy path" })
.on_click(cx.listener(move |this, _, _, cx| this.copy_path(&cwd, cx)))
                .tooltip(move |_, cx| tooltip_view(abs.clone(), cx)),
            )
            .when(can_diff, |d| {
                d.child(
                    button("ex-show-diff", BtnKind::GhostSm, false, t).gap(px(6.)).child(icon("review", 13., t.text_2)).child("Show diff")
.on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(ViewerEvent::ShowDiff(diff_path.clone()))
                        }))
                    .tooltip(|_, cx| tooltip_view("This file's changes, in Review", cx)),
                )
            })
    }

    fn body(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let center = |t: &Theme| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .text_size(px(12.5))
                .text_color(t.text_3)
        };
        let Some(active) = self.active.clone() else {
            return center(t)
                .child("Pick a file on the left")
                .child(small_btn("ex-empty-goto", "Go to file", t).child(kbd(&keys("⌘P", false), t))
.on_click(cx.listener(|_, _, _, cx| cx.emit(ViewerEvent::QuickOpen))))
                .into_any_element();
        };
        let doc = self.docs.get(&active);
        if let Some(e) = doc.and_then(|d| d.error.clone()) {
            let path = active.clone();
            return div()
                .p(px(14.))
                .flex()
                .items_center()
                .gap(px(10.))
                .text_size(px(12.))
                .text_color(t.red)
                .child(format!(
                    "Couldn't read {active}: {}",
                    e.lines().next().unwrap_or_default()
                ))
                .child(small_btn("ex-retry", "Retry", t)
.on_click(cx.listener(move |this, _, _, cx| this.read(&path, false, false, cx))))
                .into_any_element();
        }
        let Some(v) = doc.and_then(|d| d.view.clone()) else {
            return div()
                .p(px(14.))
                .text_size(px(12.))
                .text_color(t.text_3)
                .child("Reading…")
                .into_any_element();
        };
        match v.kind {
            ContentKind::Binary => center(t)
                .child(format!("Binary file · {} · not shown", size_label(v.size)))
                .into_any_element(),
            ContentKind::TooLarge => {
                let loading = doc.is_some_and(|d| d.loading);
                let path = active.clone();
                center(t)
                    .child(format!("Large file · {} · not opened", size_label(v.size)))
                    .child(if v.size <= LARGE_CAP {
                        small_btn("ex-load-anyway", if loading { "Reading…" } else { "Load anyway" }, t)
.on_click(cx.listener(move |this, _, _, cx| this.load_anyway(&path, cx)))
                        .into_any_element()
                    } else {
                        div()
                            .text_size(px(11.5))
                            .text_color(t.text_4)
                            .child(format!(
                                "Files over {} aren't shown.",
                                size_label(LARGE_CAP)
                            ))
                            .into_any_element()
                    })
                    .into_any_element()
            }
            ContentKind::Text => {
                let text = v.text.clone().unwrap_or_default();
                code_view(&active, &text, self.target.as_ref(), window, cx)
            }
        }
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let a = self.agent(cx);
        let stale = self.is_stale();
        let active = self.active.clone();
        let body = self.body(&t, window, cx);
        let tabs = self.tabs.clone();
        self.tab_focus.retain(|p, _| tabs.contains(p));
        for p in tabs {
            self.tab_focus.entry(p).or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
        div()
            .id("ex-viewer")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::back))
            .on_action(|_: &NextTab, window, _| window.focus_next())
            .on_action(|_: &PrevTab, window, _| window.focus_prev())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .child(self.bar(&t, a.as_ref(), cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.side(&t, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .bg(t.term_bg)
                            .when(!self.tabs.is_empty(), |d| d.child(self.tabs_row(&t, window, cx)))
                            .when_some(active, |d, p| d.child(self.head(&t, a.as_ref(), &p, cx)))
                            .when(stale, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .px(px(12.))
                                        .py(px(5.))
                                        .flex()
                                        .items_center()
                                        .gap(px(10.))
                                        .border_b_1()
                                        .border_color(t.line)
                                        .bg(t.amber_soft)
                                        .text_size(px(12.))
                                        .text_color(t.text_2)
                                        .child("Changed on disk since it was opened.")
                                        .child(small_btn("ex-reload", "Reload", &t)
.on_click(cx.listener(|this, _, _, cx| this.reload(cx)))),
                                )
                            })
                            .child(div().flex_1().min_h_0().flex().flex_col().child(body)),
                    ),
            )
    }
}
