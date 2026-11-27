//! Write [Pitwall](https://github.com/pitwalls/pitwall) tiles in Rust.
//!
//! A tile is a WebAssembly component that Pitwall runs in a sandbox and
//! draws with its own UI code: the tile describes what to show as a tree
//! of elements, and Pitwall lays it out in its own theme and sends back
//! what the user did. This crate is the tile's side of the
//! `pitwall:tile@0.1.0` world: elements that read like GPUI's, a [`Tile`]
//! trait, typed events, and Pitwall's services ([`log`], [`storage`],
//! [`settings`], [`ui`] and timers on [`Cx`]; Pitwall's agents and their
//! Next up in [`agents`], sidebar [`badges`] and questions in the floating
//! control, [`asks`]).
//!
//! ```ignore
//! use pitwall_extension::prelude::*;
//!
//! #[derive(Default)]
//! struct Counter { clicks: u32 }
//!
//! impl Tile for Counter {
//!     fn new(_cx: &mut Cx<Self>) -> Self {
//!         Counter { clicks: storage::get_json("clicks").ok().flatten().unwrap_or(0) }
//!     }
//!
//!     fn render(&mut self, _view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
//!         v_flex().gap_2().p_3()
//!             .child(text(format!("Clicked {} times", self.clicks)).text_color(Token::Text2))
//!             .child(button("Click me").primary().on_click(cx.listener(|this, _, cx| {
//!                 this.clicks += 1;
//!                 let _ = storage::set_json("clicks", &this.clicks);
//!                 cx.notify();
//!             })))
//!     }
//! }
//!
//! pitwall_extension::export!(Counter);
//! ```
//!
//! Build it with `cargo build --release --target wasm32-wasip2` (the
//! crate is a `cdylib`), or let `pitwall extension dev` build it, load it
//! into the running Pitwall and reload it on every save.
//! `pitwall extension new my-tile --template blank` writes a project to
//! start from, with tests that run the built tile in Pitwall's own runtime
//! (the `pitwall-extension-test` crate).
//!
//! **What a tile has besides Pitwall's APIs.** WASI 0.2 for what Rust's
//! standard library needs: the clocks (`std::time::Instant` and
//! `SystemTime` work, for "5 min ago" and debouncing), random numbers
//! (`getrandom`, hash maps' seeds), and stdout and stderr (`println!` and
//! `eprintln!` go to the tile's Log). No environment, arguments, files or
//! sockets: keep data with [`storage`]. In the test harness
//! (`pitwall-extension-test`) both clocks are virtual and move only as the
//! test lets time pass, so a timer test doesn't sleep.
//!
//! **Opening a tile with content.** A tile's manifest entry can name a JSON
//! schema (`"input"`); the user or an agent then opens a view of the tile
//! with content matching it. The tile hears [`Tile::on_content`] and reads
//! the payload with [`Content::parse`] into its own `serde` type.

// So the WIT glue's paths resolve inside this crate as they do in a tile's.
extern crate self as pitwall_extension;

#[doc(hidden)]
pub mod bindings;
pub mod blocks;
pub mod canvas;
pub mod components;
pub mod element;
pub mod events;
mod group;
mod host;
pub mod style;
mod tile;

pub use element::{button, div, h_flex, icon, icon_button, input, text, v_flex, AnyElement, Button, Div, Handler, Icon, IconElement, Input, Interactive, IntoElement, ParentElement, Text};
pub use blocks::{code_view, diff_view, diff_view_unified, CodeView, DiffView};
pub use canvas::{canvas, Canvas, Viewport};
pub use events::{Blur, Event, Focused};
pub use group::{agents, asks, badges};
pub use host::{log, settings, storage, ui, HostError, Tone};
pub use style::{px, relative, rems, Align, Length, Styled, Token};
pub use tile::{Content, ContentError, Cx, SavedState, Tile, Timer, ViewInfo};
pub use components::{choice, column, image_asset, image_data, markdown, menu_button, menu_icon_button, menu_item, select, separator, tab, table, tabs, tree, tree_row, virtual_list, Column, IntoRow, MenuEntry, Rows, TreeRow};
pub use events::{ColumnWidth, LinkClick, Picked, SortChange, SortDirection, Toggled};

/// The WIT world this SDK builds tiles for, as a manifest's tile entry
/// names it (`"world"`).
pub const WORLD: &str = "pitwall:tile@0.1.0";

/// Everything a tile usually needs: `use pitwall_extension::prelude::*;`.
pub mod prelude {
    pub use crate::element::{button, div, h_flex, icon, icon_button, input, text, v_flex, AnyElement, Button, Div, Handler, Icon, IconElement, Input, Interactive, IntoElement, ParentElement, Text};
    pub use crate::blocks::{code_view, diff_view, diff_view_unified, CodeView, DiffView, Mark};
    pub use crate::canvas::{canvas, Canvas, Label, Paint, Region, Shape, Viewport};
    pub use crate::events::{Blur, Click, CodeSelection, Dropped, Event, Focused, Hover, InputChange, KeyPress, LineClick, Pointer, PointerKind, Side, Submit, Theme, ThemeMode};
    pub use crate::group::agents::{self, Agent};
    pub use crate::group::asks::{self, Answer, Ask};
    pub use crate::group::badges::{self, Badge, Row};
    pub use crate::host::{log, settings, storage, ui, HostError, Tone};
    pub use crate::style::{px, relative, rems, Length, Styled, Token};
    pub use crate::tile::{Content, Cx, SavedState, Tile, Timer, ViewInfo};
    pub use std::time::Duration;
    pub use crate::components::{choice, column, image_asset, image_data, markdown, menu_button, menu_icon_button, menu_item, select, separator, tab, table, tabs, tree, tree_row, virtual_list, Column, MenuEntry, Rows, TreeRow};
    pub use crate::events::{ColumnWidth, LinkClick, Picked, SortChange, SortDirection, Toggled};
}

/// Make `$tile` (a type implementing [`Tile`]) the component's tile: this
/// writes the glue Pitwall calls. Once per crate, at its root:
/// `pitwall_extension::export!(MyTile);`.
#[macro_export]
macro_rules! export {
    ($tile:ty) => {
        #[doc(hidden)]
        struct __PitwallTile;

        impl $crate::__private::Guest for __PitwallTile {
            fn init(cx: $crate::__private::InitContext) -> ::core::result::Result<(), ::std::string::String> {
                $crate::__private::init::<$tile>(cx)
            }
            fn render(view: ::std::string::String) -> $crate::__private::Frame {
                $crate::__private::render::<$tile>(view)
            }
            fn handle(event: $crate::__private::WitEvent) -> bool {
                $crate::__private::handle::<$tile>(event)
            }
            fn save_state() -> ::std::vec::Vec<u8> {
                $crate::__private::save_state::<$tile>()
            }
            fn rows(view: ::std::string::String, key_path: ::std::string::String, start: u32, count: u32) -> ::std::vec::Vec<$crate::__private::Row> {
                $crate::__private::rows::<$tile>(view, key_path, start, count)
            }
        }

        $crate::__private::bindings::__export_tile_glue!(__PitwallTile with_types_in $crate::__private::bindings);
    };
}

/// The glue `export!` expands to. Not part of the API.
#[doc(hidden)]
pub mod __private {
    pub use crate::bindings;
    pub use crate::bindings::exports::pitwall::tile::guest::Guest;
    pub use crate::bindings::pitwall::tile::types::{Event as WitEvent, Frame, InitContext};
    pub use crate::bindings::pitwall::tile::types::Row;
    pub use crate::tile::{handle, init, render, save_state};
    pub use crate::tile::rows;
    pub use wit_bindgen;
}
