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

use crate::events::{Blur, ChangeEvent, Click, Focused, Hover, Pointer, ScrollPosition, Selection, Submit};
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

/// The events a node can listen for. `Focus` and `Blur` are the SDK's
/// own: it calls them when `focus-changed` moves to or from the node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Listen {
    Click,
    Change,
    Select,
    Submit,
    Scroll,
    Hover,
    Pointer,
    Focus,
    Blur,
}

/// One of Pitwall's icons, drawn in the node's text colour: the whole set
/// Pitwall draws, by the names its own screens use (the app's tests keep
/// this list equal to its icons, and `pitwall extension check` checks a
/// manifest's icons against the same list).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Icon {
    /// A sidebar: show or hide a side list. (`sidebar`)
    Sidebar,
    /// A magnifier: search, filter. (`search`)
    Search,
    /// A gear: settings. (`gear`)
    Gear,
    /// A plus: add, new. (`plus`)
    Plus,
    /// A cross: close, remove. (`x`)
    X,
    /// An arrow right: send. (`send`)
    Send,
    /// A square: stop. (`stop`)
    Stop,
    /// A turning arrow, left: restart. (`restart`)
    Restart,
    /// A turning arrow, right: refresh, reload. (`refresh`)
    Refresh,
    /// A bin: delete. (`trash`)
    Trash,
    /// A version-control branch. (`branch`)
    Branch,
    /// A folder. (`folder`)
    Folder,
    /// A chevron pointing right: more, open a group. (`chevron`)
    Chevron,
    /// Four squares: a grid of things. (`grid`)
    Grid,
    /// Six panes: Pitwall's wall of agents. (`wall`)
    Wall,
    /// Corners outward: maximise. (`maximize`)
    Maximize,
    /// Corners inward: restore. (`restore`)
    Restore,
    /// Two windows: a window of its own. (`window`)
    Window,
    /// Three dots: more actions. (`more`)
    More,
    /// A page with lines: a review. (`review`)
    Review,
    /// A terminal. (`terminal`)
    Terminal,
    /// A file. (`file`)
    File,
    /// Two arrows apart: expand. (`expand`)
    Expand,
    /// Two pages: copy. (`copy`)
    Copy,
    /// A chevron pointing down: a menu, an open group. (`chevron-down`)
    ChevronDown,
    /// A check mark. (`check`)
    Check,
    /// A headset. (`headset`)
    Headset,
    /// A person: an account. (`user`)
    User,
    /// A laptop: this computer. (`laptop`)
    Laptop,
    /// A server: another machine. (`server`)
    Server,
    /// An eye: view only, show. (`eye`)
    Eye,
    /// An arrow out of a box: opens elsewhere. (`external`)
    External,
    /// A picture. (`image`)
    Image,
    /// A browser window: a web page. (`browser`)
    Browser,
    /// A chevron pointing left: back. (`back`)
    Back,
    /// A phone: a device. (`phone`)
    Phone,
    /// Angle brackets: code, inspect. (`inspect`)
    Inspect,
    /// A page with a pane on the right. (`page-right`)
    PageRight,
    /// A page with a pane on the left. (`page-left`)
    PageLeft,
    /// A page with a pane below. (`page-below`)
    PageBelow,
    /// A pin: keep on top. (`pin`)
    Pin,
    /// Six dots: drag to move. (`grip`)
    Grip,
    /// A minus: remove, zoom out. (`minus`)
    Minus,
    /// A chevron pointing up: previous, close a group. (`chevron-up`)
    ChevronUp,
    /// A padlock: can't be changed. (`lock`)
    Lock,
    /// A warning triangle. (`alert`)
    Alert,
    /// A puzzle piece: an extension. (`extension`)
    Extension,
    /// An inbox tray. (`inbox`)
    Inbox,
    /// A bell: notifications. (`bell`)
    Bell,
    /// A speech bubble: a conversation. (`chat`)
    Chat,
    /// A bulleted list. (`list`)
    List,
    /// An icon by name, for a name newer than this SDK. An unknown name
    /// draws nothing, and the test harness notes it.
    Named(String),
}

impl Icon {
    /// Every icon of the set, in the order of the app's list.
    pub const ALL: &'static [Icon] = &[Icon::Sidebar, Icon::Search, Icon::Gear, Icon::Plus, Icon::X, Icon::Send, Icon::Stop, Icon::Restart, Icon::Refresh, Icon::Trash, Icon::Branch, Icon::Folder, Icon::Chevron, Icon::Grid, Icon::Wall, Icon::Maximize, Icon::Restore, Icon::Window, Icon::More, Icon::Review, Icon::Terminal, Icon::File, Icon::Expand, Icon::Copy, Icon::ChevronDown, Icon::Check, Icon::Headset, Icon::User, Icon::Laptop, Icon::Server, Icon::Eye, Icon::External, Icon::Image, Icon::Browser, Icon::Back, Icon::Phone, Icon::Inspect, Icon::PageRight, Icon::PageLeft, Icon::PageBelow, Icon::Pin, Icon::Grip, Icon::Minus, Icon::ChevronUp, Icon::Lock, Icon::Alert, Icon::Extension, Icon::Inbox, Icon::Bell, Icon::Chat, Icon::List];

    /// Its name in Pitwall's icon set.
    pub fn name(&self) -> &str {
        match self {
            Icon::Sidebar => "sidebar",
            Icon::Search => "search",
            Icon::Gear => "gear",
            Icon::Plus => "plus",
            Icon::X => "x",
            Icon::Send => "send",
            Icon::Stop => "stop",
            Icon::Restart => "restart",
            Icon::Refresh => "refresh",
            Icon::Trash => "trash",
            Icon::Branch => "branch",
            Icon::Folder => "folder",
            Icon::Chevron => "chevron",
            Icon::Grid => "grid",
            Icon::Wall => "wall",
            Icon::Maximize => "maximize",
            Icon::Restore => "restore",
            Icon::Window => "window",
            Icon::More => "more",
            Icon::Review => "review",
            Icon::Terminal => "terminal",
            Icon::File => "file",
            Icon::Expand => "expand",
            Icon::Copy => "copy",
            Icon::ChevronDown => "chevron-down",
            Icon::Check => "check",
            Icon::Headset => "headset",
            Icon::User => "user",
            Icon::Laptop => "laptop",
            Icon::Server => "server",
            Icon::Eye => "eye",
            Icon::External => "external",
            Icon::Image => "image",
            Icon::Browser => "browser",
            Icon::Back => "back",
            Icon::Phone => "phone",
            Icon::Inspect => "inspect",
            Icon::PageRight => "page-right",
            Icon::PageLeft => "page-left",
            Icon::PageBelow => "page-below",
            Icon::Pin => "pin",
            Icon::Grip => "grip",
            Icon::Minus => "minus",
            Icon::ChevronUp => "chevron-up",
            Icon::Lock => "lock",
            Icon::Alert => "alert",
            Icon::Extension => "extension",
            Icon::Inbox => "inbox",
            Icon::Bell => "bell",
            Icon::Chat => "chat",
            Icon::List => "list",
            Icon::Named(n) => n,
        }
    }

    /// The icon called `name`, if the set has it.
    pub fn from_name(name: &str) -> Option<Icon> {
        Icon::ALL.iter().find(|i| i.name() == name).cloned()
    }
}

/// A button's look, as Pitwall's own buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    Primary,
    Ghost,
    Danger,
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
    Button { label: String, kind: ButtonKind, small: bool, icon: Option<Icon>, disabled: bool },
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

    /// Call `handler` when it gets keyboard focus (a text field, a
    /// [`focusable`](Interactive::focusable) node).
    fn on_focus(mut self, handler: Handler<Focused>) -> Self {
        self.node().listen(Listen::Focus, handler);
        self
    }

    /// Call `handler` when it loses keyboard focus: the user moved to
    /// another node or out of the tile, or the node went away. A text
    /// field's last edit has arrived before this (save it here).
    fn on_blur(mut self, handler: Handler<Blur>) -> Self {
        self.node().listen(Listen::Blur, handler);
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

    /// An ordinary action: a hairline button (the default; small, it is
    /// Pitwall's row and toolbar button).
    pub fn ghost(self) -> Button {
        self.set_kind(ButtonKind::Ghost)
    }

    /// A destructive action.
    pub fn danger(self) -> Button {
        self.set_kind(ButtonKind::Danger)
    }

    /// Smaller (26 px instead of 30), for rows, toolbars and a tile's
    /// bars. The size is apart from the look: `.small().danger()` and
    /// `.danger().small()` are both a small danger button.
    pub fn small(mut self) -> Button {
        if let Kind::Button { small, .. } = &mut self.0.kind {
            *small = true;
        }
        self
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
    Button(AnyElement::new(Kind::Button { label: label.into(), kind: ButtonKind::Ghost, small: false, icon: None, disabled: false }))
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
