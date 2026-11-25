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
//! their pixel size in the header, an SVG also as its source (Picture /
//! Source, ⌘⇧V); PDFs (macOS: `crate::platform::pdf`) are drawn page by
//! page, only around the view ([`super::pdf_view`]), with zoom, the page
//! in view and Find; a picture or PDF larger than its pane is dragged
//! or scrolled any way, and a pinch or ⌘-scroll zooms it about the
//! pointer ([`super::pan_zoom`]); other binary files (and PDFs
//! elsewhere) on this computer offer "Open in default app".
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
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::kit::HoverText;
use gpui::{
    actions, canvas, div, prelude::*, px, AnyElement, App, Bounds, ClipboardItem, Context, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, MouseButton, Pixels,
    PromptLevel, RenderImage, ScrollHandle, SharedString, Size, Subscription, Task, Window,
};

use pitwall_core::explorer::{is_pdf, is_svg, ContentKind, FileView, Saved, MAX_PDF};
use pitwall_proto::AgentView;

use super::code_view::{FileCode, Target};
use crate::code_view::{CodeView, CodeViewEvent};
use crate::kit::file_icons::file_icon;
use crate::kit::{InputEvent, TextInput};
use crate::platform::pdf::{Hit, OpenError, Pdf};
use super::column;
use super::image_view::{self, Zoom};
use super::pan_zoom;
use super::pdf_view::{self, Layout};
use super::logic::{absolute_path, short_path, size_label, split_path};
use super::search::{Blurred, OpenHit, SearchPane};
use super::source::{Res, Source, LARGE_CAP, TEXT_CAP};
use super::tree::{now_ms, TreeModel};
use super::root_menu::{RootMenu, RootMenuEvent, Start};
use super::tree_view::{FolderMenu, OpenFile, TreeView};
use super::widgets::{refresh_control, status_mark};
use crate::kit::{icon, icon_btn_state, kbd, keys, small_btn, tool_btn, tooltip_view, MONO_FONT};
use crate::agents::AgentStore;
use crate::markdown_view::{model::is_markdown, MarkdownView, MAX_PREVIEW_BYTES};
use crate::theme::{self, Theme, RADIUS, RADIUS_SM};

actions!(explorer_viewer, [Back, NextTab, PrevTab, PickTab, SaveFile, ToggleTree, SwitchView, FindInPdf]);

/// Switch an SVG between its picture and its source, a Markdown file
/// between Code and Preview (⌘⇧V, as VS Code's Markdown preview; on
/// Windows and Linux Ctrl+Shift+Alt+V: Ctrl+Shift+V pastes in terminals).
const SWITCH_VIEW_KEY: &str = if cfg!(target_os = "macos") { "cmd-shift-v" } else { "ctrl-shift-alt-v" };

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
        KeyBinding::new(SWITCH_VIEW_KEY, SwitchView, Some(CONTEXT)),
        // A PDF's search (a text file's Find is the code view's own).
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-f" } else { "ctrl-f" }, FindInPdf, Some(CONTEXT)),
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
    Place(super::dock::FilesPlace, Option<String>),
    /// The grip pressed, or the bar (or the tab strip's empty end) pressed
    /// and dragged: the files follow the pointer until it is let go (into
    /// the layout of the Pitwall window under it, or out of it; a window
    /// of its own moves with it).
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
    /// An image's (or a PDF's) zoom.
    zoom: Zoom,
    /// How far an image larger than its pane is moved (points, from its
    /// room's top left; `None`: centred). A PDF's is its `scroll`.
    pan: Option<(f32, f32)>,
    /// The image's size in pixels as last drawn (a new size read from
    /// disk centres it again).
    pixels: Option<(i32, i32)>,
    /// An SVG's picture was made from this text (by its identity).
    svg_from: Option<(usize, usize)>,
    /// A PDF the viewer draws (macOS), once its bytes are asked for.
    pdf: Option<PdfDoc>,
}

impl Doc {
    fn dirty(&self, cx: &App) -> bool {
        self.code.as_ref().is_some_and(|c| c.code.read(cx).is_dirty())
    }

    /// Its PDF's drawn pages, to let go of (`drop_images`).
    fn release_pdf(&mut self) -> Vec<Arc<RenderImage>> {
        self.pdf.as_mut().map(|p| p.pages.drain().map(|(_, pg)| pg.image).collect()).unwrap_or_default()
    }
}

/// A PDF in the viewer: the document (on its own thread), the pages drawn
/// of it, where it is scrolled and its search.
struct PdfDoc {
    /// Which read this is (an answer for an older one is dropped).
    id: u64,
    state: PdfState,
    pages: HashMap<usize, PageImage>,
    /// Pages being drawn.
    pending: HashSet<usize>,
    scroll: ScrollHandle,
    /// The scale last laid out at (a new one keeps the place in view).
    scale: Option<f32>,
    /// The page being read, as last laid out.
    current: usize,
    /// Pages the engine couldn't draw (not asked for again).
    failed: HashSet<usize>,
    find: Option<PdfFind>,
    /// A pinch or ⌘-scroll is going on: the pages drawn are scaled as
    /// they are, and drawn again once it settles (`_settle`).
    zooming: bool,
    _settle: Option<Task<()>>,
}

enum PdfState {
    Reading,
    Open(Arc<Pdf>),
    /// Why it isn't shown.
    Failed(String),
}

/// A page drawn, `w` × `h` pixels.
struct PageImage {
    image: Arc<RenderImage>,
    w: u32,
    h: u32,
}

/// A PDF's search: the field, what was found, the hit shown.
struct PdfFind {
    field: Entity<TextInput>,
    query: String,
    hits: Vec<Hit>,
    current: Option<usize>,
    searching: bool,
    /// Typing settles before it searches.
    _wait: Option<Task<()>>,
    _search: Option<Task<()>>,
    _sub: Subscription,
}

impl PdfDoc {
    fn new(id: u64) -> PdfDoc {
        PdfDoc {
            id,
            state: PdfState::Reading,
            pages: HashMap::new(),
            pending: HashSet::new(),
            scroll: ScrollHandle::new(),
            scale: None,
            current: 0,
            failed: HashSet::new(),
            find: None,
            zooming: false,
            _settle: None,
        }
    }

    fn open(&self) -> Option<Arc<Pdf>> {
        match &self.state {
            PdfState::Open(p) => Some(p.clone()),
            _ => None,
        }
    }
}

/// What the viewer says when a PDF can't be shown.
fn pdf_error(e: OpenError) -> String {
    match e {
        OpenError::Locked => "This PDF is protected with a password: Pitwall can't open it yet.".into(),
        OpenError::Damaged(why) => format!("This PDF can't be read ({why}). It may be damaged."),
        OpenError::Unsupported => PDF_UNSUPPORTED.into(),
    }
}

/// Windows and Linux (`platform::can_view_pdf`).
const PDF_UNSUPPORTED: &str = "Viewing PDFs in Pitwall isn't available on Windows and Linux yet.";

/// A drawn page as GPUI draws it (it is BGRA already).
fn page_image(b: crate::platform::pdf::Bitmap) -> Option<PageImage> {
    let (w, h) = (b.w, b.h);
    let buffer = image::RgbaImage::from_raw(w, h, b.bgra)?;
    Some(PageImage { image: Arc::new(RenderImage::new([image::Frame::new(buffer)])), w, h })
}

/// Let pictures go from every window's textures (once this update is
/// over, when every window can be reached).
fn drop_images(images: Vec<Arc<RenderImage>>, cx: &mut App) {
    if images.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        for i in images {
            cx.drop_image(i, None);
        }
    });
}

/// A press on an image or a PDF larger than its pane, at `at` (window
/// points) with the picture moved `from` (points).
#[derive(Clone, Copy, Debug)]
struct PanPress {
    at: gpui::Point<Pixels>,
    from: (f32, f32),
    pdf: bool,
    moved: bool,
}

/// Where a pane is in the window, as drawn (no redraw for it).
fn place(cell: Rc<Cell<Option<Bounds<Pixels>>>>) -> impl IntoElement {
    canvas(move |b, _, _| cell.set(Some(b)), |_, _, _, _| {}).absolute().inset_0()
}

/// `at` (window points) in a pane at `pane`, `view` (w, h) points large;
/// no point (the − and + buttons): the pane's centre.
fn in_pane(at: Option<gpui::Point<Pixels>>, pane: Option<Bounds<Pixels>>, view: (f32, f32)) -> (f32, f32) {
    match (at, pane) {
        (Some(at), Some(b)) => (f32::from(at.x - b.origin.x), f32::from(at.y - b.origin.y)),
        _ => (view.0 / 2., view.1 / 2.),
    }
}

/// How far an image `len` (w, h) points large is moved in a pane `view`
/// large: `pan`, or centred, kept in reach.
fn image_offset(pan: Option<(f32, f32)>, len: (f32, f32), view: (f32, f32)) -> (f32, f32) {
    let pad = image_view::PAD;
    let (x, y) = pan.unwrap_or((pan_zoom::max_offset(len.0, pad, view.0) / 2., pan_zoom::max_offset(len.1, pad, view.1) / 2.));
    (pan_zoom::clamp(x, len.0, pad, view.0), pan_zoom::clamp(y, len.1, pad, view.1))
}

/// SVGs shown as their source (by viewer key and path; the session's
/// choice, a picture otherwise).
#[derive(Default)]
struct SvgSource(HashSet<String>);

impl gpui::Global for SvgSource {}

fn svg_key(agent: &str, path: &str) -> String {
    format!("{agent}\u{0}{path}")
}

/// The ids of PDF reads.
static PDF_READS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The room a PDF's pages have in its pane (`area`, as measured): its
/// width less the scrollbar's gutter.
fn pdf_room(area: Size<Pixels>) -> (f32, f32) {
    ((f32::from(area.width) - PDF_GUTTER).max(1.), f32::from(area.height).max(1.))
}

/// The scrollbar's gutter beside a PDF's pages (`kit::scrollbar`).
const PDF_GUTTER: f32 = 10.;

/// A search hit on a page, and the one shown (fixed colours: the page is
/// white paper in either theme).
const HIT: gpui::Hsla = gpui::Hsla { h: 0.15, s: 1., l: 0.5, a: 0.35 };
const HIT_ON: gpui::Hsla = gpui::Hsla { h: 0.08, s: 1., l: 0.5, a: 0.55 };

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

/// `path`, read as `v`, is a PDF the viewer draws here (macOS, up to
/// [`MAX_PDF`]).
fn pdf_viewable(path: &str, v: &Loaded) -> bool {
    v.kind == ContentKind::Binary && is_pdf(path) && v.size <= MAX_PDF && crate::platform::can_view_pdf()
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
    /// The id of the agent `owner` names (whose tile or window it is).
    owner_id: Option<String>,
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
    /// The "Dock…" / "Move…" menu, open at this point of the viewer, with
    /// the terminals the files could replace (`(id, name)`).
    place_menu: Option<(gpui::Point<gpui::Pixels>, Vec<Terminal>)>,
    /// Where the bar (or the tab strip's empty end) was pressed: a drag
    /// once the pointer travels [`BAR_DRAG_THRESHOLD`] from there, else a
    /// click.
    bar_press: Option<gpui::Point<gpui::Pixels>>,
    /// A press on an image or a PDF larger than its pane: it moves once
    /// the pointer travels [`pan_zoom::DRAG_THRESHOLD`].
    pan_press: Option<PanPress>,
    /// The last such press moved it (its release is no click).
    pan_moved: bool,
    /// The tree column as toggled in this viewer (`None`: as kept for its
    /// kind of place, see [`column::shown`]).
    tree_open: Option<bool>,
    /// The column's width while its edge is dragged.
    tree_drag_w: Option<f32>,
    /// The viewer's size and the image pane's, as last laid out.
    measured: Rc<Cell<Option<Size<Pixels>>>>,
    image_area: Rc<Cell<Option<Size<Pixels>>>>,
    pdf_area: Rc<Cell<Option<Size<Pixels>>>>,
    /// Where the image pane and the PDF pane are in the window, as last
    /// drawn (zoom keeps the point under the pointer).
    image_box: Rc<Cell<Option<Bounds<Pixels>>>>,
    pdf_box: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The display's pixels per point, as last drawn (PDF pages are drawn
    /// for it).
    dpr: f32,
    /// The tab strip: scrolled to the active tab; more tabs than fit
    /// (`tabs_overflow`) offer a list of them all (`tabs_menu`).
    tabs_scroll: ScrollHandle,
    tabs_overflow: Rc<Cell<bool>>,
    tabs_menu: bool,
    /// The tab last scrolled into view.
    tabs_scrolled_to: Option<String>,
    /// The root menu (the folder in the bar clicked), open at this point.
    root_menu: Option<(gpui::Point<Pixels>, Entity<RootMenu>, Subscription)>,
    /// A folder's menu in the tree ("Open as root"), open at this point.
    tree_menu: Option<(gpui::Point<Pixels>, String)>,
    /// Why the root couldn't change (from the tree's menu).
    root_error: Option<String>,
    /// Where the folder in the bar is, as last drawn (the menu opens under
    /// it from the command palette).
    hint_box: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The last press on the bar dragged the files (its release is no
    /// click on the folder).
    bar_dragged: bool,
    _subs: Vec<Subscription>,
}

/// A terminal the files could take the place of: its agent's id and name.
type Terminal = (String, String);

/// The tree column's id (its edge's drags say it).
const SIDE: &str = "ex-side";
/// A press on the bar travels this far (px) before it drags the files.
pub const BAR_DRAG_THRESHOLD: f32 = 4.;

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
            cx.subscribe(&tree_view, |this, _, e: &FolderMenu, cx| this.folder_menu(e.path.clone(), e.at, cx)),
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
            // Gone: its PDFs' drawn pages leave the windows' textures.
            cx.on_release(|this: &mut Viewer, cx| {
                let images: Vec<_> = this.docs.values_mut().flat_map(|d| d.release_pdf()).collect();
                for i in images {
                    cx.drop_image(i, None);
                }
            }),
        ];
        super::unsaved::register(cx.weak_entity(), cx);
        Viewer {
            source,
            store,
            agent_id,
            host: Host::Main,
            owner: None,
            owner_id: None,
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
            bar_press: None,
            pan_press: None,
            pan_moved: false,
            tree_open: None,
            tree_drag_w: None,
            measured: Rc::default(),
            image_area: Rc::default(),
            pdf_area: Rc::default(),
            image_box: Rc::default(),
            pdf_box: Rc::default(),
            dpr: 2.,
            tabs_scroll: ScrollHandle::new(),
            tabs_overflow: Rc::default(),
            tabs_menu: false,
            tabs_scrolled_to: None,
            root_menu: None,
            tree_menu: None,
            root_error: None,
            hint_box: Rc::default(),
            bar_dragged: false,
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

    /// Laid out as `other` was, its tree shown or hidden as there (a
    /// viewer replacing it in the same place: the tree doesn't slide in or
    /// out).
    pub fn size_as(&mut self, other: &Viewer) {
        self.measured.set(other.measured.get());
        self.tree_open = other.tree_open;
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
        let pdf = self.active_pdf().map(|(_, pdf, zoom)| self.pdf_scale(zoom, &pdf));
        let s = match (now, pdf) {
            (_, Some(s)) => s,
            (Zoom::Fit, None) => 1.,
            (Zoom::Scale(s), None) => s,
        };
        let z = match (what, pdf.is_some()) {
            ("fit", _) => Zoom::Fit,
            ("100", _) => Zoom::Scale(1.),
            ("in", false) => Zoom::Scale(image_view::zoom_in(s)),
            ("out", false) => Zoom::Scale(image_view::zoom_out(s)),
            ("in", true) => Zoom::Scale(pdf_view::zoom_in(s)),
            ("out", true) => Zoom::Scale(pdf_view::zoom_out(s)),
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
        let mut gone = Vec::new();
        self.docs.retain(|p, d| {
            let keep = Some(p) == active.as_ref() || d.dirty(cx);
            if !keep {
                gone.extend(d.release_pdf());
            }
            keep
        });
        drop_images(gone, cx);
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
        if let Some(mut d) = self.docs.remove(path) {
            drop_images(d.release_pdf(), cx);
        }
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
        let mut gone = Vec::new();
        match res {
            Ok(v) => {
                let v = Loaded::from(v);
                // Its picture is read on its own (an SVG's is made from its
                // text when that came).
                let image = v.kind == ContentKind::Image && v.text.is_none();
                let pdf = pdf_viewable(&path, &v);
                if quiet && d.view.is_some() {
                    let on_disk = d.base.as_ref().or(d.view.as_ref().and_then(|w| w.text.as_ref()));
                    if dirty || d.save_blocked {
                        // Unsaved changes: never overwritten; the bar asks.
                        let was = d.view.as_ref().map(|w| w.kind);
                        let differs = v.text.as_ref() != on_disk || Some(v.kind) != was || v.text.is_none();
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
                        d.svg_from = None;
                        // (A PDF read again keeps its place and search.)
                        gone.extend(d.release_pdf());
                        if !pdf {
                            d.pdf = None;
                        }
                        if image {
                            self.read_image(&path, cx);
                        }
                        if pdf {
                            self.read_pdf(&path, cx);
                        }
                    }
                } else {
                    let code = d.code.take();
                    let sub = d._code_sub.take();
                    // (A PDF or an image read again keeps its zoom and place.)
                    let (zoom, pan, pixels) = (d.zoom, d.pan, d.pixels);
                    gone.extend(d.release_pdf());
                    *d = Doc {
                        view: Some(v),
                        local: here,
                        code,
                        _code_sub: sub,
                        zoom,
                        pan,
                        pixels,
                        ..Doc::default()
                    };
                    if image {
                        self.read_image(&path, cx);
                    }
                    if pdf {
                        self.read_pdf(&path, cx);
                    }
                }
            }
            Err(e) => {
                if !(quiet && d.view.is_some()) {
                    gone.extend(d.release_pdf());
                    *d = Doc {
                        error: Some(e),
                        ..Doc::default()
                    };
                }
            }
        }
        drop_images(gone, cx);
        cx.notify();
    }

    /// A PDF's bytes, read and opened by the system's PDF engine (off the
    /// UI thread).
    fn read_pdf(&mut self, path: &str, cx: &mut Context<Self>) {
        let id = PDF_READS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Some(d) = self.docs.get_mut(path) else { return };
        // (Read again: the same place in it, and the old one shown until
        // the new one is open.)
        let mut doc = PdfDoc::new(id);
        if let Some(old) = d.pdf.take() {
            doc.scroll = old.scroll;
            doc.find = old.find;
            doc.scale = old.scale;
            if matches!(old.state, PdfState::Open(_)) {
                doc.state = old.state;
            }
        }
        d.pdf = Some(doc);
        let (source, agent, p) = (self.source.clone(), self.agent_id.clone(), path.to_string());
        let load = cx.background_executor().spawn(async move {
            let bytes = if super::logic::outside(&p) { source.read_outside_image(&p) } else { source.read_image(&agent, &p) };
            let bytes = bytes.map_err(|e| format!("Couldn't read the PDF: {e}"))?;
            crate::platform::pdf::open(bytes).map_err(pdf_error)
        });
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            let res = load.await;
            let _ = this.update(cx, |this, cx| {
                let Some(pd) = this.docs.get_mut(&path).and_then(|d| d.pdf.as_mut()).filter(|p| p.id == id) else { return };
                pd.state = match res {
                    Ok(pdf) => PdfState::Open(Arc::new(pdf)),
                    Err(e) => PdfState::Failed(e),
                };
                // The old one's pages go (answers still coming for it are
                // dropped: `page_drawn`).
                let gone: Vec<_> = pd.pages.drain().map(|(_, p)| p.image).collect();
                pd.pending.clear();
                pd.failed.clear();
                drop_images(gone, cx);
                // A search open while it was read again: run it on this one.
                if pd.find.as_ref().is_some_and(|f| !f.query.is_empty()) {
                    this.run_find(&path, false, cx);
                }
                cx.notify();
            });
        })
        .detach();
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

    /// The picture drawn for `path` is made of `bytes` (tests).
    #[cfg(test)]
    pub fn picture_is(&self, path: &str, bytes: &str) -> bool {
        self.docs.get(path).and_then(|d| d.image.as_ref()).is_some_and(|i| i.bytes() == bytes.as_bytes())
    }

    /// `path`'s PDF: `Some(Ok(pages))` open, `Some(Err(why))` not shown,
    /// `None` not read as one (tests).
    #[cfg(test)]
    pub fn pdf_state(&self, path: &str) -> Option<Result<usize, String>> {
        match &self.docs.get(path)?.pdf.as_ref()?.state {
            PdfState::Open(p) => Some(Ok(p.pages().len())),
            PdfState::Failed(e) => Some(Err(e.clone())),
            PdfState::Reading => None,
        }
    }

    /// `path`'s PDF pages drawn and being drawn (tests).
    #[cfg(test)]
    pub fn pdf_drawn(&self, path: &str) -> (Vec<usize>, usize) {
        let Some(pd) = self.docs.get(path).and_then(|d| d.pdf.as_ref()) else { return (Vec::new(), 0) };
        let mut drawn: Vec<usize> = pd.pages.keys().copied().collect();
        drawn.sort();
        (drawn, pd.pending.len())
    }

    /// How wide (pixels) page `page` of `path`'s PDF is drawn (tests).
    #[cfg(test)]
    pub fn pdf_page_w(&self, path: &str, page: usize) -> Option<u32> {
        self.docs.get(path)?.pdf.as_ref()?.pages.get(&page).map(|p| p.w)
    }

    /// `path`'s PDF search: what was found and the hit shown (tests).
    #[cfg(test)]
    pub fn pdf_hits(&self, path: &str) -> Option<(Vec<usize>, Option<usize>)> {
        let f = self.docs.get(path)?.pdf.as_ref()?.find.as_ref()?;
        Some((f.hits.iter().map(|h| h.page).collect(), f.current))
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
                    image = n.kind == ContentKind::Image && n.text.is_none();
                    d.view = Some(n);
                    d.image = None;
                    d.image_error = None;
                    d.svg_from = None;
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

    // ── an SVG's picture and source ─────────────────────────────────────

    /// `path` is an SVG shown as its source (the session's choice; its
    /// picture otherwise).
    fn svg_source(&self, path: &str, cx: &App) -> bool {
        is_svg(path) && cx.try_global::<SvgSource>().is_some_and(|s| s.0.contains(&svg_key(&self.agent_id, path)))
    }

    /// `path` is an SVG whose source can be shown (its text was read).
    fn has_source(&self, path: &str) -> bool {
        is_svg(path) && self.docs.get(path).and_then(|d| d.view.as_ref()).is_some_and(|v| v.kind == ContentKind::Image && v.text.is_some())
    }

    /// `path` is shown as text: a text file, or an SVG's source.
    fn as_text(&self, path: &str, cx: &App) -> bool {
        let text = self.docs.get(path).and_then(|d| d.view.as_ref()).is_some_and(|v| v.kind == ContentKind::Text);
        text || (self.has_source(path) && self.svg_source(path, cx))
    }

    /// Show the active SVG as its source (or its picture).
    pub(crate) fn set_svg_source(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone().filter(|p| is_svg(p)) else { return };
        let key = svg_key(&self.agent_id, &active);
        let mut s = cx.try_global::<SvgSource>().map(|s| s.0.clone()).unwrap_or_default();
        if on {
            s.insert(key);
        } else {
            s.remove(&key);
        }
        cx.set_global(SvgSource(s));
        cx.notify();
    }

    /// ⌘⇧V: an SVG's picture or source, a Markdown file's Code or
    /// Preview.
    fn switch_view(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else { return };
        if self.has_source(&active) {
            let on = !self.svg_source(&active, cx);
            self.set_svg_source(on, cx);
        } else if self.md_text().is_some() {
            self.set_preview(!self.preview, cx);
        }
    }

    /// The active SVG's picture, made again from its text when that
    /// changed (edited, saved, read again).
    fn sync_svg(&mut self, cx: &App) {
        let Some(active) = self.active.clone().filter(|p| self.has_source(p)) else { return };
        let Some(d) = self.docs.get_mut(&active) else { return };
        let live = d.live.as_ref().map(|(_, t)| t.clone()).filter(|_| d.dirty(cx));
        let Some(text) = live.or_else(|| d.view.as_ref().and_then(|v| v.text.clone())) else { return };
        let id = (text.as_ptr() as usize, text.len());
        if d.svg_from == Some(id) && d.image.is_some() {
            return;
        }
        d.svg_from = Some(id);
        d.image = Some(Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Svg, text.as_bytes().to_vec())));
        d.image_error = None;
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
            // An SVG's source is text.
            ContentKind::Image if self.has_source(path) => {}
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
        if !self.as_text(&active, cx) {
            return;
        }
        let Some(d) = self.docs.get_mut(&active) else { return };
        if d.code.is_none() {
            let fc = FileCode::new(cx);
            let path = active.clone();
            d._code_sub = Some(cx.subscribe(&fc.code, move |this, code, e: &CodeViewEvent, cx| match e {
                CodeViewEvent::Save => this.save(&path, cx).detach(),
                CodeViewEvent::Edited => {
                    if let Some(d) = this.docs.get_mut(&path) {
                        let c = code.read(cx);
                        // (The Preview, an SVG's picture follow the edits.)
                        d.live = ((is_markdown(&path) || is_svg(&path)) && c.is_dirty()).then(|| (c.version(), c.text().unwrap_or_default().into()));
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
        ) + &self.debug_pdf_state(&active, cx)
    }

    /// The SVG's and the PDF's part of [`Self::debug_state`].
    fn debug_pdf_state(&self, active: &str, cx: &App) -> String {
        let mut out = String::new();
        if self.has_source(active) {
            out += &format!(" svg_source={}", self.svg_source(active, cx));
        }
        if let Some(pd) = self.docs.get(active).and_then(|d| d.pdf.as_ref()) {
            let state = match &pd.state {
                PdfState::Reading => "reading".to_string(),
                PdfState::Open(p) => format!("open pages={}", p.pages().len()),
                PdfState::Failed(e) => format!("failed {e:?}"),
            };
            let bytes: usize = pd.pages.values().map(|i| i.w as usize * i.h as usize * 4).sum();
            let mut drawn: Vec<usize> = pd.pages.keys().copied().collect();
            drawn.sort();
            out += &format!(" pdf={state} scale={:?} current={} drawn={drawn:?} drawn_mb={} pending={}", pd.scale, pd.current + 1, bytes / (1024 * 1024), pd.pending.len());
            if let Some(f) = &pd.find {
                out += &format!(" find={:?} hits={} hit={:?}", f.query, f.hits.len(), f.current.and_then(|c| f.hits.get(c)));
            }
        }
        out
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
                this.place_menu = match this.place_menu {
                    Some(_) => None,
                    None => {
                        let me = cx.entity();
                        let replace = super::dock::owner_of(&me, cx).map(|o| super::dock::replaceable(&o, cx)).unwrap_or_default();
                        Some((e.position(), replace))
                    }
                };
                cx.notify();
            }))
    }

    fn place_menu_overlay(&self, at: gpui::Point<gpui::Pixels>, replace: Vec<(String, String)>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let me = cx.entity().downgrade();
        let pick = move |p: Option<(super::dock::FilesPlace, Option<String>)>, cx: &mut App| {
            let _ = me.update(cx, |this, cx| {
                this.place_menu = None;
                if let Some((p, of)) = p {
                    cx.emit(ViewerEvent::Place(p, of));
                }
                cx.notify();
            });
        };
        super::dock::menu(at, self.host == Host::Tile, Some(self.owner.clone().unwrap_or_else(|| "the agent".into())), replace, "files", t, pick)
    }

    /// A press on the bar's (or the tab strip's) empty part: the files are
    /// dragged by it once it travels (a tile or a window; not the full
    /// viewer). A double click doesn't start one.
    fn arm_bar(&mut self, e: &gpui::MouseDownEvent) {
        self.bar_dragged = false;
        if self.host != Host::Main && e.click_count < 2 {
            self.bar_press = Some(e.position);
        }
    }

    /// The pointer moved with the bar pressed: far enough, the drag the
    /// grip starts.
    fn bar_moved(&mut self, e: &gpui::MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(at) = self.bar_press else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.bar_press = None;
        } else if (e.position - at).magnitude() > f64::from(BAR_DRAG_THRESHOLD) {
            self.bar_press = None;
            self.bar_dragged = true;
            cx.emit(ViewerEvent::Grip);
        }
    }

    /// The places menu is open (tests).
    #[cfg(test)]
    pub(crate) fn place_menu_open(&self) -> bool {
        self.place_menu.is_some()
    }

    /// Its root can be switched here: a files tile or window (not the
    /// main area's viewer) on a folder on this computer. Another machine's
    /// agent's viewer reads its whole folder there: no root menu.
    pub fn can_switch_root(&self, cx: &App) -> bool {
        self.host != Host::Main && self.local(cx)
    }

    /// The agent whose files tile or window this is (said by its place:
    /// a window can't be looked into while it draws).
    pub fn set_owner_id(&mut self, id: &str) {
        self.owner_id = Some(id.to_string());
    }

    fn owner_id(&self, cx: &mut Context<Self>) -> Option<String> {
        if let Some(id) = &self.owner_id {
            return Some(id.clone());
        }
        let me = cx.entity();
        super::dock::owner_of(&me, cx)
    }

    /// The root menu, under the folder in the bar (or at `at`), the
    /// keyboard in the field it opens on.
    pub fn open_root_menu(&mut self, start: Start, at: Option<gpui::Point<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_switch_root(cx) {
            return;
        }
        let Some(owner) = self.owner_id(cx) else { return };
        let Some(root) = self.root(cx).map(|r| super::show::real(&r)) else { return };
        let own = self.store.read(cx).agent(&owner).map(super::show::root_of);
        let agent_root = own.filter(|o| *o != root || self.agent_id != owner);
        let at = at.or_else(|| self.hint_box.get().map(|b| b.bottom_left())).unwrap_or_default();
        let menu = cx.new(|cx| RootMenu::new(root, agent_root, start, cx));
        let sub = cx.subscribe(&menu, move |this, _, e: &RootMenuEvent, cx| match e {
            RootMenuEvent::Pick(p) => {
                let key = super::show::key_for(&owner, p, cx);
                this.request_root(key, cx);
            }
            RootMenuEvent::AgentFolder => this.request_root(owner.clone(), cx),
            RootMenuEvent::Dismissed => this.close_root_menus(cx),
        });
        let field = menu.read(cx).field(cx);
        window.focus(&field, cx);
        self.tree_menu = None;
        self.root_menu = Some((at, menu, sub));
        cx.notify();
    }

    /// The root menu, open (tests).
    #[cfg(test)]
    pub(crate) fn root_menu(&self) -> Option<Entity<RootMenu>> {
        self.root_menu.as_ref().map(|(_, m, _)| m.clone())
    }

    /// Why the root couldn't change, said in the viewer (tests).
    #[cfg(test)]
    pub(crate) fn root_error(&self) -> Option<&str> {
        self.root_error.as_deref()
    }

    fn close_root_menus(&mut self, cx: &mut Context<Self>) {
        self.root_menu = None;
        self.tree_menu = None;
        cx.notify();
    }

    /// A folder of the tree right-clicked: its menu ("Open as root").
    fn folder_menu(&mut self, path: String, at: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        if !self.can_switch_root(cx) {
            return;
        }
        self.root_menu = None;
        self.tree_menu = Some((at, path));
        cx.notify();
    }

    /// The tree's folder `rel` as the root.
    fn open_as_root(&mut self, rel: &str, cx: &mut Context<Self>) {
        let (Some(owner), Some(root)) = (self.owner_id(cx), self.root(cx)) else { return };
        let path = super::show::real(&std::path::Path::new(&root).join(rel).to_string_lossy());
        let key = super::show::key_for(&owner, &path, cx);
        self.request_root(key, cx);
    }

    /// Root the viewer at `key`, once this update is over (its place makes
    /// a new viewer, or says why not: shown in the root menu, else in the
    /// viewer).
    fn request_root(&mut self, key: String, cx: &mut Context<Self>) {
        let me = cx.entity();
        cx.defer(move |cx| match super::show::switch_root(&me, &key, cx) {
            Ok(_) => {
                me.update(cx, |v, cx| {
                    v.root_error = None;
                    v.close_root_menus(cx);
                });
                cx.refresh_windows();
            }
            Err(e) => me.update(cx, |v, cx| {
                match v.root_menu.as_ref().map(|(_, m, _)| m.clone()) {
                    Some(m) => m.update(cx, |m, cx| m.set_error(e, cx)),
                    None => v.root_error = Some(e),
                }
                v.tree_menu = None;
                cx.notify();
            }),
        });
    }

    fn root_menu_overlay(&self, at: gpui::Point<Pixels>, menu: Entity<RootMenu>, cx: &mut Context<Self>) -> AnyElement {
        gpui::deferred(
            gpui::anchored().position(at).snap_to_window().child(
                div()
                    .mt(px(4.))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_root_menus(cx)))
                    .child(menu),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn tree_menu_overlay(&self, at: gpui::Point<Pixels>, rel: String, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let ft = t.float();
        gpui::deferred(
            gpui::anchored().position(at).snap_to_window().child(
                crate::kit::menu_panel(&ft)
                    .id("ex-tree-menu")
                    .occlude()
                    .border_1()
                    .border_color(ft.line_strong)
                    .child(
                        crate::kit::menu::menu_item("ex-open-as-root", "folder", "Open as root", None, false, &ft)
                            .debug_selector(|| "ex-open-as-root".into())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tree_menu = None;
                                this.open_as_root(&rel, cx);
                            })),
                    )
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.tree_menu = None;
                        cx.notify();
                    })),
            ),
        )
        .with_priority(2)
        .into_any_element()
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
        let switch = self.can_switch_root(cx);
        let hint_box = self.hint_box.clone();
        let sep = || div().flex_none().w(px(1.)).h(px(16.)).mx(px(2.)).bg(t.line);
        let stop = |d: gpui::Stateful<gpui::Div>| d.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        div()
            .id("ex-viewer-bar")
            .debug_selector(|| "ex-viewer-bar".into())
            // Its empty part drags the files (and a window of its own).
            .when(host != Host::Main, |d| d.cursor_grab())
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &gpui::MouseDownEvent, _, _| this.arm_bar(e)))
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
                    .cursor_pointer()
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
                            .relative()
                            .min_w_0()
                            .flex_shrink(1.)
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .font_family(MONO_FONT)
                            .text_size(px(11.5))
                            .text_color(t.text_3)
                            .child(place(hint_box))
                            .child(div().min_w_0().ellipsis().child(crate::kit::one_line(root)))
                            .map(|d| {
                                if switch {
                                    let open = self.root_menu.is_some();
                                    let hover = t.surface_3;
                                    d.px(px(4.))
                                        .mx(px(-4.))
                                        .rounded(RADIUS_SM)
                                        .cursor_pointer()
                                        .when(open, |d| d.bg(hover).text_color(t.text_2))
                                        .hover_probed(move |s| s.bg(hover))
                                        .child(icon("chevron-down", 10., t.text_4))
                                        .tooltip(move |_, cx| tooltip_view(format!("{root_full}\nClick to show another worktree or folder"), cx))
                                        .on_click(cx.listener(|this, e: &gpui::ClickEvent, window, cx| {
                                            if this.bar_dragged {
                                                return;
                                            }
                                            if this.root_menu.is_some() {
                                                this.close_root_menus(cx);
                                            } else {
                                                this.open_root_menu(Start::Worktrees, Some(e.position()), window, cx);
                                            }
                                        }))
                                } else {
                                    d.tooltip(move |_, cx| tooltip_view(root_full.clone(), cx))
                                }
                            })
                            // (After the hover style, which marks it in tests.)
                            .debug_selector(|| "ex-viewer-hint".into()),
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
            .child(div().flex_none().debug_selector(|| "ex-viewer-refresh".into()).cursor_default().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(refresh_control(
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
            )))
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
                                cx.emit(ViewerEvent::Place(super::dock::FilesPlace::Next(crate::main_screen::tree::Side::Right), None))
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
        tabs.push(
            div()
                .id("ex-tabs-rest")
                .debug_selector(|| "ex-tabs-rest".into())
                .flex_1()
                .min_w(px(8.))
                .h(px(32.))
                .border_b_1()
                .border_color(t.line)
                // Drags the files, as the bar does.
                .when(self.host != Host::Main, |d| d.cursor_grab())
                .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &gpui::MouseDownEvent, _, _| this.arm_bar(e)))
                .into_any_element(),
        );
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
        // (An SVG shown as its source is text too.)
        let is_text = view.is_some() && self.as_text(active, cx);
        let pixels = pixels.filter(|_| !is_text);
        let svg_switch = self.has_source(active).then(|| self.svg_source(active, cx));
        // A PDF shown: its page, zoom and search.
        let pdf = self.active_pdf().map(|(pd, pdf, zoom)| (pd.current, pdf.pages().len(), zoom, self.pdf_scale(zoom, &pdf), pd.find.is_some()));
        let here = doc.and_then(|d| d.local.clone()).filter(|_| pdf.is_some());
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
            .when_some(pdf, |d, (current, n, _, _, _)| {
                let label = if narrow { format!("{}/{n}", (current + 1).min(n)) } else { pdf_view::page_label(current, n) };
                d.child(meta(label).debug_selector(|| "ex-pdf-page".into()))
            })
            .child(div().flex_1())
            .when_some(zoom, |d, zoom| {
                let now = self.image_scale(zoom, pixels);
                d.child(self.zoom_controls(zoom, now, false, t, cx))
            })
            .when_some(pdf, |d, (_, _, zoom, now, finding)| {
                d.child(self.zoom_controls(zoom, now, true, t, cx)).child(
                    tool_btn("ex-pdf-find", "search", if narrow { "" } else { "Find" }, String::new(), finding, t)
                        .debug_selector(|| "ex-pdf-find".into())
                        .on_click(cx.listener(|this, _, window, cx| this.open_find(window, cx)))
                        .tooltip(|_, cx| tooltip_view(keys("Find in the PDF (⌘F)", false), cx)),
                )
            })
            .when_some(here, |d, here| {
                d.child(
                    tool_btn("ex-pdf-open-default", "external", "", String::new(), false, t)
                        .debug_selector(|| "ex-pdf-open-default".into())
                        .on_click(cx.listener(move |_, _, _, cx| crate::platform::files::open_in_default_app(cx, &here)))
                        .tooltip(|_, cx| tooltip_view("Open in default app", cx)),
                )
            })
            .when_some(svg_switch, |d, on| {
                let tip = |_: &mut Window, cx: &mut App| tooltip_view(keys("Switch between the picture and its source (⌘⇧V)", false), cx);
                d.child(
                    crate::kit::segments(t)
                        .debug_selector(|| "ex-svg-switch".into())
                        .child(crate::kit::segment("ex-svg-picture", "Picture".into(), !on, true, t).tooltip(tip).on_click(cx.listener(|v, _, _, cx| v.set_svg_source(false, cx))))
                        .child(crate::kit::segment("ex-svg-source", "Source".into(), on, false, t).tooltip(tip).on_click(cx.listener(|v, _, _, cx| v.set_svg_source(true, cx)))),
                )
            })
            .when(self.md_text().is_some(), |d| {
                let on = self.preview;
                d.child(
                    crate::kit::segments(t)
                        .debug_selector(|| "ex-md-switch".into())
                        .child(crate::kit::segment("ex-md-code", "Code".into(), !on, true, t).on_click(cx.listener(|v, _, _, cx| v.set_preview(false, cx))))
                        .child(crate::kit::segment("ex-md-preview", "Preview".into(), on, false, t).on_click(cx.listener(|v, _, _, cx| v.set_preview(true, cx)))),
                )
            })
            .when(is_text && self.local(cx) && !self.agent(cx).is_some_and(|a| crate::agents::mirrored(&a)), |d| {
                // The text editor, at the same line (docs/spec/editor.md).
                let (path, code) = (abs.to_string(), doc.and_then(|d| d.code.as_ref()).map(|c| c.code.clone()));
                d.child(
                    tool_btn("ex-open-in-editor", "file", if narrow { "" } else { "Open in editor" }, String::new(), false, t)
                        .debug_selector(|| "ex-open-in-editor".into())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let line = code.as_ref().and_then(|c| c.read(cx).caret()).map(|(l, _)| l as u32 + 1);
                            let path = path.clone();
                            cx.defer(move |cx| crate::main_screen::editor_tile::open_path(&path, line, cx));
                        }))
                        .tooltip(|_, cx| tooltip_view("Open this file in the text editor", cx)),
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

    /// An image's (or a PDF's: `pdf`) Fit / 100 % and − / + (and the zoom
    /// it is at, `now`). A PDF fits its width.
    fn zoom_controls(&self, zoom: Zoom, now: f32, pdf: bool, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let fit = zoom == Zoom::Fit;
        let actual = !fit && (now - 1.).abs() < 1e-3;
        let (zoom_out, zoom_in) = if pdf { (pdf_view::zoom_out(now), pdf_view::zoom_in(now)) } else { (image_view::zoom_out(now), image_view::zoom_in(now)) };
        let fit_tip = if pdf { "Fit the pages to the width" } else { "Fit the image to the pane" };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .debug_selector(|| "ex-image-zoom".into())
            .child(
                crate::kit::icon_btn("ex-zoom-out", "minus", "Zoom out", true, t)
                    .on_click(cx.listener(move |v, _, _, cx| v.set_zoom(Zoom::Scale(zoom_out), cx))),
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
                    .on_click(cx.listener(move |v, _, _, cx| v.set_zoom(Zoom::Scale(zoom_in), cx))),
            )
            .child(
                crate::kit::segments(t)
                    .child(crate::kit::segment("ex-zoom-fit", "Fit".into(), fit, true, t).tooltip(move |_, cx| tooltip_view(fit_tip, cx)).on_click(cx.listener(|v, _, _, cx| v.set_zoom(Zoom::Fit, cx))))
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

    /// The −, +, Fit and 100 % buttons: the zoom, about the view's centre.
    pub fn set_zoom(&mut self, zoom: Zoom, cx: &mut Context<Self>) {
        self.zoom_at(zoom, None, false, cx);
    }

    /// The active image or PDF at `zoom`, the point at `at` (window
    /// points; none: the view's centre) kept where it is. `gesture`: a
    /// pinch or ⌘-scroll, after which a PDF's pages are drawn again once
    /// it settles.
    fn zoom_at(&mut self, zoom: Zoom, at: Option<gpui::Point<Pixels>>, gesture: bool, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else { return };
        if self.active_pdf().is_some() {
            self.zoom_pdf(&active, zoom, at, gesture, cx);
            return;
        }
        let Some(d) = self.docs.get(&active) else { return };
        let pan = match (self.image_frame(d.zoom, d.pixels), self.image_frame(zoom, d.pixels)) {
            (Some((old, view)), Some((new, _))) => {
                let off = image_offset(d.pan, old, view);
                let at = in_pane(at, self.image_box.get(), view);
                let pad = image_view::PAD;
                Some((
                    pan_zoom::anchored(off.0, old.0, new.0, pad, view.0, at.0),
                    pan_zoom::anchored(off.1, old.1, new.1, pad, view.1, at.1),
                ))
            }
            _ => d.pan,
        };
        if let Some(d) = self.docs.get_mut(&active) {
            d.zoom = zoom;
            d.pan = pan;
        }
        cx.notify();
    }

    /// The active image drawn at `zoom`, and its pane, `(w, h)` points
    /// each (once laid out).
    fn image_frame(&self, zoom: Zoom, pixels: Option<(i32, i32)>) -> Option<((f32, f32), (f32, f32))> {
        let ((w, h), area) = (pixels?, self.image_area.get()?);
        let s = self.image_scale(zoom, pixels);
        Some((((w as f32 * s).max(1.), (h as f32 * s).max(1.)), (f32::from(area.width), f32::from(area.height))))
    }

    /// The active image drawn at `pixels`: a size other than the one it
    /// was drawn at (read again, changed on disk) centres it again; the
    /// same size keeps where it was moved.
    fn sync_pixels(&mut self, pixels: Option<(i32, i32)>) {
        let Some(now) = pixels else { return };
        let Some(d) = self.active.clone().and_then(|p| self.docs.get_mut(&p)) else { return };
        if d.pixels.is_some_and(|was| was != now) {
            d.pan = None;
        }
        d.pixels = Some(now);
    }

    /// A pinch over the image or the PDF: zoomed by it about the pointer.
    fn pinched(&mut self, e: &gpui::PinchEvent, cx: &mut Context<Self>) {
        // (A pinch's start and end carry no change: Fit stays Fit.)
        if let Some((now, (min, max))) = self.zoom_range().filter(|_| e.delta != 0.) {
            let z = Zoom::Scale(pan_zoom::pinch(now, e.delta, min, max));
            self.zoom_at(z, Some(e.position), true, cx);
        }
        if e.phase == gpui::TouchPhase::Ended {
            self.settled(cx);
        }
    }

    /// The active image's or PDF's scale now and the range a pinch or
    /// ⌘-scroll zooms it in.
    fn zoom_range(&self) -> Option<(f32, (f32, f32))> {
        if let Some((_, pdf, zoom)) = self.active_pdf() {
            let s = self.pdf_scale(zoom, &pdf);
            return Some((s, (pdf_view::STEPS[0], pdf_view::STEPS[pdf_view::STEPS.len() - 1])));
        }
        let d = self.docs.get(self.active.as_ref()?)?;
        let ((w, h), area) = (d.pixels?, self.image_area.get()?);
        let fitted = image_view::fit_scale((w as f32, h as f32), (f32::from(area.width), f32::from(area.height)));
        Some((self.image_scale(d.zoom, d.pixels), image_view::limits(fitted)))
    }

    /// A scroll over the image (a PDF scrolls itself): ⌘ (Ctrl) held,
    /// zoomed about the pointer; else the image moves, any way at once.
    /// Whether it did anything.
    fn image_scrolled(&mut self, e: &gpui::ScrollEvent, cx: &mut Context<Self>) -> bool {
        if e.modifiers.secondary(cx) {
            return self.wheel_zoom(e, cx);
        }
        let d = e.delta.pixel_delta(px(pan_zoom::LINE));
        let (mut dx, mut dy) = (f32::from(d.x), f32::from(d.y));
        // Shift and a wheel that only goes up and down: sideways.
        if e.modifiers.shift && dx == 0. && !e.delta.precise() {
            (dx, dy) = (dy, 0.);
        }
        let Some(active) = self.active.clone() else { return false };
        let Some((len, view)) = self.docs.get(&active).and_then(|d| self.image_frame(d.zoom, d.pixels)) else { return false };
        let Some(d) = self.docs.get_mut(&active) else { return false };
        let off = image_offset(d.pan, len, view);
        let pad = image_view::PAD;
        let to = (pan_zoom::clamp(off.0 - dx, len.0, pad, view.0), pan_zoom::clamp(off.1 - dy, len.1, pad, view.1));
        if to == off {
            return false;
        }
        d.pan = Some(to);
        cx.notify();
        true
    }

    /// ⌘ (Ctrl) + scroll over the image or the PDF: zoomed about the
    /// pointer (up: in).
    fn wheel_zoom(&mut self, e: &gpui::ScrollEvent, cx: &mut Context<Self>) -> bool {
        let d = e.delta.pixel_delta(px(pan_zoom::LINE));
        let dy = if d.y == px(0.) { f32::from(d.x) } else { f32::from(d.y) };
        let Some((now, (min, max))) = self.zoom_range() else { return false };
        self.zoom_at(Zoom::Scale(pan_zoom::wheel(now, dy, min, max)), Some(e.position), true, cx);
        true
    }

    /// A press on the image or the PDF (`pdf`) where it is larger than its
    /// pane: it may become a drag that moves it.
    fn pan_down(&mut self, e: &gpui::MouseDownEvent, pdf: bool, cx: &mut Context<Self>) {
        self.pan_moved = false;
        let from = if pdf { self.pdf_offset() } else { self.image_offset_now() };
        self.pan_press = from.map(|from| PanPress { at: e.position, from, pdf, moved: false });
        cx.notify();
    }

    /// The pointer moved with the picture pressed: past the threshold, the
    /// picture follows it, kept in view.
    fn pan_move(&mut self, e: &gpui::MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(mut p) = self.pan_press else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.pan_up(cx);
            return;
        }
        let (at, now) = ((f32::from(p.at.x), f32::from(p.at.y)), (f32::from(e.position.x), f32::from(e.position.y)));
        if !p.moved && !pan_zoom::dragged(at, now) {
            return;
        }
        p.moved = true;
        self.pan_press = Some(p);
        // (Its release is no click, whichever hears it first.)
        self.pan_moved = true;
        let want = (p.from.0 - (now.0 - at.0), p.from.1 - (now.1 - at.1));
        if p.pdf {
            self.set_pdf_offset(want);
        } else if let Some(active) = self.active.clone() {
            let frame = self.docs.get(&active).and_then(|d| self.image_frame(d.zoom, d.pixels));
            if let (Some((len, view)), Some(d)) = (frame, self.docs.get_mut(&active)) {
                let pad = image_view::PAD;
                d.pan = Some((pan_zoom::clamp(want.0, len.0, pad, view.0), pan_zoom::clamp(want.1, len.1, pad, view.1)));
            }
        }
        cx.notify();
    }

    /// The press let go (anywhere).
    fn pan_up(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.pan_press.take() {
            self.pan_moved = p.moved;
            cx.notify();
        }
    }

    /// How far the active image is moved, when it is larger than its pane.
    fn image_offset_now(&self) -> Option<(f32, f32)> {
        let d = self.docs.get(self.active.as_ref()?)?;
        let (len, view) = self.image_frame(d.zoom, d.pixels)?;
        let pad = image_view::PAD;
        let movable = pan_zoom::max_offset(len.0, pad, view.0) > 0. || pan_zoom::max_offset(len.1, pad, view.1) > 0.;
        movable.then(|| image_offset(d.pan, len, view))
    }

    /// How far the image or the PDF is moved, and whether the last press
    /// moved it (tests).
    #[cfg(test)]
    pub fn pan_state(&self) -> (Option<(f32, f32)>, bool) {
        let pan = match self.active_pdf() {
            Some(_) => self.pdf_offset(),
            None => self.active.as_ref().and_then(|p| self.docs.get(p)).and_then(|d| {
                let (len, view) = self.image_frame(d.zoom, d.pixels)?;
                Some(image_offset(d.pan, len, view))
            }),
        };
        (pan, self.pan_moved)
    }

    /// The active image's or PDF's scale (tests).
    #[cfg(test)]
    pub fn shown_scale(&self) -> Option<f32> {
        self.zoom_range().map(|r| r.0)
    }

    /// The active PDF is being zoomed by a gesture (tests).
    #[cfg(test)]
    pub fn pdf_zooming(&self) -> bool {
        self.active_pdf().is_some_and(|(pd, _, _)| pd.zooming)
    }

    // ── a PDF ───────────────────────────────────────────────────────────

    /// The active PDF, open, with its zoom.
    fn active_pdf(&self) -> Option<(&PdfDoc, Arc<Pdf>, Zoom)> {
        let d = self.docs.get(self.active.as_ref()?)?;
        let pd = d.pdf.as_ref()?;
        Some((pd, pd.open()?, d.zoom))
    }

    /// The scale the active PDF is shown at.
    fn pdf_scale(&self, zoom: Zoom, pdf: &Pdf) -> f32 {
        let (aw, _) = self.pdf_area.get().map(pdf_room).unwrap_or((800., 600.));
        pdf_view::scale(zoom, pdf.pages(), aw)
    }

    /// The active PDF at `zoom`, the place at `at` (window points; none:
    /// the view's centre) kept where it is. A gesture's pages are scaled
    /// as drawn until it settles (`settled`), then drawn again.
    fn zoom_pdf(&mut self, path: &str, zoom: Zoom, at: Option<gpui::Point<Pixels>>, gesture: bool, cx: &mut Context<Self>) {
        let area = self.pdf_area.get().map(pdf_room);
        let pane = self.pdf_box.get();
        let Some(d) = self.docs.get_mut(path) else { return };
        let old_zoom = std::mem::replace(&mut d.zoom, zoom);
        let Some((pd, pdf)) = d.pdf.as_mut().and_then(|pd| pd.open().map(|p| (pd, p))) else { return };
        if let Some((aw, ah)) = area {
            let pages = pdf.pages();
            let s0 = pd.scale.unwrap_or_else(|| pdf_view::scale(old_zoom, pages, aw));
            let s1 = pdf_view::scale(zoom, pages, aw);
            let (old, new) = (Layout::new(pages, s0), Layout::new(pages, s1));
            let at = in_pane(at, pane, (aw, ah));
            let off = pd.scroll.offset();
            let wide = pdf_view::widest(pages);
            let x = pan_zoom::anchored(-f32::from(off.x), wide * s0, wide * s1, pdf_view::PAD, aw, at.0);
            let y = pdf_view::anchored_top(&old, &new, -f32::from(off.y), at.1, ah);
            pd.scroll.set_offset(gpui::point(px(-x), px(-y)));
            // (Laid out at it already: `pdf_tick` keeps this place.)
            pd.scale = Some(s1);
        }
        if gesture {
            pd.zooming = true;
            let path = path.to_string();
            pd._settle = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(pan_zoom::SETTLE_MS)).await;
                let _ = this.update(cx, |this, cx| {
                    if this.active.as_deref() == Some(path.as_str()) {
                        this.settled(cx);
                    }
                });
            }));
        }
    }

    /// A pinch or ⌘-scroll over: the active PDF's pages drawn again at
    /// the scale it left them at.
    fn settled(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else { return };
        if let Some(pd) = self.docs.get_mut(&active).and_then(|d| d.pdf.as_mut()) {
            if pd.zooming {
                pd.zooming = false;
                pd._settle = None;
                cx.notify();
            }
        }
    }

    /// How far the active PDF is scrolled (x, y points).
    fn pdf_offset(&self) -> Option<(f32, f32)> {
        let (pd, _, _) = self.active_pdf()?;
        let off = pd.scroll.offset();
        Some((-f32::from(off.x), -f32::from(off.y)))
    }

    /// The active PDF scrolled to `to` (x, y points), as far as it goes.
    fn set_pdf_offset(&mut self, to: (f32, f32)) {
        let Some((aw, ah)) = self.pdf_area.get().map(pdf_room) else { return };
        let Some((pd, pdf, zoom)) = self.active_pdf() else { return };
        let s = pd.scale.unwrap_or_else(|| pdf_view::scale(zoom, pdf.pages(), aw));
        let layout = Layout::new(pdf.pages(), s);
        let x = to.0.clamp(0., (layout.width - aw).max(0.));
        let y = to.1.clamp(0., (layout.height - ah).max(0.));
        pd.scroll.set_offset(gpui::point(px(-x), px(-y)));
    }

    /// Before the active PDF is drawn: the place in view kept at a new
    /// scale, the pages in view drawn (nearest first, a few at a time),
    /// those far away let go over the budget. Other PDFs let all theirs go.
    fn pdf_tick(&mut self, cx: &mut Context<Self>) {
        let active = self.active.clone();
        let mut gone = Vec::new();
        for (p, d) in self.docs.iter_mut() {
            if Some(p) != active.as_ref() {
                gone.extend(d.release_pdf());
            }
        }
        let (area, dpr) = (self.pdf_area.get(), self.dpr);
        let doc = active.as_ref().zip(area).and_then(|(a, area)| Some((a.clone(), area, self.docs.get_mut(a)?)));
        if let Some((active, area, d)) = doc {
            let zoom = d.zoom;
            if let Some((pd, pdf)) = d.pdf.as_mut().and_then(|pd| pd.open().map(|p| (pd, p))) {
                let (aw, ah) = pdf_room(area);
                let s = pdf_view::scale(zoom, pdf.pages(), aw);
                let layout = Layout::new(pdf.pages(), s);
                if let Some(old) = pd.scale.filter(|o| (o - s).abs() > 1e-4) {
                    // The same place at the new scale.
                    let off = pd.scroll.offset();
                    let top = layout.top_of(Layout::new(pdf.pages(), old).anchor(-f32::from(off.y)));
                    pd.scroll.set_offset(gpui::point(off.x * (s / old), px(-top)));
                }
                pd.scale = Some(s);
                let top = -f32::from(pd.scroll.offset().y);
                let keep = layout.visible(top, ah, pdf_view::MARGIN);
                pd.current = layout.current(top, ah);
                let kept: Vec<(usize, usize)> = pd.pages.iter().map(|(p, i)| (*p, i.w as usize * i.h as usize * 4)).collect();
                for p in pdf_view::evict(&kept, keep.clone(), pdf_view::BUDGET) {
                    gone.extend(pd.pages.remove(&p).map(|i| i.image));
                }
                for p in pdf_view::wanted(keep, pd.current) {
                    if pd.pending.len() >= pdf_view::IN_FLIGHT {
                        break;
                    }
                    let (w, h) = pdf_view::render_px(pdf.pages()[p], s, dpr);
                    if pd.pending.contains(&p) || pd.failed.contains(&p) || !pdf_view::render_now(pd.pages.get(&p).map(|i| i.w), w, pd.zooming) {
                        continue;
                    }
                    pd.pending.insert(p);
                    let engine = pdf.clone();
                    let job = cx.background_executor().spawn(async move { engine.render(p, w, h) });
                    let (path, pdf) = (active.clone(), pdf.clone());
                    cx.spawn(async move |this, cx| {
                        let res = job.await;
                        let _ = this.update(cx, |this, cx| this.page_drawn(&path, &pdf, p, res, cx));
                    })
                    .detach();
                }
            }
        }
        drop_images(gone, cx);
    }

    /// Page `page` of `pdf` drawn (or why not).
    fn page_drawn(&mut self, path: &str, pdf: &Arc<Pdf>, page: usize, res: Result<crate::platform::pdf::Bitmap, String>, cx: &mut Context<Self>) {
        let mut gone = Vec::new();
        let pd = self.docs.get_mut(path).and_then(|d| d.pdf.as_mut()).filter(|pd| pd.open().is_some_and(|p| Arc::ptr_eq(&p, pdf)));
        if let Some(pd) = pd {
            pd.pending.remove(&page);
            match res.map(page_image) {
                Ok(Some(img)) => gone.extend(pd.pages.insert(page, img).map(|old| old.image)),
                _ => {
                    pd.failed.insert(page);
                }
            }
        }
        drop_images(gone, cx);
        cx.notify();
    }

    /// ⌘F on a PDF: its search, the field taking the keyboard.
    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else { return };
        let Some(pd) = self.docs.get_mut(&active).and_then(|d| d.pdf.as_mut()).filter(|pd| pd.open().is_some()) else { return };
        if let Some(f) = &pd.find {
            f.field.update(cx, |t, cx| {
                t.select_all_text(cx);
                t.focus(window, cx);
            });
            return;
        }
        let field = cx.new(|cx| TextInput::new(cx, "", "Find in PDF").plain());
        let path = active.clone();
        let sub = cx.subscribe(&field, move |this, field, e: &InputEvent, cx| match e {
            InputEvent::Changed => {
                let q = field.read(cx).text().to_string();
                this.set_find_query(&path, q, cx);
            }
            InputEvent::Submit => this.find_step(&path, true, cx),
            InputEvent::SubmitShift => this.find_step(&path, false, cx),
            InputEvent::Cancel => this.close_find(&path, cx),
            InputEvent::Blur => {}
        });
        pd.find = Some(PdfFind { field: field.clone(), query: String::new(), hits: Vec::new(), current: None, searching: false, _wait: None, _search: None, _sub: sub });
        field.update(cx, |t, cx| t.focus(window, cx));
        cx.notify();
    }

    fn pdf_find_mut(&mut self, path: &str) -> Option<&mut PdfFind> {
        self.docs.get_mut(path)?.pdf.as_mut()?.find.as_mut()
    }

    /// The search's text changed: searched once typing settles.
    fn set_find_query(&mut self, path: &str, q: String, cx: &mut Context<Self>) {
        let Some(f) = self.pdf_find_mut(path) else { return };
        if f.query == q {
            return;
        }
        f.query = q;
        let p = path.to_string();
        f._wait = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let _ = this.update(cx, |this, cx| this.run_find(&p, true, cx));
        }));
        cx.notify();
    }

    /// Search the PDF for the search's text (`jump`: show the first hit
    /// from the page being read).
    fn run_find(&mut self, path: &str, jump: bool, cx: &mut Context<Self>) {
        let Some(pd) = self.docs.get_mut(path).and_then(|d| d.pdf.as_mut()) else { return };
        let (Some(pdf), from) = (pd.open(), pd.current) else { return };
        let Some(f) = pd.find.as_mut() else { return };
        let query = f.query.clone();
        if query.trim().is_empty() {
            f.hits.clear();
            f.current = None;
            f.searching = false;
            f._search = None;
            cx.notify();
            return;
        }
        f.searching = true;
        let q = query.clone();
        let job = cx.background_executor().spawn(async move { pdf.find(&q) });
        let p = path.to_string();
        f._search = Some(cx.spawn(async move |this, cx| {
            let hits = job.await;
            let _ = this.update(cx, |this, cx| {
                let Some(f) = this.pdf_find_mut(&p).filter(|f| f.query == query) else { return };
                f.searching = false;
                let pages: Vec<usize> = hits.iter().map(|h| h.page).collect();
                f.current = if jump { pdf_view::first_from(&pages, from) } else { f.current.filter(|c| *c < hits.len()) };
                f.hits = hits;
                if jump {
                    this.show_hit(&p, cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// ↵ / ⇧↵ in the search: the next or previous hit, round the ends.
    fn find_step(&mut self, path: &str, forward: bool, cx: &mut Context<Self>) {
        let Some(f) = self.pdf_find_mut(path) else { return };
        f.current = pdf_view::step(f.current, f.hits.len(), forward);
        self.show_hit(path, cx);
        cx.notify();
    }

    /// Esc in the search: it closes, the keyboard back to the viewer.
    fn close_find(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(pd) = self.docs.get_mut(path).and_then(|d| d.pdf.as_mut()) {
            pd.find = None;
        }
        self.refocus = true;
        cx.notify();
    }

    /// Scroll the search's hit into view (when it isn't).
    fn show_hit(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some((aw, ah)) = self.pdf_area.get().map(pdf_room) else { return };
        let Some(d) = self.docs.get_mut(path) else { return };
        let zoom = d.zoom;
        let Some(pd) = d.pdf.as_mut() else { return };
        let Some(pdf) = pd.open() else { return };
        let Some(hit) = pd.find.as_ref().and_then(|f| f.hits.get(f.current?)).cloned() else { return };
        let s = pd.scale.unwrap_or_else(|| pdf_view::scale(zoom, pdf.pages(), aw));
        let layout = Layout::new(pdf.pages(), s);
        let Some(page_top) = layout.tops.get(hit.page).copied() else { return };
        let [x, y, w, h] = hit.rect.map(|v| v * s);
        let off = pd.scroll.offset();
        let (top, left) = (-f32::from(off.y), -f32::from(off.x));
        let (hit_top, hit_left) = (page_top + y, layout.left(hit.page, aw) + x);
        let new_top = if hit_top < top || hit_top + h > top + ah { layout.top_showing(hit.page, y, ah) } else { top };
        let new_left = if hit_left < left || hit_left + w > left + aw { (hit_left - aw / 3.).max(0.) } else { left };
        pd.scroll.set_offset(gpui::point(px(-new_left), px(-new_top)));
        cx.notify();
    }

    /// Debug builds: the active PDF's search (`find <text>`, `next`,
    /// `prev`, `close`) or scrolled to a page (`page <n>`, 1-based).
    pub fn debug_pdf(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else { return };
        let (verb, arg) = what.split_once(' ').unwrap_or((what, ""));
        match verb {
            "find" => {
                self.open_find(window, cx);
                if let Some(f) = self.pdf_find_mut(&active) {
                    let field = f.field.clone();
                    field.update(cx, |t, cx| t.set_text(arg.to_string(), cx));
                }
                self.set_find_query(&active, arg.to_string(), cx);
            }
            "next" => self.find_step(&active, true, cx),
            "prev" => self.find_step(&active, false, cx),
            "close" => self.close_find(&active, cx),
            "page" => {
                let n = arg.trim().parse::<usize>().unwrap_or(1).max(1) - 1;
                let Some((pd, pdf, zoom)) = self.active_pdf() else { return };
                let layout = Layout::new(pdf.pages(), self.pdf_scale(zoom, &pdf));
                if let Some(t) = layout.tops.get(n) {
                    pd.scroll.set_offset(gpui::point(px(0.), px(-(t - pdf_view::PAD))));
                }
                cx.notify();
            }
            _ => eprintln!("pitwall-debug: pdf {verb}?"),
        }
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
        // An SVG shown as its source: as a text file.
        let source = v.kind == ContentKind::Image && self.as_text(&active, cx);
        match v.kind {
            ContentKind::Binary if is_pdf(&active) => self.pdf_body(&active, &v, here, t, cx),
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
            ContentKind::Image if !source => match doc.and_then(|d| d.image.clone()) {
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
            ContentKind::Text | ContentKind::Image => {
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

    /// A PDF: its pages (macOS), or why they aren't shown, with "Open in
    /// default app" for a file on this computer.
    fn pdf_body(&self, active: &str, v: &Loaded, here: Option<std::path::PathBuf>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let pd = self.docs.get(active).and_then(|d| d.pdf.as_ref());
        let why = if !crate::platform::can_view_pdf() {
            Some(format!("A PDF of {}. {PDF_UNSUPPORTED}", size_label(v.size)))
        } else if v.size > MAX_PDF {
            Some(format!("A PDF of {}, larger than {}: not shown here.", size_label(v.size), size_label(MAX_PDF)))
        } else {
            match pd.map(|p| &p.state) {
                Some(PdfState::Failed(e)) => Some(e.clone()),
                Some(PdfState::Open(pdf)) => return self.pdf_pages(pd.expect("open"), pdf, t, cx),
                _ => None,
            }
        };
        let Some(why) = why else {
            return div().flex_1().flex().items_center().justify_center().text_size(px(12.5)).text_color(t.text_3).child("Reading…").into_any_element();
        };
        div()
            .id("ex-pdf-not-shown")
            .debug_selector(|| "ex-pdf-not-shown".into())
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .text_size(px(12.5))
            .child(file_icon(active, 28.))
            .child(div().max_w(px(420.)).text_color(t.text_2).child(crate::kit::para(why)))
            .when_some(here, |d, here| {
                d.child(
                    small_btn("ex-open-default", "Open in default app", t)
                        .on_click(cx.listener(move |_, _, _, cx| crate::platform::files::open_in_default_app(cx, &here)))
                        .tooltip(|_, cx| tooltip_view("Open it with the app your system uses for it", cx)),
                )
            })
            .into_any_element()
    }

    /// A PDF's pages, one under the other as white paper on the pane's
    /// ground; only those in view (and around) are laid out, each drawn
    /// once the engine has drawn it (blank paper until then), with the
    /// search's hits marked.
    fn pdf_pages(&self, pd: &PdfDoc, pdf: &Pdf, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let zoom = self.active.as_ref().and_then(|p| self.docs.get(p)).map(|d| d.zoom).unwrap_or_default();
        let (aw, ah) = self.pdf_area.get().map(pdf_room).unwrap_or((800., 600.));
        let s = pdf_view::scale(zoom, pdf.pages(), aw);
        let layout = Layout::new(pdf.pages(), s);
        let top = -f32::from(pd.scroll.offset().y);
        let find = pd.find.as_ref();
        let hits = find.map(|f| f.hits.as_slice()).unwrap_or_default();
        let shown_hit = find.and_then(|f| f.current);
        let pages: Vec<AnyElement> = layout
            .visible(top, ah, pdf_view::MARGIN)
            .map(|i| {
                let (w, h) = layout.sizes[i];
                let picture = pd.pages.get(&i).map(|p| p.image.clone());
                let from = hits.partition_point(|h| h.page < i);
                let marks: Vec<AnyElement> = hits[from..]
                    .iter()
                    .take_while(|h| h.page == i)
                    .enumerate()
                    .map(|(k, h)| {
                        let [x, y, hw, hh] = h.rect.map(|v| v * s);
                        div()
                            .absolute()
                            .left(px(x - 1.))
                            .top(px(y - 1.))
                            .w(px(hw + 2.))
                            .h(px(hh + 2.))
                            .rounded(px(2.))
                            .bg(if shown_hit == Some(from + k) { HIT_ON } else { HIT })
                            .into_any_element()
                    })
                    .collect();
                div()
                    .absolute()
                    .left(px(layout.left(i, aw)))
                    .top(px(layout.tops[i]))
                    .w(px(w))
                    .h(px(h))
                    .when(i == 0, |d| d.debug_selector(|| "ex-pdf-first".into()))
                    .bg(gpui::white())
                    .shadow_sm()
                    .when_some(picture, |d, picture| {
                        d.child(
                            canvas(
                                |_, _, _| {},
                                move |bounds, _, window, _| {
                                    let _ = window.paint_image(bounds, bounds, Default::default(), picture.clone(), 0, false);
                                },
                            )
                            .absolute()
                            .inset_0(),
                        )
                    })
                    .children(marks)
                    .into_any_element()
            })
            .collect();
        // Larger than the pane (either way): pressed and dragged, it moves.
        let movable = layout.width > aw || layout.height > ah;
        let dragging = self.pan_press.is_some_and(|p| p.pdf && p.moved);
        let mut scroll = div()
            .id("ex-pdf-scroll")
            .size_full()
            .overflow_scroll()
            .track_scroll(&pd.scroll)
            .when(movable, |d| d.cursor(if dragging { gpui::CursorStyle::ClosedHand } else { gpui::CursorStyle::OpenHand }))
            // A click gives the viewer the keyboard (⌘F); a drag moves the pages.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.pan_down(e, true, cx);
                }),
            )
            .child(
                div()
                    .relative()
                    .w(px(layout.width.max(aw)))
                    .h(px(layout.height))
                    .min_h_full()
                    // ⌘ (Ctrl) + scroll zooms, heard here before the pane
                    // scrolls (which it then doesn't).
                    .on_scroll(cx.listener(|v, e: &gpui::ScrollEvent, _, cx| {
                        if e.modifiers.secondary(cx) && v.wheel_zoom(e, cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .children(pages),
            )
            .when(self.pan_press.is_some_and(|p| p.pdf), |d| d.child(self.pan_listener(cx)));
        // Diagonal scrolls move the pages both ways at once.
        scroll.style().allow_concurrent_scroll = Some(true);
        div()
            .id("ex-pdf")
            .debug_selector(|| "ex-pdf".into())
            .relative()
            .flex_1()
            .min_h_0()
            .size_full()
            .bg(t.surface_2)
            .child(self.measure(self.pdf_area.clone(), cx))
            .child(place(self.pdf_box.clone()))
            .child(self.pinch_target(cx))
            .child(crate::kit::vscroll_fill("ex-pdf-bar", &pd.scroll, t, scroll).size_full())
            .when_some(find, |d, f| d.child(self.find_bar(f, t, cx)))
            .into_any_element()
    }

    /// A PDF's search: the field, how many were found, previous / next
    /// and close, floating over the pages' top right.
    fn find_bar(&self, f: &PdfFind, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let count = if f.searching {
            "…".to_string()
        } else if f.query.trim().is_empty() {
            String::new()
        } else if f.hits.is_empty() {
            "No results".to_string()
        } else {
            let more = if f.hits.len() >= crate::platform::pdf::MAX_HITS { "+" } else { "" };
            format!("{} of {}{more}", f.current.map_or(0, |c| c + 1), f.hits.len())
        };
        let path = self.active.clone().unwrap_or_default();
        let (p1, p2, p3) = (path.clone(), path.clone(), path);
        div()
            .id("ex-pdf-findbar")
            .debug_selector(|| "ex-pdf-findbar".into())
            .absolute()
            .top(px(8.))
            .right(px(PDF_GUTTER + 8.))
            .w(px(340.))
            .max_w(gpui::relative(0.9))
            .h(px(32.))
            .pl(px(8.))
            .pr(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(RADIUS)
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .shadow_md()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(icon("search", 13., t.text_3))
            .child(div().flex_1().min_w_0().text_size(px(12.5)).child(f.field.clone()))
            .child(div().flex_none().font_family(MONO_FONT).text_size(px(11.)).text_color(t.text_3).debug_selector(|| "ex-pdf-count".into()).child(count))
            .child(crate::kit::icon_btn("ex-pdf-prev", "chevron-up", "Previous (⇧↵)", true, t).on_click(cx.listener(move |this, _, _, cx| this.find_step(&p1, false, cx))))
            .child(crate::kit::icon_btn("ex-pdf-next", "chevron-down", "Next (↵)", true, t).on_click(cx.listener(move |this, _, _, cx| this.find_step(&p2, true, cx))))
            .child(crate::kit::icon_btn("ex-pdf-find-close", "x", "Close (Esc)", true, t).on_click(cx.listener(move |this, _, _, cx| this.close_find(&p3, cx))))
    }

    /// The image at its zoom, centred while smaller than the pane and,
    /// when larger, moved by a drag or a scroll ([`pan_zoom`]), over a
    /// checkerboard where it is transparent. A pinch or ⌘-scroll zooms
    /// about the pointer; a double click goes between Fit and 100 % about
    /// the point clicked.
    fn image(&self, image: Arc<gpui::Image>, zoom: Zoom, pixels: Option<(i32, i32)>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let err = t.text_3;
        let picture = || {
            gpui::img(gpui::ImageSource::Image(image.clone()))
                .size_full()
                .object_fit(gpui::ObjectFit::Fill)
                .with_fallback(move || div().text_size(px(12.5)).text_color(err).child("This image can't be drawn").into_any_element())
        };
        let d = self.active.as_ref().and_then(|p| self.docs.get(p));
        let frame = pixels.and_then(|_| self.image_frame(zoom, pixels));
        let pad = image_view::PAD;
        // Larger than the pane (either way): it can be moved.
        let movable = frame.is_some_and(|(len, view)| pan_zoom::max_offset(len.0, pad, view.0) > 0. || pan_zoom::max_offset(len.1, pad, view.1) > 0.);
        let inner = match frame {
            Some((len, view)) => {
                let off = image_offset(d.and_then(|d| d.pan), len, view);
                div()
                    .absolute()
                    .left(px(pan_zoom::start(off.0, len.0, pad, view.0)))
                    .top(px(pan_zoom::start(off.1, len.1, pad, view.1)))
                    .w(px(len.0))
                    .h(px(len.1))
                    .debug_selector(|| "ex-image-picture".into())
                    .border_1()
                    .border_color(t.line)
                    .child(image_view::checkerboard(t.surface, t.surface_3).absolute().inset_0())
                    .child(picture())
                    .into_any_element()
            }
            // Not laid out yet (a frame): fitted by the image itself.
            None => div().size_full().p(px(pad)).child(picture().object_fit(gpui::ObjectFit::ScaleDown)).into_any_element(),
        };
        let fit = zoom == Zoom::Fit;
        let dragging = self.pan_press.is_some_and(|p| !p.pdf && p.moved);
        div()
            .id("ex-image")
            .debug_selector(|| "ex-image".into())
            .relative()
            .flex_1()
            .min_h_0()
            .size_full()
            .overflow_hidden()
            .bg(t.surface_2)
            .when(movable, |d| d.cursor(if dragging { gpui::CursorStyle::ClosedHand } else { gpui::CursorStyle::OpenHand }))
            .child(self.measure(self.image_area.clone(), cx))
            .child(place(self.image_box.clone()))
            .on_mouse_down(MouseButton::Left, cx.listener(|v, e: &gpui::MouseDownEvent, _, cx| v.pan_down(e, false, cx)))
            .on_scroll(cx.listener(|v, e: &gpui::ScrollEvent, _, cx| {
                if v.image_scrolled(e, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(self.pinch_target(cx))
            // A double click: Fit or 100 %, the point clicked kept under
            // the pointer (not the end of a drag).
            .on_click(cx.listener(move |v, e: &gpui::ClickEvent, _, cx| {
                if e.click_count() == 2 && !v.pan_moved {
                    v.zoom_at(if fit { Zoom::Scale(1.) } else { Zoom::Fit }, Some(e.position()), false, cx);
                }
            }))
            .child(inner)
            .when(self.pan_press.is_some_and(|p| !p.pdf), |d| d.child(self.pan_listener(cx)))
            .into_any_element()
    }

    /// Pinches over the pane it fills. (Not `on_pinch`: that hears only a
    /// hovered element, and nothing is hovered after a key until the
    /// pointer moves; a pinch, like a scroll, goes to what is under the
    /// pointer whatever was typed last.)
    fn pinch_target(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let me = cx.entity().downgrade();
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                let v = me.clone();
                window.on_mouse_event(move |e: &gpui::PinchEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                        let _ = v.update(cx, |v, cx| v.pinched(e, cx));
                    }
                });
            },
        )
        .absolute()
        .inset_0()
    }

    /// While the picture is pressed: the window's own mouse events,
    /// wherever the pointer is (past the pane, the window's edge: the
    /// release there ends it), and the closed hand while it moves.
    fn pan_listener(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let me = cx.entity().downgrade();
        let moving = self.pan_press.is_some_and(|p| p.moved);
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                if moving {
                    window.set_window_cursor_style(gpui::CursorStyle::ClosedHand);
                }
                let v = me.clone();
                window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble {
                        let _ = v.update(cx, |v, cx| v.pan_move(e, cx));
                    }
                });
                let v = me.clone();
                window.on_mouse_event(move |e: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble && e.button == MouseButton::Left {
                        let _ = v.update(cx, |v, cx| v.pan_up(cx));
                    }
                });
            },
        )
        .size_0()
    }

    /// The changed-on-disk question, the save error, under the header.
    fn notices(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(e) = self.root_error.clone() {
            out.push(
                Self::notice("ex-root-refused", true, e.into(), t)
                    .child(
                        small_btn("ex-root-refused-ok", "OK", t)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.root_error = None;
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
        }
        let Some(d) = self.active.as_ref().and_then(|p| self.docs.get(p)) else { return out };
        let (blocked, diffing) = (d.save_blocked, d.diff.is_some());
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
        self.sync_svg(cx);
        self.dpr = window.scale_factor();
        self.pdf_tick(cx);
        let active = self.active.clone();
        let pixels = self.image_pixels(window, cx);
        self.sync_pixels(pixels);
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
            .on_action(cx.listener(|this, _: &SwitchView, _, cx| this.switch_view(cx)))
            .on_action(cx.listener(|this, _: &FindInPdf, window, cx| this.open_find(window, cx)))
            .on_action(|_: &NextTab, window, cx| window.focus_next(cx))
            .on_action(|_: &PrevTab, window, cx| window.focus_prev(cx))
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| this.bar_moved(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.bar_press = None))
            .when_some(self.place_menu.clone(), |d, (at, replace)| d.child(self.place_menu_overlay(at, replace, &t, cx)))
            .when_some(self.root_menu.as_ref().map(|(at, m, _)| (*at, m.clone())), |d, (at, m)| d.child(self.root_menu_overlay(at, m, cx)))
            .when_some(self.tree_menu.clone(), |d, (at, rel)| d.child(self.tree_menu_overlay(at, rel, &t, cx)))
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
