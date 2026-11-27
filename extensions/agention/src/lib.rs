//! The Agention extension's tile (README.md). `api`: Agention's operator
//! API, version 1; `model`: what the tile draws from it (groups, which of
//! Pitwall's agents asks, the floating control's questions); both plain
//! Rust, tested natively. `tile`: the tile itself, only in the
//! WebAssembly build.

pub mod api;
pub mod model;
#[cfg(target_arch = "wasm32")]
mod tile;
