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
}

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
listener_event!(str, Fired::Change(c) => c.value.as_str());
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

/// Events a text field's `on_change` takes: its new value as `&str`, or
/// the whole [`InputChange`].
pub trait ChangeEvent: ListenerEvent {}
impl ChangeEvent for str {}
impl ChangeEvent for InputChange {}
