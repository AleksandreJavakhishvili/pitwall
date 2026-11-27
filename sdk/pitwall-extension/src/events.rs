//! What the user did, as the listeners and [`Tile::on_event`] get it.
//!
//! A listener's closure names the event it takes: `on_click` takes
//! [`Click`], `on_change` takes the new text (`&str`) or the whole
//! [`InputChange`], `on_submit` takes [`Submit`], and so on.
//!
//! [`Tile::on_event`]: crate::Tile::on_event

use crate::bindings::pitwall::tile::types as wit;

/// A point in a node, in logical pixels from its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// From the left edge.
    pub x: f32,
    /// From the top edge.
    pub y: f32,
}

impl From<wit::Point> for Point {
    fn from(p: wit::Point) -> Point {
        Point { x: p.x, y: p.y }
    }
}

/// Which mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// The main button.
    Left,
    /// The secondary button.
    Right,
    /// The middle button or wheel.
    Middle,
}

impl From<wit::MouseButton> for MouseButton {
    fn from(b: wit::MouseButton) -> MouseButton {
        match b {
            wit::MouseButton::Left => MouseButton::Left,
            wit::MouseButton::Right => MouseButton::Right,
            wit::MouseButton::Middle => MouseButton::Middle,
        }
    }
}

/// Keys held during a click or a pointer event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// Shift.
    pub shift: bool,
    /// Control.
    pub ctrl: bool,
    /// Option on macOS, Alt elsewhere.
    pub alt: bool,
    /// ⌘ on macOS, the Windows key or Super elsewhere.
    pub platform: bool,
}

impl From<wit::Modifiers> for Modifiers {
    fn from(m: wit::Modifiers) -> Modifiers {
        let has = |f: wit::Modifiers| m.bits() & f.bits() != 0;
        Modifiers { shift: has(wit::Modifiers::SHIFT), ctrl: has(wit::Modifiers::CTRL), alt: has(wit::Modifiers::ALT), platform: has(wit::Modifiers::PLATFORM) }
    }
}

/// A click (or Space or Enter while the node has focus).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Click {
    /// Which button.
    pub button: MouseButton,
    /// Keys held.
    pub modifiers: Modifiers,
    /// 1 for a single click, 2 for a double click, and so on.
    pub count: u32,
    /// Where in the node (for a click from the keyboard: its centre).
    pub at: Point,
}

/// A text field's new value. Changes are coalesced while the user types
/// (at most one every 50 ms per field, plus one when it loses focus).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputChange {
    /// What the field holds now.
    pub value: String,
    /// The cursor or selection after the edit, in UTF-8 byte offsets.
    pub selection: std::ops::Range<usize>,
}

/// A text field's cursor or selection moved without its value changing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The cursor or selection, in UTF-8 byte offsets of the field's value.
    pub range: std::ops::Range<usize>,
}

/// Enter in a one-line text field (⌘Enter, Ctrl+Enter off macOS, in a
/// multi-line one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Submit;

/// A node got keyboard focus (see [`Interactive::on_focus`](crate::Interactive::on_focus)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Focused;

/// A node lost keyboard focus (see [`Interactive::on_blur`](crate::Interactive::on_blur)):
/// a text field's blur, where a tile saves what was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blur;

/// A key binding fired (see [`Cx::bind_key`](crate::Cx::bind_key)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPress {
    /// The binding's keys, as written when it was bound.
    pub keys: String,
}

/// Where a scroll container is scrolled to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollPosition {
    /// Pixels scrolled from the left.
    pub x: f32,
    /// Pixels scrolled from the top.
    pub y: f32,
    /// The content's width, in pixels.
    pub content_width: f32,
    /// The content's height, in pixels.
    pub content_height: f32,
}

/// The pointer entered or left a node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hover {
    /// True when it entered, false when it left.
    pub inside: bool,
    /// Where, from the node's top-left corner.
    pub at: Point,
}

/// What the pointer did.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum PointerKind {
    /// A button went down.
    Down(MouseButton),
    /// A button came up.
    Up(MouseButton),
    /// The pointer moved (coalesced: the newest position).
    Move,
    /// The wheel or a trackpad scrolled by this many pixels (positive `dy`:
    /// scrolling down, positive `dx`: scrolling right). Only for
    /// [`on_wheel`](crate::Interactive::on_wheel).
    Wheel {
        /// Rightwards.
        dx: f32,
        /// Downwards.
        dy: f32,
    },
    /// A trackpad pinch: zoom by this fraction around `at` (0.1: 10 %
    /// larger; -0.1: smaller). Only for
    /// [`on_wheel`](crate::Interactive::on_wheel).
    Pinch(f32),
}

/// A drag dropped on a node with [`on_drop`](crate::Interactive::on_drop).
#[derive(Debug, Clone, PartialEq)]
pub struct Dropped {
    /// The dragged node's kind (one the target accepts).
    pub kind: String,
    /// The dragged node's data, as [`draggable`](crate::Interactive::draggable) gave it.
    pub data: String,
    /// Where it was dropped, from the target's top-left corner.
    pub at: Point,
}

/// Which version of a diff. A code view's lines are [`Side::New`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The original (left, or the removed lines).
    Old,
    /// The changed version (right, or the added lines).
    New,
}

impl From<wit::DiffSide> for Side {
    fn from(s: wit::DiffSide) -> Side {
        match s {
            wit::DiffSide::Old => Side::Old,
            wit::DiffSide::New => Side::New,
        }
    }
}

/// A line number of a [`code_view`](crate::code_view) or
/// [`diff_view`](crate::diff_view) was clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineClick {
    /// Which version's line.
    pub side: Side,
    /// The line, 0-based.
    pub line: usize,
}

/// A place in a code or diff view's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct TextPos {
    /// The line, 0-based.
    pub line: usize,
    /// The column, in UTF-8 bytes from the line's start.
    pub column: usize,
}

/// The selection in a code or diff view changed (at most one every
/// 100 ms while the user drags).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSelection {
    /// Which version it is in.
    pub side: Side,
    /// Where it starts (the earlier end).
    pub start: TextPos,
    /// Where it ends; equal to `start` for a bare cursor.
    pub end: TextPos,
    /// The selected text, cut at 64 KiB.
    pub text: String,
}

/// A press, release or move over a node with an `on_pointer` listener.
/// While a button is held after a press on the node, its moves and the
/// release go to the same node even outside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pointer {
    /// What happened.
    pub kind: PointerKind,
    /// Where, from the node's top-left corner.
    pub at: Point,
    /// Keys held.
    pub modifiers: Modifiers,
}

/// Light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    /// The light theme.
    Light,
    /// The dark theme.
    Dark,
}

/// How much space Pitwall puts around things (Settings → Appearance).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density {
    /// The most space.
    Comfortable,
    /// The default.
    Compact,
    /// The least space.
    Dense,
}

/// The user's appearance settings that matter to a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Light or dark.
    pub mode: ThemeMode,
    /// How much space to leave.
    pub density: Density,
    /// The user asked for less motion: don't animate.
    pub reduce_motion: bool,
}

impl From<wit::ThemeInfo> for Theme {
    fn from(t: wit::ThemeInfo) -> Theme {
        Theme {
            mode: match t.mode {
                wit::ThemeMode::Light => ThemeMode::Light,
                wit::ThemeMode::Dark => ThemeMode::Dark,
            },
            density: match t.density {
                wit::Density::Comfortable => Density::Comfortable,
                wit::Density::Compact => Density::Compact,
                wit::Density::Dense => Density::Dense,
            },
            reduce_motion: t.reduce_motion,
        }
    }
}

/// Events that aren't for a listener, a timer or a view, as
/// [`Tile::on_event`](crate::Tile::on_event) gets them.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// The tile was shown (true) or hidden (false). While hidden, timers
    /// pause and nothing is rendered.
    Visibility(bool),
    /// The tile got (true) or lost (false) keyboard focus.
    Focus(bool),
    /// The focus moved to another node of a view, or off its nodes
    /// (`None`). A node's own [`on_focus`](crate::Interactive::on_focus) and
    /// [`on_blur`](crate::Interactive::on_blur) listeners are simpler.
    FocusChanged {
        /// The view.
        view: String,
        /// The focused node's key path (keys from the root down, joined by
        /// `/`, without the root's), or `None`.
        key_path: Option<String>,
    },
    /// A view's size changed (coalesced: the newest size).
    Resize {
        /// The view.
        view: String,
        /// Its width, in logical pixels.
        width: f32,
        /// Its height, in logical pixels.
        height: f32,
    },
    /// The user changed their appearance settings.
    Theme(Theme),
    /// The user changed the tile's settings; read them again with
    /// [`settings`](crate::settings).
    SettingsChanged,
    /// The user granted or took back permissions; the list is what is
    /// granted now.
    GrantsChanged(Vec<String>),
    /// Pitwall's agents changed (one came or went; a name, a state or a
    /// Next up changed): read them again with
    /// [`agents::list`](crate::agents::list). Only with `agents.read`;
    /// changes close together arrive as one.
    AgentsChanged,
    /// The user answered one of the tile's questions
    /// ([`asks::set`](crate::asks::set)) in the floating control; reply
    /// with [`asks::reply`](crate::asks::reply).
    AskAnswered(crate::asks::Answer),
    /// The extension's toolbar button was clicked (its action is `tile`
    /// or `popover`), with the id of the project whose space the window
    /// shows, if any.
    ToolbarClicked {
        /// The project, for a per-project item.
        project: Option<String>,
    },
    /// The extension's own server changed state.
    ServiceChanged(crate::service::Info),
    /// The user pressed Undo on what an agent's call (its
    /// [`Call::origin`](crate::tools::Call::origin)) put in the tile:
    /// restore what was there before it. A tile that keeps what agents
    /// show in [`Presented`](crate::tools::Presented) calls its
    /// [`undo`](crate::tools::Presented::undo).
    UndoPresent(u64),
    /// A `read` or `act` call the tile answered later
    /// ([`Reply::later`](crate::tools::Reply::later)) took too long and was
    /// given up: stop working on it.
    ToolCancelled(u64),
}

/// A listener's event: what the SDK hands a closure made with
/// [`Cx::listener`](crate::Cx::listener) when the user acts. Implemented
/// for the event types of this module and `str` (a text field's value).
pub trait ListenerEvent {
    #[doc(hidden)]
    fn pick(e: &Fired) -> Option<&Self>;
}

/// An event that reached a listener, before it is handed to the closure.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum Fired {
    Click(Click),
    Change(InputChange),
    Select(Selection),
    Submit,
    Key(KeyPress),
    Scroll(ScrollPosition),
    Hover(Hover),
    Pointer(Pointer),
    Focus,
    Blur,
    Drop(Dropped),
    LineClick(LineClick),
    CodeSelect(CodeSelection),
    Pick(Picked),
    Toggle(Toggled),
    Link(LinkClick),
    Sort(SortChange),
    ColumnResized(ColumnWidth),
    Value(ValueFired),
    Dismiss,
}

/// A control's new value as it reached a listener: the WIT value and the
/// shapes listeners take it in (`bool`, `f64`, `Option<f64>`, `str`).
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ValueFired {
    pub on: Option<bool>,
    pub number: Option<f64>,
    /// A number field's: its number, or `None` when emptied.
    pub maybe: Option<Option<f64>>,
    pub choice: Option<String>,
}

impl ValueFired {
    pub(crate) fn from_wit(v: wit::ControlValue) -> ValueFired {
        let mut f = ValueFired { on: None, number: None, maybe: None, choice: None };
        match v {
            wit::ControlValue::On(b) => f.on = Some(b),
            wit::ControlValue::Number(n) => {
                f.number = Some(n);
                f.maybe = Some(Some(n));
            }
            wit::ControlValue::Choice(k) => f.choice = Some(k),
            wit::ControlValue::Empty => f.maybe = Some(None),
        }
        f
    }
}

/// The user closed or removed something: a banner's close button, a
/// chip's ×, Esc or a click outside a popover, a confirm sheet's Cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Dismissed;

macro_rules! listener_event {
    ($t:ty, $p:pat => $v:expr) => {
        impl ListenerEvent for $t {
            fn pick(e: &Fired) -> Option<&Self> {
                match e {
                    $p => Some($v),
                    _ => None,
                }
            }
        }
    };
}

listener_event!(Click, Fired::Click(c) => c);
listener_event!(InputChange, Fired::Change(c) => c);

impl ListenerEvent for str {
    /// A text field's new value, or the key of the row, tab, option or
    /// menu item picked.
    fn pick(e: &Fired) -> Option<&Self> {
        match e {
            Fired::Change(c) => Some(c.value.as_str()),
            Fired::Pick(p) => Some(p.key.as_str()),
            // A radio group's, segmented control's, breadcrumbs' or status
            // timeline's chosen key.
            Fired::Value(v) => v.choice.as_deref(),
            _ => None,
        }
    }
}
listener_event!(Selection, Fired::Select(s) => s);
listener_event!(Submit, Fired::Submit => &Submit);
listener_event!(KeyPress, Fired::Key(k) => k);
listener_event!(ScrollPosition, Fired::Scroll(s) => s);
listener_event!(Hover, Fired::Hover(h) => h);
listener_event!(Pointer, Fired::Pointer(p) => p);
listener_event!(Focused, Fired::Focus => &Focused);
listener_event!(Blur, Fired::Blur => &Blur);
listener_event!(Dropped, Fired::Drop(d) => d);
listener_event!(LineClick, Fired::LineClick(l) => l);
listener_event!(CodeSelection, Fired::CodeSelect(s) => s);
listener_event!(Dismissed, Fired::Dismiss => &Dismissed);

/// A checkbox's or switch's new state; a disclosure is open.
impl ListenerEvent for bool {
    fn pick(e: &Fired) -> Option<&Self> {
        match e {
            Fired::Value(v) => v.on.as_ref(),
            _ => None,
        }
    }
}

/// A slider's or number field's new value; a split's first share.
impl ListenerEvent for f64 {
    fn pick(e: &Fired) -> Option<&Self> {
        match e {
            Fired::Value(v) => v.number.as_ref(),
            _ => None,
        }
    }
}

/// A number field's new value, `None` when emptied.
impl ListenerEvent for Option<f64> {
    fn pick(e: &Fired) -> Option<&Self> {
        match e {
            Fired::Value(v) => v.maybe.as_ref(),
            _ => None,
        }
    }
}

/// Events a text field's `on_change` takes: its new value as `&str`, or
/// the whole [`InputChange`].
pub trait ChangeEvent: ListenerEvent {}
impl ChangeEvent for str {}
impl ChangeEvent for InputChange {}

// ---- Components -------------------------------------------------------------

/// A row, tab, option or menu item was picked: a row of a list, table or
/// tree clicked or moved to with the keys, a tab or a select's option
/// chosen, a menu item chosen. `on_select` takes this, or the key alone
/// (`&str`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    /// The item's key: the row's (its element's `id`, or its index), the
    /// tab's, the option's or the menu item's.
    pub key: String,
    /// The row's index, or the item's place among its siblings (menu
    /// separators not counted).
    pub index: usize,
    /// Keys held: ⌘/Ctrl or Shift to add to a selection.
    pub modifiers: Modifiers,
    /// The row was activated: a double click, or Enter. Open it.
    pub activate: bool,
}

/// A tree's branch was opened or closed, or a Markdown task's box ticked
/// or cleared (`on_toggle`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toggled {
    /// A tree: the row's key. Markdown: empty.
    pub key: String,
    /// A tree: the row's index. Markdown: the task's line in the text,
    /// from 0.
    pub index: usize,
    /// Open (or ticked) now.
    pub on: bool,
}

/// A link in Markdown was clicked (`on_link`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkClick {
    /// The link's target, as written in the text.
    pub url: String,
    /// Keys held.
    pub modifiers: Modifiers,
}

/// Which way a table is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortDirection {
    /// Smallest first.
    Ascending,
    /// Largest first.
    Descending,
}

impl SortDirection {
    /// The other way.
    pub fn flip(self) -> SortDirection {
        match self {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        }
    }
}

/// A table's header was clicked to sort (`on_sort`): sort by `column`,
/// `direction` (the other way when it was already sorted by it), then
/// render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortChange {
    /// The column's key.
    pub column: String,
    /// The way asked for.
    pub direction: SortDirection,
}

/// The user resized a table's column (`on_column_resize`), when the drag
/// ended. Pitwall keeps the width while the column's own stays the same;
/// keep it (in storage) to start with it next time.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnWidth {
    /// The column's key.
    pub column: String,
    /// Its width now, in logical pixels.
    pub width: f32,
}

listener_event!(Picked, Fired::Pick(p) => p);
listener_event!(Toggled, Fired::Toggle(t) => t);
listener_event!(LinkClick, Fired::Link(l) => l);
listener_event!(SortChange, Fired::Sort(s) => s);
listener_event!(ColumnWidth, Fired::ColumnResized(c) => c);

/// Events `on_select` takes: the whole [`Picked`], or its key (`&str`).
pub trait PickEvent: ListenerEvent {}
impl PickEvent for str {}
impl PickEvent for Picked {}
