//! The C4 diagram tile (docs/spec/2026-11-27-c4-tile/). Its core is plain
//! Rust that needs no Tile SDK, so it is written and tested natively
//! (plan.md phase A) and compiles for `wasm32-wasip2`; the `tile` feature
//! (on by default) adds the Painter and the tile itself, built into
//! `tile.wasm`.
//!
//! Markdown → [`blocks`] → [`mermaid`] → [`c4`] structure → [`index`]
//! (zoom links, before/after pairs) → [`changes`] and [`layout`]; an
//! agent's Mermaid → [`show`] → the tile (`tile`).

pub mod blocks;
pub mod c4;
pub mod changes;
pub mod index;
pub mod layout;
pub mod limits;
pub mod mermaid;
#[cfg(feature = "tile")]
pub mod paint;
pub mod show;
#[cfg(feature = "tile")]
pub mod tile;
pub mod text;
