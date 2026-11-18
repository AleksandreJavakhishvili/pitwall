//! The file viewer (a port of `Viewer.tsx`): the whole main area,
//! like Review, or (an agent showed the user files: [`Host`]) a files tile
//! of its own in a Pitwall window's layout, next to the terminal tiles, or
//! a window of its own, each with a smaller bar.
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
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::kit::HoverText;
use gpui::{
    actions, canvas, div, prelude::*, px, AnyElement, App, ClipboardItem, Context, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, MouseButton, Pixels,
    PromptLevel, ScrollHandle, SharedString, Size, Subscription, Task, Window,
};

use pitwall_core::explorer::{ContentKind, FileView, Saved};
use pitwall_proto::AgentView;

use super::code_view::{FileCode, Target};
use crate::code_view::{CodeView, CodeViewEvent};
use crate::kit::file_icons::file_icon;
use super::column;
use super::image_view::{self, Zoom};
use super::logic::{absolute_path, short_path, size_label, split_path};
use super::search::{Blurred, OpenHit, SearchPane};
use super::source::{Res, Source, LARGE_CAP, TEXT_CAP};
use super::tree::{now_ms, TreeModel};
use super::tree_view::{OpenFile, TreeView};
use super::widgets::{refresh_control, status_mark};
use crate::kit::{icon, icon_btn_state, kbd, keys, small_btn, tool_btn, tooltip_view, MONO_FONT};
use crate::agents::AgentStore;
use crate::markdown_view::{model::is_markdown, MarkdownView, MAX_PREVIEW_BYTES};
use crate::theme::{self, Theme, RADIUS, RADIUS_SM};

actions!(explorer_viewer, [Back, NextTab, PrevTab, PickTab, SaveFile, ToggleTree]);

/// Hide or show the file tree (⌘⇧E, as VS Code's Explorer; ⌘B is
/// Pitwall's own sidebar). On Windows and Linux Ctrl+Shift+Alt+E, as the
/// app's other ⌘⇧ shortcuts.
const TOGGLE_TREE_KEY: &str = if cfg!(target_os = "macos") { "cmd-shift-e" } else { "ctrl-shift-alt-e" };

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
        KeyBinding::new(TOGGLE_TREE_KEY, ToggleTree, Some(CONTEXT)),
    ]
}

pub enum ViewerEvent {
    Exit,
    QuickOpen,
    ShowDiff(String),
    /// A file was opened (quick open lists recent files first).
    Opened(String),
    SetIgnored(bool),
    /// A files tile: "Close".
    Close,
    /// A files tile: "Open in window".
    Undock,
    /// "Dock right of the terminal" (the layout slot on its right), or a
    /// layout place picked in the "Dock…" / "Move…"
    /// menu: the files go there (moved, not closed: edits stay).
    Place(super::dock::FilesPlace),
    /// The grip pressed: the files follow the pointer until it is let go
    /// (into the layout of the Pitwall window under it, or out of it).
    Grip,
    /// In a window of its own: "Keep on top" toggled.
    Pin,
}

/// Where a viewer is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// The main area (the Files tab's "Open", ⌘P, ⌘-click): "Back" leaves.
    Main,
    /// A files tile in a Pitwall window's layout (a peer of the terminal
    /// tiles: `main_screen::tree`, `super::dock`).
    Tile,
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
    /// An image's zoom.
    zoom: Zoom,
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
    /// The agent it was opened for (its files tile, or its window):
    /// its name in the bar.
    owner: Option<String>,
    /// In a window: kept on top of other windows.
    pinned: bool,
    /// In a window: a Pitwall window shows the agent ("Dock right of the
    /// terminal" can put it in the layout slot there).
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
    /// The search field let the keyboard go: it comes back here at the
    /// next draw (in whichever window the viewer is in now).
    refocus: bool,
    /// The "Dock…" / "Move…" menu, open at this point of the viewer.
    place_menu: Option<gpui::Point<gpui::Pixels>>,
    /// The tree column as toggled in this viewer (`None`: as kept for its
    /// kind of place, see [`column::shown`]).
    tree_open: Option<bool>,
    /// The column's width while its edge is dragged.
    tree_drag_w: Option<f32>,
    /// The viewer's size and the image pane's, as last laid out.
    measured: Rc<Cell<Option<Size<Pixels>>>>,
    image_area: Rc<Cell<Option<Size<Pixels>>>>,
    /// The tab strip: scrolled to the active tab; more tabs than fit
    /// (`tabs_overflow`) offer a list of them all (`tabs_menu`).
    tabs_scroll: ScrollHandle,
    tabs_overflow: Rc<Cell<bool>>,
    tabs_menu: bool,
    /// The tab last scrolled into view.
    tabs_scrolled_to: Option<String>,
    _subs: Vec<Subscription>,
}

/// The tree column's id (its edge's drags say it).
const SIDE: &str = "ex-side";

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
        let _ = window;
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
            // (Not bound to a window: the viewer moves between windows.)
            cx.subscribe(&search, |this, _, _: &Blurred, cx| {
                this.refocus = true;
                cx.notify();
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
            refocus: false,
            place_menu: None,
            tree_open: None,
            tree_drag_w: None,
            measured: Rc::default(),
            image_area: Rc::default(),
            tabs_scroll: ScrollHandle::new(),
            tabs_overflow: Rc::default(),
            tabs_menu: false,
            tabs_scrolled_to: None,
            _subs: subs,
        }
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Read `key` from now on: the same folder by another key (its agent
    /// was removed and its files stay for unsaved edits). Tabs, edits and
    /// the tree's place stay; saves go to the new key.
    pub fn reroot(&mut self, key: String, cx: &mut Context<Self>) {
        if self.agent_id == key {
            return;
        }
        self.tree.update(cx, |t, cx| t.set_agent(key.clone(), cx));
        self.search.update(cx, |s, cx| s.set_key(key.clone(), cx));
        self.agent_id = key;
        cx.notify();
    }

    /// The tree it shows.
    pub fn tree(&self) -> Entity<TreeModel> {
        self.tree.clone()
    }

    pub fn host(&self) -> Host {
        self.host
    }

    /// Drawn in a files tile or a window, for agent `owner`.
    pub fn set_host(&mut self, host: Host, owner: Option<String>, cx: &mut Context<Self>) {
        if self.host != host {
            // Another kind of place: its own kept column.
            self.tree_open = None;
        }
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

    // ── the tree column ─────────────────────────────────────────────────

    /// What `ui.json` keeps of the column for this viewer's kind of place.
    fn kept_column(&self, cx: &App) -> column::Column {
        cx.try_global::<crate::ui_state::UiHandle>()
            .map(|h| h.0.read(cx).state.files_tree.get(self.host))
            .unwrap_or_default()
    }

    /// The viewer's width, as last laid out.
    fn viewer_w(&self) -> Option<f32> {
        self.measured.get().map(|s| f32::from(s.width))
    }

    /// The file tree column is shown.
    pub fn tree_shown(&self, cx: &App) -> bool {
        column::shown(self.tree_open, self.kept_column(cx), self.host, self.viewer_w())
    }

    /// The column's width now.
    pub fn tree_width(&self, cx: &App) -> f32 {
        match self.tree_drag_w {
            Some(w) => column::clamp_width(w, self.viewer_w()),
            None => column::width(self.kept_column(cx), self.host, self.viewer_w()),
        }
    }

    /// Hide or show the tree (and keep that for this kind of place). The
    /// keyboard, when it was in the tree, goes to the viewer.
    pub fn toggle_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let on = !self.tree_shown(cx);
        self.set_tree(on, window, cx);
    }

    pub fn set_tree(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !on && !self.focus.is_focused(window) && self.focus.contains_focused(window, cx) {
            let in_file = self.active_code().is_some_and(|c| c.read(cx).focus_handle(cx).contains_focused(window, cx));
            if !in_file {
                window.focus(&self.focus, cx);
            }
        }
        self.tree_open = Some(on);
        let host = self.host;
        crate::ui_state::update(cx, |s| s.files_tree.get_mut(host).open = Some(on));
        cx.notify();
    }

    /// The column dragged to `w` (kept on release).
    fn drag_tree(&mut self, w: f32, cx: &mut Context<Self>) {
        self.tree_drag_w = Some(column::clamp_width(w, self.viewer_w()));
        cx.notify();
    }

    fn keep_tree_width(&mut self, cx: &mut Context<Self>) {
        if let Some(w) = self.tree_drag_w.take() {
            let (host, w) = (self.host, column::clamp_width(w, self.viewer_w()));
            crate::ui_state::update(cx, |s| s.files_tree.get_mut(host).width = Some(w));
            cx.notify();
        }
    }

    /// Debug builds: the column's edge dragged to `w` and let go.
    pub fn debug_tree_width(&mut self, w: f32, cx: &mut Context<Self>) {
        self.drag_tree(w, cx);
        self.keep_tree_width(cx);
    }

    /// Debug builds: an image's zoom (`fit`, `100`, `in`, `out`).
    pub fn debug_zoom(&mut self, what: &str, cx: &mut Context<Self>) {
        let now = self.active.as_ref().and_then(|p| self.docs.get(p)).map(|d| d.zoom).unwrap_or_default();
        let s = match now {
            Zoom::Fit => 1.,
            Zoom::Scale(s) => s,
        };
        let z = match what {
            "fit" => Zoom::Fit,
            "100" => Zoom::Scale(1.),
            "in" => Zoom::Scale(image_view::zoom_in(s)),
            "out" => Zoom::Scale(image_view::zoom_out(s)),
            _ => return,
        };
        self.set_zoom(z, cx);
    }

    /// A double click on the column's edge: its default width again.
    pub(crate) fn reset_tree_width(&mut self, cx: &mut Context<Self>) {
        self.tree_drag_w = None;
        let host = self.host;
        crate::ui_state::update(cx, |s| s.files_tree.get_mut(host).width = None);
        cx.notify();
    }

    /// Laid out at `size`: a change of width can show or hide the tree (a
    /// narrow pane) and re-clamps its width.
    fn measure(&self, cell: Rc<Cell<Option<Size<Pixels>>>>, cx: &mut Context<Self>) -> impl IntoElement {
        crate::kit::side::measure(cell, cx.entity().downgrade())
    }

    /// Shown (again): focus it, or the search field.
    pub fn show(&mut self, pane: Option<Pane>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = pane {
            self.pane = p;
            // Asked for Explorer or Search: the column shows (in this
            // viewer only; what is kept doesn't change).
            self.tree_open = Some(true);
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
        // A folder was asked for: the tree shows it, even in a narrow pane.
        self.tree_open = Some(true);
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
        self.tree_open = Some(true);
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
            "active={active:?} tree={} tree_w={} viewer_w={:?} zoom={:?} unsaved={:?} stale={} blocked={} diff={} save_error={:?} editing={} read_only={:?} text={:?}",
            self.tree_shown(cx),
            self.tree_width(cx),
            self.viewer_w(),
            d.map(|d| d.zoom),
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

    /// The grip: pressed, the files follow the pointer ([`ViewerEvent::Grip`]).
    fn grip(&self, t: &Theme, tip: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        crate::kit::icon_btn("ex-grip", "grip", tip, true, t)
            .debug_selector(|| "ex-grip".into())
            .cursor_grab()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(ViewerEvent::Grip);
                }),
            )
    }

    /// "Dock…" / "Move…": the places menu ([`super::dock::menu`]).
    fn place_menu_btn(&self, t: &Theme, id: &'static str, label: &'static str, tip: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        tool_btn(id, "grid", label, tip, self.place_menu.is_some(), t)
            .debug_selector(move || id.into())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, e: &gpui::ClickEvent, _, cx| {
                this.place_menu = if this.place_menu.is_some() { None } else { Some(e.position()) };
                cx.notify();
            }))
    }

    fn place_menu_overlay(&self, at: gpui::Point<gpui::Pixels>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        let pick = move |p: Option<super::dock::FilesPlace>, cx: &mut App| {
            let _ = me.update(cx, |this, cx| {
                this.place_menu = None;
                if let Some(p) = p {
                    cx.emit(ViewerEvent::Place(p));
                }
                cx.notify();
            });
        };
        super::dock::menu(at, self.host == Host::Tile, self.owner.clone(), t, pick)
    }

    /// The places menu is open (tests).
    #[cfg(test)]
    pub(crate) fn place_menu_open(&self) -> bool {
        self.place_menu.is_some()
    }

    /// The bar over the viewer, the same in every place it is drawn: the
    /// tree's toggle; what is shown (the agent, its folder shortened from
    /// the middle, its branch); room for a place's own controls
    /// (`ex-bar-slot`); refresh; and the place's way out (Back; Open in
    /// window and Close in a files tile; Dock and Pin in a window).
    fn bar(&self, t: &Theme, a: Option<&AgentView>, cx: &mut Context<Self>) -> AnyElement {
        let host = self.host;
        let tree = self.tree.read(cx).state();
        let (refreshing, updated) = (tree.refreshing, tree.updated_at);
        let shown = self.tree_shown(cx);
        let wide = self.viewer_w().is_none_or(|w| w >= 640.);
        let root_full: SharedString = self.root(cx).unwrap_or_default().into();
        let root = short_path(&self.root_display(cx), if wide { 48 } else { 26 });
        // The agent's name, except in its own files tile (whose terminal's header
        // says it).
        let named = a.filter(|a| host != Host::Tile || self.owner.as_deref() != Some(a.name.as_str()));
        let branch = a.and_then(|a| a.branch.clone()).filter(|_| super::source::root_of_key(&self.agent_id).is_none());
        let can_dock = self.can_dock;
        let sep = || div().flex_none().w(px(1.)).h(px(16.)).mx(px(2.)).bg(t.line);
        let stop = |d: gpui::Stateful<gpui::Div>| d.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        div()
            .id("ex-viewer-bar")
            .debug_selector(|| "ex-viewer-bar".into())
            .h(px(36.))
            .flex_none()
            .pl(px(6.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(t.line)
            .bg(t.surface)
            .child(
                crate::kit::side::toggle("ex-tree-toggle", shown, "files", "⌘⇧E", t)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_tree(window, cx))),
            )
            // What is shown: gives way first when the bar is short.
            .child(
                div()
                    .id("ex-viewer-what")
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .pl(px(2.))
                    .when(host != Host::Window, |d| d.child(crate::kit::label("Files", t.text_3, 12.)))
                    .when_some(named, |d, a| {
                        d.child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(6.))
                                .child(crate::kit::glyph(a.status, t, true))
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(crate::kit::WEIGHT_550)
                                        .text_color(t.text)
                                        .child(crate::kit::one_line(a.name.clone())),
                                ),
                        )
                    })
                    .child(
                        div()
                            .id("ex-viewer-hint")
                            .debug_selector(|| "ex-viewer-hint".into())
                            .min_w_0()
                            .flex_shrink(1.)
                            .ellipsis()
                            .font_family(MONO_FONT)
                            .text_size(px(11.5))
                            .text_color(t.text_3)
                            .tooltip(move |_, cx| tooltip_view(root_full.clone(), cx))
                            .child(crate::kit::one_line(root)),
                    )
                    .when_some(branch, |d, b| {
                        d.child(
                            div()
                                .id("ex-viewer-branch")
                            .debug_selector(|| "ex-viewer-branch".into())
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .min_w_0()
                                .font_family(MONO_FONT)
                                .text_size(px(11.5))
                                .text_color(t.text_3)
                                .tooltip(tooltip_text(format!("On branch {b}")))
                                .child(icon("branch", 12., t.text_4))
                                .child(div().min_w_0().ellipsis().child(crate::kit::one_line(b))),
                        )
                    }),
            )
            // A place's own controls (dragging the viewer elsewhere, a
            // dock menu) go here.
            .child(
                div()
                    .id("ex-bar-slot")
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .map(|d| match host {
                        Host::Tile => d
                            .child(self.grip(t, "Drag to move it in the layout, or out of the window to open it in a window", cx))
                            .child(self.place_menu_btn(t, "ex-move", "Move…", "Move it in the layout, or to a window", cx)),
                        Host::Window if can_dock => d
                            .child(self.grip(t, "Drag into a Pitwall window to place it in the layout", cx))
                            .child(self.place_menu_btn(t, "ex-dock-menu", "Dock…", "Place it in the layout of the window that shows the agent", cx)),
                        _ => d,
                    }),
            )
            .child(refresh_control(
                t,
                "ex-viewer-refresh",
                refreshing,
                // A files tile's bar is short: only ↻.
                updated.filter(|_| host != Host::Tile && wide),
                now_ms(),
                cx.listener(|this, _, _, cx| {
                    this.tree.update(cx, |t, cx| t.refresh(false, cx));
                    this.recheck(cx);
                }),
            ))
            .map(|d| match host {
                Host::Main => d
                    .child(
                        small_btn("ex-viewer-goto", "Go to file", t)
                            .debug_selector(|| "ex-viewer-goto".into())
                            .child(kbd(&keys("⌘P", false), t))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ViewerEvent::QuickOpen)))
                            .tooltip(|_, cx| tooltip_view(keys("Go to file (⌘P)", false), cx)),
                    )
                    .child(
                        small_btn("ex-viewer-back", "Back", t)
                            .child(kbd("esc", t))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ViewerEvent::Exit))),
                    ),
                Host::Tile => d
                    .child(sep())
                    .child(stop(
                        crate::kit::icon_btn("ex-undock", "window", "Open in a window of its own", true, t)
                            .debug_selector(|| "ex-undock".into())
                            // (Moving keeps unsaved edits: nothing to ask.)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ViewerEvent::Undock))),
                    ))
                    .child(stop(
                        crate::kit::icon_btn("ex-close-tile", "x", "Close (Esc gives the keyboard back to the terminal)", true, t)
                            .debug_selector(|| "ex-close-tile".into())
                            .on_click(cx.listener(|this, _, window, cx| this.guard(None, window, cx, |_, _, cx| cx.emit(ViewerEvent::Close)))),
                    )),
                Host::Window => d
                    .child(sep())
                    .child(stop(
                        tool_btn(
                            "ex-dock",
                            "page-right",
                            "Dock",
                            if can_dock {
                                "Dock right of the agent's terminal (a slot in the layout)"
                            } else {
                                "Dock right of the agent's terminal (no Pitwall window shows this agent)"
                            },
                            false,
                            t,
                        )
                        .debug_selector(|| "ex-dock".into())
                        .when(!can_dock, |d| d.opacity(0.45))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            if can_dock {
                                cx.emit(ViewerEvent::Place(super::dock::FilesPlace::Next(crate::main_screen::tree::Side::Right)))
                            }
                        })),
                    ))
                    .when(crate::platform::can_keep_on_top(), |d| {
                        let pinned = self.pinned;
                        d.child(stop(
                            tool_btn(
                                "ex-pin",
                                "pin",
                                if pinned { "On top" } else { "Pin" },
                                if pinned { "Kept on top of other windows: click to let it go behind them" } else { "Keep on top of other windows" },
                                pinned,
                                t,
                            )
                            .debug_selector(|| "ex-pin".into())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ViewerEvent::Pin))),
                        ))
                    }),
            })
            .into_any_element()
    }

    /// The tree column: Explorer / Search and the "Ignored" switch over the
    /// tree or the search; its right edge drags its width.
    fn side(&self, t: &Theme, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let ignored = self.tree.read(cx).ignored();
        let seg = |id: &'static str, text: &'static str, pane: Pane, first: bool, cx: &mut Context<Self>| {
            crate::kit::segment(id, text.into(), self.pane == pane, first, t)
                .when(self.pane != pane, |d| d.hover_text(t.text_2, move |s| s))
                .on_click(cx.listener(move |this, _, window, cx| this.show(Some(pane), window, cx)))
        };
        div()
            .id(SIDE)
            .debug_selector(|| SIDE.into())
            .relative()
            .w(px(width))
            .h_full()
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
                    .h(px(36.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .pl(px(8.))
                    .pr(px(4.))
                    .child(
                        crate::kit::segments(t)
                            .child(seg("ex-seg-files", "Explorer", Pane::Files, true, cx))
                            .child(seg("ex-seg-search", "Search", Pane::Search, false, cx).tooltip(|_, cx| tooltip_view(keys("Search in files (⌘⇧F)", false), cx))),
                    )
                    .child(div().flex_1())
                    .when(self.pane == Pane::Files, |d| {
                        d.child(
                            icon_btn_state(
                                "ex-viewer-ignored",
                                "eye",
                                if ignored { "Hide what .gitignore leaves out" } else { "Also list what .gitignore leaves out (dimmed)" },
                                true,
                                13.,
                                Some(ignored),
                                t,
                            )
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(ViewerEvent::SetIgnored(!ignored)))),
                        )
                    }),
            )
            .child(match self.pane {
                Pane::Files => self.tree_view.clone().into_any_element(),
                Pane::Search => self.search.clone().into_any_element(),
            })
            // The edge: drag for the width, double-click for the default.
            .child({
                let me = cx.entity().downgrade();
                crate::kit::side::edge(SIDE, self.tree_drag_w.is_some(), t, move |_, cx| {
                    let _ = me.update(cx, |v, cx| v.reset_tree_width(cx));
                })
            })
    }

    /// More tabs than fit: a button listing them all.
    fn tabs_more(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let open = self.tabs_menu;
        let me = cx.entity().downgrade();
        let close: crate::kit::OnClose = Rc::new(move |_, cx| {
            let _ = me.update(cx, |v, cx| {
                v.tabs_menu = false;
                cx.notify();
            });
        });
        // The list, as values: it may be drawn in a glass pop-up, whose
        // window must not touch this viewer while it draws.
        let rows: Rc<Vec<TabRow>> = Rc::new(
            self.tabs
                .iter()
                .map(|p| TabRow {
                    path: p.clone(),
                    on: self.active.as_deref() == Some(p.as_str()),
                    dirty: self.docs.get(p).is_some_and(|d| d.dirty(cx)),
                })
                .collect(),
        );
        let me = cx.entity().downgrade();
        let place = div().absolute().right_0().top(px(32.));
        let menu = if !open {
            None
        } else if crate::kit::pop::hosted(cx) {
            let build: crate::kit::pop::Build = Rc::new(move |_: &mut Window, cx: &mut App| {
                crate::kit::menu_body().children(tab_rows(&rows, &crate::kit::pop::theme(cx), &me)).into_any_element()
            });
            let pop = crate::kit::pop::Pop { at: None, radius: crate::kit::pop::MENU_RADIUS, focus: None, build, on_dismiss: close };
            Some(place.child(crate::kit::pop::anchor("ex-tabs-pop", pop)).into_any_element())
        } else {
            let ft = t.float();
            let rows = tab_rows(&rows, &ft, &me);
            let menu = crate::kit::menu_panel(&ft)
                .id("ex-tabs-menu")
                .max_h(px(360.))
                .overflow_y_scroll()
                .occlude()
                .border_1()
                .border_color(ft.line_strong)
                .children(rows)
                .on_mouse_down_out(move |_, window, cx| close(window, cx));
            Some(place.child(gpui::deferred(gpui::anchored().snap_to_window().child(menu))).into_any_element())
        };
        div()
            .relative()
            .flex_none()
            .h_full()
            .px(px(4.))
            .flex()
            .items_center()
            .border_l_1()
            .border_b_1()
            .border_color(t.line)
            .child(
                icon_btn_state("ex-tabs-more", "chevron-down", format!("All {} open files", self.tabs.len()), true, 13., Some(open), t)
                    .debug_selector(|| "ex-tabs-more".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tabs_menu = !this.tabs_menu;
                        cx.notify();
                    })),
            )
            .children(menu)
            .into_any_element()
    }

    fn tabs_row(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs: Vec<AnyElement> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let selected = self.active.as_deref() == Some(p.as_str());
                let (p1, p2, p3) = (p.clone(), p.clone(), p.clone());
                let title: SharedString = p.clone().into();
                let focus = self.tab_focus.get(p).cloned().unwrap_or_else(|| cx.focus_handle());
                let ring = focus.is_focused(window);
                let p4 = p.clone();
                let (close_hover, close_text, hover_bg) = (t.surface_3, t.text, t.surface_2);
                let dirty = self.docs.get(p).is_some_and(|d| d.dirty(cx));
                let close_group = SharedString::from(format!("ex-tab-x-{i}"));
                div()
                    .id(("ex-tab", i))
                    .group("ex-tab")
                    .relative()
                    .h(px(32.))
                    .min_w(px(72.))
                    .max_w(px(220.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(10.))
                    .pr(px(5.))
                    .border_r_1()
                    .border_color(t.line)
                    .font_family(MONO_FONT)
                    .text_size(px(12.))
                    .cursor_pointer()
                    .text_color(if selected { t.text } else { t.text_3 })
                    // The open tab is the file's own surface: no line under
                    // it, a mark over it.
                    .map(|d| {
                        if selected {
                            d.bg(t.term_bg).child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(t.text_2))
                        } else {
                            d.border_b_1().hover_text(t.text_2, move |s| s.bg(hover_bg))
                        }
                    })
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
                    .child(div().min_w_0().flex_1().ellipsis().child(crate::kit::one_line(split_path(p).1.to_string())))
                    // Close: shown on the open tab and on hover; a file
                    // with unsaved changes shows a dot there instead, which
                    // turns into the ✕ under the pointer.
                    .child(
                        div()
                            .id(("ex-tab-close", i))
                            .group(close_group.clone())
                            .relative()
                            .size(px(18.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(RADIUS_SM)
                            .when(!selected && !dirty, |d| d.invisible().group_hover_probed("ex-tab", |s| s.visible()))
                            .hover_text(close_text, move |s| s.bg(close_hover))
                            .tooltip(move |_, cx| tooltip_view(if dirty { "Unsaved changes · Close" } else { "Close" }, cx))
                            .when(dirty, |d| d.debug_selector(move || format!("ex-tab-dirty-{i}")))
                            .map(|d| {
                                if dirty {
                                    d.child(
                                        div()
                                            .absolute()
                                            .inset_0()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .group_hover_probed(close_group.clone(), |s| s.invisible())
                                            .child(div().size(px(7.)).rounded_full().bg(t.text_2)),
                                    )
                                    .child(icon("x", 11., t.text_3).invisible().group_hover_probed(close_group.clone(), |s| s.visible()))
                                } else {
                                    d.child(icon("x", 11., t.text_3))
                                }
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.request_close_tab(&p3, window, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        // The rest of the strip's line.
        tabs.push(div().flex_1().min_w(px(8.)).h(px(32.)).border_b_1().border_color(t.line).into_any_element());
        let more = self.tabs_overflow.get() || self.tabs_menu;
        let (flag, scroll, me) = (self.tabs_overflow.clone(), self.tabs_scroll.clone(), cx.entity().downgrade());
        div()
            .relative()
            .h(px(32.))
            .flex_none()
            .flex()
            .bg(t.surface)
            .child(
                div()
                    .id("ex-tabs")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .overflow_x_scroll()
                    .track_scroll(&self.tabs_scroll)
                    .children(tabs),
            )
            // After the strip is laid out: does it scroll? (Then the list
            // button shows, on the next frame.)
            .child(
                canvas(
                    move |_, _, cx| {
                        let over = scroll.max_offset().x > px(0.5);
                        if flag.get() != over {
                            flag.set(over);
                            let me = me.clone();
                            cx.defer(move |cx| {
                                let _ = me.update(cx, |_, cx| cx.notify());
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_0(),
            )
            .when(more, |d| d.child(self.tabs_more(t, cx)))
    }

    /// A strip under the file's header that asks or tells something about
    /// it: an icon in the strip's tone, the words, and what to do. (Also
    /// over a removed agent's kept files: `super::dock::orphan_bar`.)
    pub(crate) fn notice(id: &'static str, warn: bool, text: SharedString, t: &Theme) -> gpui::Stateful<gpui::Div> {
        let (fg, bg) = if warn { (t.amber, t.amber_soft) } else { (t.red, t.red_soft) };
        div()
            .id(id)
            .debug_selector(move || id.into())
            .flex_none()
            .min_h(px(34.))
            .px(px(12.))
            .py(px(4.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(t.line)
            .bg(bg)
            .text_size(px(12.))
            .text_color(t.text)
            .child(icon("alert", 14., fg))
            .child(div().flex_1().min_w(px(160.)).child(crate::kit::para(text)))
    }

    fn head(&self, t: &Theme, a: Option<&AgentView>, active: &str, pixels: Option<(i32, i32)>, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.tree.read(cx).state().find_entry(active).and_then(|e| e.status);
        let (dir, base) = split_path(active);
        let doc = self.docs.get(active);
        let view = doc.and_then(|d| d.view.as_ref());
        let size = view.map(|v| size_label(v.size));
        let is_text = view.is_some_and(|v| v.kind == ContentKind::Text);
        let cwd = self.root(cx).unwrap_or_default();
        let abs: SharedString = absolute_path(&cwd, active).into();
        let path_tip: SharedString = active.to_string().into();
        // Read-only, and why: only where editing could be expected (an
        // image or a binary file says what it is already).
        let read_only = view.filter(|_| is_text).and_then(|_| self.edit_block(active, cx));
        let saving = doc.is_some_and(|d| d.saving);
        let dirty = doc.is_some_and(|d| d.dirty(cx));
        // (Review is a Pitwall window's: not from a window of its own.)
        // (A folder of its own: Review on its worktree, when it lists it.)
        let can_diff = status.is_some()
            && (a.is_some_and(|a| a.caps.review) || super::source::root_of_key(&self.agent_id).is_some())
            && self.host != Host::Window;
        let diff_path = active.to_string();
        // The file's side of the viewer is short (a files tile).
        let tree_w = if self.tree_shown(cx) { self.tree_width(cx) } else { 0. };
        let narrow = self.viewer_w().is_some_and(|w| w - tree_w < 520.);
        let meta = |s: String| div().flex_none().font_family(MONO_FONT).text_size(px(11.)).text_color(t.text_3).child(s);
        let zoom = (pixels.is_some() && doc.is_some_and(|d| d.image.is_some())).then(|| doc.map(|d| d.zoom).unwrap_or_default());
        div()
            .flex_none()
            .h(px(36.))
            .px(px(12.))
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
                    .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(base.to_string()))
                    .child(div().min_w_0().text_color(t.text_3).ellipsis().child(crate::kit::one_line(dir.trim_end_matches('/').to_string()))),
            )
            .when_some(status, |d, s| d.child(status_mark(t, "ex-head-status", s)))
            .when_some(pixels.filter(|_| !narrow), |d, (w, h)| d.child(meta(format!("{w} × {h} px")).debug_selector(|| "ex-image-pixels".into())))
            .when_some(size.filter(|_| !narrow), |d, s| d.child(meta(s)))
            .when(dirty && !saving, |d| d.child(meta("edited".into())))
            .when(saving, |d| d.child(meta("saving…".into())))
            .when_some(read_only, |d, why| {
                d.child(
                    div()
                        .id("ex-read-only")
                        .debug_selector(|| "ex-read-only".into())
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .h(px(20.))
                        .px(px(6.))
                        .rounded(RADIUS_SM)
                        .bg(t.surface_2)
                        .text_size(px(11.))
                        .text_color(t.text_3)
                        .tooltip(tooltip_text(format!("Read-only: {why}")))
                        .child(icon("lock", 11., t.text_3))
                        .child(if narrow { "Read-only".to_string() } else { format!("Read-only · {why}") }),
                )
            })
            .child(div().flex_1())
            .when_some(zoom, |d, zoom| d.child(self.zoom_controls(zoom, pixels, t, cx)))
            .when(self.md_text().is_some(), |d| {
                let on = self.preview;
                d.child(
                    crate::kit::segments(t)
                        .debug_selector(|| "ex-md-switch".into())
                        .child(crate::kit::segment("ex-md-code", "Code".into(), !on, true, t).on_click(cx.listener(|v, _, _, cx| v.set_preview(false, cx))))
                        .child(crate::kit::segment("ex-md-preview", "Preview".into(), on, false, t).on_click(cx.listener(|v, _, _, cx| v.set_preview(true, cx)))),
                )
            })
            .child(
                tool_btn("ex-copy-path", "copy", if self.copied { "Copied" } else if narrow { "" } else { "Copy path" }, String::new(), false, t)
                    .on_click(cx.listener(move |this, _, _, cx| this.copy_path(&cwd, cx)))
                    .tooltip(move |_, cx| tooltip_view(format!("Copy {abs}"), cx)),
            )
            .when(can_diff, |d| {
                d.child(
                    tool_btn("ex-show-diff", "review", if narrow { "" } else { "Show diff" }, String::new(), false, t)
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(ViewerEvent::ShowDiff(diff_path.clone()))))
                        .tooltip(|_, cx| tooltip_view("This file's changes, in Review", cx)),
                )
            })
    }

    /// An image's Fit / 100 % and − / + (and the zoom it is at).
    fn zoom_controls(&self, zoom: Zoom, pixels: Option<(i32, i32)>, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let now = self.image_scale(zoom, pixels);
        let fit = zoom == Zoom::Fit;
        let actual = !fit && (now - 1.).abs() < 1e-3;
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .debug_selector(|| "ex-image-zoom".into())
            .child(
                crate::kit::icon_btn("ex-zoom-out", "minus", "Zoom out", true, t)
                    .on_click(cx.listener(move |v, _, _, cx| v.set_zoom(Zoom::Scale(image_view::zoom_out(now)), cx))),
            )
            .child(
                div()
                    .w(px(40.))
                    .flex()
                    .justify_center()
                    .font_family(MONO_FONT)
                    .text_size(px(11.))
                    .text_color(t.text_2)
                    .debug_selector(|| "ex-zoom-level".into())
                    .child(image_view::percent(now)),
            )
            .child(
                crate::kit::icon_btn("ex-zoom-in", "plus", "Zoom in", true, t)
                    .on_click(cx.listener(move |v, _, _, cx| v.set_zoom(Zoom::Scale(image_view::zoom_in(now)), cx))),
            )
            .child(
                crate::kit::segments(t)
                    .child(crate::kit::segment("ex-zoom-fit", "Fit".into(), fit, true, t).on_click(cx.listener(|v, _, _, cx| v.set_zoom(Zoom::Fit, cx))))
                    .child(crate::kit::segment("ex-zoom-100", "100%".into(), actual, false, t).on_click(cx.listener(|v, _, _, cx| v.set_zoom(Zoom::Scale(1.), cx)))),
            )
    }

    /// The scale the active image is drawn at.
    fn image_scale(&self, zoom: Zoom, pixels: Option<(i32, i32)>) -> f32 {
        let (Some((w, h)), Some(area)) = (pixels, self.image_area.get()) else {
            return match zoom {
                Zoom::Fit => 1.,
                Zoom::Scale(s) => s,
            };
        };
        image_view::scale(zoom, (w as f32, h as f32), (f32::from(area.width), f32::from(area.height)))
    }

    pub fn set_zoom(&mut self, zoom: Zoom, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else { return };
        if let Some(d) = self.docs.get_mut(&active) {
            d.zoom = zoom;
        }
        cx.notify();
    }

    /// No file open: what can be done from here.
    fn empty(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.tree_shown(cx);
        let row = |id: &'static str, label: &'static str, key: Option<&str>, t: &Theme| {
            let hover = t.surface_3;
            div()
                .id(id)
                .w(px(240.))
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(RADIUS)
                .cursor_pointer()
                .text_size(px(12.5))
                .text_color(t.text_2)
                .hover_text(t.text, move |s| s.bg(hover))
                // (After the hover, which tests mark with their own.)
                .debug_selector(move || id.into())
                .child(div().flex_1().child(label))
                .when_some(key, |d, k| d.child(kbd(k, t)))
        };
        div()
            .id("ex-empty")
            .debug_selector(|| "ex-empty".into())
            .flex_1()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(2.))
            .child(icon("file", 28., t.text_4).mb(px(10.)))
            .child(div().text_size(px(13.)).text_color(t.text_2).mb(px(2.)).child("No file open"))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .mb(px(12.))
                    .child(if shown { "Pick one in the tree, or:" } else { "Find one:" }),
            )
            .child(row("ex-empty-goto", "Go to file", Some("⌘P"), t).on_click(cx.listener(|_, _, _, cx| cx.emit(ViewerEvent::QuickOpen))))
            .child(
                row("ex-empty-search", "Search in files", Some("⌘⇧F"), t)
                    .on_click(cx.listener(|this, _, window, cx| this.show(Some(Pane::Search), window, cx))),
            )
            .when(!shown, |d| {
                d.child(row("ex-empty-tree", "Show the file tree", Some("⌘⇧E"), t).on_click(cx.listener(|this, _, window, cx| this.set_tree(true, window, cx))))
            })
            .into_any_element()
    }

    fn body(&self, t: &Theme, pixels: Option<(i32, i32)>, cx: &mut Context<Self>) -> AnyElement {
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
            return self.empty(t, cx);
        };
        let doc = self.docs.get(&active);
        if let Some(e) = doc.and_then(|d| d.error.clone()) {
            let path = active.clone();
            return center(t)
                .child(icon("alert", 20., t.red))
                .child(
                    div()
                        .max_w(px(420.))
                        .text_color(t.text_2)
                        .child(crate::kit::para(format!("Couldn't read {}: {}", split_path(&active).1, e.lines().next().unwrap_or_default()))),
                )
                .child(small_btn("ex-retry", "Retry", t).on_click(cx.listener(move |this, _, _, cx| this.read(&path, false, false, cx))))
                .into_any_element();
        }
        let Some(v) = doc.and_then(|d| d.view.clone()) else {
            return center(t).child("Reading…").into_any_element();
        };
        let here = doc.and_then(|d| d.local.clone());
        match v.kind {
            ContentKind::Binary => {
                let what = if image_format(&active).is_some() {
                    format!("An image of {}, larger than {}: not shown here", size_label(v.size), size_label(pitwall_core::explorer::MAX_IMAGE))
                } else {
                    format!("A binary file of {}: not shown here", size_label(v.size))
                };
                center(t)
                    .child(file_icon(&active, 28.))
                    .child(div().text_color(t.text_2).child(what))
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
                Some(image) => self.image(image, doc.map(|d| d.zoom).unwrap_or_default(), pixels, t, cx),
                None => match doc.and_then(|d| d.image_error.clone()) {
                    Some(e) => center(t).child(format!("Couldn't read the image: {e}")).into_any_element(),
                    None => center(t).child("Reading…").into_any_element(),
                },
            },
            ContentKind::TooLarge => {
                let loading = doc.is_some_and(|d| d.loading);
                let path = active.clone();
                center(t)
                    .child(file_icon(&active, 28.))
                    .child(div().text_color(t.text_2).child(format!("A large file of {}: not opened yet", size_label(v.size))))
                    .child(if v.size <= LARGE_CAP {
                        small_btn("ex-load-anyway", if loading { "Reading…" } else { "Load anyway" }, t)
                            .on_click(cx.listener(move |this, _, _, cx| this.load_anyway(&path, cx)))
                            .into_any_element()
                    } else {
                        div().text_size(px(11.5)).text_color(t.text_4).child(format!("Files over {} aren't shown.", size_label(LARGE_CAP))).into_any_element()
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

    /// The image at its zoom, centred while smaller than the pane and
    /// scrolled when larger, over a checkerboard where it is transparent.
    /// A double click goes between Fit and 100 %.
    fn image(&self, image: Arc<gpui::Image>, zoom: Zoom, pixels: Option<(i32, i32)>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let err = t.text_3;
        let picture = || {
            gpui::img(gpui::ImageSource::Image(image.clone()))
                .size_full()
                .object_fit(gpui::ObjectFit::Fill)
                .with_fallback(move || div().text_size(px(12.5)).text_color(err).child("This image can't be drawn").into_any_element())
        };
        let measured = self.image_area.get().is_some();
        let inner = match pixels.filter(|_| measured) {
            Some((w, h)) => {
                let s = self.image_scale(zoom, pixels);
                let (dw, dh) = ((w as f32 * s).max(1.), (h as f32 * s).max(1.));
                div()
                    .relative()
                    .flex_none()
                    .w(px(dw))
                    .h(px(dh))
                    .border_1()
                    .border_color(t.line)
                    .child(image_view::checkerboard(t.surface, t.surface_3).absolute().inset_0())
                    .child(picture())
                    .into_any_element()
            }
            // Not laid out yet (a frame): fitted by the image itself.
            None => div().size_full().child(picture().object_fit(gpui::ObjectFit::ScaleDown)).into_any_element(),
        };
        let fit = zoom == Zoom::Fit;
        div()
            .id("ex-image")
            .debug_selector(|| "ex-image".into())
            .relative()
            .flex_1()
            .min_h_0()
            .size_full()
            .bg(t.surface_2)
            .child(self.measure(self.image_area.clone(), cx))
            .child(
                div()
                    .id("ex-image-scroll")
                    .size_full()
                    .overflow_scroll()
                    .on_click(cx.listener(move |v, e: &gpui::ClickEvent, _, cx| {
                        if e.click_count() == 2 {
                            v.set_zoom(if fit { Zoom::Scale(1.) } else { Zoom::Fit }, cx);
                        }
                    }))
                    .child(div().min_w_full().min_h_full().p(px(image_view::PAD)).flex().items_center().justify_center().child(inner)),
            )
            .into_any_element()
    }

    /// The changed-on-disk question, the save error, under the header.
    fn notices(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(d) = self.active.as_ref().and_then(|p| self.docs.get(p)) else { return Vec::new() };
        let (blocked, diffing) = (d.save_blocked, d.diff.is_some());
        let mut out = Vec::new();
        if d.newer.is_some() || blocked {
            let text = if blocked {
                "Changed on disk since it was read: saving would overwrite it."
            } else {
                "Changed on disk. You have unsaved changes."
            };
            out.push(
                Self::notice("ex-changed-on-disk", true, text.into(), t)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(px(6.))
                            .child(
                                small_btn("ex-keep-mine", if blocked { "Save mine" } else { "Keep mine" }, t)
                                    .debug_selector(|| "ex-keep-mine".into())
                                    .tooltip(|_, cx| tooltip_view("Keep your changes; saving writes them over the file on disk", cx))
                                    .on_click(cx.listener(|this, _, _, cx| this.keep_mine(cx))),
                            )
                            .child(
                                small_btn("ex-reload", "Take theirs", t)
                                    .debug_selector(|| "ex-reload".into())
                                    .tooltip(|_, cx| tooltip_view("Drop your changes and show the file on disk", cx))
                                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                            )
                            .child(
                                small_btn("ex-show-difference", if diffing { "Hide difference" } else { "Show difference" }, t)
                                    .debug_selector(|| "ex-show-difference".into())
                                    .tooltip(|_, cx| tooltip_view("The file on disk against your changes", cx))
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_difference(cx))),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if let Some(e) = d.save_error.clone() {
            out.push(Self::notice("ex-save-error", false, format!("Couldn't save: {}", e.lines().next().unwrap_or_default()).into(), t).into_any_element());
        }
        out
    }
}

/// One open file in the list of tabs.
struct TabRow {
    path: String,
    on: bool,
    dirty: bool,
}

/// The list of tabs' rows: picking one shows it (from a pop-up: once it
/// has closed, in the viewer's window).
fn tab_rows(rows: &[TabRow], t: &Theme, viewer: &gpui::WeakEntity<Viewer>) -> Vec<AnyElement> {
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            let (dir, base) = split_path(&r.path);
            let hint = (!dir.is_empty()).then(|| SharedString::from(dir.trim_end_matches('/').to_string()));
            let (viewer, p) = (viewer.clone(), r.path.clone());
            crate::kit::menu_item(("ex-tab-item", i), if r.on { "check" } else { "" }, base.to_string(), hint, false, t)
                .when(r.dirty, |d| d.child(div().size(px(7.)).rounded_full().bg(t.text_2)))
                .on_click(move |_, window, cx| {
                    let (viewer, p) = (viewer.clone(), p.clone());
                    crate::kit::pop::pick(window, cx, move |_, cx| {
                        let _ = viewer.update(cx, |v, cx| {
                            v.tabs_menu = false;
                            v.select(p, cx);
                        });
                    })
                })
                .into_any_element()
        })
        .collect()
}

/// A tooltip with `text`.
fn tooltip_text(text: String) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| tooltip_view(text.clone(), cx)
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        if std::mem::take(&mut self.refocus) {
            window.focus(&self.focus, cx);
        }
        let a = self.agent(cx);
        let root = self.root(cx);
        self.sync_preview(root.as_deref(), cx);
        self.ensure_code(cx);
        let active = self.active.clone();
        let pixels = self.image_pixels(window, cx);
        let body = self.body(&t, pixels, cx);
        let tabs = self.tabs.clone();
        self.tab_focus.retain(|p, _| tabs.contains(p));
        for p in tabs {
            self.tab_focus.entry(p).or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
        // A tab newly shown comes into view (once: the strip can be
        // scrolled away from it).
        if self.tabs_scrolled_to != active {
            if let Some(i) = active.as_ref().and_then(|a| self.tabs.iter().position(|p| p == a)) {
                self.tabs_scroll.scroll_to_item(i);
            }
            self.tabs_scrolled_to = active.clone();
        }
        let shown = self.tree_shown(cx);
        let width = self.tree_width(cx);
        let dragging = self.tree_drag_w.is_some();
        let notices = self.notices(&t, cx);
        let side = self.side(&t, width, cx).into_any_element();
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
            .on_action(cx.listener(|this, _: &ToggleTree, window, cx| this.toggle_tree(window, cx)))
            .on_action(|_: &NextTab, window, cx| window.focus_next(cx))
            .on_action(|_: &PrevTab, window, cx| window.focus_prev(cx))
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .when_some(self.place_menu, |d, at| d.child(self.place_menu_overlay(at, &t, cx)))
            .child(self.measure(self.measured.clone(), cx))
            .child(self.bar(&t, a.as_ref(), cx))
            .child(
                div()
                    .id("ex-viewer-main")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .on_drag_move(cx.listener(|this, e: &gpui::DragMoveEvent<crate::kit::side::SideDrag>, _, cx| {
                        if e.drag(cx).0 == SIDE {
                            this.drag_tree(crate::kit::side::dragged_width(e), cx)
                        }
                    }))
                    .on_drop(cx.listener(|this, _: &crate::kit::side::SideDrag, _, cx| this.keep_tree_width(cx)))
                    .child(crate::kit::side::column(SIDE, shown, width, dragging, side))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .bg(t.term_bg)
                            .when(!self.tabs.is_empty(), |d| d.child(self.tabs_row(&t, window, cx)))
                            .when_some(active, |d, p| d.child(self.head(&t, a.as_ref(), &p, pixels, cx)))
                            .children(notices)
                            .child(div().flex_1().min_h_0().flex().flex_col().child(body)),
                    ),
            )
    }
}
