//! The app's OS conventions behind a few functions (architecture.md §9
//! decision 7): the menu, the "needs you" badge, and whatever keeps Pitwall
//! reachable while its window is hidden.
//!
//! - macOS (and other Unix): the default app menu + Settings, the Dock badge,
//!   a Dock click reopens the window.
//! - Windows: a File menu without Edit accelerators (Ctrl+C belongs to the
//!   terminal), a tray icon (click: show Pitwall; menu: Settings, quit), the
//!   blocked count as a taskbar overlay icon and in the tray tooltip.

#[cfg_attr(not(windows), allow(dead_code))]
pub mod badge;

#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
