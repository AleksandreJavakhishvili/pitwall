//! The C4 tile (plan.md phase B): agents show Mermaid diagrams with
//! `c4__show` and the tile draws them next to their terminal, natively,
//! with click to zoom across the diagrams an agent showed, "Changes since
//! last show", a box's details in the footer and the source on request.
//! Built only on the public Tile SDK.
//!
//! Shown diagrams are kept in the tile's storage, one key per diagram
//! (`d/<id>`), per the calling agent's project (FR-41); each project has
//! its own [`Index`], where a shown diagram's "file" is its agent
//! (`shown/<agent>`), so zoom prefers the same agent's diagrams (FR-13).
//!
//! Diagrams in the docs (plan.md phase C) come from the view's project
//! with `files.read`: the Markdown files the path list names, read once,
//! then followed with `files.watch` (FR-1 to FR-4). They join the same
//! project's [`Index`] under `<folder>/<path>`, so shown and docs diagrams
//! zoom into each other, and are items too, never stored (`doc` set).

use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use pitwall_extension::blocks::Mark;
use pitwall_extension::files::{self, Root};
use pitwall_extension::widgets::{Banner, Popover};
use pitwall_extension::prelude::*;
use pitwall_extension::style::Align;
use pitwall_extension::tools::{Call, Problem, Reply, ToolError};
use serde::{Deserialize, Serialize};

use crate::c4::{name_key, Level, Structure};
use crate::changes::{self, Drawn, Merged};
use crate::docs;
use crate::index::{Entry, Index, ZoomTarget};
use crate::layout::sequence::{self as seq_layout, SeqPlaced};
use crate::layout::{build, Dagre, Layout, Placed, SizedBox};
use crate::mermaid::{Diagram, Sequence, Skipped};
use crate::paint::{self, HitBox, Look};
use crate::show;

/// Shown diagrams kept per project (FR-41, a proposal).
pub const MAX_PER_PROJECT: usize = 50;
/// Unopened this long, a diagram goes first when a project is full.
const WEEK_MS: u64 = 7 * 24 * 3_600_000;
/// Storage keys of shown diagrams.
const PREFIX: &str = "d/";
/// The bar's height, as Pitwall's own tiles' bars.
const BAR: f32 = 36.;
/// The footer's height, when it shows.
const FOOTER: f32 = 34.;
/// From this width the outline shows beside the drawing.
const OUTLINE_AT: f32 = 640.;
const OUTLINE_W: f32 = 210.;
/// Zoom steps of the keys and buttons.
const ZOOM_STEP: f32 = 1.25;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

// docs:start c4-show-input
/// `c4__show`'s input (extension.json; `item`, `where` and `of` are
/// Pitwall's).
#[derive(Deserialize)]
pub struct ShowInput {
    mermaid: String,
    title: Option<String>,
}
// docs:end

/// `c4__open`'s input (input.schema.json).
#[derive(Deserialize, Default)]
struct OpenInput {
    diagram: Option<String>,
}

/// A shown diagram as kept in storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Item {
    /// The calling agent's project folder ("" for none).
    project: String,
    agent_id: String,
    agent_name: String,
    /// The agent's id for it.
    item: String,
    title: String,
    mermaid: String,
    /// The Mermaid before the last update, for "Changes since last show"
    /// (FR-40).
    #[serde(default)]
    previous: Option<String>,
    shown_at: u64,
    opened_at: u64,
    /// A diagram in the docs: where it is (never stored).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    doc: Option<Doc>,
}

/// Where a docs diagram is in its project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Doc {
    /// The file, relative to the project's folder.
    path: String,
    /// Its opening fence's line.
    line: u32,
}

/// A view's project, as its docs were read.
#[derive(Debug, Default)]
struct ViewDocs {
    folder: String,
    name: String,
    watch: Option<files::WatchId>,
    /// Files read, with their size and time, to skip unchanged ones.
    read: BTreeMap<String, (u64, u64)>,
    /// A limit reached or a refusal, said in the outline.
    note: Option<String>,
}

/// A diagram's id: a hash of its project, agent and item.
fn id_of(project: &str, agent: &str, item: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in project.bytes().chain([0]).chain(agent.bytes()).chain([0]).chain(item.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// A diagram laid out, ready to paint.
enum Drawing {
    Structure { s: Structure, sized: Vec<SizedBox>, placed: Placed },
    Changes { merged: Merged, sized: Vec<SizedBox>, placed: Placed },
    Sequence { seq: Sequence, placed: SeqPlaced },
}

impl Drawing {
    fn bounds(&self) -> (f32, f32, f32, f32) {
        match self {
            Drawing::Structure { placed, .. } | Drawing::Changes { placed, .. } => paint::bounds(placed),
            Drawing::Sequence { placed, .. } => paint::sequence_bounds(placed),
        }
    }

    fn boxes(&self) -> Vec<crate::layout::Rect> {
        match self {
            Drawing::Structure { placed, .. } | Drawing::Changes { placed, .. } => placed.nodes.clone(),
            Drawing::Sequence { placed, .. } => placed.headers.iter().map(|h| h.rect).collect(),
        }
    }
}

/// A call whose Mermaid didn't read: kept to show the user, until the
/// agent shows that item again or the user closes it.
#[derive(Debug, Clone)]
struct Notice {
    agent_id: String,
    agent_name: String,
    item: Option<String>,
    title: Option<String>,
    message: String,
    problems: Vec<Problem>,
    source: String,
}

/// One view's place: its diagram, the zoom path above it, its mode and
/// its pan and zoom per diagram (FR-25).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ViewState {
    open: Option<String>,
    path: Vec<String>,
    #[serde(default)]
    changes: bool,
    #[serde(default)]
    source: bool,
    /// The user's choice; `None` follows the width.
    #[serde(default)]
    outline: Option<bool>,
    #[serde(default)]
    places: HashMap<String, (f32, f32, f32)>,
    #[serde(skip)]
    viewport: Viewport,
    #[serde(skip)]
    selected: Option<usize>,
    /// A box detailed by several diagrams: the menu to pick one.
    #[serde(skip)]
    menu: Option<Vec<String>>,
    /// The problems of notice `n`, with its source.
    #[serde(skip)]
    notice_open: Option<usize>,
    #[serde(skip)]
    size: (f32, f32),
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    views: BTreeMap<String, ViewState>,
}

pub struct C4 {
    items: BTreeMap<String, Item>,
    /// By project.
    indexes: HashMap<String, Index>,
    /// By (id, Changes view).
    drawings: HashMap<(String, bool), Drawing>,
    views: BTreeMap<String, ViewState>,
    restored: BTreeMap<String, ViewState>,
    notices: Vec<Notice>,
    /// By view: its project's docs.
    docs: BTreeMap<String, ViewDocs>,
}

impl Tile for C4 {
    fn new(cx: &mut Cx<Self>) -> Self {
        let items: BTreeMap<String, Item> = match storage::entries_json::<Item>(PREFIX) {
            Ok(all) => all.into_iter().filter_map(|(k, v)| Some((k.strip_prefix(PREFIX)?.to_string(), v))).collect(),
            Err(e) => {
                log::error(format!("the shown diagrams couldn't be read: {e}"));
                BTreeMap::new()
            }
        };
        let mut indexes: HashMap<String, Index> = HashMap::new();
        for (id, it) in &items {
            indexes.entry(it.project.clone()).or_default().set_diagram(&path_of(&it.agent_id), id, Some(&it.title), &it.mermaid);
        }
        let restored = cx.restore::<Saved>().map(|s| s.views).unwrap_or_default();
        C4 { items, indexes, drawings: HashMap::new(), views: BTreeMap::new(), restored, notices: Vec::new(), docs: BTreeMap::new() }
    }

    fn on_view_opened(&mut self, view: &ViewInfo, _cx: &mut Cx<Self>) {
        self.scan(&view.id);
        let mut vs = self.restored.remove(&view.id).unwrap_or_default();
        if vs.open.as_ref().is_some_and(|id| !self.items.contains_key(id)) {
            vs.open = None;
            vs.path.clear();
        }
        vs.path.retain(|id| self.items.contains_key(id));
        if vs.open.is_none() {
            vs.open = self.items.iter().filter(|(_, it)| it.doc.is_none()).max_by_key(|(_, it)| it.shown_at).map(|(id, _)| id.clone()).or_else(|| self.first_doc(&view.id));
        }
        if let Some(id) = vs.open.clone() {
            if let Some((s, x, y)) = vs.places.get(&id).copied() {
                vs.viewport.scale = s;
                vs.viewport.x = x;
                vs.viewport.y = y;
            }
            self.ensure(&id, vs.changes);
        }
        self.views.insert(view.id.clone(), vs);
    }

    fn on_content(&mut self, view: &ViewInfo, _cx: &mut Cx<Self>) {
        let Some(content) = &view.content else { return };
        let input: OpenInput = content.parse().unwrap_or_default();
        let Some(wanted) = input.diagram.filter(|d| !d.trim().is_empty()) else { return };
        let agent = content.caller().map(|c| c.agent_id.clone());
        let folder = self.docs.get(&view.id).map(|d| d.folder.clone());
        let in_docs = self.items.iter().find(|(_, it)| it.doc.is_some() && it.item == wanted && Some(&it.project) == folder.as_ref()).map(|(id, _)| id.clone());
        let found = in_docs.or_else(|| self.items.iter().filter(|(_, it)| it.doc.is_none() && it.item == wanted && agent.as_ref().is_none_or(|a| *a == it.agent_id)).max_by_key(|(_, it)| it.shown_at).map(|(id, _)| id.clone()));
        match found {
            Some(id) => {
                self.open(&view.id, &id, false);
                if let Some(call) = content.call() {
                    let title = self.items.get(&id).map(|i| i.title.clone()).unwrap_or_default();
                    pitwall_extension::tools::reply(call, Ok(Reply::text(format!("Showing “{title}” in the C4 tile.")).item(wanted)));
                }
            }
            None => {
                if let Some(call) = content.call() {
                    pitwall_extension::tools::reply(call, Err(ToolError::plain(format!("No diagram “{wanted}” of yours is shown in the C4 tile (it was closed, or never shown), nor is one in the project's docs (as docs/hla.md#containers). Show it with c4__show."))));
                }
            }
        }
    }

    fn on_view_closed(&mut self, view: &str, _cx: &mut Cx<Self>) {
        self.views.remove(view);
        if let Some(d) = self.docs.remove(view) {
            self.drop_docs(&d.folder);
        }
    }

    fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
        match event {
            Event::Resize { view, width, height } => {
                let first = {
                    let vs = self.view_mut(view);
                    let first = vs.size.0 <= 0.;
                    vs.size = (*width, *height);
                    first
                };
                let unplaced = self.views.get(view.as_str()).and_then(|v| v.open.as_ref().map(|id| !v.places.contains_key(id))).unwrap_or(false);
                if first && unplaced {
                    self.fit(view);
                }
                cx.notify();
            }
            Event::Theme(_) => cx.notify(),
            // Its project, its grants or its path list changed, or it is
            // shown again after a suspension: the docs read again (only
            // the files that changed are parsed).
            Event::ProjectChanged(view) => {
                if let Some(d) = self.docs.get_mut(view) {
                    d.watch = None;
                }
                self.scan(view);
                self.settle_views();
                cx.notify();
            }
            Event::GrantsChanged(_) | Event::SettingsChanged | Event::Visibility(true) => {
                let views: Vec<String> = self.views.keys().cloned().collect();
                for v in views {
                    self.scan(&v);
                }
                self.settle_views();
                cx.notify();
            }
            Event::FilesChanged { watch, paths } => {
                if let Some(view) = self.docs.iter().find(|(_, d)| d.watch == Some(*watch)).map(|(v, _)| v.clone()) {
                    self.follow(&view, paths);
                    self.settle_views();
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn save_state(&self) -> Option<SavedState> {
        SavedState::json(&Saved { views: self.views.clone() })
    }

    fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        let vs = self.views.get(view).cloned().unwrap_or_default();
        self.bind_keys(view, &vs, cx);
        let wide = vs.size.0 >= OUTLINE_AT || vs.size.0 <= 0.;
        let outline_on = vs.outline.unwrap_or(wide) && !self.items.is_empty();
        let mut body = h_flex().id("body").flex_1().min_h(px(0.)).items(Align::Stretch);
        if outline_on {
            body = body.child(self.outline(view, &vs, cx));
        }
        body = body.child(self.main(view, &vs, cx));
        let mut root = v_flex().size_full().bg(Token::Surface).text_color(Token::Text).child(self.bar(view, &vs, outline_on, cx));
        for (n, notice) in self.notices.iter().enumerate().rev().take(2) {
            root = root.child(self.notice(view, n, notice, cx));
        }
        root = root.child(body);
        if let Some(footer) = self.footer(view, &vs, cx) {
            root = root.child(footer);
        }
        root
    }
}

fn path_of(agent: &str) -> String {
    format!("shown/{agent}")
}

// ---- Agents' calls ----------------------------------------------------------

// docs:start c4-show
#[pitwall_extension::tools]
impl C4 {
    /// `c4__show`: draw the agent's Mermaid, all or nothing (FR-34 to
    /// FR-38).
    #[tool(kind = "present")]
    fn show(&mut self, call: Call<ShowInput>, cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        let caller = call.caller().clone();
        let ready = match show::read(&call.input().mermaid, call.input().title.as_deref()) {
            Ok(r) => r,
            Err(refused) => {
                // Nothing changes; each problem goes back with its line.
                let problems: Vec<Problem> = refused.problems.iter().map(|p| Problem::at(p.line, p.column, p.message.clone())).collect();
                // docs:end c4-show
                self.notices.retain(|n| !(n.agent_id == caller.agent_id && n.item.as_deref() == call.item()));
                self.notices.push(Notice {
                    agent_id: caller.agent_id.clone(),
                    agent_name: caller.agent_name.clone(),
                    item: call.item().map(String::from),
                    title: call.input().title.clone(),
                    message: refused.message.clone(),
                    problems: problems.clone(),
                    source: show::unfence(&call.input().mermaid).0.to_string(),
                });
                if self.notices.len() > 5 {
                    self.notices.remove(0);
                }
                cx.notify();
                return Err(ToolError::problems(refused.message, problems));
            }
        };
        let project = caller.project.clone().unwrap_or_default();
        let item = match call.item() {
            Some(i) => i.to_string(),
            None => show::item_for(&ready.title, |c| self.items.contains_key(&id_of(&project, &caller.agent_id, c))),
        };
        let id = id_of(&project, &caller.agent_id, &item);
        let now = now_ms();
        let before = self.items.get(&id).cloned();
        // An update without a title keeps the one it had.
        let title = match (&call.input().title, &before) {
            (None, Some(b)) => b.title.clone(),
            _ => ready.title.clone(),
        };
        let previous = match &before {
            Some(b) if b.mermaid != ready.source => Some(b.mermaid.clone()),
            Some(b) => b.previous.clone(),
            None => None,
        };
        let entry = Item {
            project: project.clone(),
            agent_id: caller.agent_id.clone(),
            agent_name: caller.agent_name.clone(),
            item: item.clone(),
            title: title.clone(),
            mermaid: ready.source.clone(),
            previous,
            shown_at: now,
            opened_at: now,
            doc: None,
        };
        let mut evicted = Vec::new();
        if before.is_none() {
            evicted.extend(self.evict_for(&project, now));
        }
        if let Err(e) = storage::set_json(&format!("{PREFIX}{id}"), &entry) {
            // Full: make room once, by the same rule, and try again.
            let more = self.evict_for_space(&project, &id, now);
            if more.is_empty() || storage::set_json(&format!("{PREFIX}{id}"), &entry).is_err() {
                log::warn(format!("diagram {id} wasn't kept: {e}"));
                return Err(ToolError::plain(format!("The C4 tile's storage is full ({e}); the user can close diagrams they no longer need. Nothing was changed.")));
            }
            evicted.extend(more);
        }
        self.items.insert(id.clone(), entry);
        self.indexes.entry(project.clone()).or_default().set_diagram(&path_of(&caller.agent_id), &id, Some(&title), &ready.source);
        self.drawings.retain(|(d, _), _| *d != id);
        self.notices.retain(|n| !(n.agent_id == caller.agent_id && n.item.as_deref().is_none_or(|i| i == item)));
        // The view it went to shows it, kept where it was when it still
        // fits (FR-36), else fitted.
        let view = call.view().to_string();
        let same = self.views.get(&view).is_some_and(|v| v.open.as_deref() == Some(&id));
        self.ensure(&id, false);
        {
            let vs = self.view_mut(&view);
            vs.changes = false;
            vs.source = false;
            vs.menu = None;
            vs.selected = None;
            vs.notice_open = None;
            if !same {
                vs.path.clear();
                vs.open = Some(id.clone());
            }
        }
        if !same || !self.fits(&view, &id) {
            self.fit(&view);
        }
        cx.notify();

        let counts = show::counts(&ready.diagram);
        let mut text = match &before {
            None => format!("Showing “{}” in the C4 tile (item “{item}”): {counts}.", title),
            Some(_) => format!("Updated “{}” in place (item “{item}”): {counts}.", title),
        };
        if let (Some(b), Diagram::Flowchart(_)) = (&before, &ready.diagram) {
            if let (Some(old), Some(new)) = (structure_of(&b.mermaid), structure_of(&ready.source)) {
                let v = changes::compare(&old, &new);
                let added = v.boxes.iter().filter(|c| **c == changes::Change::Added).count();
                let changed = v.boxes.iter().filter(|c| **c == changes::Change::Changed).count();
                text.push_str(&format!(" Since the last show: {added} boxes added, {changed} changed, {} removed; the user can see the changes.", v.removed_boxes.len()));
            }
        }
        if let Some(s) = show::skipped_words(&ready.skipped) {
            text.push(' ');
            text.push_str(&s);
        }
        let zooms = self.zoom_links(&id);
        if zooms > 0 {
            text.push_str(&format!(" {zooms} of its boxes open another diagram you showed or one in the project's docs."));
        }
        if !evicted.is_empty() {
            text.push_str(&format!(" The oldest {} diagrams of this project were closed to make room.", evicted.len()));
        }
        Ok(Reply::text(text).item(item))
    }
}

fn structure_of(mermaid: &str) -> Option<Structure> {
    match crate::mermaid::parse(mermaid) {
        Diagram::Flowchart(f) => Some(crate::c4::read(&f)),
        _ => None,
    }
}

// ---- Diagrams in the docs ----------------------------------------------------

/// The index's path of docs file `rel` in `folder` (unique across
/// projects, so are its diagrams' ids).
fn doc_path(folder: &str, rel: &str) -> String {
    format!("{}/{rel}", folder.trim_end_matches(['/', '\\']))
}

impl C4 {
    /// Read view `view`'s project's docs (FR-1, FR-2): the Markdown files
    /// the path list names that changed since last read, then watch it
    /// (FR-4). Without `files.read` or a project there are none.
    fn scan(&mut self, view: &str) {
        let project = files::project(view);
        let old = self.docs.get(view).map(|d| d.folder.clone());
        if old.is_some() && old.as_deref() != project.as_ref().map(|p| p.folder.as_str()) {
            if let Some(d) = self.docs.remove(view) {
                if let Some(w) = d.watch {
                    files::unwatch(w);
                }
                self.drop_docs(&d.folder);
            }
        }
        let Some(p) = project else {
            self.docs.remove(view);
            return;
        };
        let root = Root::project(view);
        let globs = docs::globs(settings::text("paths").as_deref());
        let mut d = self.docs.remove(view).unwrap_or_default();
        (d.folder, d.name) = (p.folder.clone(), p.name.clone());
        let listed = match files::list(&root, "", true) {
            Ok(l) => l,
            Err(e) => {
                d.note = Some(format!("The docs couldn't be listed: {e}"));
                self.docs.insert(view.to_string(), d);
                return;
            }
        };
        let mut wanted: Vec<files::Entry> = listed.into_iter().filter(|e| !e.is_folder() && docs::wanted(&globs, &e.path)).collect();
        let mut note = None;
        if wanted.len() > docs::MAX_FILES {
            note = Some(format!("Only the first {} Markdown files are read.", docs::MAX_FILES));
            wanted.truncate(docs::MAX_FILES);
        }
        let gone: Vec<String> = d.read.keys().filter(|k| !wanted.iter().any(|e| &e.path == *k)).cloned().collect();
        for g in gone {
            d.read.remove(&g);
            self.remove_doc_file(&p.folder, &g);
        }
        let mut big = 0;
        for e in &wanted {
            if e.size > docs::MAX_FILE_BYTES {
                big += 1;
                continue;
            }
            if d.read.get(&e.path) == Some(&(e.size, e.modified)) {
                continue;
            }
            match files::read_text(&root, &e.path) {
                Ok(text) => {
                    d.read.insert(e.path.clone(), (e.size, e.modified));
                    self.set_doc_file(&p.folder, &e.path, &text);
                }
                Err(err) => log::warn(format!("{} wasn't read: {err}", e.path)),
            }
        }
        if big > 0 {
            note = Some(if big == 1 { "A Markdown file over 1 MiB is left out.".to_string() } else { format!("{big} Markdown files over 1 MiB are left out.") });
        }
        d.note = note;
        if d.watch.is_none() {
            match files::watch(&root, "") {
                Ok(w) => d.watch = Some(w),
                Err(e) => log::warn(format!("the docs aren't followed: {e}")),
            }
        }
        self.docs.insert(view.to_string(), d);
    }

    /// The watch on view `view`'s project told `paths` changed (FR-4).
    fn follow(&mut self, view: &str, paths: &[String]) {
        let Some(folder) = self.docs.get(view).map(|d| d.folder.clone()) else { return };
        let root = Root::project(view);
        let globs = docs::globs(settings::text("paths").as_deref());
        for path in paths {
            match files::stat(&root, path) {
                // A folder came or moved: read all again.
                Ok(e) if e.is_folder() => return self.scan(view),
                Ok(e) if docs::wanted(&globs, path) && e.size <= docs::MAX_FILE_BYTES => {
                    if let Ok(text) = files::read_text(&root, path) {
                        if let Some(d) = self.docs.get_mut(view) {
                            d.read.insert(path.clone(), (e.size, e.modified));
                        }
                        self.set_doc_file(&folder, path, &text);
                    }
                }
                Ok(_) => {}
                Err(_) => {
                    // Gone: the file, or a folder of them.
                    let under = format!("{path}/");
                    let gone: Vec<String> = self.docs.get(view).map(|d| d.read.keys().filter(|k| *k == path || k.starts_with(&under)).cloned().collect()).unwrap_or_default();
                    for g in gone {
                        if let Some(d) = self.docs.get_mut(view) {
                            d.read.remove(&g);
                        }
                        self.remove_doc_file(&folder, &g);
                    }
                }
            }
        }
    }

    /// Docs file `rel` of project `folder` reads `text`: its diagrams
    /// replace the ones it had.
    fn set_doc_file(&mut self, folder: &str, rel: &str, text: &str) {
        let path = doc_path(folder, rel);
        let index = self.indexes.entry(folder.to_string()).or_default();
        index.set_file(&path, text);
        let found: Vec<Entry> = index.entries().iter().filter(|e| e.path == path).cloned().collect();
        self.items.retain(|id, it| !(it.project == folder && it.doc.as_ref().is_some_and(|d| d.path == rel)) || found.iter().any(|e| e.id() == id));
        let prefix = format!("{}/", folder.trim_end_matches(['/', '\\']));
        for e in found {
            let id = e.id().to_string();
            let title = e.title().map(String::from).unwrap_or_else(|| rel.rsplit('/').next().unwrap_or(rel).to_string());
            let item = Item {
                project: folder.to_string(),
                agent_id: String::new(),
                agent_name: String::new(),
                item: id.strip_prefix(&prefix).unwrap_or(&id).to_string(),
                title,
                mermaid: e.block.text.clone(),
                previous: None,
                shown_at: 0,
                opened_at: 0,
                doc: Some(Doc { path: rel.to_string(), line: e.block.fence_line }),
            };
            self.items.insert(id, item);
        }
        self.relink_docs(folder);
    }

    fn remove_doc_file(&mut self, folder: &str, rel: &str) {
        if let Some(index) = self.indexes.get_mut(folder) {
            index.remove_file(&doc_path(folder, rel));
        }
        self.items.retain(|_, it| !(it.project == folder && it.doc.as_ref().is_some_and(|d| d.path == rel)));
        self.relink_docs(folder);
    }

    /// No view shows project `folder` any more: its docs go.
    fn drop_docs(&mut self, folder: &str) {
        if self.docs.values().any(|d| d.folder == folder) {
            return;
        }
        let files: Vec<String> = self.items.values().filter(|it| it.project == folder).filter_map(|it| it.doc.as_ref().map(|d| d.path.clone())).collect();
        for f in files {
            if let Some(index) = self.indexes.get_mut(folder) {
                index.remove_file(&doc_path(folder, &f));
            }
        }
        self.items.retain(|_, it| !(it.project == folder && it.doc.is_some()));
        let items = &self.items;
        self.drawings.retain(|(id, _), _| items.contains_key(id));
    }

    /// Before/after pairs in the docs (FR-17): the after's Changes are
    /// against its before; the project's drawings laid out again.
    fn relink_docs(&mut self, folder: &str) {
        let pairs: Vec<(String, String)> = self.indexes.get(folder).map(|x| x.pairs().iter().map(|p| (p.before.clone(), p.after.clone())).collect()).unwrap_or_default();
        let ids: Vec<String> = self.items.iter().filter(|(_, it)| it.project == folder && it.doc.is_some()).map(|(id, _)| id.clone()).collect();
        for id in &ids {
            let before = pairs.iter().find(|(_, a)| a == id).and_then(|(b, _)| self.items.get(b)).map(|b| b.mermaid.clone());
            if let Some(it) = self.items.get_mut(id) {
                it.previous = before;
            }
        }
        let items = &self.items;
        self.drawings.retain(|(id, _), _| items.get(id).is_some_and(|it| it.project != folder));
    }

    /// The first docs diagram of view `view`'s project, in file order.
    fn first_doc(&self, view: &str) -> Option<String> {
        let folder = &self.docs.get(view)?.folder;
        self.indexes.get(folder)?.entries().iter().find(|e| self.items.get(e.id()).is_some_and(|it| it.doc.is_some())).map(|e| e.id().to_string())
    }

    /// After the docs changed: a view whose diagram went opens another;
    /// the rest are laid out again and keep their place.
    fn settle_views(&mut self) {
        let views: Vec<String> = self.views.keys().cloned().collect();
        for v in views {
            let Some(vs) = self.views.get(&v) else { continue };
            let (open, changes) = (vs.open.clone(), vs.changes);
            match open {
                Some(id) if self.items.contains_key(&id) => {
                    let had = self.views.get(&v).is_some_and(|s| s.places.contains_key(&id));
                    self.ensure(&id, changes);
                    if !had {
                        self.fit(&v);
                    }
                }
                _ => {
                    let next = self.first_doc(&v).or_else(|| self.items.iter().filter(|(_, it)| it.doc.is_none()).max_by_key(|(_, it)| it.shown_at).map(|(id, _)| id.clone()));
                    {
                        let vs = self.view_mut(&v);
                        vs.path.clear();
                        vs.open = None;
                        vs.selected = None;
                        vs.menu = None;
                        vs.changes = false;
                    }
                    if let Some(id) = next {
                        self.open(&v, &id, false);
                    }
                }
            }
            let items = &self.items;
            if let Some(vs) = self.views.get_mut(&v) {
                vs.path.retain(|p| items.contains_key(p));
            }
        }
    }
}

impl C4 {
    fn view_mut(&mut self, view: &str) -> &mut ViewState {
        self.views.entry(view.to_string()).or_default()
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        let it = self.items.get(id)?;
        self.indexes.get(&it.project)?.get(id)
    }

    /// How many boxes of diagram `id` zoom to another.
    fn zoom_links(&self, id: &str) -> usize {
        let (Some(it), Some(e)) = (self.items.get(id), self.entry(id)) else { return 0 };
        let Some(index) = self.indexes.get(&it.project) else { return 0 };
        e.structure.as_ref().map_or(0, |s| (0..s.boxes.len()).filter(|&i| index.is_detailed(id, i)).count())
    }

    /// Diagrams of `project` to close before a new one: past the limit,
    /// the oldest not opened for a week first, else the least recently
    /// opened (FR-41).
    fn evict_for(&mut self, project: &str, now: u64) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            let mine: Vec<(&String, &Item)> = self.items.iter().filter(|(_, it)| it.project == project && it.doc.is_none()).collect();
            if mine.len() < MAX_PER_PROJECT {
                break;
            }
            let stale = mine.iter().filter(|(_, it)| now.saturating_sub(it.opened_at) >= WEEK_MS).min_by_key(|(_, it)| it.opened_at);
            let Some(id) = stale.or_else(|| mine.iter().min_by_key(|(_, it)| it.opened_at)).map(|(id, _)| (*id).clone()) else { break };
            self.close(&id);
            out.push(id);
        }
        out
    }

    /// The storage is full: close the oldest diagram of `project` (not
    /// `keep`), by the same rule.
    fn evict_for_space(&mut self, project: &str, keep: &str, now: u64) -> Vec<String> {
        let mine: Vec<(&String, &Item)> = self.items.iter().filter(|(id, it)| it.project == project && it.doc.is_none() && id.as_str() != keep).collect();
        let stale = mine.iter().filter(|(_, it)| now.saturating_sub(it.opened_at) >= WEEK_MS).min_by_key(|(_, it)| it.opened_at);
        let Some(id) = stale.or_else(|| mine.iter().min_by_key(|(_, it)| it.opened_at)).map(|(id, _)| (*id).clone()) else { return Vec::new() };
        self.close(&id);
        vec![id]
    }

    /// Close diagram `id`: gone from storage, the index and every view.
    fn close(&mut self, id: &str) {
        if self.items.get(id).is_some_and(|it| it.doc.is_some()) {
            return;
        }
        if let Err(e) = storage::delete(&format!("{PREFIX}{id}")) {
            log::warn(format!("diagram {id} wasn't removed: {e}"));
        }
        if let Some(it) = self.items.remove(id) {
            if let Some(index) = self.indexes.get_mut(&it.project) {
                index.remove(id);
            }
        }
        self.drawings.retain(|(d, _), _| d != id);
        let next = self.items.iter().filter(|(_, it)| it.doc.is_none()).max_by_key(|(_, it)| it.shown_at).map(|(id, _)| id.clone());
        for vs in self.views.values_mut() {
            vs.path.retain(|p| p != id);
            vs.places.remove(id);
            if vs.open.as_deref() == Some(id) {
                vs.open = vs.path.pop().or_else(|| next.clone());
                vs.selected = None;
                vs.changes = false;
                vs.menu = None;
            }
        }
        let opened: Vec<(String, Option<String>)> = self.views.iter().map(|(v, s)| (v.clone(), s.open.clone())).collect();
        for (v, open) in opened {
            if let Some(o) = open {
                self.ensure(&o, false);
                if !self.views.get(&v).is_some_and(|s| s.places.contains_key(&o)) {
                    self.fit(&v);
                }
            }
        }
    }

    /// Lay out diagram `id` (its Changes view with `changes`) if it isn't.
    fn ensure(&mut self, id: &str, changes: bool) {
        let key = (id.to_string(), changes);
        if self.drawings.contains_key(&key) {
            return;
        }
        let Some(entry) = self.entry(id) else { return };
        let drawing = match (&entry.diagram, &entry.structure) {
            (Diagram::Sequence(s), _) => Drawing::Sequence { seq: s.clone(), placed: seq_layout::layout(s) },
            (_, Some(s)) if changes => {
                let Some(before) = self.items.get(id).and_then(|it| it.previous.as_deref()).and_then(structure_of) else { return };
                let merged = changes::merged(&before, s);
                let (graph, sized) = build(&merged.structure);
                let placed = Dagre::default().layout(&graph);
                Drawing::Changes { merged, sized, placed }
            }
            (_, Some(s)) => {
                let (graph, sized) = build(s);
                let placed = Dagre::default().layout(&graph);
                Drawing::Structure { s: s.clone(), sized, placed }
            }
            _ => return,
        };
        self.drawings.insert(key, drawing);
    }

    /// The canvas's size in view `view`.
    fn canvas_size(&self, view: &str) -> (f32, f32) {
        let Some(vs) = self.views.get(view) else { return (800., 600.) };
        let (w, h) = if vs.size.0 > 0. { vs.size } else { (800., 600.) };
        let outline = if vs.outline.unwrap_or(w >= OUTLINE_AT) && !self.items.is_empty() { OUTLINE_W } else { 0. };
        ((w - outline).max(100.), (h - BAR - FOOTER).max(100.))
    }

    fn drawing_of(&self, vs: &ViewState) -> Option<&Drawing> {
        let id = vs.open.as_ref()?;
        self.drawings.get(&(id.clone(), vs.changes)).or_else(|| self.drawings.get(&(id.clone(), false)))
    }

    /// Fit the open diagram into view `view`.
    fn fit(&mut self, view: &str) {
        let (w, h) = self.canvas_size(view);
        let Some(vs) = self.views.get(view) else { return };
        let Some(b) = self.drawing_of(vs).map(Drawing::bounds) else { return };
        let vs = self.view_mut(view);
        vs.viewport.fit(b.0, b.1, b.2, b.3, w, h, 8.);
        if let Some(id) = vs.open.clone() {
            vs.places.insert(id, (vs.viewport.scale, vs.viewport.x, vs.viewport.y));
        }
    }

    /// Whether diagram `id`'s drawing fits view `view` as it is panned.
    fn fits(&self, view: &str, id: &str) -> bool {
        let (w, h) = self.canvas_size(view);
        let (Some(vs), Some(d)) = (self.views.get(view), self.drawings.get(&(id.to_string(), false))) else { return false };
        let (x, y, bw, bh) = d.bounds();
        let (x0, y0) = vs.viewport.to_view(x, y);
        let (x1, y1) = vs.viewport.to_view(x + bw, y + bh);
        x0 >= -1. && y0 >= -1. && x1 <= w + 1. && y1 <= h + 1.
    }

    /// Keep view `view`'s pan and zoom for its diagram.
    fn remember(&mut self, view: &str) {
        let vs = self.view_mut(view);
        if let Some(id) = vs.open.clone() {
            vs.places.insert(id, (vs.viewport.scale, vs.viewport.x, vs.viewport.y));
        }
    }

    /// Show diagram `id` in view `view`; `down` keeps the zoom path (a
    /// click into a box), else it starts from `id`.
    fn open(&mut self, view: &str, id: &str, down: bool) {
        self.remember(view);
        let now = now_ms();
        if let Some(it) = self.items.get_mut(id).filter(|it| it.doc.is_none()) {
            // Kept again only when it matters for the limit (an hour).
            if now.saturating_sub(it.opened_at) > 3_600_000 {
                it.opened_at = now;
                if let Err(e) = storage::set_json(&format!("{PREFIX}{id}"), &*it) {
                    log::warn(format!("diagram {id}'s last open wasn't kept: {e}"));
                }
            }
        }
        self.ensure(id, false);
        let vs = self.view_mut(view);
        if down {
            if let Some(from) = vs.open.clone() {
                if from != id {
                    vs.path.push(from);
                }
            }
        } else {
            vs.path.clear();
        }
        vs.open = Some(id.to_string());
        vs.selected = None;
        vs.changes = false;
        vs.source = false;
        vs.menu = None;
        match vs.places.get(id).copied() {
            Some((s, x, y)) => {
                vs.viewport.scale = s;
                vs.viewport.x = x;
                vs.viewport.y = y;
            }
            None => self.fit(view),
        }
    }

    /// Up one level of the zoom path (FR-14).
    fn up(&mut self, view: &str) {
        let Some(up) = self.views.get_mut(view).and_then(|v| v.path.pop()) else { return };
        let path = self.views.get(view).map(|v| v.path.clone()).unwrap_or_default();
        self.open(view, &up, false);
        self.view_mut(view).path = path;
    }

    /// A click on box `i` (FR-13, FR-15): open the diagram that details
    /// it, offer a menu when several do, else select it.
    fn clicked(&mut self, view: &str, i: usize) {
        let Some(vs) = self.views.get(view) else { return };
        let Some(id) = vs.open.clone() else { return };
        let target = match (self.items.get(&id), self.entry(&id)) {
            (Some(it), Some(e)) if e.structure.as_ref().is_some_and(|s| i < s.boxes.len()) => {
                let path: Vec<&str> = vs.path.iter().map(String::as_str).collect();
                self.indexes.get(&it.project).map_or(ZoomTarget::None, |x| x.zoom_target(&id, i, &path))
            }
            _ => ZoomTarget::None,
        };
        match target {
            ZoomTarget::One(to) => self.open(view, &to, true),
            ZoomTarget::Several(to) => {
                let vs = self.view_mut(view);
                vs.selected = Some(i);
                vs.menu = Some(to);
            }
            ZoomTarget::None => {
                let vs = self.view_mut(view);
                vs.selected = if vs.selected == Some(i) { None } else { Some(i) };
                vs.menu = None;
            }
        }
    }

    /// Enter on the selected box: as a click on it.
    fn enter(&mut self, view: &str) {
        if let Some(i) = self.views.get(view).and_then(|v| v.selected) {
            self.clicked(view, i);
            // A box that zooms nowhere stays selected.
            if let Some(vs) = self.views.get_mut(view) {
                if vs.open.is_some() && vs.selected.is_none() && vs.menu.is_none() {
                    vs.selected = Some(i);
                }
            }
        }
    }

    /// The arrows move the selection to the nearest box that way (FR-26).
    fn step(&mut self, view: &str, dx: f64, dy: f64) {
        let Some(vs) = self.views.get(view) else { return };
        let Some(boxes) = self.drawing_of(vs).map(Drawing::boxes) else { return };
        if boxes.is_empty() {
            return;
        }
        let centre = |r: &crate::layout::Rect| (r.x + r.w / 2., r.y + r.h / 2.);
        let next = match vs.selected.and_then(|s| boxes.get(s).map(|r| (s, centre(r)))) {
            None => (0..boxes.len()).min_by(|&a, &b| {
                let (ca, cb) = (centre(&boxes[a]), centre(&boxes[b]));
                (ca.1 + ca.0 * 0.01).total_cmp(&(cb.1 + cb.0 * 0.01))
            }),
            Some((s, (x, y))) => (0..boxes.len())
                .filter(|&j| j != s)
                .filter_map(|j| {
                    let (cx_, cy_) = centre(&boxes[j]);
                    let (ex, ey) = (cx_ - x, cy_ - y);
                    let along = ex * dx + ey * dy;
                    let across = (ex * dy - ey * dx).abs();
                    (along > 1.).then_some((j, along + 2. * across))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(j, _)| j),
        };
        if let Some(n) = next {
            let vs = self.view_mut(view);
            vs.selected = Some(n);
            vs.menu = None;
        }
    }

    fn zoom(&mut self, view: &str, factor: f32) {
        let (w, h) = self.canvas_size(view);
        let vs = self.view_mut(view);
        vs.viewport.zoom_at(factor, w / 2., h / 2.);
        self.remember(view);
    }

    fn bind_keys(&mut self, view: &str, vs: &ViewState, cx: &mut Cx<Self>) {
        if vs.open.is_none() {
            return;
        }
        let bind = |cx: &mut Cx<Self>, keys: &str, f: fn(&mut C4, &str)| {
            let v = view.to_string();
            let h = cx.listener(move |this: &mut C4, _: &KeyPress, cx| {
                f(this, &v);
                cx.notify();
            });
            cx.bind_key(keys, h);
        };
        // ⌘+, ⌘− and ⌘0 are Pitwall's (its font size): the plain keys.
        bind(cx, "+", |t, v| t.zoom(v, ZOOM_STEP));
        bind(cx, "=", |t, v| t.zoom(v, ZOOM_STEP));
        bind(cx, "-", |t, v| t.zoom(v, 1. / ZOOM_STEP));
        bind(cx, "f", |t, v| {
            t.view_mut(v).selected = None;
            t.fit(v)
        });
        bind(cx, "0", |t, v| t.fit(v));
        bind(cx, "left", |t, v| t.step(v, -1., 0.));
        bind(cx, "right", |t, v| t.step(v, 1., 0.));
        bind(cx, "up", |t, v| t.step(v, 0., -1.));
        bind(cx, "down", |t, v| t.step(v, 0., 1.));
        bind(cx, "enter", |t, v| t.enter(v));
        bind(cx, "backspace", |t, v| t.up(v));
        bind(cx, "b", |t, v| t.toggle_changes(v));
    }

    fn has_changes(&self, id: &str) -> bool {
        self.items.get(id).and_then(|it| it.previous.as_deref()).and_then(structure_of).is_some() && self.entry(id).is_some_and(|e| e.structure.is_some())
    }

    fn toggle_changes(&mut self, view: &str) {
        let Some(id) = self.views.get(view).and_then(|v| v.open.clone()) else { return };
        if !self.has_changes(&id) {
            return;
        }
        let on = !self.views.get(view).is_some_and(|v| v.changes);
        self.set_changes(view, on);
    }

    fn set_changes(&mut self, view: &str, on: bool) {
        let Some(id) = self.views.get(view).and_then(|v| v.open.clone()) else { return };
        self.ensure(&id, on);
        let vs = self.view_mut(view);
        vs.changes = on;
        vs.selected = None;
        vs.menu = None;
        self.fit(view);
    }
}

// ---- Rendering ----------------------------------------------------------------

fn level_word(e: &Entry) -> &'static str {
    match (&e.diagram, e.level) {
        (Diagram::Sequence(_), _) => "Sequence",
        (_, Some(Level::Context)) => "Context",
        (_, Some(Level::Container)) => "Container",
        (_, Some(Level::Component)) => "Component",
        _ => "Diagram",
    }
}

impl C4 {
    fn bar(&self, view: &str, vs: &ViewState, outline_on: bool, cx: &mut Cx<Self>) -> Div {
        let mut bar = h_flex().id("bar").flex_none().h(px(BAR)).px_2().gap_1().border_b_1().border_color(Token::Line);
        if !self.items.is_empty() {
            let v = view.to_string();
            bar = bar.child(icon_button(Icon::Sidebar, if outline_on { "Hide the list" } else { "Show the list" }).id("outline").on_click(cx.listener(move |this: &mut C4, _, cx| {
                let wide = this.views.get(&v).is_none_or(|s| s.size.0 >= OUTLINE_AT || s.size.0 <= 0.);
                let vs = this.view_mut(&v);
                vs.outline = Some(!vs.outline.unwrap_or(wide));
                cx.notify();
            })));
        }
        let Some(id) = vs.open.clone() else {
            return bar.child(text("C4 diagrams").id("title").text_sm().font_semibold().truncate().flex_1());
        };
        // The zoom path, then this diagram (FR-14).
        let mut crumbs = breadcrumbs().id("path");
        for (n, p) in vs.path.iter().enumerate() {
            crumbs = crumbs.crumb(format!("{n}"), self.items.get(p).map_or("Diagram", |i| i.title.as_str()));
        }
        crumbs = crumbs.crumb("here", self.items.get(&id).map_or("Diagram", |i| i.title.as_str()));
        let v = view.to_string();
        crumbs = crumbs.on_pick(cx.listener(move |this: &mut C4, key: &str, cx| {
            let Ok(n) = key.parse::<usize>() else { return };
            let path = this.views.get(&v).map(|s| s.path.clone()).unwrap_or_default();
            if let Some(to) = path.get(n).cloned() {
                this.open(&v, &to, false);
                this.view_mut(&v).path = path[..n].to_vec();
                cx.notify();
            }
        }));
        bar = bar.child(h_flex().id("crumbs").flex_1().min_w(px(0.)).child(crumbs));
        if self.has_changes(&id) && !vs.source {
            let v = view.to_string();
            bar = bar.child(
                segmented()
                    .id("mode")
                    .option("now", "Now")
                    .option("changes", "Changes")
                    .selected(if vs.changes { "changes" } else { "now" })
                    .tooltip(if self.items.get(&id).is_some_and(|i| i.doc.is_some()) { "Before and after, from the doc (B)" } else { "Changes since the last show (B)" })
                    .on_change(cx.listener(move |this: &mut C4, key: &str, cx| {
                        this.set_changes(&v, key == "changes");
                        cx.notify();
                    })),
            );
        }
        if !vs.source {
            let (v1, v2, v3) = (view.to_string(), view.to_string(), view.to_string());
            bar = bar
                .child(text(format!("{} %", (vs.viewport.scale * 100.).round())).id("zoom").text_xs().text_color(Token::Text3).flex_none())
                .child(icon_button(Icon::Named("minus".into()), "Zoom out (−)").id("zoom-out").on_click(cx.listener(move |this: &mut C4, _, cx| {
                    this.zoom(&v1, 1. / ZOOM_STEP);
                    cx.notify();
                })))
                .child(icon_button(Icon::Plus, "Zoom in (+)").id("zoom-in").on_click(cx.listener(move |this: &mut C4, _, cx| {
                    this.zoom(&v2, ZOOM_STEP);
                    cx.notify();
                })))
                .child(icon_button(Icon::Maximize, "Fit (F)").id("fit").on_click(cx.listener(move |this: &mut C4, _, cx| {
                    this.view_mut(&v3).selected = None;
                    this.fit(&v3);
                    cx.notify();
                })));
        }
        let (v1, v2) = (view.to_string(), view.to_string());
        let source_label = if vs.source { "Show the drawing" } else { "Show the source" };
        let doc = self.items.get(&id).and_then(|i| i.doc.clone().map(|d| (i.project.clone(), d)));
        let more = match &doc {
            // A docs diagram is closed by taking it out of the doc.
            Some(_) => vec![menu_item("source", source_label).icon(Icon::File), menu_item("open-file", "Open in the editor").icon(Icon::External)],
            None => vec![menu_item("source", source_label).icon(Icon::File), separator(), menu_item("close", "Close this diagram").icon(Icon::X).danger()],
        };
        bar.child(
            menu_icon_button(Icon::More, "More", more)
                .id("more")
                .on_select(cx.listener(move |this: &mut C4, p: &Picked, cx| {
                    match p.key.as_str() {
                        "source" => {
                            let vs = this.view_mut(&v1);
                            vs.source = !vs.source;
                            vs.notice_open = None;
                        }
                        "close" => {
                            if let Some(id) = this.views.get(&v2).and_then(|s| s.open.clone()) {
                                this.close(&id);
                            }
                        }
                        // The block's line in Pitwall's editor (FR-24).
                        "open-file" => {
                            if let Some((folder, d)) = &doc {
                                if let Err(e) = pitwall_extension::show::file(&doc_path(folder, &d.path), Some(d.line)) {
                                    log::warn(format!("{} wasn't opened: {e}", d.path));
                                }
                            }
                        }
                        _ => {}
                    }
                    cx.notify();
                })),
        )
    }

    /// The menu of a box several diagrams detail (FR-13), anchored under
    /// the box (`at`, in the canvas node's pixels).
    fn menu(&self, view: &str, targets: &[String], at: (f32, f32), cx: &mut Cx<Self>) -> Popover {
        let v = view.to_string();
        let mut list = v_flex().id("targets").gap_0p5().p_1().child(text("Open which diagram?").text_xs().text_color(Token::Text3).px_2().py_1());
        for (n, t) in targets.iter().enumerate() {
            let (Some(it), Some(e)) = (self.items.get(t), self.entry(t)) else { continue };
            let (v, t) = (view.to_string(), t.clone());
            list = list.child(
                h_flex()
                    .id(format!("t{n}"))
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .focusable()
                    .hover_bg(Token::Surface2)
                    .child(text(it.title.clone()).text_sm().truncate().flex_1())
                    .child(text(format!("{} · {}", level_word(e), it.doc.as_ref().map_or(it.agent_name.as_str(), |d| d.path.as_str()))).text_xs().text_color(Token::Text3).flex_none())
                    .on_click(cx.listener(move |this: &mut C4, _, cx| {
                        this.open(&v, &t, true);
                        cx.notify();
                    })),
            );
        }
        popover().id("menu").width(300.).at(at.0, at.1).on_dismiss(cx.listener(move |this: &mut C4, _: &Dismissed, cx| {
            this.view_mut(&v).menu = None;
            cx.notify();
        })).child(list)
    }

    fn notice(&self, view: &str, n: usize, notice: &Notice, cx: &mut Cx<Self>) -> Banner {
        let what = notice.title.as_deref().or(notice.item.as_deref()).map(|t| format!(" “{t}”")).unwrap_or_default();
        let first = notice.problems.first().map(|p| match (p.line, p.column) {
            (Some(l), Some(c)) => format!("line {l}, column {c}: {}", p.message),
            (Some(l), None) => format!("line {l}: {}", p.message),
            _ => p.message.clone(),
        });
        let more = notice.problems.len().saturating_sub(1);
        let text = match first {
            Some(f) if more > 0 => format!("{f} (and {more} more)"),
            Some(f) => f,
            None => notice.message.clone(),
        };
        let (v1, v2) = (view.to_string(), view.to_string());
        let shown = self.views.get(view).and_then(|s| s.notice_open) == Some(n);
        banner(Tone::Bad, text)
            .id(format!("notice{n}"))
            .title(format!("{}'s diagram{what} wasn't drawn", notice.agent_name))
            .action(if shown { "Hide the source" } else { "Show the source" }, cx.listener(move |this: &mut C4, _: &Click, cx| {
                let vs = this.view_mut(&v1);
                vs.notice_open = if vs.notice_open == Some(n) { None } else { Some(n) };
                cx.notify();
            }))
            .on_dismiss(cx.listener(move |this: &mut C4, _: &Dismissed, cx| {
                if n < this.notices.len() {
                    this.notices.remove(n);
                }
                for vs in this.views.values_mut() {
                    vs.notice_open = None;
                }
                let _ = &v2;
                cx.notify();
            }))
    }

    /// The list of shown diagrams, by agent (FR-23).
    fn outline(&self, view: &str, vs: &ViewState, cx: &mut Cx<Self>) -> Div {
        let mut by_agent: BTreeMap<(String, String), Vec<(&String, &Item)>> = BTreeMap::new();
        for (id, it) in self.items.iter().filter(|(_, it)| it.doc.is_none()) {
            by_agent.entry((it.agent_name.clone(), it.agent_id.clone())).or_default().push((id, it));
        }
        let projects = self.items.values().filter(|i| i.doc.is_none()).map(|i| i.project.as_str()).collect::<std::collections::BTreeSet<_>>().len();
        let mut list = v_flex().id("outline").w(px(OUTLINE_W)).flex_none().min_h(px(0.)).overflow_y_scroll().border_r_1().border_color(Token::Line).py_1();
        if !by_agent.is_empty() {
            list = list.child(text("Shown").text_xs().font_semibold().text_color(Token::Text3).px_3().py_1());
        }
        for ((agent, agent_id), mut items) in by_agent {
            items.sort_by_key(|a| std::cmp::Reverse(a.1.shown_at));
            list = list.child(h_flex().id(format!("a-{agent_id}")).px_3().pt_2().pb_1().gap_1().child(icon(Icon::Terminal).text_color(Token::Text3)).child(text(format!("from {agent}")).text_xs().text_color(Token::Text2).truncate()));
            for (id, it) in items {
                let on = vs.open.as_deref() == Some(id.as_str());
                let kind = self.entry(id).map_or("Diagram", level_word);
                let place = if projects > 1 { format!("{kind} · {}", project_name(&it.project)) } else { kind.to_string() };
                let (v, i1, i2) = (view.to_string(), id.clone(), id.clone());
                list = list.child(
                    h_flex()
                        .id(format!("d-{id}"))
                        .group("row")
                        .pl_3()
                        .pr_1()
                        .py_1()
                        .gap_1()
                        .cursor_pointer()
                        .focusable()
                        .hover_bg(Token::Surface2)
                        .when(on, |r| r.bg(Token::Surface2))
                        .child(v_flex().flex_1().min_w(px(0.)).child(text(it.title.clone()).text_sm().when(on, |t| t.font_medium()).truncate()).child(text(place).text_xs().text_color(Token::Text3).truncate()))
                        .child(icon_button(Icon::X, "Close this diagram").id("close").small().visible_on_group_hover("row").on_click(cx.listener(move |this: &mut C4, _, cx| {
                            this.close(&i2);
                            cx.notify();
                        })))
                        .on_click(cx.listener(move |this: &mut C4, _, cx| {
                            this.open(&v, &i1, false);
                            cx.notify();
                        })),
                );
            }
        }
        self.docs_outline(list, view, vs, cx)
    }

    /// The outline's docs part: the view's project's files and the
    /// diagrams in each, in file order (FR-23).
    fn docs_outline(&self, mut list: Div, view: &str, vs: &ViewState, cx: &mut Cx<Self>) -> Div {
        let Some(d) = self.docs.get(view) else { return list };
        list = list.child(h_flex().id("docs").px_3().pt_2().pb_1().gap_1().child(text(format!("In {}'s docs", d.name)).text_xs().font_semibold().text_color(Token::Text3).truncate()));
        let mut last_file = None;
        let mut any = false;
        let entries = self.indexes.get(&d.folder).map(|x| x.entries()).unwrap_or(&[]);
        for (n, e) in entries.iter().enumerate() {
            let Some(it) = self.items.get(e.id()) else { continue };
            let Some(doc) = &it.doc else { continue };
            any = true;
            if last_file != Some(doc.path.as_str()) {
                last_file = Some(doc.path.as_str());
                list = list.child(h_flex().id(format!("f{n}")).px_3().pt_2().pb_1().gap_1().child(icon(Icon::File).text_color(Token::Text3)).child(text(doc.path.clone()).text_xs().text_color(Token::Text2).truncate()));
            }
            let on = vs.open.as_deref() == Some(e.id());
            let problems = !e.problems().is_empty();
            let place = if problems { format!("{} · line {} · can't be drawn", level_word(e), doc.line) } else { format!("{} · line {}", level_word(e), doc.line) };
            let (v, id) = (view.to_string(), e.id().to_string());
            list = list.child(
                h_flex()
                    .id(format!("doc{n}"))
                    .pl_3()
                    .pr_1()
                    .py_1()
                    .gap_1()
                    .cursor_pointer()
                    .focusable()
                    .hover_bg(Token::Surface2)
                    .when(on, |r| r.bg(Token::Surface2))
                    .child(v_flex().flex_1().min_w(px(0.)).child(text(it.title.clone()).text_sm().when(on, |t| t.font_medium()).truncate()).child(text(place).text_xs().text_color(if problems { Token::Warn } else { Token::Text3 }).truncate()))
                    .on_click(cx.listener(move |this: &mut C4, _, cx| {
                        this.open(&v, &id, false);
                        cx.notify();
                    })),
            );
        }
        if !any {
            list = list.child(text("No Mermaid diagrams in the docs yet.").id("no-docs").text_xs().text_color(Token::Text3).px_3().py_1());
        }
        if let Some(note) = &d.note {
            list = list.child(text(note.clone()).id("docs-note").text_xs().text_color(Token::Warn).px_3().py_1());
        }
        list
    }

    /// The drawing, the source, a notice's source, or the empty state.
    fn main(&mut self, view: &str, vs: &ViewState, cx: &mut Cx<Self>) -> Div {
        let pane = v_flex().id("main").flex_1().min_w(px(0.)).min_h(px(0.));
        if let Some(n) = vs.notice_open.filter(|n| *n < self.notices.len()) {
            let notice = &self.notices[n];
            let mut code = code_view(notice.source.clone()).id("problems").flex_1();
            let mut first = None;
            for p in &notice.problems {
                if let Some(l) = p.line {
                    let l = l.saturating_sub(1) as usize;
                    code = code.mark_with(l..l + 1, Mark::Error, p.message.clone());
                    first = first.or(Some(l));
                }
            }
            if let Some(l) = first {
                code = code.reveal(l);
            }
            let mut list = v_flex().id("lines").flex_none().px_3().py_2().gap_0p5().border_b_1().border_color(Token::Line);
            for p in &notice.problems {
                let at = match (p.line, p.column) {
                    (Some(l), Some(c)) => format!("Line {l}, column {c}"),
                    (Some(l), None) => format!("Line {l}"),
                    _ => String::new(),
                };
                list = list.child(h_flex().gap_2().child(text(at).text_xs().font_mono().text_color(Token::Bad).flex_none()).child(text(p.message.clone()).text_xs().truncate()));
            }
            return pane.child(list).child(code);
        }
        let Some(id) = vs.open.clone() else {
            return pane.items_center().justify_center().child(
                empty_state("No diagrams yet")
                    .icon(Icon::Grid)
                    .text(if self.docs.contains_key(view) {
                        "Ask an agent to draw one, for example: “draw the containers of this repo as a C4 diagram and show it on the right”, or write a mermaid block in the project's docs."
                    } else {
                        "Ask an agent to draw one, for example: “draw the containers of this repo as a C4 diagram and show it on the right”. Open the tile in a project, with “Read files in your projects” granted, to see the diagrams in its docs."
                    }),
            );
        };
        if vs.source {
            let src = self.items.get(&id).map(|i| i.mermaid.clone()).unwrap_or_default();
            let mut pane = pane;
            if let Some(d) = self.items.get(&id).and_then(|i| i.doc.as_ref()) {
                pane = pane.child(text(format!("{}, line {}", d.path, d.line)).id("source-at").text_xs().font_mono().text_color(Token::Text3).px_3().py_1().border_b_1().border_color(Token::Line));
            }
            return pane.child(code_view(src).id("source").flex_1());
        }
        self.ensure(&id, vs.changes);
        let Some(it) = self.items.get(&id) else { return pane };
        let index = self.indexes.get(&it.project);
        let Some(drawing) = self.drawing_of(vs) else {
            return pane.p_4().child(text("This diagram can't be drawn.").text_sm().text_color(Token::Text3));
        };
        let base = canvas().id("diagram").flex_1().bg(Token::Surface).transform(vs.viewport).label(format!("Diagram: {}", it.title));
        let path: Vec<&str> = vs.path.iter().map(String::as_str).collect();
        let (c, hits, detailed) = match drawing {
            Drawing::Structure { s, sized, placed } => {
                let detailed: Vec<bool> = (0..s.boxes.len()).map(|i| index.is_some_and(|x| x.zoom_target(&id, i, &path) != ZoomTarget::None)).collect();
                let (c, hits) = paint::paint_with(base, s, sized, placed, &Look { detailed: &detailed, selected: vs.selected, changes: None });
                (c, hits, detailed)
            }
            Drawing::Changes { merged, sized, placed } => {
                let (c, hits) = paint::paint_with(base, &merged.structure, sized, placed, &Look { detailed: &[], selected: vs.selected, changes: Some((&merged.boxes, &merged.arrows)) });
                (c, hits, Vec::new())
            }
            Drawing::Sequence { seq, placed } => {
                let (c, hits) = paint::paint_sequence(base, seq, placed, vs.selected);
                (c, hits, Vec::new())
            }
        };
        let mut c = c;
        let mut menu_at = None;
        for HitBox { index: i, key, rect, label } in hits {
            if vs.menu.is_some() && vs.selected == Some(i) {
                // Under the box, at its left edge.
                menu_at = Some(vs.viewport.to_view(rect.x as f32, (rect.y + rect.h) as f32));
            }
            let tip = if detailed.get(i).copied().unwrap_or(false) { format!("{label}: click to open its diagram") } else { label.clone() };
            let v = view.to_string();
            c = c.region(Region::new(key, rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32, label).tooltip(tip).on_click(cx.listener(move |this: &mut C4, _: &Click, cx| {
                this.clicked(&v, i);
                cx.notify();
            })));
        }
        if let Some(targets) = &vs.menu {
            c = c.popover(self.menu(view, targets, menu_at.unwrap_or((16., 16.)), cx));
        }
        let v = view.to_string();
        let pan = move |this: &mut C4, p: &Pointer, cx: &mut Cx<C4>| {
            if this.view_mut(&v).viewport.on_pointer(p) {
                this.remember(&v);
                cx.notify();
            }
        };
        let c = c.on_pointer(cx.listener(pan.clone())).on_wheel(cx.listener(pan)).focusable();
        pane.child(c)
    }

    /// The selected box (FR-15), the legend (FR-10) and what isn't shown
    /// (FR-20).
    fn footer(&self, view: &str, vs: &ViewState, cx: &mut Cx<Self>) -> Option<Div> {
        let id = vs.open.clone()?;
        if vs.source || vs.notice_open.is_some() {
            return None;
        }
        let it = self.items.get(&id)?;
        let e = self.entry(&id)?;
        // The legend first: Pitwall's "from <agent>" mark sits over the
        // footer's right end.
        let mut lead = h_flex().flex_none().gap_1p5();
        let mut row = h_flex().id("info").flex_1().min_w(px(0.)).gap_2().overflow_hidden();
        let mut any = false;
        let drawing = self.drawing_of(vs);
        // The selection.
        match (drawing, vs.selected) {
            (Some(Drawing::Structure { s, .. }), Some(i)) | (Some(Drawing::Changes { merged: Merged { structure: s, .. }, .. }), Some(i)) => {
                if let Some(b) = s.boxes.get(i) {
                    any = true;
                    let mut info = h_flex().id("selected").gap_1p5().min_w(px(0.)).child(text(b.name.clone()).text_sm().font_semibold().flex_none());
                    if let Some(k) = &b.kind {
                        info = info.child(text(format!("[{}]", k.text)).text_xs().text_color(Token::Text3).flex_none());
                    }
                    if let Some(p) = &b.purpose {
                        info = info.child(text(p.clone()).text_xs().text_color(Token::Text2).truncate());
                    }
                    row = row.child(info);
                    // Sequences with a participant of the same Name.
                    let key = name_key(&b.name);
                    let seqs: Vec<(String, String)> = self
                        .items
                        .iter()
                        .filter(|(_, x)| x.project == it.project)
                        .filter(|(sid, _)| self.entry(sid).is_some_and(|e| matches!(&e.diagram, Diagram::Sequence(s) if s.participants.iter().any(|p| name_key(&p.label) == key))))
                        .map(|(sid, x)| (sid.clone(), x.title.clone()))
                        .collect();
                    for (n, (sid, title)) in seqs.into_iter().take(3).enumerate() {
                        let v = view.to_string();
                        row = row.child(button(title).small().ghost().icon(Icon::External).id(format!("seq{n}")).tooltip("Open this sequence diagram").on_click(cx.listener(move |this: &mut C4, _, cx| {
                            this.open(&v, &sid, true);
                            cx.notify();
                        })));
                    }
                }
            }
            (Some(Drawing::Sequence { seq, .. }), Some(i)) => {
                if let Some(p) = seq.participants.get(i) {
                    any = true;
                    row = row.child(text(p.label.clone()).id("selected").text_sm().font_semibold());
                }
            }
            _ => {}
        }
        // The legend: the classes this diagram uses, or the Changes marks.
        let mut legend: Vec<(&str, Tone)> = Vec::new();
        match drawing {
            Some(Drawing::Changes { merged, .. }) => {
                for (d, word, tone) in [(Drawn::Added, "Added", Tone::Good), (Drawn::Changed, "Changed", Tone::Warn), (Drawn::Removed, "Removed", Tone::Info)] {
                    if merged.boxes.contains(&d) || merged.arrows.contains(&d) {
                        legend.push((word, tone));
                    }
                }
            }
            Some(Drawing::Structure { s, .. }) => {
                if s.boxes.iter().any(|b| b.marks.new) {
                    legend.push(("New", Tone::Good));
                }
                if s.boxes.iter().any(|b| b.marks.changed) {
                    legend.push(("Changed", Tone::Warn));
                }
                if s.boxes.iter().any(|b| b.marks.external) {
                    legend.push(("External", Tone::Info));
                }
            }
            _ => {}
        }
        for (n, (word, tone)) in legend.into_iter().enumerate() {
            any = true;
            lead = lead.child(chip(word).tone(tone).id(format!("legend{n}")));
        }
        let skipped: &[Skipped] = e.diagram.skipped();
        if !skipped.is_empty() {
            any = true;
            let list: Vec<String> = skipped.iter().map(|s| format!("line {}: {}", s.line, s.what)).collect();
            lead = lead.child(chip(format!("Not shown: {}", skipped.len())).tone(Tone::Warn).id("skipped").tooltip(list.join("\n")));
        }
        any.then(|| h_flex().id("footer").flex_none().h(px(FOOTER)).px_3().gap_3().border_t_1().border_color(Token::Line).overflow_hidden().child(lead).child(row))
    }
}

fn project_name(p: &str) -> String {
    if p.is_empty() {
        return "no project".into();
    }
    p.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

#[cfg(target_arch = "wasm32")]
pitwall_extension::export!(C4);
