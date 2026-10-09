//! A reusable GPUI terminal view for Pitwall.
//!
//! - [`Terminal`]: an `alacritty_terminal` emulator fed by a [`TermStream`]
//!   (a local PTY, a Pitwall holder, a test), shared by any number of views.
//! - [`TerminalView`]: draws it (batched, cell-aligned text runs; damage-based
//!   reshaping; box drawing as quads) and turns input into bytes (xterm keys,
//!   IME, paste, mouse reporting, focus reporting, selection, links, search).
//! - [`ViewMode::Tile`]: a light read-only render for Wall tiles.
//!
//! Written from the GPUI and alacritty_terminal APIs; no code from Zed's
//! GPL `terminal` / `terminal_view` crates (see README.md).

pub mod boxdraw;
mod element;
pub mod frames;
mod frozen;
#[cfg(feature = "holder")]
pub mod holder;
pub mod keys;
pub mod mouse;
pub mod paths;
#[cfg(feature = "pty")]
pub mod pty;
pub mod runs;
pub mod terminal;
pub mod theme;
mod view;

pub use element::{register_fonts, RenderStats, TermFont};
pub use terminal::{Feed, NullStream, TermEvent, TermSize, TermStream, Terminal, TerminalConfig};
pub use theme::TermTheme;
pub use view::{
    default_key_bindings, Clear, Copy, DecreaseFontSize, IncreaseFontSize, Paste, ResetFontSize, ScrollLineDown,
    ScrollLineUp, ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop, SearchNext, SearchPrevious, SelectAll,
    FileLinks, OpenFile, TerminalView, ToggleSearch, ViewMode, ViewSettings, KEY_CONTEXT,
};

pub use alacritty_terminal;
pub use gpui;
