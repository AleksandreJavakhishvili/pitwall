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
//!   [`brand`]: the Pitwall mark;
//!   [`file_icons`]: the coloured Material file and folder icons.
//! - [`text`]: letter-spaced labels; [`status`]: status glyphs and words.
//! - [`button`], [`chip`], [`controls`] (checkbox, toggle, radio, seg,
//!   select, spinner), [`tooltip`], [`menu`], [`modal`], [`input`] (the
//!   text field, IME included), [`glass`] (Liquid Glass regions).

pub mod brand;
pub mod button;
pub mod chip;
pub mod controls;
pub mod dialog;
pub mod edit;
pub mod file_icons;
pub mod fresh;
pub mod fonts;
pub mod glass;
pub mod hover;
pub mod icons;
pub mod input;
pub mod menu;
pub mod modal;
pub mod motion;
pub mod scm;
pub mod scrollbar;
pub mod status;
pub mod text;
pub mod tooltip;

pub use brand::brand_mark;
pub use button::{
    accent_button, btn_radius, button, glyph_btn, icon_btn, icon_btn_state, kbd, kbd_on_primary,
    keys, keys_for, on_primary, small_btn, text_button, BtnKind,
};
pub use chip::{chip, chip_sm, chip_subtle, chip_tone, Tone};
pub use controls::{
    check, check_row, checkbox, resize_grip, GripDrag, checkbox_box, radio, seg, select, widest, select_menu, select_trigger, spinner, spinner_ms, toggle,
    OnClose, OnPick, SegItem,
};
pub use file_icons::{file_icon, folder_icon, svg_image};
pub use fresh::{fresh_note, refresh_control, updated_label};
pub use fonts::{BODY_LINE_HEIGHT, LABEL_FONT, MONO_FONT, UI_FONT, WEIGHT_550, WEIGHT_650};
pub use hover::HoverText;
pub use glass::{chrome_panel, glass_region, region_sweeper, PANEL_RADIUS};
pub use icons::{chevron, chevron_at, icon, Assets};
pub use input::{InputEvent, TextFieldEvent, TextInput};
pub use menu::{menu_item, menu_panel};
pub use modal::{scrim_in, Modal};
pub use scm::{
    count_badge, diffstat_inline, file_status, status_letter, status_letter_text, status_title,
};
pub use status::{
    diffstat, glyph, glyph_play, glyph_still, status_glyph, status_glyph_el, status_label,
    text_glow, word_color, GlyphMotion, GlyphSize,
};
pub use text::{
    one_line, para, rich, Ellipsis, Para, Span, error_text, hint, label, label_fit, label_t, mono_text, muted, project_name, tracked, Tracked,
};
pub use scrollbar::{hscroll, scroll_area, vscroll, vscroll_fill, vscroll_list, vscroll_list_fill, ScrollArea, Scrollbar};
pub use tooltip::{tooltip, tooltip_view, Tooltip};

use gpui::App;

/// Once at start: the fonts and the kit's key bindings.
pub fn init(cx: &mut App) {
    fonts::register(cx);
    cx.bind_keys(input::bindings());
    cx.bind_keys(modal::bindings());
}
