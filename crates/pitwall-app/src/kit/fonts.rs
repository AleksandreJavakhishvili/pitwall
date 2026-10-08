//! The app's fonts: Inter (UI), Barlow Condensed (labels) and JetBrains
//! Mono (code and terminals), OFL-1.1 (`LICENSES/`). Static TTF instances
//! of the `@fontsource` files the web UI loads (CoreText can't load woff2,
//! and an in-memory variable font draws bold as regular), registered once
//! at start so every screen and the terminal view find them by name.

use std::borrow::Cow;

use gpui::{App, Global};

/// `--font-ui`.
pub const UI_FONT: &str = "Inter";
/// `--font-label`.
pub const LABEL_FONT: &str = "Barlow Condensed";
/// `--font-mono` (also the terminal's first family).
pub const MONO_FONT: &str = "JetBrains Mono";

const FONTS: [&[u8]; 11] = [
    include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/Inter-Bold.ttf"),
    include_bytes!("../../assets/fonts/BarlowCondensed-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/BarlowCondensed-Bold.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-BoldItalic.ttf"),
];

struct FontsLoaded;

impl Global for FontsLoaded {}

/// Register the bundled fonts (once per app; later calls do nothing).
pub fn register(cx: &mut App) {
    if cx.has_global::<FontsLoaded>() {
        return;
    }
    let fonts = FONTS.iter().map(|b| Cow::Borrowed(*b)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("pitwall: could not load the bundled fonts: {e}");
    }
    cx.set_global(FontsLoaded);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_bundled_fonts_load_once(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            register(cx);
            register(cx);
            assert!(cx.has_global::<FontsLoaded>());
        });
    }
}
