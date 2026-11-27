//! Elements: what [`Tile::render`](crate::Tile::render) returns, built
//! the way GPUI builds its elements.
//!
//! ```ignore
//! v_flex().gap_2().p_3()
//!     .child(text("Runs").text_lg().font_semibold())
//!     .child(h_flex().gap_2()
//!         .child(input("filter").placeholder("Filter by branch"))
//!         .child(button("Refresh").on_click(cx.listener(|this, _, cx| this.refresh(cx)))))
//! ```
//!
//! Every node has a key, unique among its siblings: Pitwall keeps scroll
//! positions, focus and typed text by key path across frames. Give nodes
//! that move (rows of a list) a key of their own with [`Interactive::id`];
//! the others get their place among their siblings.

use std::marker::PhantomData;

use crate::events::{ChangeEvent, Click, Hover, Pointer, ScrollPosition, Selection, Submit};
use crate::style::{StyleSet, Styled};

/// A function of the tile that an element calls when the user acts, made
/// with [`Cx::listener`](crate::Cx::listener) while rendering. `E` is the
/// event its closure takes.
pub struct Handler<E: ?Sized> {
    pub(crate) slot: u32,
    pub(crate) generation: u64,
    _event: PhantomData<fn(&E)>,
}

impl<E: ?Sized> Handler<E> {
    pub(crate) fn new(slot: u32, generation: u64) -> Handler<E> {
        Handler { slot, generation, _event: PhantomData }
    }
}

impl<E: ?Sized> Clone for Handler<E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: ?Sized> Copy for Handler<E> {}

impl<E: ?Sized> std::fmt::Debug for Handler<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Handler({})", self.slot)
    }
}

/// The events a node can listen for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Listen {
    Click,
    Change,
    Select,
    Submit,
    Scroll,
    Hover,
    Pointer,
}

/// One of Pitwall's icons, drawn in the node's text colour.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Icon {
    /// Refresh, reload.
    Refresh,
    /// A check mark.
    Check,
    /// A cross: close, remove.
    X,
    /// A plus: add, new.
    Plus,
    /// A magnifier: search, filter.
    Search,
    /// A gear: settings.
    Gear,
    /// A folder.
    Folder,
    /// A file.
    File,
    /// A git branch.
    Branch,
    /// A terminal.
    Terminal,
    /// A bin: delete.
    Trash,
    /// Two pages: copy.
    Copy,
    /// An arrow out of a box: opens elsewhere.
    External,
    /// Another of Pitwall's icons, by name. An unknown name draws a
    /// placeholder.
    Named(String),
}

impl Icon {
    /// Its name in Pitwall's icon set.
    pub fn name(&self) -> &str {
        match self {
            Icon::Refresh => "refresh",
            Icon::Check => "check",
            Icon::X => "x",
            Icon::Plus => "plus",
            Icon::Search => "search",
            Icon::Gear => "gear",
            Icon::Folder => "folder",
            Icon::File => "file",
            Icon::Branch => "branch",
            Icon::Terminal => "terminal",
            Icon::Trash => "trash",
            Icon::Copy => "copy",
            Icon::External => "external",
            Icon::Named(n) => n,
        }
    }
}

/// A button's look, as Pitwall's own buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    Primary,
    Ghost,
    Danger,
    Small,
    Link,
}

/// Which ways a scroll container scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Axis {
    Vertical,
    Horizontal,
    Both,
}

#[derive(Debug, Clone)]
pub(crate) enum Kind {
    Box,
    Scroll(Axis),
    Text(String),
    Button { label: String, kind: ButtonKind, icon: Option<Icon>, disabled: bool },
    Icon(Icon),
    Input(InputSpec),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InputSpec {
    pub value: Option<String>,
    pub selection: Option<std::ops::Range<usize>>,
    pub placeholder: Option<String>,
    pub multiline: bool,
    pub rows: Option<u16>,
    pub disabled: bool,
}

/// An element, ready to be a child or a view's root.
#[derive(Debug, Clone)]
pub struct AnyElement {
    pub(crate) key: Option<String>,
    pub(crate) kind: Kind,
    pub(crate) style: StyleSet,
    pub(crate) listeners: Vec<(Listen, u32, u64)>,
    pub(crate) focusable: bool,
    pub(crate) tooltip: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) children: Vec<AnyElement>,
}

impl AnyElement {
    fn new(kind: Kind) -> AnyElement {
        AnyElement { key: None, kind, style: StyleSet::default(), listeners: Vec::new(), focusable: false, tooltip: None, label: None, children: Vec::new() }
    }

    fn listen<E: ?Sized>(&mut self, on: Listen, h: Handler<E>) {
        self.listeners.retain(|(l, _, _)| *l != on);
        self.listeners.push((on, h.slot, h.generation));
    }
}

/// Anything that can be an element: the builders, `AnyElement`, and text
/// (`&str`, `String`), which becomes a [`text`] node.
pub trait IntoElement {
    /// This as an element.
    fn into_element(self) -> AnyElement;

    /// This as an element, so different builders fit one type (for the
    /// arms of a `match`).
    fn into_any(self) -> AnyElement
    where
        Self: Sized,
    {
        self.into_element()
    }
}

impl IntoElement for AnyElement {
    fn into_element(self) -> AnyElement {
        self
    }
}

impl IntoElement for &str {
    fn into_element(self) -> AnyElement {
        text(self).into_element()
    }
}

impl IntoElement for String {
    fn into_element(self) -> AnyElement {
        text(self).into_element()
    }
}

impl<T: IntoElement> IntoElement for Option<T> {
    /// `None` is an empty box.
    fn into_element(self) -> AnyElement {
        match self {
            Some(e) => e.into_element(),
            None => AnyElement::new(Kind::Box),
        }
    }
}

/// What every element has: a key, an accessible name, a tooltip, focus,
/// and listeners for clicks and the pointer.
pub trait Interactive: Sized {
    #[doc(hidden)]
    fn node(&mut self) -> &mut AnyElement;

    /// Its key, unique among its siblings (at most 128 bytes; `/` is
    /// replaced). Pitwall keeps its scroll position, focus and typed text
    /// while its key path stays the same, so give list rows their item's
    /// id.
    fn id(mut self, key: impl Into<String>) -> Self {
        self.node().key = Some(key.into());
        self
    }

    /// Text shown when the pointer rests on it.
    fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.node().tooltip = Some(text.into());
        self
    }

    /// Its accessible name, read by screen readers. Required for a button
    /// with only an icon ([`icon_button`] sets it).
    fn label(mut self, name: impl Into<String>) -> Self {
        self.node().label = Some(name.into());
        self
    }

    /// It can take keyboard focus (Tab moves among focusable nodes in tree
    /// order). Buttons and text fields are focusable anyway.
    fn focusable(mut self) -> Self {
        self.node().focusable = true;
        self
    }

    /// Call `handler` when it is clicked (or Space or Enter while it has
    /// focus).
    fn on_click(mut self, handler: Handler<Click>) -> Self {
        self.node().listen(Listen::Click, handler);
        self
    }

    /// Call `handler` when the pointer enters or leaves it.
    fn on_hover(mut self, handler: Handler<Hover>) -> Self {
        self.node().listen(Listen::Hover, handler);
        self
    }

    /// Call `handler` for pointer presses, releases and moves over it, with
    /// their position (for drawing and dragging inside a node).
    fn on_pointer(mut self, handler: Handler<Pointer>) -> Self {
        self.node().listen(Listen::Pointer, handler);
        self
    }

    /// Apply `then` when `condition` holds.
    fn when(self, condition: bool, then: impl FnOnce(Self) -> Self) -> Self {
        if condition {
            then(self)
        } else {
            self
        }
    }

    /// Apply `then` with the value when there is one.
    fn when_some<V>(self, value: Option<V>, then: impl FnOnce(Self, V) -> Self) -> Self {
        match value {
            Some(v) => then(self, v),
            None => self,
        }
    }
}

/// Elements with children.
pub trait ParentElement: Sized {
    #[doc(hidden)]
    fn children_mut(&mut self) -> &mut Vec<AnyElement>;

    /// Add a child.
    fn child(mut self, child: impl IntoElement) -> Self {
        self.children_mut().push(child.into_element());
        self
    }

    /// Add children.
    fn children<I: IntoIterator>(mut self, children: I) -> Self
    where
        I::Item: IntoElement,
    {
        self.children_mut().extend(children.into_iter().map(IntoElement::into_element));
        self
    }
}

macro_rules! builder {
    ($t:ident) => {
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

/// A box: a flex container (a column unless told otherwise), the usual
/// root. Made by [`div`], [`h_flex`] and [`v_flex`].
#[derive(Debug, Clone)]
pub struct Div(AnyElement);
builder!(Div);

impl ParentElement for Div {
    fn children_mut(&mut self) -> &mut Vec<AnyElement> {
        &mut self.0.children
    }
}

impl Div {
    /// Its children scroll up and down inside it. Pitwall keeps the
    /// position by key path and draws the scrollbar.
    pub fn overflow_y_scroll(mut self) -> Div {
        self.0.kind = Kind::Scroll(Axis::Vertical);
        self
    }

    /// Its children scroll left and right inside it.
    pub fn overflow_x_scroll(mut self) -> Div {
        self.0.kind = Kind::Scroll(Axis::Horizontal);
        self
    }

    /// Its children scroll both ways inside it.
    pub fn overflow_scroll(mut self) -> Div {
        self.0.kind = Kind::Scroll(Axis::Both);
        self
    }

    /// Call `handler` when it is scrolled (a scroll container only: call
    /// one of the `overflow_*_scroll` methods), for loading more at the
    /// end.
    pub fn on_scroll(mut self, handler: Handler<ScrollPosition>) -> Div {
        self.0.listen(Listen::Scroll, handler);
        self
    }
}

/// A box, laid out as a column. Style it into a row with `.flex()`.
pub fn div() -> Div {
    Div(AnyElement::new(Kind::Box))
}

/// A row with its children centred across it.
pub fn h_flex() -> Div {
    div().flex_row().items_center()
}

/// A column.
pub fn v_flex() -> Div {
    div().flex_col()
}

/// Text. Children (more [`text`]) are inline runs of one paragraph, each
/// with its own styles: `text("Built in ").child(text("3 s").font_bold())`.
#[derive(Debug, Clone)]
pub struct Text(AnyElement);
builder!(Text);

impl Text {
    /// Add an inline run.
    pub fn child(mut self, run: Text) -> Text {
        self.0.children.push(run.0);
        self
    }
}

/// Text.
pub fn text(s: impl Into<String>) -> Text {
    Text(AnyElement::new(Kind::Text(s.into())))
}

/// A button, as Pitwall's own. Made by [`button`] and [`icon_button`].
#[derive(Debug, Clone)]
pub struct Button(AnyElement);
builder!(Button);

impl Button {
    fn set_kind(mut self, k: ButtonKind) -> Button {
        if let Kind::Button { kind, .. } = &mut self.0.kind {
            *kind = k;
        }
        self
    }

    /// The main action of a tile or a form; one per area.
    pub fn primary(self) -> Button {
        self.set_kind(ButtonKind::Primary)
    }

    /// An ordinary action: a hairline button (the default).
    pub fn ghost(self) -> Button {
        self.set_kind(ButtonKind::Ghost)
    }

    /// A destructive action.
    pub fn danger(self) -> Button {
        self.set_kind(ButtonKind::Danger)
    }

    /// A small button for rows and toolbars.
    pub fn small(self) -> Button {
        self.set_kind(ButtonKind::Small)
    }

    /// A button that looks like a link.
    pub fn link(self) -> Button {
        self.set_kind(ButtonKind::Link)
    }

    /// An icon before the label.
    pub fn icon(mut self, i: Icon) -> Button {
        if let Kind::Button { icon, .. } = &mut self.0.kind {
            *icon = Some(i);
        }
        self
    }

    /// Drawn dimmed, and clicks aren't sent.
    pub fn disabled(mut self, yes: bool) -> Button {
        if let Kind::Button { disabled, .. } = &mut self.0.kind {
            *disabled = yes;
        }
        self
    }
}

/// A button with a label. Its click goes to [`Interactive::on_click`].
pub fn button(label: impl Into<String>) -> Button {
    Button(AnyElement::new(Kind::Button { label: label.into(), kind: ButtonKind::Ghost, icon: None, disabled: false }))
}

/// A small button with only an icon; `label` is its accessible name and
/// tooltip.
pub fn icon_button(icon: Icon, label: impl Into<String>) -> Button {
    let label = label.into();
    button("").small().icon(icon).label(label.clone()).tooltip(label)
}

/// An icon. Made by [`icon`].
#[derive(Debug, Clone)]
pub struct IconElement(AnyElement);
builder!(IconElement);

/// An icon, in the node's text colour and text size (or its width and
/// height, when set).
pub fn icon(i: Icon) -> IconElement {
    IconElement(AnyElement::new(Kind::Icon(i)))
}

/// A text field. Made by [`input`].
///
/// Pitwall owns what the user types (typing, selection, undo and IME
/// never wait for the tile); the tile hears about it with `on_change`.
/// [`Input::value`] sets the text: the SDK sends it only when it differs
/// from what the field holds, so passing the tile's own copy on every
/// render is fine and never undoes what the user typed meanwhile.
#[derive(Debug, Clone)]
pub struct Input(AnyElement);
builder!(Input);

impl Input {
    fn spec(&mut self) -> &mut InputSpec {
        match &mut self.0.kind {
            Kind::Input(s) => s,
            _ => unreachable!("an input"),
        }
    }

    /// The text it should hold. Sent only when it differs from what the
    /// field holds now.
    pub fn value(mut self, v: impl Into<String>) -> Input {
        self.spec().value = Some(v.into());
        self
    }

    /// Where the cursor or selection goes when [`Input::value`] changes
    /// the text (UTF-8 byte offsets; default: the end).
    pub fn select(mut self, range: std::ops::Range<usize>) -> Input {
        self.spec().selection = Some(range);
        self
    }

    /// Shown in faint text while it is empty.
    pub fn placeholder(mut self, p: impl Into<String>) -> Input {
        self.spec().placeholder = Some(p.into());
        self
    }

    /// Several lines: Enter adds a line, ⌘Enter (Ctrl+Enter off macOS)
    /// submits.
    pub fn multiline(mut self) -> Input {
        self.spec().multiline = true;
        self
    }

    /// For a multi-line field without a height, its height in lines
    /// (default 3); it scrolls past that.
    pub fn rows(mut self, rows: u16) -> Input {
        self.spec().rows = Some(rows);
        self
    }

    /// Drawn dimmed and can't be edited.
    pub fn disabled(mut self, yes: bool) -> Input {
        self.spec().disabled = yes;
        self
    }

    /// Call `handler` when the text changes, with the new text (`&str`) or
    /// the whole [`InputChange`](crate::events::InputChange).
    pub fn on_change<E: ChangeEvent + ?Sized>(mut self, handler: Handler<E>) -> Input {
        self.0.listen(Listen::Change, handler);
        self
    }

    /// Call `handler` on Enter (⌘Enter when multi-line).
    pub fn on_submit(mut self, handler: Handler<Submit>) -> Input {
        self.0.listen(Listen::Submit, handler);
        self
    }

    /// Call `handler` when the cursor or selection moves without the text
    /// changing.
    pub fn on_select(mut self, handler: Handler<Selection>) -> Input {
        self.0.listen(Listen::Select, handler);
        self
    }
}

/// A text field with key `key` (its key is required: Pitwall keeps its
/// text by key path).
pub fn input(key: impl Into<String>) -> Input {
    Input(AnyElement::new(Kind::Input(InputSpec::default()))).id(key)
}
