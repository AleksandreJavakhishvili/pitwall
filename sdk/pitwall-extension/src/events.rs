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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    /// A button went down.
    Down(MouseButton),
    /// A button came up.
    Up(MouseButton),
    /// The pointer moved (coalesced: the newest position).
    Move,
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

/// Events a text field's `on_change` takes: its new value as `&str`, or
/// the whole [`InputChange`].
pub trait ChangeEvent: ListenerEvent {}
impl ChangeEvent for str {}
impl ChangeEvent for InputChange {}
