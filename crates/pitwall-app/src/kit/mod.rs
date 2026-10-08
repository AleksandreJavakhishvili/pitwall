//! The in-house component kit (docs/spec/gpui/in-house.md): the widgets
//! every screen draws with, matching the React app's CSS (`base.css`,
//! `overlays.css`, `terminals.css`, `glass.css`). Built from GPUI's own API;
//! nothing from Zed's GPL crates.
//!
//! Widgets are plain functions over a [`Theme`](crate::theme::Theme): pass
//! `theme.chrome()` inside chrome (top bar, sidebar, panels) and
//! `theme.float()` inside dialogs and menus, and they follow Flat or Glass
//! (pill buttons, glass surfaces) by themselves.
//!
//! - [`fonts`]: the bundled faces, registered once ([`init`]).
//! - [`icons`]: the app's one `AssetSource` and its stroke icons;
//!   [`file_icons`]: the coloured Material file and folder icons.
//! - [`text`]: letter-spaced labels; [`status`]: status glyphs and words.
//! - [`button`], [`chip`], [`controls`] (checkbox, toggle, radio, seg,
//!   select, spinner), [`tooltip`], [`menu`], [`modal`], [`input`] (the
//!   text field, IME included), [`glass`] (Liquid Glass regions).

pub mod button;
pub mod chip;
pub mod controls;
pub mod file_icons;
pub mod fonts;
pub mod glass;
pub mod hover;
pub mod icons;
pub mod input;
pub mod menu;
pub mod modal;
pub mod status;
pub mod text;
pub mod tooltip;

pub use button::{
    accent_button, btn_radius, button, glyph_btn, icon_btn, icon_btn_state, kbd, kbd_on_primary,
    keys, keys_for, on_primary, small_btn, text_button, BtnKind,
};
pub use chip::{chip, chip_sm, chip_subtle, chip_tone, Tone};
pub use controls::{
    check, checkbox, checkbox_box, radio, seg, select_menu, select_trigger, spinner, toggle,
    OnClose, OnPick, SegItem,
};
pub use file_icons::{file_icon, folder_icon, svg_image};
pub use fonts::{LABEL_FONT, MONO_FONT, UI_FONT};
pub use hover::HoverText;
pub use glass::{chrome_panel, glass_region, region_sweeper, PANEL_RADIUS};
pub use icons::{chevron, icon, Assets};
pub use input::{InputEvent, TextInput};
pub use menu::{menu_item, menu_panel};
pub use modal::Modal;
pub use status::{diffstat, glyph, status_glyph_el, status_label, word_color, GlyphSize};
pub use text::{
    error_text, hint, label, label_fit, label_t, mono_text, muted, project_name, tracked, Tracked,
};
pub use tooltip::{tooltip, tooltip_view, Tooltip};

use gpui::App;

/// Once at start: the fonts and the kit's key bindings.
pub fn init(cx: &mut App) {
    fonts::register(cx);
    cx.bind_keys(input::bindings());
    cx.bind_keys(modal::bindings());
}
