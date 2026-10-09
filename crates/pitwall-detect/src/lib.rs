//! Pitwall's screen-based status detection (architecture.md §1).
//!
//! `Screen` is the headless terminal each agent's output is fed into;
//! `detect` matches the per-kind TOML rules (`detect/*.toml`, plus user
//! overrides in `~/.pitwall/detect/`) against its text.

pub mod detect;
pub mod screen;

pub use detect::{builtin_rule_kinds, detect, Detected, Detection};
pub use screen::{Screen, ScreenSource};
