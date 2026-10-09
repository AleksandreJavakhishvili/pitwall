//! The command palette (⌘K, Ctrl+Shift+K elsewhere; inventory §17): a
//! port of `CommandPalette.tsx` and its `overlays.css` rules. It opens on
//! [`ScreenEvent::OpenPalette`](crate::main_screen::ScreenEvent) (⌘K, the
//! top bar's Search button, the status strip's "⌘K commands"), lists rows
//! matching every typed word, runs the chosen one with ↵ or a click, and
//! closes on Esc, ⌘K or a click outside. "name: text" turns into a single
//! "Queue for name" row.
//!
//! # Adding commands
//!
//! Every row comes from a provider in the [`registry`]; the built-in rows
//! ([`builtin`]) are registered the same way. Another module adds its own
//! once at start, without touching this one:
//!
//! ```ignore
//! use pitwall_app::palette::{self, rank, Command};
//!
//! palette::register(cx, |pc, _cx| {
//!     vec![Command::new(
//!         "engineer",                       // unique id
//!         "Race Engineer",                  // label
//!         "race engineer preset assistant", // words it is found by
//!         |window, cx| { /* run it */ },
//!     )
//!     .glyph("◎")                           // or .icon("search"), .lead(Lead::Status(..))
//!     .keys("⌘⇧E")                          // shortcut label, as macOS writes it
//!     .rank(rank::NEW + 50)                 // after "New agent"
//!     .query_only()]                        // hidden until something is typed
//! });
//! ```
//!
//! - The provider runs on open and on every keystroke; [`PaletteContext`]
//!   gives it the query, the agents in sidebar order, the focused agent,
//!   projects and fitting presets, and weak handles to the main screen and
//!   the explorer. Return nothing to hide a row (say, without a selection).
//! - Rows are listed by [`rank`], then in the provider's order. The
//!   built-in sections own the round hundreds.
//! - A row closes the palette, gives the keyboard back to where it was, and
//!   then runs; [`Command::fill`] instead types text and keeps it open.
//! - Labels are [`Span`]s (plain, bold, dim sub-text, quote, mono); key
//!   hints are written as on macOS and shown per desktop (`kit::keys`).

pub mod builtin;
pub mod matching;
pub mod registry;
pub mod view;

#[cfg(test)]
mod tests;

use gpui::App;

pub use registry::{rank, register, Command, Hint, Lead, PaletteContext, Run, Span};
pub use view::{on_actions, Palette, PaletteEvent, PaletteHost, PaletteHostEvent};

/// Once at start: the palette's keys and the built-in rows.
pub fn init(cx: &mut App) {
    cx.bind_keys(view::bindings());
    register(cx, builtin::commands);
}
