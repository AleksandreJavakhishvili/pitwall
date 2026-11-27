//! Components Pitwall draws for a tile with its own code: long lists,
//! tables and trees whose rows the tile renders only as they come into
//! sight, tabs, dropdowns, menus, Markdown and images. They look and
//! behave as Pitwall's own (keyboard, focus, scroll bars, menus that
//! follow the theme), and the tile hears what the user picked.
//!
//! ```ignore
//! // A list of 100 000 rows: only the rows in sight are ever rendered.
//! virtual_list("runs", self.runs.len(), cx.rows(|this: &mut Self, i, _cx| {
//!     let r = &this.runs[i];
//!     h_flex().id(r.id.clone()).gap_2().px_3()
//!         .child(icon(Icon::Check).text_color(Token::Good))
//!         .child(text(&r.name).truncate())
//! }))
//! .selected(self.selected.clone())
//! .on_select(cx.listener(|this, key: &str, cx| { this.selected = Some(key.into()); cx.notify(); }))
//! ```

use std::any::Any;
use std::rc::Rc;

use crate::bindings::pitwall::tile::types as wit;
use crate::element::{AnyElement, Handler, Icon, Interactive, IntoElement, Kind, Listen};
use crate::events::{ColumnWidth, LinkClick, PickEvent, SortChange, SortDirection, Toggled};
use crate::style::{Align, StyleSet, Styled};

/// What a component node is, before it becomes WIT.
#[derive(Debug, Clone)]
pub(crate) enum Spec {
    List(ListSpec),
    Tabs { items: Vec<Tab>, selected: Option<String> },
    Select { options: Vec<Choice>, selected: Option<String>, placeholder: Option<String>, disabled: bool },
    MenuButton { label: String, icon: Option<Icon>, entries: Vec<MenuEntry>, disabled: bool },
    ContextMenu(Vec<MenuEntry>),
    Markdown(String),
    Image { source: wit::ImageSource, fit: wit::ImageFit },
}

/// Which kind of list.
#[derive(Debug, Clone)]
pub(crate) enum ListForm {
    Plain,
    Table { columns: Vec<Column>, sort: Option<(String, SortDirection)> },
    Tree,
}

#[derive(Debug, Clone)]
pub(crate) struct ListSpec {
    pub form: ListForm,
    pub count: usize,
    pub row_height: f32,
    pub revision: Option<u64>,
    pub selected: Vec<String>,
    pub rows: Rows,
}

/// How a list's rows are made: a closure of the tile's, made with
/// [`Cx::rows`](crate::Cx::rows) while rendering. Pitwall calls it for
/// the rows coming into sight, with the tile's state as it is then.
#[derive(Clone)]
pub struct Rows(pub(crate) Rc<dyn Any>);

impl std::fmt::Debug for Rows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rows")
    }
}

/// What a row closure returns: an element (any builder), or a
/// [`TreeRow`] for a tree.
pub trait IntoRow {
    #[doc(hidden)]
    fn into_row(self) -> TreeRow;
}

impl<E: IntoElement> IntoRow for E {
    fn into_row(self) -> TreeRow {
        TreeRow { element: self.into_element(), depth: 0, expanded: None }
    }
}

/// A tree's row: what it shows, how deep it is, and whether it is a
/// branch (open or closed) or a leaf. Made by [`tree_row`].
#[derive(Debug, Clone)]
pub struct TreeRow {
    pub(crate) element: AnyElement,
    pub(crate) depth: u16,
    pub(crate) expanded: Option<bool>,
}

impl TreeRow {
    /// How deep it is: 0 at the top, 1 under a top node, and so on.
    pub fn depth(mut self, depth: usize) -> TreeRow {
        self.depth = depth.min(u16::MAX as usize) as u16;
        self
    }

    /// It is a branch, open (`true`) or closed: Pitwall draws its chevron.
    /// Without this it is a leaf.
    pub fn branch(mut self, open: bool) -> TreeRow {
        self.expanded = Some(open);
        self
    }
}

impl IntoRow for TreeRow {
    fn into_row(self) -> TreeRow {
        self
    }
}

/// A tree's row showing `element` (its `id` is the row's key: give it the
/// node's id). Set [`TreeRow::depth`] and, for a branch,
/// [`TreeRow::branch`].
pub fn tree_row(element: impl IntoElement) -> TreeRow {
    TreeRow { element: element.into_element(), depth: 0, expanded: None }
}

macro_rules! component {
    ($(#[$doc:meta])* $t:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone)]
        pub struct $t(AnyElement);

        impl Styled for $t {
            fn style(&mut self) -> &mut StyleSet {
                &mut self.0.style
            }
        }
        impl Interactive for $t {
            fn node(&mut self) -> &mut AnyElement {
                &mut self.0
            }
        }
        impl IntoElement for $t {
            fn into_element(self) -> AnyElement {
                self.0
            }
        }
    };
}

fn make(key: impl Into<String>, spec: Spec) -> AnyElement {
    let mut e = AnyElement::new(Kind::Component(spec));
    e.key = Some(key.into());
    e
}

fn spec(e: &mut AnyElement) -> &mut Spec {
    match &mut e.kind {
        Kind::Component(s) => s,
        _ => unreachable!("a component"),
    }
}

fn list(e: &mut AnyElement) -> &mut ListSpec {
    match spec(e) {
        Spec::List(l) => l,
        _ => unreachable!("a list"),
    }
}

/// The height Pitwall's own list rows have.
pub const ROW_HEIGHT: f32 = 28.;

macro_rules! list_methods {
    ($t:ident) => {
        impl $t {
            /// Every row's height in logical pixels (12–200; default 28,
            /// Pitwall's own rows').
            pub fn row_height(mut self, px: f32) -> $t {
                list(&mut self.0).row_height = px;
                self
            }

            /// The selected rows, by key (the row element's `id`, or its
            /// index when it has none): drawn selected.
            pub fn selected<I: IntoIterator<Item = S>, S: Into<String>>(mut self, keys: I) -> $t {
                list(&mut self.0).selected = keys.into_iter().map(Into::into).collect();
                self
            }

            /// The version of the rows' data. By default every render
            /// counts as new data and Pitwall asks for the rows in sight
            /// again; give a number that changes only when the rows do
            /// (a revision of your data) to save those calls.
            pub fn revision(mut self, revision: u64) -> $t {
                list(&mut self.0).revision = Some(revision);
                self
            }

            /// Call `handler` when a row is clicked or moved to with the
            /// keys (Up, Down, Home, End, Page Up, Page Down), and when it
            /// is activated (a double click, or Enter: `activate`). It
            /// takes the row's key (`&str`) or the whole
            /// [`Picked`](crate::events::Picked). Draw the selection with
            /// [`selected`](Self::selected).
            pub fn on_select<E: PickEvent + ?Sized>(mut self, handler: Handler<E>) -> $t {
                self.0.listen(Listen::Select, handler);
                self
            }
        }
    };
}

component!(
    /// A long list drawn by Pitwall, its rows rendered by the tile only as
    /// they come into sight. Made by [`virtual_list`].
    VirtualList
);
list_methods!(VirtualList);

/// A list of `count` rows with key `key`; `rows` (made with
/// [`Cx::rows`](crate::Cx::rows)) renders row `i` when it comes into
/// sight. 100 000 rows scroll as smoothly as 100: Pitwall lays out only
/// the rows in sight and asks for them a page at a time. A row holds
/// boxes, text, icons, buttons, images and a context menu; give it the
/// item's id with `.id(..)` so the selection follows the item.
///
/// It fills its parent: give the parent a height (or `flex_1`).
pub fn virtual_list(key: impl Into<String>, count: usize, rows: Rows) -> VirtualList {
    VirtualList(make(key, Spec::List(ListSpec { form: ListForm::Plain, count, row_height: ROW_HEIGHT, revision: None, selected: Vec::new(), rows })))
}

/// A table's column. Made by [`column`].
#[derive(Debug, Clone)]
pub struct Column {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) width: Option<f32>,
    pub(crate) align: Align,
    pub(crate) sortable: bool,
}

impl Column {
    /// Its width in logical pixels. Without one, it shares the room the
    /// columns with widths leave. The user can drag its edge.
    pub fn width(mut self, px: f32) -> Column {
        self.width = Some(px);
        self
    }

    /// Its cells' text at the right (numbers, sizes, dates).
    pub fn align_end(mut self) -> Column {
        self.align = Align::End;
        self
    }

    /// Its cells' text centred.
    pub fn align_center(mut self) -> Column {
        self.align = Align::Center;
        self
    }

    /// Its header can be clicked to sort by it (the table's `on_sort`).
    pub fn sortable(mut self) -> Column {
        self.sortable = true;
        self
    }

    pub(crate) fn wit(&self) -> wit::Column {
        wit::Column { key: self.key.clone(), title: self.title.clone(), width: self.width, align: self.align.wit(), sortable: self.sortable }
    }
}

/// A column with key `key` and header `title`.
pub fn column(key: impl Into<String>, title: impl Into<String>) -> Column {
    Column { key: key.into(), title: title.into(), width: None, align: Align::Start, sortable: false }
}

component!(
    /// A table drawn by Pitwall: a header of columns over a virtual list.
    /// Made by [`table`].
    Table
);
list_methods!(Table);

impl Table {
    /// It is sorted by `column`, `direction`: an arrow in that header.
    pub fn sort(mut self, column: impl Into<String>, direction: SortDirection) -> Table {
        if let ListForm::Table { sort, .. } = &mut list(&mut self.0).form {
            *sort = Some((column.into(), direction));
        }
        self
    }

    /// Call `handler` when a sortable column's header is clicked, with
    /// the column and the direction asked for. Sort your rows, keep the
    /// sort for [`Table::sort`], and render.
    pub fn on_sort(mut self, handler: Handler<SortChange>) -> Table {
        self.0.listen(Listen::Sort, handler);
        self
    }

    /// Call `handler` when the user has resized a column (when the drag
    /// ends), to keep its width for next time.
    pub fn on_column_resize(mut self, handler: Handler<ColumnWidth>) -> Table {
        self.0.listen(Listen::ColumnResized, handler);
        self
    }
}

/// A table with key `key`, these `columns` and `count` rows; `rows`
/// renders row `i` as a row whose children are its cells, one per column
/// in order (`h_flex().child(text(name)).child(text(size))`). Pitwall
/// sizes the cells to the columns, draws the header, and lets the user
/// resize and sort.
pub fn table(key: impl Into<String>, columns: Vec<Column>, count: usize, rows: Rows) -> Table {
    Table(make(key, Spec::List(ListSpec { form: ListForm::Table { columns, sort: None }, count, row_height: ROW_HEIGHT, revision: None, selected: Vec::new(), rows })))
}

component!(
    /// A tree drawn by Pitwall: the open nodes, flattened, as a virtual
    /// list. Made by [`tree`].
    Tree
);
list_methods!(Tree);

impl Tree {
    /// Call `handler` when a branch is opened or closed (its chevron
    /// clicked, Right or Left, or a double click). Open or close it in
    /// your state and render with the new count.
    pub fn on_toggle(mut self, handler: Handler<Toggled>) -> Tree {
        self.0.listen(Listen::Toggle, handler);
        self
    }
}

/// A tree with key `key` showing `count` rows: the nodes that are open to
/// view, flattened in order (a node, then its children when it is open).
/// `rows` returns each as a [`tree_row`] with its depth and, for a
/// branch, whether it is open; Pitwall draws the indent and the chevron.
pub fn tree(key: impl Into<String>, count: usize, rows: Rows) -> Tree {
    Tree(make(key, Spec::List(ListSpec { form: ListForm::Tree, count, row_height: ROW_HEIGHT, revision: None, selected: Vec::new(), rows })))
}

/// One tab. Made by [`tab`].
#[derive(Debug, Clone)]
pub struct Tab {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) icon: Option<Icon>,
}

impl Tab {
    /// An icon before its label.
    pub fn icon(mut self, icon: Icon) -> Tab {
        self.icon = Some(icon);
        self
    }
}

/// A tab with key `key` and label `label`.
pub fn tab(key: impl Into<String>, label: impl Into<String>) -> Tab {
    Tab { key: key.into(), label: label.into(), icon: None }
}

component!(
    /// A row of tabs drawn as Pitwall's own. Made by [`tabs`].
    Tabs
);

impl Tabs {
    /// The selected tab, by key.
    pub fn selected(mut self, key: impl Into<String>) -> Tabs {
        if let Spec::Tabs { selected, .. } = spec(&mut self.0) {
            *selected = Some(key.into());
        }
        self
    }

    /// Call `handler` with the tab chosen (its key as `&str`, or the
    /// whole [`Picked`](crate::events::Picked)).
    pub fn on_select<E: PickEvent + ?Sized>(mut self, handler: Handler<E>) -> Tabs {
        self.0.listen(Listen::Select, handler);
        self
    }
}

/// A row of tabs with key `key`. Show the selected tab's content below it
/// yourself.
pub fn tabs(key: impl Into<String>, items: impl IntoIterator<Item = Tab>) -> Tabs {
    Tabs(make(key, Spec::Tabs { items: items.into_iter().collect(), selected: None }))
}

/// One option of a [`select`]. Made by [`choice`].
#[derive(Debug, Clone)]
pub struct Choice {
    pub(crate) key: String,
    pub(crate) label: String,
}

/// An option with key `key` showing `label`.
pub fn choice(key: impl Into<String>, label: impl Into<String>) -> Choice {
    Choice { key: key.into(), label: label.into() }
}

component!(
    /// A dropdown drawn as Pitwall's own. Made by [`select`].
    Select
);

impl Select {
    /// The chosen option, by key.
    pub fn selected(mut self, key: impl Into<String>) -> Select {
        if let Spec::Select { selected, .. } = spec(&mut self.0) {
            *selected = Some(key.into());
        }
        self
    }

    /// Shown while nothing is chosen.
    pub fn placeholder(mut self, text: impl Into<String>) -> Select {
        if let Spec::Select { placeholder, .. } = spec(&mut self.0) {
            *placeholder = Some(text.into());
        }
        self
    }

    /// Drawn dimmed and can't be opened.
    pub fn disabled(mut self, yes: bool) -> Select {
        if let Spec::Select { disabled, .. } = spec(&mut self.0) {
            *disabled = yes;
        }
        self
    }

    /// Call `handler` with the option chosen (its key as `&str`, or the
    /// whole [`Picked`](crate::events::Picked)).
    pub fn on_select<E: PickEvent + ?Sized>(mut self, handler: Handler<E>) -> Select {
        self.0.listen(Listen::Select, handler);
        self
    }
}

/// A dropdown with key `key` and these options.
pub fn select(key: impl Into<String>, options: impl IntoIterator<Item = Choice>) -> Select {
    Select(make(key, Spec::Select { options: options.into_iter().collect(), selected: None, placeholder: None, disabled: false }))
}

/// A line of a menu: an item ([`menu_item`]) or a [`separator`].
#[derive(Debug, Clone)]
pub struct MenuEntry(pub(crate) Option<MenuItemSpec>);

#[derive(Debug, Clone)]
pub(crate) struct MenuItemSpec {
    key: String,
    label: String,
    icon: Option<Icon>,
    hint: Option<String>,
    disabled: bool,
    danger: bool,
}

impl MenuEntry {
    fn item(&mut self) -> Option<&mut MenuItemSpec> {
        self.0.as_mut()
    }

    /// An icon before its label.
    pub fn icon(mut self, icon: Icon) -> MenuEntry {
        if let Some(i) = self.item() {
            i.icon = Some(icon);
        }
        self
    }

    /// A shortcut shown at its right (`"⌘D"`). Only shown: bind the key
    /// with [`Cx::bind_key`](crate::Cx::bind_key).
    pub fn hint(mut self, text: impl Into<String>) -> MenuEntry {
        if let Some(i) = self.item() {
            i.hint = Some(text.into());
        }
        self
    }

    /// Drawn dimmed and can't be chosen.
    pub fn disabled(mut self, yes: bool) -> MenuEntry {
        if let Some(i) = self.item() {
            i.disabled = yes;
        }
        self
    }

    /// A destructive choice, drawn in red.
    pub fn danger(mut self) -> MenuEntry {
        if let Some(i) = self.item() {
            i.danger = true;
        }
        self
    }

    pub(crate) fn wit(&self) -> wit::MenuEntry {
        match &self.0 {
            None => wit::MenuEntry::Separator,
            Some(i) => wit::MenuEntry::Item(wit::MenuItem {
                key: i.key.clone(),
                label: i.label.clone(),
                icon: i.icon.as_ref().map(|i| wit::Icon::Named(i.name().to_string())),
                hint: i.hint.clone(),
                disabled: i.disabled,
                danger: i.danger,
            }),
        }
    }
}

/// A menu item with key `key` (what `on_select` gets) showing `label`.
pub fn menu_item(key: impl Into<String>, label: impl Into<String>) -> MenuEntry {
    MenuEntry(Some(MenuItemSpec { key: key.into(), label: label.into(), icon: None, hint: None, disabled: false, danger: false }))
}

/// A thin line between groups of menu items.
pub fn separator() -> MenuEntry {
    MenuEntry(None)
}

component!(
    /// A button that opens a menu, drawn by Pitwall. Made by
    /// [`menu_button`].
    MenuButton
);

impl MenuButton {
    /// An icon before the label (with an empty label: a small square icon
    /// button; give it a [`label`](Interactive::label)).
    pub fn icon(mut self, i: Icon) -> MenuButton {
        if let Spec::MenuButton { icon, .. } = spec(&mut self.0) {
            *icon = Some(i);
        }
        self
    }

    /// Drawn dimmed and can't be opened.
    pub fn disabled(mut self, yes: bool) -> MenuButton {
        if let Spec::MenuButton { disabled, .. } = spec(&mut self.0) {
            *disabled = yes;
        }
        self
    }

    /// Call `handler` with the item chosen (its key as `&str`, or the
    /// whole [`Picked`](crate::events::Picked)).
    pub fn on_select<E: PickEvent + ?Sized>(mut self, handler: Handler<E>) -> MenuButton {
        self.0.listen(Listen::Select, handler);
        self
    }
}

/// A button labelled `label` that opens a menu of `entries` under it.
pub fn menu_button(label: impl Into<String>, entries: Vec<MenuEntry>) -> MenuButton {
    let label = label.into();
    let key = if label.is_empty() { "menu".to_string() } else { label.clone() };
    MenuButton(make(key, Spec::MenuButton { label, icon: None, entries, disabled: false }))
}

/// A small square button with only `icon` that opens a menu; `label` is
/// its accessible name and tooltip.
pub fn menu_icon_button(icon: Icon, label: impl Into<String>, entries: Vec<MenuEntry>) -> MenuButton {
    let label = label.into();
    menu_button("", entries).icon(icon).label(label.clone()).tooltip(label)
}

component!(
    /// Markdown drawn by Pitwall. Made by [`markdown`].
    Markdown
);

impl Markdown {
    /// Call `handler` when a link is clicked, with its target as written
    /// (relative links too: open another note, say). Without it, web and
    /// mail links open in the browser and others do nothing.
    pub fn on_link(mut self, handler: Handler<LinkClick>) -> Markdown {
        self.0.listen(Listen::Link, handler);
        self
    }

    /// Call `handler` when a task's box (`- [ ]`) is clicked, with its
    /// line in the text (from 0) and whether it is ticked now. Change the
    /// text and render. Without it, the boxes can't be clicked.
    pub fn on_toggle(mut self, handler: Handler<Toggled>) -> Markdown {
        self.0.listen(Listen::Toggle, handler);
        self
    }
}

/// `text` as Markdown, drawn as Pitwall's own Preview: headings, lists,
/// task lists, tables, highlighted code, quotes. Inert: HTML is shown as
/// text and nothing is fetched (remote images show their description).
/// At most 1 MiB.
pub fn markdown(text: impl Into<String>) -> Markdown {
    Markdown(make("markdown", Spec::Markdown(text.into())))
}

component!(
    /// An image. Made by [`image_asset`] and [`image_data`].
    Image
);

impl Image {
    fn fit(mut self, f: wit::ImageFit) -> Image {
        if let Spec::Image { fit, .. } = spec(&mut self.0) {
            *fit = f;
        }
        self
    }

    /// Whole, as large as fits its box (the default).
    pub fn contain(self) -> Image {
        self.fit(wit::ImageFit::Contain)
    }

    /// Filling its box, cut at the edges (an avatar, a cover).
    pub fn cover(self) -> Image {
        self.fit(wit::ImageFit::Cover)
    }

    /// Stretched to its box.
    pub fn fill(self) -> Image {
        self.fit(wit::ImageFit::Fill)
    }
}

/// An image from the extension's folder, by its path there
/// (`"assets/logo.png"`): PNG, JPEG, GIF, WebP or SVG, at most 16 MiB.
/// Size it with `w`/`h` (without a width it is as wide as its box); give
/// it a [`label`](Interactive::label) describing it.
pub fn image_asset(path: impl Into<String>) -> Image {
    Image(make("image", Spec::Image { source: wit::ImageSource::Asset(path.into()), fit: wit::ImageFit::Contain }))
}

/// An image from bytes the tile handed Pitwall with
/// [`ui::put_image`](crate::ui::put_image) under `name` (a picture a task
/// made, an attachment). Nothing shows until it is put.
pub fn image_data(name: impl Into<String>) -> Image {
    Image(make("image", Spec::Image { source: wit::ImageSource::Data(name.into()), fit: wit::ImageFit::Contain }))
}

impl Spec {
    /// The WIT element, but for lists (made by the runtime, which keeps
    /// their row closures).
    pub(crate) fn wit(&self) -> Option<wit::Element> {
        Some(match self {
            Spec::List(_) => return None,
            Spec::Tabs { items, selected } => wit::Element::Tabs(wit::Tabs {
                items: items.iter().map(|t| wit::Tab { key: t.key.clone(), label: t.label.clone(), icon: t.icon.as_ref().map(|i| wit::Icon::Named(i.name().to_string())) }).collect(),
                selected: selected.clone(),
            }),
            Spec::Select { options, selected, placeholder, disabled } => wit::Element::Select(wit::Select {
                options: options.iter().map(|o| wit::SelectOption { key: o.key.clone(), label: o.label.clone() }).collect(),
                selected: selected.clone(),
                placeholder: placeholder.clone(),
                disabled: *disabled,
            }),
            Spec::MenuButton { label, icon, entries, disabled } => wit::Element::MenuButton(wit::MenuButton {
                label: label.clone(),
                icon: icon.as_ref().map(|i| wit::Icon::Named(i.name().to_string())),
                entries: entries.iter().map(MenuEntry::wit).collect(),
                disabled: *disabled,
            }),
            Spec::ContextMenu(entries) => wit::Element::ContextMenu(entries.iter().map(MenuEntry::wit).collect()),
            Spec::Markdown(text) => wit::Element::Markdown(wit::Markdown { text: text.clone() }),
            Spec::Image { source, fit } => wit::Element::Image(wit::Image { source: source.clone(), fit: *fit }),
        })
    }
}

impl SortDirection {
    pub(crate) fn wit(self) -> wit::SortDirection {
        match self {
            SortDirection::Ascending => wit::SortDirection::Ascending,
            SortDirection::Descending => wit::SortDirection::Descending,
        }
    }
}

impl From<wit::SortDirection> for SortDirection {
    fn from(d: wit::SortDirection) -> SortDirection {
        match d {
            wit::SortDirection::Ascending => SortDirection::Ascending,
            wit::SortDirection::Descending => SortDirection::Descending,
        }
    }
}
