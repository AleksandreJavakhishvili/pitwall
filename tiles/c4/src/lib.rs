//! The C4 diagram tile's core (docs/spec/2026-11-27-c4-tile/): plain Rust
//! that needs no Tile SDK, so it is written and tested natively (plan.md
//! phase A) and compiles for `wasm32-wasip2`.
//!
//! Markdown → [`blocks`] → [`mermaid`] → [`c4`] structure → [`index`]
//! (zoom links, before/after pairs) → [`changes`] and [`layout`].

pub mod blocks;
pub mod c4;
pub mod changes;
pub mod index;
pub mod layout;
pub mod limits;
pub mod mermaid;
#[cfg(feature = "tile")]
pub mod paint;
pub mod text;
