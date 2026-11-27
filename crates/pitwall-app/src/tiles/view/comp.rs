//! The components of a tile's view (docs/spec/tile-sdk/ui.md §3.3), drawn
//! as Pitwall's own:
//!
//! - Virtual lists, tables and trees: a `uniform_list`, so only the rows
//!   in sight are laid out. Their rows come from the tile a page at a time
//!   as they come into sight ([`Comps`] keeps them by list and revision);
//!   until a page arrives its rows are blank, and after the list's revision
//!   moves the old rows stay drawn until the new ones come. Click selects,
//!   a double click or Enter activates, Up/Down/Home/End/Page keys move;
//!   in a tree Left and Right close and open, and the chevron toggles. A
//!   table has a header: sortable titles and edges to drag.
//! - Tabs (`kit::seg`), selects (`kit::select`), menu buttons and context
//!   menus (`kit::menu`, as a glass pop-up under Liquid Glass).
//! - Markdown: Pitwall's Preview ([`MarkdownView`]), inert; links and task
//!   boxes go to the tile when it listens.
//! - Images: a file of the extension's folder (checked to stay inside it)
//!   or bytes the tile put; never the network.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{anchored, deferred, div, img, prelude::*, px, uniform_list, AnyElement, App, Context, CursorStyle, Entity, FocusHandle, FontStyle, HighlightStyle, KeyDownEvent, MouseButton, ObjectFit, Pixels, Point, ScrollStrategy, SharedString, StyledText, UnderlineStyle, UniformListScrollHandle, Window};
use pitwall_tile_host::components::{self as hc, ImageFormat};
use pitwall_tile_host::limits::{MAX_ASSET_BYTES, ROWS_PAGE};
use pitwall_tile_host::ui::{self, Element, Event, ListRow as Row, Listen, MenuEntry, Node};
use pitwall_tile_host::RowsUpdate;

use super::{bounds_probe, handler, modifiers, NodeState, TileView};
use crate::kit;
use crate::markdown_view::{Hooks, MarkdownView};
use crate::theme::{Theme, RADIUS_SM};
use crate::tiles::render::{self, Inherit};

/// Rows kept per list; the ones fetched longest ago go first. (The SDK
/// keeps the listeners of more, so a row drawn always has them.)
const KEEP_ROWS: usize = 2048;
/// A page asked for whose rows still aren't here after this is asked
/// again (it was let go, or lost in a restart).
const ASK_AGAIN: Duration = Duration::from_secs(1);
/// A table's header height.
const HEADER_H: f32 = 28.;
/// The narrowest a user drags a column.
const MIN_COL: f32 = 40.;
/// A tree's indent per level.
const INDENT: f32 = 14.;

// ---- What a running component's views show --------------------------------

/// The rows a component's lists, tables and trees have shown and the
/// images it put, kept with the component ([`crate::tiles::Running`]).
#[derive(Default)]
pub struct Comps {
    lists: HashMap<(String, String), RowCache>,
    images: HashMap<String, Arc<gpui::Image>>,
}

#[derive(Default)]
struct RowCache {
    /// Each row by index: the revision it was fetched for, the row, and
    /// when (a count).
    rows: HashMap<u32, (u64, Row, u64)>,
    order: VecDeque<(u32, u64)>,
    seq: u64,
}

impl Comps {
    /// Rows the tile sent ([`crate::tiles::Tiles::ask_rows`]).
    pub fn rows_arrived(&mut self, u: RowsUpdate) {
        let c = self.lists.entry((u.view, u.list)).or_default();
        for (k, row) in u.rows.iter().enumerate() {
            let ix = u.start + k as u32;
            c.seq += 1;
            c.rows.insert(ix, (u.revision, row.clone(), c.seq));
            c.order.push_back((ix, c.seq));
        }
        while c.rows.len() > KEEP_ROWS {
            let Some((ix, seq)) = c.order.pop_front() else { break };
            if c.rows.get(&ix).is_some_and(|r| r.2 == seq) {
                c.rows.remove(&ix);
            }
        }
        if c.order.len() > KEEP_ROWS * 4 {
            let rows = &c.rows;
            c.order.retain(|(ix, seq)| rows.get(ix).is_some_and(|r| r.2 == *seq));
        }
    }

    /// `ui.put-image`.
    pub fn put_image(&mut self, name: String, format: ImageFormat, bytes: &[u8]) {
        let f = match format {
            ImageFormat::Png => gpui::ImageFormat::Png,
            ImageFormat::Jpeg => gpui::ImageFormat::Jpeg,
            ImageFormat::Gif => gpui::ImageFormat::Gif,
            ImageFormat::Webp => gpui::ImageFormat::Webp,
            ImageFormat::Svg => gpui::ImageFormat::Svg,
        };
        self.images.insert(name, Arc::new(gpui::Image::from_bytes(f, bytes.to_vec())));
    }

    /// `ui.drop-image`.
    pub fn drop_image(&mut self, name: &str) {
        self.images.remove(name);
    }

    fn row(&self, view: &str, list: &str, ix: u32) -> Option<&(u64, Row, u64)> {
        self.lists.get(&(view.to_string(), list.to_string()))?.rows.get(&ix)
    }

    /// The rows kept of the list at `list` in `view` (tests, bench).
    pub fn rows_kept(&self, view: &str, list: &str) -> usize {
        self.lists.get(&(view.to_string(), list.to_string())).map_or(0, |c| c.rows.len())
    }

    /// The row keys kept of that list, by index (tests).
    pub fn row_keys(&self, view: &str, list: &str) -> Vec<(u32, String)> {
        let mut v: Vec<(u32, String)> = self.lists.get(&(view.to_string(), list.to_string())).map(|c| c.rows.iter().map(|(i, r)| (*i, r.1.key.clone())).collect()).unwrap_or_default();
        v.sort();
        v
    }
}

// ---- A node's own state ---------------------------------------------------

/// What a component node keeps across frames.
#[derive(Default)]
pub(super) struct Comp {
    list: Option<ListUi>,
    /// A select is open.
    open: bool,
    md: Option<Entity<MarkdownView>>,
    /// An image asset: its file, checked (the path it was found for).
    asset: Option<(String, Result<PathBuf, String>)>,
}

pub(super) struct ListUi {
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    /// Pages asked for: the revision, when.
    asked: HashMap<u32, (u64, Instant)>,
    /// The row the keys move from.
    cursor: Option<usize>,
    /// Columns the user resized: the column's own width then, and theirs.
    widths: HashMap<String, (Option<f32>, f32)>,
    /// A column edge being dragged: the column, where the drag started,
    /// its width then.
    drag: Option<(String, f32, f32)>,
    /// A row moved to with the keys before it arrived: told when it does.
    pending: Option<usize>,
}

/// The menus of a view: which node's right-click opens which menu, and
/// the one open.
#[derive(Default)]
pub(super) struct ViewUi {
    menus: HashMap<String, usize>,
    open: Option<OpenMenu>,
    /// Rows' image assets, checked once each (rows are drawn often).
    assets: std::cell::RefCell<HashMap<String, Option<PathBuf>>>,
    /// Nodes whose bounds are kept for screenshots (tests).
    #[cfg(test)]
    probe: std::collections::HashSet<String>,
}

impl ViewUi {
    /// Whether the node at `path` keeps its bounds even with no listener.
    #[cfg(test)]
    pub(super) fn probed(&self, path: &str) -> bool {
        self.probe.contains(path)
    }

    #[cfg(not(test))]
    pub(super) fn probed(&self, _path: &str) -> bool {
        false
    }
}

#[derive(Clone)]
struct OpenMenu {
    entries: Vec<MenuEntry>,
    handler: Option<u32>,
    at: Point<Pixels>,
}

/// A column edge's drag (only for `on_drag_move`).
struct ColumnDrag;

struct NoGhost;

impl Render for NoGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn list_of(n: &Node) -> Option<&ui::VirtualList> {
    hc::rows_of(n)
}

/// Whether node `n` is drawn by this module.
pub(super) fn is_comp(e: &Element) -> bool {
    matches!(e, Element::VirtualList(_) | Element::Table(_) | Element::Tree(_) | Element::Tabs(_) | Element::Select(_) | Element::MenuButton(_) | Element::ContextMenu(_) | Element::Markdown(_) | Element::Image(_))
}

/// Whether a node's styles give it a height of its own (else Markdown is
/// as tall as its text).
fn sized(styles: &[ui::Style]) -> bool {
    styles.iter().any(|s| matches!(s, ui::Style::Height(_) | ui::Style::Grow(_) | ui::Style::MaxHeight(_)))
}

/// An image asset's file in extension folder `dir`, checked: inside it
/// (links followed), an image file, at most 16 MiB.
pub(super) fn asset_file(dir: &Path, path: &str) -> Result<PathBuf, String> {
    hc::asset_path(path)?;
    let root = dir.canonicalize().map_err(|e| format!("the extension's folder: {e}"))?;
    let file = dir.join(path).canonicalize().map_err(|_| format!("{path} isn't in the extension's folder"))?;
    if !file.starts_with(&root) {
        return Err(format!("{path} is outside the extension's folder"));
    }
    let meta = std::fs::metadata(&file).map_err(|e| format!("{path}: {e}"))?;
    if !meta.is_file() || meta.len() > MAX_ASSET_BYTES {
        return Err(format!("{path} isn't a file of at most {} MiB", MAX_ASSET_BYTES >> 20));
    }
    let head = std::fs::read(&file).map(|b| b[..b.len().min(1024)].to_vec()).map_err(|e| format!("{path}: {e}"))?;
    hc::image_format(&head).ok_or_else(|| format!("{path} isn't a PNG, JPEG, GIF, WebP or SVG image"))?;
    Ok(file)
}

impl ListUi {
    fn new(cx: &mut Context<TileView>) -> ListUi {
        ListUi { scroll: UniformListScrollHandle::new(), focus: cx.focus_handle().tab_stop(true), asked: HashMap::new(), cursor: None, widths: HashMap::new(), drag: None, pending: None }
    }

    /// Column `c`'s width now: the user's while the column's own is what
    /// it was when they set it.
    fn width(&self, c: &ui::Column) -> Option<f32> {
        match self.widths.get(&c.key) {
            Some((own, theirs)) if *own == c.width => Some(*theirs),
            _ => c.width.map(|w| w.max(MIN_COL)),
        }
    }
}

impl TileView {
    /// A frame was applied: the menus by the node they open on.
    pub(super) fn comps_applied(&mut self) {
        let Some(frame) = self.frame.clone() else { return };
        self.ui.menus.clear();
        for (i, n) in frame.nodes.iter().enumerate() {
            if let (Element::ContextMenu(_), Some(p)) = (&n.element, n.parent) {
                self.ui.menus.entry(self.paths[p as usize].clone()).or_insert(i);
            }
        }
    }

    /// Node `node` at `path` was applied: its component's state made or
    /// updated.
    pub(super) fn sync_comp(&mut self, st: &mut NodeState, node: &Node, path: &str, cx: &mut Context<Self>) {
        if !is_comp(&node.element) {
            st.comp = None;
            return;
        }
        let c = st.comp.get_or_insert_with(Comp::default);
        match &node.element {
            Element::VirtualList(_) | Element::Table(_) | Element::Tree(_) => {
                let l = c.list.get_or_insert_with(|| ListUi::new(cx));
                if st.focus.is_none() {
                    st.focus = Some(l.focus.clone());
                }
            }
            Element::Markdown(m) => {
                let md = c.md.get_or_insert_with(|| cx.new(MarkdownView::new)).clone();
                let (tiles, ext) = (self.tiles.clone(), self.ext.clone());
                let link = handler(node, Listen::Link).map(|h| {
                    let (tiles, ext) = (tiles.clone(), ext.clone());
                    Rc::new(move |url: &str, _: &mut Window, cx: &mut App| {
                        tiles.read(cx).send(&ext, Event::Link(ui::LinkClick { handler: h, url: url.to_string(), modifiers: ui::Modifiers::empty() }));
                    }) as Rc<dyn Fn(&str, &mut Window, &mut App)>
                });
                let task = handler(node, Listen::Toggle).map(|h| {
                    Rc::new(move |line: usize, on: bool, _: &mut Window, cx: &mut App| {
                        tiles.read(cx).send(&ext, Event::Toggle(ui::Toggled { handler: h, key: String::new(), index: line as u32, on }));
                    }) as Rc<dyn Fn(usize, bool, &mut Window, &mut App)>
                });
                let text: Arc<str> = m.text.as_str().into();
                let inline = !sized(&node.style);
                let id = format!("tile:{}/{}/{path}", self.ext, self.tile);
                md.update(cx, |v, cx| {
                    v.set_hooks(Hooks { link, task }, cx);
                    v.set_inline(inline, cx);
                    v.set(&id, text, None, None, cx);
                });
            }
            Element::Image(im) => {
                if let ui::ImageSource::Asset(p) = &im.source {
                    if c.asset.as_ref().is_none_or(|(had, _)| had != p) {
                        let file = crate::extensions::folder_of(&self.ext, cx).ok_or_else(|| "the extension's folder isn't found".to_string()).and_then(|d| asset_file(&d, p));
                        c.asset = Some((p.clone(), file));
                    }
                }
            }
            _ => {}
        }
    }

    /// Component node `i` as an element.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_comp(&self, i: usize, node: &Node, path: &str, inherit: Inherit, window: &mut Window, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = SharedString::from(format!("tile-{}-{}-{path}", self.ext, self.tile));
        let sel = format!("tile-node-{path}");
        let st = self.nodes.get(path);
        let _ = (i, inherit);
        match &node.element {
            Element::VirtualList(_) | Element::Table(_) | Element::Tree(_) => self.list_el(node, path, id, window, t, cx),
            Element::Tabs(tabs) => {
                let h = handler(node, Listen::Select);
                let keys: Vec<String> = tabs.items.iter().map(|t| t.key.clone()).collect();
                let items = tabs
                    .items
                    .iter()
                    .map(|tab| {
                        let mut s = kit::SegItem::new(tab.label.clone(), tabs.selected.as_deref() == Some(tab.key.as_str()));
                        s.icon = tab.icon.as_ref().map(|ui::Icon::Named(n)| n.clone()).filter(|n| kit::has_icon(n)).map(Into::into);
                        s
                    })
                    .collect();
                let (tiles, ext) = (self.tiles.clone(), self.ext.clone());
                let pick: kit::OnPick = Rc::new(move |ix, _, cx| {
                    if let (Some(h), Some(key)) = (h, keys.get(ix)) {
                        tiles.read(cx).send(&ext, Event::Select(ui::Selection { handler: h, key: key.clone(), index: ix as u32, modifiers: ui::Modifiers::empty(), activate: false }));
                    }
                });
                let (d, _) = render::apply(div(), &node.style, t);
                d.flex_row().debug_selector(move || sel.clone()).child(kit::seg(id, items, t, pick)).into_any_element()
            }
            Element::Select(s) => {
                let open = st.and_then(|s| s.comp.as_ref()).is_some_and(|c| c.open) && !s.disabled;
                let h = handler(node, Listen::Select);
                let chosen = s.selected.as_ref().and_then(|k| s.options.iter().find(|o| &o.key == k));
                let text = chosen.map(|o| o.label.clone()).or_else(|| s.placeholder.clone()).unwrap_or_default();
                let items: Vec<(SharedString, bool)> = s.options.iter().map(|o| (SharedString::from(o.label.clone()), s.selected.as_ref() == Some(&o.key))).collect();
                let keys: Vec<String> = s.options.iter().map(|o| o.key.clone()).collect();
                let me = cx.entity().downgrade();
                let p = path.to_string();
                let toggle = {
                    let (me, p) = (me.clone(), p.clone());
                    move |_: &mut Window, cx: &mut App| {
                        let _ = me.update(cx, |v, cx| v.set_open(&p, None, cx));
                    }
                };
                let close: kit::OnClose = {
                    let (me, p) = (me.clone(), p.clone());
                    Rc::new(move |_, cx| {
                        let _ = me.update(cx, |v, cx| v.set_open(&p, Some(false), cx));
                    })
                };
                let pick: kit::OnPick = Rc::new(move |ix, _, cx| {
                    let _ = me.update(cx, |v, cx| {
                        v.set_open(&p, Some(false), cx);
                        if let (Some(h), Some(key)) = (h, keys.get(ix)) {
                            v.send(Event::Select(ui::Selection { handler: h, key: key.clone(), index: ix as u32, modifiers: ui::Modifiers::empty(), activate: false }), cx);
                        }
                    });
                });
                let full = node.style.iter().any(|s| matches!(s, ui::Style::Width(_) | ui::Style::Grow(_)));
                let (d, _) = render::apply(div(), &node.style, t);
                let el = kit::select(id, text, 26., 12., full, open, items, t, toggle, pick, close);
                d.debug_selector(move || sel.clone()).when(s.disabled, |d| d.opacity(0.5)).child(el).into_any_element()
            }
            Element::MenuButton(m) => {
                let small = m.label.is_empty();
                let mut el = kit::button(id, if small { kit::BtnKind::Small } else { kit::BtnKind::Ghost }, m.disabled, t).debug_selector(move || sel.clone());
                if let Some(ui::Icon::Named(name)) = &m.icon {
                    if kit::has_icon(name) {
                        el = el.child(kit::icon(name, 13., t.text_2));
                    }
                }
                if small {
                    el = el.w(px(26.)).px(px(0.)).justify_center();
                } else {
                    el = el.child(m.label.clone()).child(kit::icon("chevron-down", 11., t.text_3));
                }
                let el = el.when_some(node.tooltip.clone(), |d, tip| d.tooltip(kit::tooltip(tip)));
                if m.disabled {
                    return el.into_any_element();
                }
                let entries = m.entries.clone();
                let h = handler(node, Listen::Select);
                let bounds = st.map(|s| s.bounds.clone()).unwrap_or_default();
                el.relative()
                    .child(bounds_probe(st))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |v, e: &gpui::ClickEvent, _, cx| {
                        cx.stop_propagation();
                        let at = bounds.get().map(|b| b.bottom_left() + gpui::point(px(0.), px(4.))).unwrap_or(e.position());
                        v.ui.open = Some(OpenMenu { entries: entries.clone(), handler: h, at });
                        cx.notify();
                    }))
                    .into_any_element()
            }
            // Not drawn where it is: out of the flow (no gap for it).
            Element::ContextMenu(_) => div().absolute().size_0().into_any_element(),
            Element::Markdown(_) => {
                let Some(md) = st.and_then(|s| s.comp.as_ref()).and_then(|c| c.md.clone()) else { return div().into_any_element() };
                let inline = !sized(&node.style);
                let (d, _) = render::apply(div(), &node.style, t);
                d.id(id).min_h_0().min_w_0().when(!inline, |d| d.overflow_hidden()).debug_selector(move || sel.clone()).child(md).into_any_element()
            }
            Element::Image(im) => {
                let (d, _) = render::apply(div(), &node.style, t);
                let fit = match im.fit {
                    ui::ImageFit::Contain => ObjectFit::Contain,
                    ui::ImageFit::Cover => ObjectFit::Cover,
                    ui::ImageFit::Fill => ObjectFit::Fill,
                };
                let muted = t.text_3;
                let fallback = move || div().size_full().flex().items_center().justify_center().child(kit::icon("image", 16., muted)).into_any_element();
                let image = match &im.source {
                    ui::ImageSource::Asset(_) => match st.and_then(|s| s.comp.as_ref()).and_then(|c| c.asset.as_ref()) {
                        Some((_, Ok(file))) => Some(img(file.clone())),
                        _ => None,
                    },
                    ui::ImageSource::Data(name) => self.tiles.read(cx).get(&self.ext).and_then(|r| r.comps.images.get(name).cloned()).map(img),
                };
                let has_w = node.style.iter().any(|s| matches!(s, ui::Style::Width(_)));
                let has_h = node.style.iter().any(|s| matches!(s, ui::Style::Height(_)));
                let el = match image {
                    Some(i) => i.when(has_w, |i| i.w_full()).when(!has_w, |i| i.max_w_full()).when(has_h, |i| i.h_full()).object_fit(fit).with_fallback(fallback).into_any_element(),
                    None => fallback(),
                };
                d.id(id).overflow_hidden().debug_selector(move || sel.clone()).child(el).into_any_element()
            }
            _ => div().into_any_element(),
        }
    }

    /// Open (`Some(true)`), close or toggle (`None`) the select at `path`.
    fn set_open(&mut self, path: &str, open: Option<bool>, cx: &mut Context<Self>) {
        if let Some(c) = self.nodes.get_mut(path).and_then(|s| s.comp.as_mut()) {
            c.open = open.unwrap_or(!c.open);
            cx.notify();
        }
    }

    // ---- Menus ----------------------------------------------------------

    /// `d`, opening the context menu of node `k` on a right-click.
    pub(super) fn context_menu_on(&self, d: gpui::Stateful<gpui::Div>, path: &str, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let Some(&k) = self.ui.menus.get(path) else { return d };
        let Some(node) = self.frame.as_ref().and_then(|f| f.nodes.get(k)) else { return d };
        let Element::ContextMenu(entries) = &node.element else { return d };
        let (entries, h) = (entries.clone(), handler(node, Listen::Select));
        d.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |v, e: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
                v.ui.open = Some(OpenMenu { entries: entries.clone(), handler: h, at: e.position });
                cx.notify();
            }),
        )
    }

    /// Close the open menu; whether one was.
    pub fn close_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let was = self.ui.open.take().is_some();
        if was {
            cx.notify();
        }
        was
    }

    /// The open menu, drawn above the window (a glass pop-up under Liquid
    /// Glass).
    pub(super) fn menu_overlay(&self, window: &Window, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let m = self.ui.open.clone()?;
        let me = cx.entity().downgrade();
        let rows = move |t: &Theme, hosted: bool| {
            let mut n = 0u32;
            let mut out: Vec<AnyElement> = Vec::new();
            for (j, e) in m.entries.iter().enumerate() {
                match e {
                    MenuEntry::Separator => out.push(div().h(px(1.)).mx(px(4.)).my(px(4.)).bg(t.line).into_any_element()),
                    MenuEntry::Item(item) => {
                        let index = n;
                        n += 1;
                        let icon = item.icon.as_ref().map(|ui::Icon::Named(n)| n.as_str()).filter(|n| kit::has_icon(n)).unwrap_or("");
                        let key = item.key.clone();
                        let row = kit::menu_item(SharedString::from(format!("tile-menu-{j}")), icon, item.label.clone(), item.hint.clone().map(Into::into), false, t).debug_selector({
                            let k = item.key.clone();
                            move || format!("tile-menu-item-{k}")
                        });
                        let row = row.when(item.danger, |d| d.text_color(t.red));
                        if item.disabled {
                            out.push(row.opacity(0.45).cursor_default().into_any_element());
                            continue;
                        }
                        let (me, h) = (me.clone(), m.handler);
                        let run = move |_: &mut Window, cx: &mut App| {
                            let _ = me.update(cx, |v, cx| {
                                v.ui.open = None;
                                if let Some(h) = h {
                                    v.send(Event::Select(ui::Selection { handler: h, key: key.clone(), index, modifiers: ui::Modifiers::empty(), activate: false }), cx);
                                }
                                cx.notify();
                            });
                        };
                        out.push(
                            row.on_click(move |_, window, cx| {
                                let run = run.clone();
                                if hosted {
                                    kit::pop::pick(window, cx, run);
                                } else {
                                    run(window, cx);
                                }
                            })
                            .into_any_element(),
                        );
                    }
                }
            }
            out
        };
        if kit::pop::hosted(cx) && kit::pop::parent_of(window).is_none() {
            let me = cx.entity().downgrade();
            let rows = Rc::new(rows);
            let build: kit::pop::Build = Rc::new(move |_: &mut Window, cx: &mut App| kit::menu_body().children(rows(&kit::pop::theme(cx), true)).into_any_element());
            let on_dismiss: kit::pop::OnDismiss = Rc::new(move |_, cx| {
                let _ = me.update(cx, |v, cx| {
                    v.ui.open = None;
                    cx.notify();
                });
            });
            let pop = kit::pop::Pop { at: Some(m.at), radius: kit::pop::MENU_RADIUS, focus: None, build, on_dismiss };
            return Some(kit::pop::anchor("tile-menu-pop", pop).into_any_element());
        }
        let ft = t.float();
        let panel = kit::menu_panel(&ft)
            .id("tile-menu")
            .debug_selector(|| "tile-menu".into())
            .occlude()
            .children(rows(&ft, false))
            .on_mouse_down_out(cx.listener(|v, _, _, cx| {
                v.close_menu(cx);
            }));
        Some(deferred(anchored().position(m.at).snap_to_window().child(panel)).with_priority(1).into_any_element())
    }

    // ---- Lists, tables, trees ---------------------------------------------

    fn list_el(&self, node: &Node, path: &str, id: SharedString, window: &mut Window, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(vl) = list_of(node) else { return div().into_any_element() };
        let Some(lu) = self.nodes.get(path).and_then(|s| s.comp.as_ref()).and_then(|c| c.list.as_ref()) else { return div().into_any_element() };
        let p = path.to_string();
        let list = uniform_list(id.clone(), vl.count as usize, cx.processor(move |this: &mut TileView, range: std::ops::Range<usize>, window, cx| this.list_rows(&p, range, window, cx))).track_scroll(&lu.scroll).size_full();
        let body = kit::vscroll_list_fill(SharedString::from(format!("{id}-bar")), &lu.scroll, t, list).flex_1().min_h_0();
        let header = match &node.element {
            Element::Table(table) => Some(self.table_header(table, path, lu, t, cx)),
            _ => None,
        };
        let sel = format!("tile-node-{path}");
        let (d, _) = render::apply(div().flex_1().min_h_0().min_w_0().w_full(), &node.style, t);
        let p = path.to_string();
        let _ = window;
        d.id(id)
            .flex()
            .flex_col()
            .debug_selector(move || sel.clone())
            .track_focus(&lu.focus)
            .on_key_down(cx.listener(move |v, e: &KeyDownEvent, window, cx| v.list_key(&p, e, window, cx)))
            .children(header)
            .child(body)
            .into_any_element()
    }

    fn table_header(&self, table: &ui::Table, path: &str, lu: &ListUi, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let node = self.node(path);
        let (sort_h, resize_h) = (node.and_then(|n| handler(n, Listen::Sort)), node.and_then(|n| handler(n, Listen::ColumnResized)));
        let gutter = if lu.scroll.0.borrow().base_handle.max_offset().y > px(0.) { kit::scrollbar::GUTTER } else { 0. };
        let mut row = div().flex().flex_row().flex_none().h(px(HEADER_H)).pr(px(gutter)).border_b_1().border_color(t.line).text_size(px(12.)).text_color(t.text_3).font_weight(gpui::FontWeight::MEDIUM);
        for (j, c) in table.columns.iter().enumerate() {
            let sorted = table.sort.as_ref().filter(|s| s.column == c.key).map(|s| s.direction);
            let w = lu.width(c);
            let (key, p) = (c.key.clone(), path.to_string());
            let mut cell = div()
                .id(SharedString::from(format!("tile-col-{path}-{j}")))
                .debug_selector({
                    let k = c.key.clone();
                    let p = path.to_string();
                    move || format!("tile-col-{p}-{k}")
                })
                .relative()
                .h_full()
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(4.))
                .min_w_0()
                .map(|d| match w {
                    Some(w) => d.w(px(w)).flex_none(),
                    None => d.flex_1(),
                })
                .map(|d| match c.align {
                    ui::Align::End => d.justify_end(),
                    ui::Align::Center => d.justify_center(),
                    _ => d,
                })
                .child(div().min_w_0().truncate().child(c.title.clone()))
                .when_some(sorted, |d, dir| d.text_color(t.text_2).child(kit::icon(if dir == ui::SortDirection::Ascending { "chevron-up" } else { "chevron-down" }, 10., t.text_2)));
            if c.sortable {
                if let Some(h) = sort_h {
                    let next = match sorted {
                        Some(ui::SortDirection::Ascending) => ui::SortDirection::Descending,
                        _ => ui::SortDirection::Ascending,
                    };
                    let hover = t.text;
                    cell = cell.cursor_pointer().hover(move |s| s.text_color(hover)).on_click(cx.listener(move |v, _, _, cx| {
                        v.send(Event::Sort(ui::SortChange { handler: h, column: key.clone(), direction: next }), cx);
                    }));
                }
            }
            // The edge to drag.
            let (col, start_w) = (c.key.clone(), w);
            let grip = div()
                .id(SharedString::from(format!("tile-col-grip-{path}-{j}")))
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(-3.))
                .w(px(6.))
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, e: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let measured = start_w.unwrap_or(120.);
                        if let Some(l) = v.list_ui(&p) {
                            l.drag = Some((col.clone(), f32::from(e.position.x), measured));
                        }
                    }),
                )
                .on_drag(ColumnDrag, |_, _, _, cx| cx.new(|_| NoGhost));
            row = row.child(cell.child(grip));
        }
        let (p, p2) = (path.to_string(), path.to_string());
        let cols = table.columns.clone();
        row.id(SharedString::from(format!("tile-head-{path}")))
            .on_drag_move::<ColumnDrag>(cx.listener(move |v, e: &gpui::DragMoveEvent<ColumnDrag>, _, cx| {
                let x = f32::from(e.event.position.x);
                let own = |k: &str| cols.iter().find(|c| c.key == k).and_then(|c| c.width);
                if let Some(l) = v.list_ui(&p) {
                    if let Some((col, from, w0)) = l.drag.clone() {
                        let w = (w0 + x - from).max(MIN_COL);
                        l.widths.insert(col.clone(), (own(&col), w));
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(move |v, _, _, cx| v.end_drag(&p2, resize_h, cx)))
            .into_any_element()
    }

    fn end_drag(&mut self, path: &str, h: Option<u32>, cx: &mut Context<Self>) {
        let Some(l) = self.list_ui(path) else { return };
        let Some((col, _, _)) = l.drag.take() else { return };
        let width = l.widths.get(&col).map(|(_, w)| *w);
        if let (Some(h), Some(width)) = (h, width) {
            self.send(Event::ColumnResized(ui::ColumnWidth { handler: h, column: col, width }), cx);
        }
    }

    fn list_ui(&mut self, path: &str) -> Option<&mut ListUi> {
        self.nodes.get_mut(path)?.comp.as_mut()?.list.as_mut()
    }

    /// The rows of the list at `path` in `range`, as elements: the ones the
    /// tile sent, blank ones while a page is on its way (asked for here,
    /// with a page either side).
    fn list_rows(&mut self, path: &str, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(node) = self.node(path).cloned() else { return vec![] };
        let Some(vl) = list_of(&node).cloned() else { return vec![] };
        if range.is_empty() {
            return vec![];
        }
        let rev = vl.revision;
        let view = self.tile.clone();
        let rows: Vec<Option<(u64, Row)>> = {
            let tiles = self.tiles.read(cx);
            let comps = tiles.get(&self.ext).map(|r| &r.comps);
            range.clone().map(|ix| comps.and_then(|c| c.row(&view, path, ix as u32)).map(|(r, row, _)| (*r, row.clone()))).collect()
        };
        // Ask for the pages in sight and one either side.
        let page = |ix: usize| ix as u32 / ROWS_PAGE;
        let (first, last) = (page(range.start).saturating_sub(1), page(range.end.saturating_sub(1)) + 1);
        let now = Instant::now();
        let mut asks = Vec::new();
        if let Some(l) = self.list_ui(path) {
            for p in first..=last {
                let start = p * ROWS_PAGE;
                if start >= vl.count {
                    break;
                }
                let missing = range.clone().zip(rows.iter()).any(|(ix, r)| page(ix) == p && r.is_none());
                let need = match l.asked.get(&p) {
                    Some((r, at)) => *r != rev || (missing && now.duration_since(*at) > ASK_AGAIN),
                    None => true,
                };
                if need {
                    l.asked.insert(p, (rev, now));
                    asks.push(start);
                }
            }
        }
        for start in asks {
            self.tiles.read(cx).ask_rows(&self.ext, &view, path, start, ROWS_PAGE);
        }
        // A row moved to before it arrived: told now.
        let pending = self.list_ui(path).and_then(|l| l.pending);
        if let Some(ix) = pending {
            if let Some(Some((_, row))) = range.clone().position(|i| i == ix).map(|k| &rows[k]) {
                let key = row.key.clone();
                if let Some(l) = self.list_ui(path) {
                    l.pending = None;
                }
                if let Some(h) = handler(&node, Listen::Select) {
                    self.send(Event::Select(ui::Selection { handler: h, key, index: ix as u32, modifiers: ui::Modifiers::empty(), activate: false }), cx);
                }
            }
        }
        let t = crate::theme::theme(cx).clone();
        let focused = self.nodes.get(path).and_then(|s| s.comp.as_ref()).and_then(|c| c.list.as_ref()).is_some_and(|l| l.focus.contains_focused(window, cx));
        let cursor = self.nodes.get(path).and_then(|s| s.comp.as_ref()).and_then(|c| c.list.as_ref()).and_then(|l| l.cursor);
        range.zip(rows).map(|(ix, row)| self.row_el(&node, &vl, path, ix, row.as_ref().map(|r| &r.1), focused && cursor == Some(ix), &t, cx)).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn row_el(&self, list: &Node, vl: &ui::VirtualList, path: &str, ix: usize, row: Option<&Row>, at_cursor: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let h = px(vl.row_height);
        let Some(row) = row else {
            return div().id(("tile-row", ix)).h(h).w_full().into_any_element();
        };
        let selected = vl.selected.contains(&row.key);
        let hover = t.surface_2;
        let (on_select, on_toggle) = (handler(list, Listen::Select), handler(list, Listen::Toggle));
        let key = row.key.clone();
        let tree = matches!(list.element, Element::Tree(_));
        let branch = if tree { row.expanded } else { None };
        let p = path.to_string();
        let sel_key = format!("tile-row-{path}/{}", row.key);
        let menu = row.nodes.iter().find_map(|n| match &n.element {
            Element::ContextMenu(e) => Some((e.clone(), handler(n, Listen::Select))),
            _ => None,
        });
        let mut d = div()
            .id(("tile-row", ix))
            .debug_selector(move || sel_key.clone())
            .h(h)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .rounded(RADIUS_SM)
            .when(selected, |d| d.bg(t.surface_3))
            .when(!selected, |d| d.hover(move |s| s.bg(hover)))
            .when(at_cursor && !selected, |d| d.border_1().border_color(t.focus))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_click(cx.listener({
                let (key, p) = (key.clone(), p.clone());
                move |v, e: &gpui::ClickEvent, window, cx| {
                    if let Some(l) = v.list_ui(&p) {
                        l.cursor = Some(ix);
                        let f = l.focus.clone();
                        window.focus(&f, cx);
                    }
                    let twice = e.click_count() >= 2;
                    match (twice, branch, on_toggle) {
                        (true, Some(open), Some(h)) => {
                            v.send(Event::Toggle(ui::Toggled { handler: h, key: key.clone(), index: ix as u32, on: !open }), cx);
                        }
                        _ => {
                            if let Some(h) = on_select {
                                v.send(Event::Select(ui::Selection { handler: h, key: key.clone(), index: ix as u32, modifiers: modifiers(&e.modifiers()), activate: twice }), cx);
                            }
                        }
                    }
                    cx.notify();
                }
            }));
        if let Some((entries, mh)) = menu {
            let key = key.clone();
            d = d.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |v, e: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some(h) = on_select {
                        v.send(Event::Select(ui::Selection { handler: h, key: key.clone(), index: ix as u32, modifiers: ui::Modifiers::empty(), activate: false }), cx);
                    }
                    v.ui.open = Some(OpenMenu { entries: entries.clone(), handler: mh, at: e.position });
                    cx.notify();
                }),
            );
        }
        if tree {
            d = d.pl(px(4. + INDENT * row.depth as f32)).child(match branch {
                Some(open) => {
                    let key = key.clone();
                    div()
                        .id(("tile-chevron", ix))
                        .debug_selector({
                            let s = format!("tile-chevron-{path}/{}", row.key);
                            move || s.clone()
                        })
                        .w(px(16.))
                        .h_full()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .child(kit::chevron(("tile-chev", ix), open, 10., t.text_4))
                        .on_click(cx.listener(move |v, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(h) = on_toggle {
                                v.send(Event::Toggle(ui::Toggled { handler: h, key: key.clone(), index: ix as u32, on: !open }), cx);
                            }
                        }))
                        .into_any_element()
                }
                None => div().w(px(16.)).flex_none().into_any_element(),
            });
        } else {
            d = d.px(px(if matches!(list.element, Element::Table(_)) { 0. } else { 4. }));
        }
        let kids = row_kids(&row.nodes);
        let inherit = Inherit::root(t);
        let id = format!("tile-{}-{}-{path}/{}", self.ext, self.tile, row.key);
        match &list.element {
            Element::Table(table) => {
                let lu = self.nodes.get(path).and_then(|s| s.comp.as_ref()).and_then(|c| c.list.as_ref());
                let cells: Vec<usize> = kids.first().cloned().unwrap_or_default();
                for (j, c) in table.columns.iter().enumerate() {
                    let w = lu.and_then(|l| l.width(c));
                    let content = cells.get(j).map(|&k| self.row_node(&row.nodes, &kids, k, inherit.with(&row.nodes[0].style, t), &id, t, cx));
                    d = d.child(
                        div()
                            .h_full()
                            .px(px(8.))
                            .min_w_0()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .map(|d| match w {
                                Some(w) => d.w(px(w)).flex_none(),
                                None => d.flex_1(),
                            })
                            .map(|d| match c.align {
                                ui::Align::End => d.justify_end(),
                                ui::Align::Center => d.justify_center(),
                                _ => d,
                            })
                            .children(content),
                    );
                }
            }
            _ => {
                d = d.child(div().flex_1().min_w_0().h_full().flex().items_center().child(self.row_node(&row.nodes, &kids, 0, inherit, &id, t, cx)));
            }
        }
        d.into_any_element()
    }

    /// Node `k` of a row, and what is under it.
    #[allow(clippy::too_many_arguments)]
    fn row_node(&self, nodes: &[Node], kids: &[Vec<usize>], k: usize, inherit: Inherit, id: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let node = &nodes[k];
        let inherit = inherit.with(&node.style, t);
        let eid = SharedString::from(format!("{id}#{k}"));
        let click = handler(node, Listen::Click);
        let send_click = move |v: &mut TileView, e: &gpui::ClickEvent, cx: &mut Context<TileView>| {
            if let Some(h) = click {
                cx.stop_propagation();
                v.send(Event::Click(ui::Click { handler: h, button: ui::MouseButton::Left, modifiers: modifiers(&e.modifiers()), count: e.click_count() as u32, at: ui::Point { x: 0., y: 0. } }), cx);
            }
        };
        match &node.element {
            Element::Text(text) => {
                let mut all = text.clone();
                let mut runs = Vec::new();
                for &c in &kids[k] {
                    if let Element::Text(run) = &nodes[c].element {
                        let start = all.len();
                        all.push_str(run);
                        let mut h = HighlightStyle::default();
                        for s in &nodes[c].style {
                            match s {
                                ui::Style::Color(c) => h.color = Some(render::color(*c, t)),
                                ui::Style::Weight(w) => h.font_weight = Some(render::weight(*w)),
                                ui::Style::Italic => h.font_style = Some(FontStyle::Italic),
                                ui::Style::Underline => h.underline = Some(UnderlineStyle { thickness: px(1.), ..Default::default() }),
                                _ => {}
                            }
                        }
                        runs.push((start..all.len(), h));
                    }
                }
                let (d, f) = render::apply(div(), &node.style, t);
                row_flags(d.id(eid), &f, t, node.tooltip.clone(), click.is_some()).when(click.is_some(), |d| d.on_click(cx.listener(move |v, e, _, cx| send_click(v, e, cx)))).child(StyledText::new(all).with_highlights(runs)).into_any_element()
            }
            Element::Icon(ui::Icon::Named(name)) => {
                let name = if kit::has_icon(name) { name.as_str() } else { "dot" };
                let (d, f) = render::apply(div(), &node.style, t);
                row_flags(d.id(eid).flex_none().items_center().justify_center(), &f, t, node.tooltip.clone(), click.is_some()).child(kit::icon(name, inherit.size + 1., inherit.color)).into_any_element()
            }
            Element::Button(b) => {
                let (kind, shrink) = render::button_kind(b.kind, b.size);
                let fg = match kind {
                    kit::BtnKind::Primary => kit::on_primary(t),
                    kit::BtnKind::Danger => gpui::white(),
                    kit::BtnKind::Small => t.text,
                    _ => t.text_2,
                };
                let mut el = kit::button(eid, kind, b.disabled, t);
                if shrink {
                    el = el.h(px(22.)).px(px(8.)).text_size(px(12.));
                }
                if let Some(ui::Icon::Named(name)) = &b.icon {
                    if kit::has_icon(name) {
                        el = el.child(kit::icon(name, 13., fg));
                    }
                }
                if b.label.is_empty() && b.icon.is_some() {
                    el = el.w(px(22.)).h(px(22.)).px(px(0.)).justify_center();
                }
                el.child(b.label.clone())
                    .when_some(node.tooltip.clone(), |d, tip| d.tooltip(kit::tooltip(tip)))
                    .when(!b.disabled && click.is_some(), |d| d.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(move |v, e, _, cx| send_click(v, e, cx))))
                    .into_any_element()
            }
            Element::Image(im) => {
                let (d, _) = render::apply(div(), &node.style, t);
                let image = match &im.source {
                    ui::ImageSource::Data(name) => self.tiles.read(cx).get(&self.ext).and_then(|r| r.comps.images.get(name).cloned()).map(img),
                    ui::ImageSource::Asset(p) => {
                        let known = self.ui.assets.borrow().get(p).cloned();
                        let file = known.unwrap_or_else(|| {
                            let f = crate::extensions::folder_of(&self.ext, cx).and_then(|d| asset_file(&d, p).ok());
                            self.ui.assets.borrow_mut().insert(p.clone(), f.clone());
                            f
                        });
                        file.map(img)
                    }
                };
                d.id(eid).children(image.map(|i| i.size_full().object_fit(ObjectFit::Contain))).into_any_element()
            }
            Element::Box => {
                let children: Vec<AnyElement> = kids[k].iter().filter(|&&c| !matches!(nodes[c].element, Element::ContextMenu(_))).map(|&c| self.row_node(nodes, kids, c, inherit, id, t, cx)).collect();
                let base = if k == 0 { div().flex_1().min_w_0() } else { div() };
                let (d, f) = render::apply(base, &node.style, t);
                row_flags(d.id(eid), &f, t, node.tooltip.clone(), click.is_some()).when(click.is_some(), |d| d.on_click(cx.listener(move |v, e, _, cx| send_click(v, e, cx)))).children(children).into_any_element()
            }
            _ => self.row_widget(node, inherit, t, cx).unwrap_or_else(|| div().into_any_element()),
        }
    }

    /// Keys in a list with focus: move the selection, activate, open and
    /// close tree branches.
    fn list_key(&mut self, path: &str, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &e.keystroke;
        if ks.modifiers.platform || ks.modifiers.control || ks.modifiers.alt {
            return;
        }
        let Some(node) = self.node(path).cloned() else { return };
        let Some(vl) = list_of(&node).cloned() else { return };
        let count = vl.count as usize;
        if count == 0 {
            return;
        }
        let view = self.tile.clone();
        // Where the keys start: the cursor, else the first selected row in
        // what is kept.
        let cursor = self.list_ui(path).and_then(|l| l.cursor).or_else(|| {
            let tiles = self.tiles.read(cx);
            let keys = tiles.get(&self.ext).map(|r| r.comps.row_keys(&view, path)).unwrap_or_default();
            keys.into_iter().find(|(_, k)| vl.selected.contains(k)).map(|(i, _)| i as usize)
        });
        let visible = (f32::from(self.list_ui(path).map(|l| l.scroll.0.borrow().base_handle.bounds().size.height).unwrap_or_default()) / vl.row_height).floor().max(1.) as usize;
        let row_at = |v: &TileView, ix: usize, cx: &App| v.tiles.read(cx).get(&v.ext).and_then(|r| r.comps.row(&view, path, ix as u32)).map(|(_, row, _)| row.clone());
        let tree = matches!(node.element, Element::Tree(_));
        let c = cursor.unwrap_or(0);
        let to = match ks.key.as_str() {
            "up" => cursor.map_or(0, |c| c.saturating_sub(1)),
            "down" => cursor.map_or(0, |c| (c + 1).min(count - 1)),
            "home" => 0,
            "end" => count - 1,
            "pageup" => c.saturating_sub(visible),
            "pagedown" => (c + visible).min(count - 1),
            "enter" => {
                if let (Some(c), Some(h)) = (cursor, handler(&node, Listen::Select)) {
                    if let Some(row) = row_at(self, c, cx) {
                        self.send(Event::Select(ui::Selection { handler: h, key: row.key, index: c as u32, modifiers: ui::Modifiers::empty(), activate: true }), cx);
                    }
                }
                cx.stop_propagation();
                return;
            }
            "left" | "right" if tree => {
                let Some(c) = cursor else { return };
                let Some(row) = row_at(self, c, cx) else { return };
                let open = ks.key == "right";
                match (row.expanded, handler(&node, Listen::Toggle)) {
                    (Some(was), Some(h)) if was != open => {
                        self.send(Event::Toggle(ui::Toggled { handler: h, key: row.key, index: c as u32, on: open }), cx);
                        cx.stop_propagation();
                        return;
                    }
                    // Left on a leaf or a closed branch: to its parent.
                    _ if !open && row.depth > 0 => (0..c).rev().find(|&i| row_at(self, i, cx).is_some_and(|r| r.depth < row.depth)).unwrap_or(c),
                    _ => {
                        cx.stop_propagation();
                        return;
                    }
                }
            }
            _ => return,
        };
        cx.stop_propagation();
        let row = row_at(self, to, cx);
        if let Some(l) = self.list_ui(path) {
            l.cursor = Some(to);
            l.scroll.scroll_to_item(to, ScrollStrategy::Nearest);
            l.pending = if row.is_none() { Some(to) } else { None };
        }
        if let (Some(row), Some(h)) = (row, handler(&node, Listen::Select)) {
            self.send(Event::Select(ui::Selection { handler: h, key: row.key, index: to as u32, modifiers: modifiers(&ks.modifiers), activate: false }), cx);
        }
        let _ = window;
        cx.notify();
    }

    // ---- For tests and bench commands (nothing is clicked on the screen) --

    /// The keys of the rows Pitwall has of the list at `path`, by index.
    pub fn row_keys(&self, path: &str, cx: &App) -> Vec<(u32, String)> {
        self.tiles.read(cx).get(&self.ext).map(|r| r.comps.row_keys(&self.tile, path)).unwrap_or_default()
    }

    /// Keep the bounds of the nodes at `paths` as they're drawn, for
    /// screenshots of one component (tests).
    #[cfg(test)]
    pub fn probe(&mut self, paths: &[&str]) {
        self.ui.probe.extend(paths.iter().map(|p| p.to_string()));
    }

    /// Where the node at `path` was last drawn (probed or listening).
    #[cfg(test)]
    pub fn node_bounds(&self, path: &str) -> Option<gpui::Bounds<Pixels>> {
        self.nodes.get(path).and_then(|s| s.bounds.get())
    }

    /// Scroll the list at `path` to row `ix`.
    pub fn scroll_list_to(&self, path: &str, ix: usize) {
        if let Some(l) = self.nodes.get(path).and_then(|s| s.comp.as_ref()).and_then(|c| c.list.as_ref()) {
            l.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        }
    }

    /// Open the context menu of the node at `path` (or the select there)
    /// at `at`, as a right-click (a click) would.
    pub fn open_menu_of(&mut self, path: &str, at: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        if let Some(c) = self.nodes.get_mut(path).and_then(|s| s.comp.as_mut()) {
            if !c.open && self.node(path).is_some_and(|n| matches!(n.element, Element::Select(_))) {
                self.set_open(path, Some(true), cx);
                return true;
            }
        }
        let menu = match self.node(path).map(|n| &n.element) {
            Some(Element::MenuButton(m)) => self.node(path).map(|n| (m.entries.clone(), handler(n, Listen::Select))),
            _ => self.ui.menus.get(path).and_then(|&k| self.frame.as_ref()?.nodes.get(k)).and_then(|n| match &n.element {
                Element::ContextMenu(e) => Some((e.clone(), handler(n, Listen::Select))),
                _ => None,
            }),
        };
        let Some((entries, handler)) = menu else { return false };
        self.ui.open = Some(OpenMenu { entries, handler, at });
        cx.notify();
        true
    }

    /// The labels of the open menu's items.
    pub fn open_menu(&self) -> Option<Vec<String>> {
        let m = self.ui.open.as_ref()?;
        Some(
            m.entries
                .iter()
                .filter_map(|e| match e {
                    MenuEntry::Item(i) => Some(i.label.clone()),
                    MenuEntry::Separator => None,
                })
                .collect(),
        )
    }
}

/// Hover, cursor and tooltip of a row's node.
fn row_flags(d: gpui::Stateful<gpui::Div>, f: &render::Flags, t: &Theme, tooltip: Option<String>, clicks: bool) -> gpui::Stateful<gpui::Div> {
    d.when_some(f.hover_bg, |d, c| {
        let c = render::color(c, t);
        d.hover(move |s| s.bg(c))
    })
    .when_some(tooltip, |d, tip| d.tooltip(kit::tooltip(tip)))
    .map(|d| match f.cursor {
        Some(ui::Cursor::Pointer) => d.cursor_pointer(),
        Some(ui::Cursor::Text) => d.cursor_text(),
        Some(ui::Cursor::Default) => d.cursor_default(),
        None if clicks => d.cursor_pointer(),
        None => d,
    })
}

/// Each row node's children, by index.
fn row_kids(nodes: &[Node]) -> Vec<Vec<usize>> {
    let mut kids = vec![Vec::new(); nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        if let Some(p) = n.parent.map(|p| p as usize).filter(|p| *p < i) {
            kids[p].push(i);
        }
    }
    kids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_stay_in_the_folder() {
        let t = pitwall_core::testing::TempDir::new("tile-assets");
        let dir = t.path().join("ext");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/logo.svg"), "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
        std::fs::write(dir.join("assets/notes.txt"), "hello").unwrap();
        std::fs::write(t.path().join("outside.svg"), "<svg/>").unwrap();
        assert!(asset_file(&dir, "assets/logo.svg").is_ok());
        assert!(asset_file(&dir, "assets/notes.txt").unwrap_err().contains("isn't a PNG"));
        assert!(asset_file(&dir, "../outside.svg").is_err());
        assert!(asset_file(&dir, "assets/missing.png").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(t.path().join("outside.svg"), dir.join("assets/link.svg")).unwrap();
            assert!(asset_file(&dir, "assets/link.svg").unwrap_err().contains("outside"));
        }
    }

    #[test]
    fn rows_are_kept_by_index_and_the_oldest_let_go() {
        let mut c = Comps::default();
        let row = |k: &str| Row { key: k.into(), nodes: vec![], depth: 0, expanded: None };
        let u = |start: u32, n: u32, rev: u64| RowsUpdate { view: "v".into(), list: "l".into(), revision: rev, start, rows: Arc::new((start..start + n).map(|i| row(&i.to_string())).collect()), took: Duration::ZERO };
        c.rows_arrived(u(0, 64, 1));
        assert_eq!(c.row("v", "l", 10).map(|r| (r.0, r.1.key.as_str())), Some((1, "10")));
        c.rows_arrived(u(0, 64, 2));
        assert_eq!(c.row("v", "l", 10).map(|r| r.0), Some(2));
        for p in 1..40 {
            c.rows_arrived(u(p * 64, 64, 2));
        }
        assert_eq!(c.rows_kept("v", "l"), KEEP_ROWS);
        assert!(c.row("v", "l", 0).is_none(), "the oldest went");
        assert!(c.row("v", "l", 39 * 64).is_some());
    }
}
