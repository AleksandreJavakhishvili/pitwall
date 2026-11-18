//! The file viewer (a port of `Viewer.tsx`): the whole main area,
//! like Review, or (an agent showed the user files: [`Host`]) a pane beside
//! that agent's terminal or a window of its own, each with a smaller bar.
//! Left: the agent's tree (same open folders as the Files tab)
//! or Search; centre: tabs of open files, a header (icon, path, change
//! letter, size, "Copy path", "Show diff") and the text. Binary files say
//! so; files over 2 MiB offer "Load anyway" (up to 10 MiB); an open file
//! that changed on disk offers "Reload"; Markdown files switch between
//! Code and a formatted Preview ([`crate::markdown_view`]); images (PNG,
//! JPEG, GIF, WebP, SVG, up to 10 MiB) are drawn fitted to the pane with
//! their pixel size in the header; other binary files on this computer
//! offer "Open in default app".
//!
//! Text files of agents on this computer (and folders here) can be edited
//! for quick fixes (`agents.editFiles`, on by default): ⌘S saves through
//! [`Source::write_file`], a dot marks a tab with unsaved changes, and
//! closing it, the pane or the window, moving between them or quitting
//! asks first (Save / Don't Save / Cancel). A file that changed on disk
//! is read again quietly when it has no unsaved changes; with some, a bar
//! offers Keep mine / Take theirs / Show difference, and a save over a
//! changed file asks first. Other files say in the header why they are
//! read-only. Agents can't write through this: only the user saves.

use crate::kit::Ellipsis as _;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::kit::HoverText;
use gpui::{
    actions, div, prelude::*, px, AnyElement, App, ClipboardItem, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, KeyBinding, MouseButton, PromptLevel, SharedString,
    Subscription, Task, Window,
};

use pitwall_core::explorer::{ContentKind, FileView, Saved};
use pitwall_proto::{AgentView, FileStatus};

use super::code_view::{FileCode, Target};
use crate::code_view::{CodeView, CodeViewEvent};
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
use crate::markdown_view::{model::is_markdown, MarkdownView, MAX_PREVIEW_BYTES};
use crate::theme::{self, Theme, RADIUS, RADIUS_SM};

actions!(explorer_viewer, [Back, NextTab, PrevTab, PickTab, SaveFile]);

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
        // ⌘S with the keyboard anywhere in the viewer saves the open file.
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" }, SaveFile, Some(CONTEXT)),
    ]
}

pub enum ViewerEvent {
    Exit,
    QuickOpen,
    ShowDiff(String),
    /// A file was opened (quick open lists recent files first).
    Opened(String),
    SetIgnored(bool),
    /// Beside a terminal: "Close".
    Close,
    /// Beside a terminal: "Open in window".
    Undock,
    /// In a window of its own: "Dock beside terminal".
    Dock,
    /// In a window of its own: "Keep on top" toggled.
    Pin,
}

/// Where a viewer is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// The main area (the Files tab's "Open", ⌘P, ⌘-click): "Back" leaves.
    Main,
    /// A pane split next to an agent's terminal.
    Beside,
    /// A window of its own.
    Window,
}

/// What a viewer has open, to carry it between a pane and a window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Opened {
    pub tabs: Vec<String>,
    pub active: Option<String>,
    pub pane_files: bool,
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
    /// Not valid UTF-8 (shown with replacement characters; not edited).
    lossy: bool,
}

impl From<FileView> for Loaded {
    fn from(v: FileView) -> Loaded {
        Loaded {
            size: v.size,
            kind: v.kind,
            text: v.text.map(SharedString::from),
            lossy: v.lossy,
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
    /// An image's bytes, once read (`ContentKind::Image`).
    image: Option<Arc<gpui::Image>>,
    /// Why its bytes couldn't be read.
    image_error: Option<String>,
    /// The file is on this computer, here ("Open in default app").
    local: Option<std::path::PathBuf>,
    /// Its text's view (and the edits made in it), once shown.
    code: Option<FileCode>,
    _code_sub: Option<Subscription>,
    /// What a save compares the file on disk with (`None`: `view`'s
    /// text): set by "Keep mine".
    base: Option<SharedString>,
    /// A save found the file changed on disk: the bar asks.
    save_blocked: bool,
    saving: bool,
    save_error: Option<String>,
    /// "Show difference": the file on disk against the edits.
    diff: Option<Entity<CodeView>>,
    /// The edited text, for the Markdown Preview (by its version).
    live: Option<(u64, SharedString)>,
}

impl Doc {
    fn dirty(&self, cx: &App) -> bool {
        self.code.as_ref().is_some_and(|c| c.code.read(cx).is_dirty())
    }
}

/// A decision on unsaved changes (the guard's prompt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsaved {
    Save,
    Discard,
    Cancel,
}

/// Closing `window` (its close button, ⌘W) asks first when the viewer
/// `viewer` finds has unsaved changes; it closes after Save or Don't Save.
pub fn guard_window_close(window: &mut Window, cx: &App, viewer: impl Fn(&App) -> Option<Entity<Viewer>> + 'static) {
    window.on_window_should_close(cx, move |window, cx| {
        let Some(viewer) = viewer(cx) else { return true };
        if viewer.read(cx).unsaved(cx).is_empty() {
            return true;
        }
        viewer.update(cx, |v, cx| v.guard(None, window, cx, |_, window, _| window.remove_window()));
        false
    });
}

/// The guard's prompt: Save / Don't Save / Cancel about `paths`.
pub fn ask_unsaved(paths: &[String], window: &mut Window, cx: &mut App) -> Task<Unsaved> {
    let what = match paths {
        [one] => format!("Save your changes to {}?", split_path(one).1),
        many => format!("Save your changes to {} files?", many.len()),
    };
    let answer = window.prompt(PromptLevel::Warning, &what, Some("Your changes are lost if you don't save them."), &["Save", "Don't Save", "Cancel"], cx);
    cx.spawn(async move |_| match answer.await {
        Ok(0) => Unsaved::Save,
        Ok(1) => Unsaved::Discard,
        _ => Unsaved::Cancel,
    })
}

/// GPUI's format for an image the viewer draws (by its name).
pub fn image_format(path: &str) -> Option<gpui::ImageFormat> {
    let ext = path.rsplit('/').next()?.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => gpui::ImageFormat::Png,
        "jpg" | "jpeg" => gpui::ImageFormat::Jpeg,
        "gif" => gpui::ImageFormat::Gif,
        "webp" => gpui::ImageFormat::Webp,
        "svg" => gpui::ImageFormat::Svg,
        _ => return None,
    })
}

pub struct Viewer {
    source: Arc<dyn Source>,
    store: Entity<AgentStore>,
    /// What it reads: an agent's id, or a folder's `path:` key.
    agent_id: String,
    host: Host,
    /// The agent it was opened for (beside its terminal, or its window):
    /// its name in the bar.
    owner: Option<String>,
    /// In a window: kept on top of other windows.
    pinned: bool,
    /// In a window: a Pitwall window shows the agent ("Dock beside
    /// terminal" can put it there).
    can_dock: bool,
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
    /// Markdown files show their Preview (the session's choice).
    preview: bool,
    pub(crate) md: Entity<MarkdownView>,
    /// What the Preview was given: the path and its text (by identity).
    md_shown: Option<(String, (usize, usize))>,
    _subs: Vec<Subscription>,
}

/// The file viewer's Code / Preview choice for Markdown (kept for the app
/// session; off: Code).
#[derive(Default)]
struct ViewerPreview(bool);

impl gpui::Global for ViewerPreview {}

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
        let ui = crate::ui_state::store(cx);
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
            cx.subscribe_in(&search, window, |this, _, _: &Blurred, window, cx| {
                window.focus(&this.focus, cx)
            }),
            cx.observe(&tree, |_, _, cx| cx.notify()),
            cx.observe(&store, |_, _, cx| cx.notify()),
            // `agents.editFiles` turned on or off.
            cx.observe(&ui, |_, _, cx| cx.notify()),
        ];
        super::unsaved::register(cx.weak_entity(), cx);
        Viewer {
            source,
            store,
            agent_id,
            host: Host::Main,
            owner: None,
            pinned: false,
            can_dock: false,
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
            preview: cx.try_global::<ViewerPreview>().is_some_and(|p| p.0),
            md: cx.new(MarkdownView::new),
            md_shown: None,
            _subs: subs,
        }
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// The tree it shows.
    pub fn tree(&self) -> Entity<TreeModel> {
        self.tree.clone()
    }

    pub fn host(&self) -> Host {
        self.host
    }

    /// Drawn beside a terminal or in a window, for agent `owner`.
    pub fn set_host(&mut self, host: Host, owner: Option<String>, cx: &mut Context<Self>) {
        self.host = host;
        self.owner = owner;
        cx.notify();
    }

    pub fn set_pinned(&mut self, on: bool, cx: &mut Context<Self>) {
        self.pinned = on;
        cx.notify();
    }

    pub fn set_can_dock(&mut self, can: bool, cx: &mut Context<Self>) {
        if self.can_dock != can {
            self.can_dock = can;
            cx.notify();
        }
    }

    /// Its tabs, the active one and the side pane (moving it).
    pub fn opened(&self) -> Opened {
        Opened { tabs: self.tabs.clone(), active: self.active.clone(), pane_files: self.pane == Pane::Files }
    }

    /// Take over what another viewer had open.
    pub fn restore(&mut self, o: &Opened, cx: &mut Context<Self>) {
        for p in &o.tabs {
            if !self.tabs.contains(p) {
                self.tabs.push(p.clone());
            }
        }
        if !o.pane_files {
            self.pane = Pane::Search;
        }
        if let Some(a) = o.active.clone() {
            self.select(a, cx);
        }
        cx.notify();
    }

    /// The folder it shows: the `path:` key's, else the agent's.
    pub fn root(&self, cx: &App) -> Option<String> {
        match super::source::root_of_key(&self.agent_id) {
            Some(r) => Some(r.to_string()),
            None => self.agent(cx).map(|a| a.cwd),
        }
    }

    /// The folder it shows, for people (`~` for the home folder).
    fn root_display(&self, cx: &App) -> String {
        match super::source::root_of_key(&self.agent_id) {
            Some(r) => pitwall_core::paths::tildify(r),
            None => self.agent(cx).map(|a| a.cwd_display).unwrap_or_default(),
        }
    }

    /// The files are on this computer.
    fn local(&self, cx: &App) -> bool {
        super::source::root_of_key(&self.agent_id).is_some()
            || self.agent(cx).is_some_and(|a| a.machine.provider == pitwall_core::provider::ProviderId::LOCAL)
    }

    /// The scroll target (tests).
    #[cfg(test)]
    pub(crate) fn target_line(&self) -> Option<u32> {
        self.target.as_ref().map(|t| t.line)
    }

    /// `path`'s document reads `text` (tests).
    #[cfg(test)]
    pub(crate) fn shows_text(&self, path: &str, text: &str) -> bool {
        self.docs
            .get(path)
            .and_then(|d| d.view.as_ref())
            .and_then(|v| v.text.as_ref())
            .is_some_and(|t| t.as_ref() == text)
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
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// Leaving: a running search stops; closed-over documents are dropped
    /// (tabs stay for the window's session).
    pub fn hidden(&mut self, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.cancel(cx));
        let active = self.active.clone();
        // (Unsaved changes stay: the tab is still there to save them.)
        self.docs.retain(|p, d| Some(p) == active.as_ref() || d.dirty(cx));
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
        // (Outside the folder: not in the tree.)
        let in_tree = !super::logic::outside(&path);
        if in_tree {
            self.tree.update(cx, |t, cx| t.reveal(&path, cx));
        }
        let selected = in_tree.then(|| path.clone());
        self.tree_view.update(cx, |v, cx| v.set_selected(selected, cx));
        if !self.docs.contains_key(&path) {
            self.read(&path, false, false, cx);
        }
        cx.emit(ViewerEvent::Opened(path));
        cx.notify();
    }

    fn select(&mut self, path: String, cx: &mut Context<Self>) {
        self.target = None;
        let selected = (!super::logic::outside(&path)).then(|| path.clone());
        self.tree_view.update(cx, |v, cx| v.set_selected(selected, cx));
        if !self.docs.contains_key(&path) {
            self.read(&path, false, false, cx);
        }
        self.active = Some(path);
        cx.notify();
    }

    /// Show folder `dir` (relative; "" = the root) opened and selected in
    /// the tree (an agent asked to show it).
    pub fn reveal_folder(&mut self, dir: &str, cx: &mut Context<Self>) {
        self.pane = Pane::Files;
        if dir.is_empty() {
            return;
        }
        self.tree.update(cx, |t, cx| {
            t.reveal(dir, cx);
            t.expand(dir, cx);
        });
        let dir = dir.to_string();
        self.tree_view.update(cx, |v, cx| v.set_selected(Some(dir), cx));
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn tree_is_open(&self, dir: &str, cx: &App) -> bool {
        self.tree.read(cx).is_open(dir)
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
        // On this computer: where it is here (for "Open in default app").
        let here = self.root(cx).filter(|_| self.local(cx)).map(|root| absolute_path(&root, path));
        let load = cx
            .background_executor()
            .spawn(async move {
                let res = if super::logic::outside(&p) {
                    source.read_outside(&p, large)
                } else {
                    source.read_file(&agent, &p, large)
                };
                let here = here.map(std::path::PathBuf::from).filter(|h| h.is_file());
                (res, here)
            });
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            let (res, here): (Res<FileView>, _) = load.await;
            let _ = this.update(cx, |this, cx| {
                this.read_done(path, quiet, res, here, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn read_done(
        &mut self,
        path: String,
        quiet: bool,
        res: Res<FileView>,
        here: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        // Closed meanwhile: dropped.
        if !self.tabs.contains(&path) {
            return;
        }
        let dirty = self.docs.get(&path).is_some_and(|d| d.dirty(cx));
        let d = self.docs.entry(path.clone()).or_default();
        match res {
            Ok(v) => {
                let v = Loaded::from(v);
                let image = v.kind == ContentKind::Image;
                if quiet && d.view.is_some() {
                    let on_disk = d.base.as_ref().or(d.view.as_ref().and_then(|w| w.text.as_ref()));
                    if dirty || d.save_blocked {
                        // Unsaved changes: never overwritten; the bar asks.
                        let differs = v.text.as_ref() != on_disk || v.kind != ContentKind::Text;
                        d.newer = differs.then_some(v);
                        if let (Some(diff), Some(theirs)) = (d.diff.clone(), d.newer.as_ref().and_then(|n| n.text.clone())) {
                            let mine = d.code.as_ref().and_then(|c| c.code.read(cx).text()).unwrap_or_default();
                            diff.update(cx, |c, cx| c.set_diff(&path, theirs.to_string(), mine, cx));
                        }
                    } else if d.view.as_ref() != Some(&v) {
                        // Nothing unsaved: the new text, quietly (the view
                        // keeps its scroll position and caret).
                        d.view = Some(v);
                        d.newer = None;
                        d.base = None;
                        d.image = None;
                        d.image_error = None;
                        if image {
                            self.read_image(&path, cx);
                        }
                    }
                } else {
                    let code = d.code.take();
                    let sub = d._code_sub.take();
                    *d = Doc {
                        view: Some(v),
                        local: here,
                        code,
                        _code_sub: sub,
                        ..Doc::default()
                    };
                    if image {
                        self.read_image(&path, cx);
                    }
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

    /// An image's bytes, for drawing it.
    fn read_image(&mut self, path: &str, cx: &mut Context<Self>) {
        let (source, agent, p) = (self.source.clone(), self.agent_id.clone(), path.to_string());
        let load = cx.background_executor().spawn(async move {
            if super::logic::outside(&p) {
                source.read_outside_image(&p)
            } else {
                source.read_image(&agent, &p)
            }
        });
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            let res = load.await;
            let _ = this.update(cx, |this, cx| {
                let Some(d) = this.docs.get_mut(&path) else { return };
                match (res, image_format(&path)) {
                    (Ok(bytes), Some(f)) => d.image = Some(Arc::new(gpui::Image::from_bytes(f, bytes))),
                    (Ok(_), None) => d.image_error = Some("not an image the viewer draws".into()),
                    (Err(e), _) => d.image_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The shown image's bytes are read (tests).
    #[cfg(test)]
    pub fn image_loaded(&self, path: &str) -> bool {
        self.docs.get(path).is_some_and(|d| d.image.is_some())
    }

    /// The active image's size in pixels, once drawn.
    fn image_pixels(&self, window: &mut Window, cx: &mut App) -> Option<(i32, i32)> {
        let image = self.docs.get(self.active.as_ref()?)?.image.clone()?;
        let r = image.get_render_image(window, cx)?;
        let s = r.size(0);
        Some((s.width.0, s.height.0))
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

    /// "Take theirs": the file on disk replaces the edits.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else {
            return;
        };
        let mut image = false;
        let mut again = false;
        if let Some(d) = self.docs.get_mut(&active) {
            d.save_blocked = false;
            d.diff = None;
            d.base = None;
            d.live = None;
            match d.newer.take() {
                Some(n) => {
                    image = n.kind == ContentKind::Image;
                    d.view = Some(n);
                    d.image = None;
                    d.image_error = None;
                }
                None => again = true,
            }
            // The same text as read: the view drops its edits itself.
            if let Some(c) = &d.code {
                c.code.update(cx, |c, cx| {
                    if c.is_dirty() {
                        c.set_editable(false, cx);
                    }
                });
            }
        }
        if image {
            self.read_image(&active, cx);
        }
        if again {
            self.read(&active, false, false, cx);
        }
        cx.notify();
    }

    /// "Keep mine": the edits stay; a save now writes over what is on
    /// disk (it was seen), and goes on when a save was waiting.
    pub fn keep_mine(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else { return };
        let Some(d) = self.docs.get_mut(&active) else { return };
        let was_blocked = std::mem::take(&mut d.save_blocked);
        if let Some(n) = d.newer.take() {
            d.base = n.text;
        }
        d.diff = None;
        if was_blocked {
            self.save(&active, cx).detach();
        }
        cx.notify();
    }

    /// "Show difference": the file on disk against the edits, in place of
    /// the editor (again: back to editing).
    pub fn toggle_difference(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else { return };
        let Some(d) = self.docs.get_mut(&active) else { return };
        if d.diff.take().is_some() {
            cx.notify();
            return;
        }
        let theirs = d.newer.as_ref().and_then(|n| n.text.clone()).or(d.base.clone()).or(d.view.as_ref().and_then(|v| v.text.clone()));
        let mine = d.code.as_ref().and_then(|c| c.code.read(cx).text());
        let (Some(theirs), Some(mine)) = (theirs, mine) else { return };
        let view = cx.new(CodeView::new);
        view.update(cx, |c, cx| {
            c.set_layout(crate::code_view::Layout::Unified, cx);
            c.set_diff(&active, theirs.to_string(), mine, cx);
        });
        d.diff = Some(view);
        cx.notify();
    }

    // ── editing and saving ──────────────────────────────────────────────

    /// Why `path` can't be edited here (`None`: it can).
    fn edit_block(&self, path: &str, cx: &App) -> Option<&'static str> {
        let v = self.docs.get(path)?.view.as_ref()?;
        if !super::show::edit_enabled(cx) {
            return Some("editing is off in Settings");
        }
        match v.kind {
            ContentKind::Text => {}
            ContentKind::Image => return Some("images aren't edited here"),
            ContentKind::Binary => return Some("binary file"),
            ContentKind::TooLarge => return Some("too large"),
        }
        if super::logic::outside(path) {
            return Some("outside the folder");
        }
        let mirrored = self.agent(cx).is_some_and(|a| crate::agents::mirrored(&a));
        if !self.local(cx) || mirrored {
            return Some("on another machine");
        }
        if v.size > TEXT_CAP {
            return Some("too large to edit");
        }
        if v.lossy {
            return Some("not UTF-8 text");
        }
        None
    }

    /// The active text file's view, made once, following the edit
    /// setting and the file's limits.
    fn ensure_code(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else { return };
        let block = self.edit_block(&active, cx);
        let Some(d) = self.docs.get_mut(&active) else { return };
        if !d.view.as_ref().is_some_and(|v| v.kind == ContentKind::Text) {
            return;
        }
        if d.code.is_none() {
            let fc = FileCode::new(cx);
            let path = active.clone();
            d._code_sub = Some(cx.subscribe(&fc.code, move |this, code, e: &CodeViewEvent, cx| match e {
                CodeViewEvent::Save => this.save(&path, cx).detach(),
                CodeViewEvent::Edited => {
                    if let Some(d) = this.docs.get_mut(&path) {
                        let c = code.read(cx);
                        d.live = (is_markdown(&path) && c.is_dirty()).then(|| (c.version(), c.text().unwrap_or_default().into()));
                    }
                    cx.notify();
                }
                CodeViewEvent::Loaded => {
                    if let Some(d) = this.docs.get_mut(&path) {
                        d.live = None;
                    }
                    cx.notify();
                }
                CodeViewEvent::Comment { .. } => {}
            }));
            d.code = Some(fc);
        }
        let code = d.code.as_ref().map(|c| c.code.clone()).expect("made");
        let dirty = code.read(cx).is_dirty();
        let want = block.is_none() || dirty;
        code.update(cx, |c, cx| {
            if c.wants_editing() != want {
                c.set_editable(want, cx);
            }
            let hint: SharedString = match block {
                Some(why) => format!("Read-only: {why}").into(),
                None => "Read-only".into(),
            };
            c.set_readonly_text(hint, cx);
        });
    }

    /// The active file's code view (debug commands).
    pub fn active_code(&self) -> Option<Entity<CodeView>> {
        let d = self.docs.get(self.active.as_ref()?)?;
        d.code.as_ref().map(|c| c.code.clone())
    }

    /// What the debug command `files-edit … state` prints.
    pub fn debug_state(&self, cx: &App) -> String {
        let active = self.active.clone().unwrap_or_default();
        let d = self.docs.get(&active);
        let code = self.active_code();
        format!(
            "active={active:?} unsaved={:?} stale={} blocked={} diff={} save_error={:?} editing={} read_only={:?} text={:?}",
            self.unsaved(cx),
            self.is_stale(),
            d.is_some_and(|d| d.save_blocked),
            d.is_some_and(|d| d.diff.is_some()),
            d.and_then(|d| d.save_error.clone()),
            code.as_ref().is_some_and(|c| c.read(cx).is_editing()),
            self.edit_block(&active, cx),
            code.and_then(|c| c.read(cx).text()).map(|t| t.chars().take(160).collect::<String>()),
        )
    }

    /// Files with unsaved changes.
    pub fn unsaved(&self, cx: &App) -> Vec<String> {
        self.tabs.iter().filter(|p| self.docs.get(*p).is_some_and(|d| d.dirty(cx))).cloned().collect()
    }

    /// `path` has unsaved changes.
    pub fn is_dirty(&self, path: &str, cx: &App) -> bool {
        self.docs.get(path).is_some_and(|d| d.dirty(cx))
    }

    /// Save `path`'s edits; whether they were written.
    pub fn save(&mut self, path: &str, cx: &mut Context<Self>) -> Task<bool> {
        let Some(d) = self.docs.get_mut(path) else { return Task::ready(false) };
        let Some(text) = d.code.as_ref().and_then(|c| c.code.read(cx).text()) else { return Task::ready(false) };
        if d.saving {
            return Task::ready(false);
        }
        let base = d.base.clone().or(d.view.as_ref().and_then(|v| v.text.clone())).map(|s| s.to_string());
        d.saving = true;
        d.save_error = None;
        let (source, agent, p) = (self.source.clone(), self.agent_id.clone(), path.to_string());
        let t = text.clone();
        let job = cx.background_executor().spawn(async move { source.write_file(&agent, &p, &t, base.as_deref()) });
        let path = path.to_string();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = job.await;
            this.update(cx, |this, cx| this.saved(&path, text, res, cx)).unwrap_or(false)
        })
    }

    fn saved(&mut self, path: &str, text: String, res: Res<Saved>, cx: &mut Context<Self>) -> bool {
        let large = self.docs.get(path).and_then(|d| d.view.as_ref()).is_some_and(|v| v.size > TEXT_CAP);
        let Some(d) = self.docs.get_mut(path) else { return false };
        d.saving = false;
        let ok = match res {
            Ok(Saved::Written) => {
                let s: SharedString = text.into();
                if let Some(v) = d.view.as_mut() {
                    v.size = s.len() as u64;
                    v.text = Some(s.clone());
                }
                if let Some(c) = &d.code {
                    c.adopt(&s);
                    c.code.update(cx, |c, cx| c.mark_saved(&s, cx));
                }
                d.base = None;
                d.newer = None;
                d.save_blocked = false;
                d.diff = None;
                true
            }
            Ok(Saved::ChangedOnDisk) => {
                d.save_blocked = true;
                self.read(path, large, true, cx);
                false
            }
            Err(e) => {
                d.save_error = Some(e);
                false
            }
        };
        cx.notify();
        ok
    }

    /// Save every file with unsaved changes; whether all were written.
    pub fn save_all(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        let saves: Vec<Task<bool>> = self.unsaved(cx).iter().map(|p| self.save(p, cx)).collect();
        cx.spawn(async move |_, _| {
            let mut ok = true;
            for s in saves {
                ok &= s.await;
            }
            ok
        })
    }

    /// Before dropping `paths` (all unsaved files when `None`): ask about
    /// unsaved changes, then `go` (after saving them, when asked and it
    /// worked; not at all on Cancel).
    pub fn guard(
        &mut self,
        paths: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
        go: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let dirty: Vec<String> = match paths {
            Some(p) => p.into_iter().filter(|p| self.is_dirty(p, cx)).collect(),
            None => self.unsaved(cx),
        };
        if dirty.is_empty() {
            return go(self, window, cx);
        }
        let answer = ask_unsaved(&dirty, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let saved = match answer.await {
                Unsaved::Cancel => return,
                Unsaved::Discard => None,
                Unsaved::Save => {
                    let Ok(saves) = this.update(cx, |this, cx| dirty.iter().map(|p| this.save(p, cx)).collect::<Vec<_>>()) else { return };
                    let mut ok = true;
                    for s in saves {
                        ok &= s.await;
                    }
                    Some(ok)
                }
            };
            if saved == Some(false) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| go(this, window, cx));
        })
        .detach();
    }

    /// The tab's close button (and middle click): asks first when it has
    /// unsaved changes.
    pub fn request_close_tab(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let p = path.to_string();
        self.guard(Some(vec![p.clone()]), window, cx, move |this, _, cx| this.close_tab(&p, cx));
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
        // A window of its own has nothing to go back to.
        if self.host != Host::Window {
            cx.emit(ViewerEvent::Exit);
        }
    }

    /// The active file's text when it can be previewed (Markdown, read,
    /// not too large).
    pub(crate) fn md_text(&self) -> Option<(String, SharedString)> {
        let active = self.active.clone().filter(|p| is_markdown(p))?;
        let d = self.docs.get(&active)?;
        let v = d.view.as_ref()?;
        // Being edited: the Preview follows the edits.
        let text = d.live.as_ref().map(|(_, t)| t.clone()).or_else(|| v.text.clone()).filter(|_| v.kind == ContentKind::Text)?;
        (text.len() <= MAX_PREVIEW_BYTES).then_some((active, text))
    }

    /// The Preview is shown instead of the code.
    pub(crate) fn showing_preview(&self) -> bool {
        self.preview && self.md_text().is_some()
    }

    pub(crate) fn set_preview(&mut self, on: bool, cx: &mut Context<Self>) {
        cx.set_global(ViewerPreview(on));
        self.preview = on;
        let root = self.root(cx);
        self.sync_preview(root.as_deref(), cx);
        cx.notify();
    }

    /// The active Markdown file's text, into the Preview (when it changed).
    fn sync_preview(&mut self, root: Option<&str>, cx: &mut Context<Self>) {
        let Some((path, text)) = self.md_text().filter(|_| self.preview) else {
            return;
        };
        let id = (text.as_ptr() as usize, text.len());
        if self.md_shown.as_ref() == Some(&(path.clone(), id)) {
            return;
        }
        self.md_shown = Some((path.clone(), id));
        let base = root.and_then(|root| {
            std::path::Path::new(&absolute_path(root, &path))
                .parent()
                .map(|p| p.to_path_buf())
        });
        let text: Arc<str> = Arc::from(text.as_ref());
        self.md.update(cx, |m, cx| m.set(&path, text, None, base, cx));
    }

    fn agent(&self, cx: &App) -> Option<AgentView> {
        self.store.read(cx).agent(&self.agent_id).cloned()
    }

    /// Beside a terminal or in a window: the folder, refresh, and the way
    /// to the other place (and close, or keep on top).
    fn small_bar(&self, t: &Theme, a: Option<&AgentView>, cx: &mut Context<Self>) -> AnyElement {
        let tree = self.tree.read(cx).state();
        let (refreshing, updated) = (tree.refreshing, tree.updated_at);
        let root = self.root_display(cx);
        let tip: SharedString = self.root(cx).unwrap_or_default().into();
        let owner = self.owner.clone().or_else(|| a.map(|a| a.name.clone()));
        let btn = |id: &'static str, name: &str, tip: &'static str, on: Option<bool>| {
            crate::kit::icon_btn_state(id, name, tip, true, 13., on, t)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let host = self.host;
        let can_dock = self.can_dock;
        let editable = super::show::edit_enabled(cx) && self.local(cx) && !a.is_some_and(crate::agents::mirrored);
        div()
            .id("ex-viewer-bar")
            .debug_selector(|| "ex-viewer-bar".into())
            .h(px(32.))
            .flex_none()
            .pl(px(10.))
            .pr(px(4.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(t.line)
            .bg(t.surface)
            .child(crate::kit::label("Files", t.text_3, 12.))
            .when_some(owner, |d, name| {
                d.child(div().flex_none().child(crate::kit::label(name, t.text_3, 12.).text_color(t.text)))
            })
            .child(
                div()
                    .id("ex-viewer-hint")
                    .min_w_0()
                    .flex_1()
                    .ellipsis()
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .tooltip(move |_, cx| tooltip_view(tip.clone(), cx))
                    .child(crate::kit::one_line(if editable { root } else { format!("read-only · {root}") })),
            )
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
            .map(|d| match host {
                Host::Beside => d
                    .child(
                        btn("ex-undock", "window", "Open in window", None)
                            .debug_selector(|| "ex-undock".into())
                            .on_click(cx.listener(|this, _, window, cx| this.guard(None, window, cx, |_, _, cx| cx.emit(ViewerEvent::Undock)))),
                    )
                    .child(
                        btn("ex-close-beside", "x", "Close (Esc gives the keyboard back to the terminal)", None)
                            .debug_selector(|| "ex-close-beside".into())
                            .on_click(cx.listener(|this, _, window, cx| this.guard(None, window, cx, |_, _, cx| cx.emit(ViewerEvent::Close)))),
                    ),
                _ => d
                    .child(
                        btn(
                            "ex-dock",
                            "page-right",
                            if can_dock { "Dock beside terminal" } else { "Dock beside terminal (no Pitwall window shows this agent)" },
                            None,
                        )
                        .debug_selector(|| "ex-dock".into())
                        .when(!can_dock, |d| d.opacity(0.4))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if can_dock {
                                this.guard(None, window, cx, |_, _, cx| cx.emit(ViewerEvent::Dock))
                            }
                        })),
                    )
                    .when(crate::platform::can_keep_on_top(), |d| {
                        d.child(
                            btn("ex-pin", "pin", "Keep on top of other windows", Some(self.pinned))
                                .debug_selector(|| "ex-pin".into())
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(ViewerEvent::Pin))),
                        )
                    }),
            })
            .into_any_element()
    }

    fn bar(&self, t: &Theme, a: Option<&AgentView>, cx: &mut Context<Self>) -> AnyElement {
        if self.host != Host::Main {
            return self.small_bar(t, a, cx);
        }
        let tree = self.tree.read(cx).state();
        let (refreshing, updated) = (tree.refreshing, tree.updated_at);
        let cwd_tip: SharedString = a.map(|a| a.cwd.clone()).unwrap_or_default().into();
        let editable = super::show::edit_enabled(cx) && self.local(cx) && !a.is_some_and(crate::agents::mirrored);
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
                        .child(crate::kit::one_line(if editable { a.cwd_display.clone() } else { format!("read-only · {}", a.cwd_display) })),
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
            .into_any_element()
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
            .w(px(if self.host == Host::Main { 300. } else { 220. }))
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
                let dirty = self.docs.get(p).is_some_and(|d| d.dirty(cx));
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
                    .on_action(|_: &NextTab, window, cx| window.focus_next(cx))
                    .on_action(|_: &PrevTab, window, cx| window.focus_prev(cx))
                    .on_action(cx.listener(move |this, _: &PickTab, _, cx| this.select(p4.clone(), cx)))
                    .tooltip(move |_, cx| tooltip_view(title.clone(), cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(p1.clone(), cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.request_close_tab(&p2, window, cx)),
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
                            .when(!selected && !dirty, |d| {
                                d.invisible().group_hover_probed("ex-tab", |s| s.visible())
                            })
                            .hover_text(close_text, move |s| s.bg(close_hover))
                            .tooltip(move |_, cx| tooltip_view(if dirty { "Unsaved changes · Close" } else { "Close" }, cx))
                            .when(dirty, |d| d.debug_selector(move || format!("ex-tab-dirty-{i}")))
                            .child(if dirty {
                                div().size(px(8.)).rounded_full().bg(t.text_2).into_any_element()
                            } else {
                                icon("x", 11., t.text_3).into_any_element()
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.request_close_tab(&p3, window, cx);
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
        pixels: Option<(i32, i32)>,
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
        let cwd = self.root(cx).unwrap_or_default();
        let abs: SharedString = absolute_path(&cwd, active).into();
        let path_tip: SharedString = active.to_string().into();
        let read_only = self.docs.get(active).and_then(|d| d.view.as_ref()).and_then(|_| self.edit_block(active, cx));
        let saving = self.docs.get(active).is_some_and(|d| d.saving);
        // (Review is a Pitwall window's: not from a window of its own.)
        let can_diff = status.is_some() && a.is_some_and(|a| a.caps.review) && self.host != Host::Window;
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
            .when_some(pixels, |d, (w, h)| {
                d.child(
                    div()
                        .flex_none()
                        .debug_selector(|| "ex-image-pixels".into())
                        .font_family(MONO_FONT)
                        .text_size(px(11.))
                        .text_color(t.text_3)
                        .child(format!("{w} × {h} px")),
                )
            })
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
            .when_some(read_only, |d, why| {
                d.child(
                    div()
                        .id("ex-read-only")
                        .debug_selector(|| "ex-read-only".into())
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(t.text_3)
                        .child(format!("Read-only · {why}")),
                )
            })
            .when(saving, |d| d.child(div().flex_none().text_size(px(11.)).text_color(t.text_3).child("Saving…")))
            .child(div().flex_1())
            .when(self.md_text().is_some(), |d| {
                let on = self.preview;
                d.child(
                    crate::kit::segments(t)
                        .debug_selector(|| "ex-md-switch".into())
                        .child(
                            crate::kit::segment("ex-md-code", "Code".into(), !on, true, t)
                                .on_click(cx.listener(|v, _, _, cx| v.set_preview(false, cx))),
                        )
                        .child(
                            crate::kit::segment("ex-md-preview", "Preview".into(), on, false, t)
                                .on_click(cx.listener(|v, _, _, cx| v.set_preview(true, cx))),
                        ),
                )
            })
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

    fn body(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
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
        let here = doc.and_then(|d| d.local.clone());
        match v.kind {
            ContentKind::Binary => {
                let what = if image_format(&active).is_some() {
                    format!("Image · {} · larger than {} · not shown", size_label(v.size), size_label(pitwall_core::explorer::MAX_IMAGE))
                } else {
                    format!("Binary file · {} · not shown", size_label(v.size))
                };
                center(t)
                    .child(what)
                    .when_some(here, |d, here| {
                        d.child(
                            small_btn("ex-open-default", "Open in default app", t)
                                .on_click(cx.listener(move |_, _, _, cx| crate::platform::files::open_in_default_app(cx, &here)))
                                .tooltip(|_, cx| tooltip_view("Open it with the app your system uses for it", cx)),
                        )
                    })
                    .into_any_element()
            }
            ContentKind::Image => match doc.and_then(|d| d.image.clone()) {
                Some(image) => {
                    let err = t.text_3;
                    div()
                        .id("ex-image")
                        .debug_selector(|| "ex-image".into())
                        .flex_1()
                        .min_h_0()
                        .size_full()
                        .p(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui::img(gpui::ImageSource::Image(image))
                                .size_full()
                                .object_fit(gpui::ObjectFit::ScaleDown)
                                .with_fallback(move || {
                                    div().text_size(px(12.5)).text_color(err).child("This image can't be drawn").into_any_element()
                                }),
                        )
                        .into_any_element()
                }
                None => match doc.and_then(|d| d.image_error.clone()) {
                    Some(e) => center(t).child(format!("Couldn't read the image: {e}")).into_any_element(),
                    None => center(t).child("Reading…").into_any_element(),
                },
            },
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
            ContentKind::Text if self.showing_preview() => div()
                .size_full()
                .min_h_0()
                .debug_selector(|| "ex-md-body".into())
                .child(self.md.clone())
                .into_any_element(),
            ContentKind::Text => {
                if let Some(diff) = doc.and_then(|d| d.diff.clone()) {
                    return div().id("ex-difference").debug_selector(|| "ex-difference".into()).size_full().min_h_0().flex().flex_col().child(diff).into_any_element();
                }
                let text = v.text.clone().unwrap_or_default();
                match doc.and_then(|d| d.code.as_ref()) {
                    Some(code) => code.element(&active, &text, self.target.as_ref(), cx),
                    None => div().into_any_element(),
                }
            }
        }
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let a = self.agent(cx);
        let stale = self.is_stale();
        let root = self.root(cx);
        self.sync_preview(root.as_deref(), cx);
        self.ensure_code(cx);
        let active = self.active.clone();
        let pixels = self.image_pixels(window, cx);
        let body = self.body(&t, cx);
        let (blocked, diffing, save_error) = active
            .as_ref()
            .and_then(|p| self.docs.get(p))
            .map_or((false, false, None), |d| (d.save_blocked, d.diff.is_some(), d.save_error.clone()));
        let stale = stale || blocked;
        let tabs = self.tabs.clone();
        self.tab_focus.retain(|p, _| tabs.contains(p));
        for p in tabs {
            self.tab_focus.entry(p).or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
        div()
            .id("ex-viewer")
            .debug_selector(|| "ex-viewer".into())
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(|this, _: &SaveFile, _, cx| {
                if let Some(p) = this.active.clone() {
                    this.save(&p, cx).detach();
                }
            }))
            .on_action(|_: &NextTab, window, cx| window.focus_next(cx))
            .on_action(|_: &PrevTab, window, cx| window.focus_prev(cx))
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
                            .when_some(active, |d, p| d.child(self.head(&t, a.as_ref(), &p, pixels, cx)))
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
                                        .debug_selector(|| "ex-changed-on-disk".into())
                                        .child(if blocked {
                                            "Changed on disk since it was read: saving would overwrite it."
                                        } else {
                                            "Changed on disk. You have unsaved changes."
                                        })
                                        .child(small_btn("ex-keep-mine", if blocked { "Save mine" } else { "Keep mine" }, &t)
                                            .debug_selector(|| "ex-keep-mine".into())
                                            .tooltip(|_, cx| tooltip_view("Keep your changes; saving writes them over the file on disk", cx))
                                            .on_click(cx.listener(|this, _, _, cx| this.keep_mine(cx))))
                                        .child(small_btn("ex-reload", "Take theirs", &t)
                                            .debug_selector(|| "ex-reload".into())
                                            .tooltip(|_, cx| tooltip_view("Drop your changes and show the file on disk", cx))
                                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))))
                                        .child(small_btn("ex-show-difference", if diffing { "Hide difference" } else { "Show difference" }, &t)
                                            .debug_selector(|| "ex-show-difference".into())
                                            .tooltip(|_, cx| tooltip_view("The file on disk against your changes", cx))
                                            .on_click(cx.listener(|this, _, _, cx| this.toggle_difference(cx)))),
                                )
                            })
                            .when_some(save_error, |d, e| {
                                d.child(
                                    div()
                                        .id("ex-save-error")
                                        .debug_selector(|| "ex-save-error".into())
                                        .flex_none()
                                        .px(px(12.))
                                        .py(px(5.))
                                        .flex()
                                        .items_center()
                                        .gap(px(10.))
                                        .border_b_1()
                                        .border_color(t.line)
                                        .bg(t.red_soft)
                                        .text_size(px(12.))
                                        .text_color(t.red)
                                        .child(format!("Couldn't save: {}", e.lines().next().unwrap_or_default())),
                                )
                            })
                            .child(div().flex_1().min_h_0().flex().flex_col().child(body)),
                    ),
            )
    }
}
