//! The Wall's typography: the app's fonts (Inter, Barlow Condensed,
//! JetBrains Mono; OFL-1.1, LICENSES/), letter-spaced uppercase labels and
//! the status glyph. Kept here until the shared kit has them (promote:
//! `ensure_fonts`, `tracked_label`, `glyph`).

use std::borrow::Cow;

use gpui::{
    canvas, div, font, point, prelude::*, px, App, FontWeight, Global, Hsla, IntoElement,
    PathBuilder, SharedString, TextRun, Window,
};

use pitwall_proto::Status;

use crate::agents::status_glyph;
use crate::theme::Theme;

/// `--font-ui`, `--font-label`, `--font-mono`.
pub const UI: &str = "Inter";
pub const LABEL: &str = "Barlow Condensed";
pub const MONO: &str = "JetBrains Mono";

struct FontsLoaded;
impl Global for FontsLoaded {}

/// Register the bundled fonts once per app.
pub fn ensure_fonts(cx: &mut App) {
    if cx.has_global::<FontsLoaded>() {
        return;
    }
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Bold.ttf")),
        Cow::Borrowed(include_bytes!(
            "../../assets/fonts/BarlowCondensed-SemiBold.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../assets/fonts/BarlowCondensed-Bold.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../assets/fonts/JetBrainsMono-Regular.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../assets/fonts/JetBrainsMono-SemiBold.ttf"
        )),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("pitwall: could not load the bundled fonts: {e}");
    }
    cx.set_global(FontsLoaded);
}

/// The gap after each letter for CSS `letter-spacing: <em>em`.
pub fn tracking(size: f32, em: f32) -> f32 {
    size * em
}

/// An uppercase label with CSS `letter-spacing` (GPUI has none): each
/// letter is drawn at its own advance plus the gap, as browsers do; the
/// box is exactly as wide as the browser's.
pub fn tracked_label(
    window: &Window,
    text: &str,
    family: &'static str,
    size: f32,
    weight: FontWeight,
    em: f32,
    color: Hsla,
) -> impl IntoElement {
    let gap = tracking(size, em);
    let mut f = font(family);
    f.weight = weight;
    let ts = window.text_system();
    let id = ts.resolve_font(&f);
    let chars: Vec<(char, f32)> = text
        .to_uppercase()
        .chars()
        .map(|c| {
            let w = ts
                .advance(id, px(size), c)
                .map(|s| f32::from(s.width))
                .unwrap_or(size * 0.5);
            (c, w)
        })
        .collect();
    let width: f32 = chars.iter().map(|(_, w)| w + gap).sum();
    let line_h = (size * LABEL_LINE).round();
    canvas(
        |_, _, _| {},
        move |b, _, window, cx| {
            let mut x = b.left();
            for (c, w) in &chars {
                let s: SharedString = c.to_string().into();
                let run = TextRun {
                    len: s.len(),
                    font: f.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(s, px(size), &[run], None);
                let _ = line.paint(point(x, b.top()), b.size.height, window, cx);
                x += px(w + gap);
            }
        },
    )
    .w(px(width))
    .h(px(line_h))
    .flex_none()
}

/// `line-height: normal` for the label font, near enough.
const LABEL_LINE: f32 = 1.2;

/// `StatusGlyph` (`.glyph`, 16 px box). Dots and the blocked triangle are
/// drawn (the bundled fonts don't have those characters, and fallback
/// fonts draw them at other sizes); the other flags are text.
pub fn glyph(s: Status, t: &Theme) -> impl IntoElement {
    let color = t.status_color(s);
    let b = div()
        .size(px(16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(color);
    match s {
        // `.pulse-dot` (still: the Wall has no motion).
        Status::Working => b.child(div().size(px(7.)).rounded_full().bg(color)),
        // `●` at 0.8em, as the system font draws it.
        Status::Idle => b.child(div().size(px(8.)).rounded_full().bg(color)),
        Status::Blocked => b.child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let (l, tp) = (bounds.left(), bounds.top());
                    let mut p = PathBuilder::fill();
                    p.move_to(point(l + px(3.5), tp));
                    p.line_to(point(l + px(7.), tp + px(6.5)));
                    p.line_to(point(l, tp + px(6.5)));
                    p.close();
                    if let Ok(path) = p.build() {
                        window.paint_path(path, color);
                    }
                },
            )
            .w(px(7.))
            .h(px(6.5)),
        ),
        _ => {
            let size = match s {
                Status::Done => 11. * 1.15,
                Status::Stopped | Status::Exited => 11. * 0.75,
                _ => 11.,
            };
            let b = b.text_size(px(size)).line_height(px(16.));
            if s == Status::Unknown {
                b.font_family(MONO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(status_glyph(s))
            } else {
                b.child(status_glyph(s))
            }
        }
    }
}

/// The project's name for headings: the last part of its folder (the
/// engine gives `~/code/checkout-web`; the heading says `checkout-web`).
pub fn project_name(display: &str) -> &str {
    let t = display.trim_end_matches(['/', '\\']);
    match t.rfind(['/', '\\']) {
        Some(i) if i + 1 < t.len() => &t[i + 1..],
        _ if t.is_empty() => display,
        _ => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_use_the_folder_name() {
        assert_eq!(project_name("~/code/checkout-web"), "checkout-web");
        assert_eq!(project_name("/srv/work/api/"), "api");
        assert_eq!(project_name(r"C:\work\beta"), "beta");
        assert_eq!(project_name("handbook"), "handbook");
        assert_eq!(project_name("~"), "~");
        assert_eq!(project_name("/"), "/");
    }

    #[test]
    fn tracking_is_em_based() {
        assert!((tracking(14., 0.12) - 1.68).abs() < 1e-5);
    }

    #[gpui::test]
    fn the_bundled_fonts_load(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            ensure_fonts(cx);
            ensure_fonts(cx);
            assert!(cx.has_global::<FontsLoaded>());
        });
    }
}
