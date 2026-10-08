//! The Pitwall mark (`assets/brand/pitwall.svg`, the 32 px cut of the
//! Apex P): a solid P with a short chequered tail. The top bar draws it as
//! the website's header does, without the icon's tile: the P in the page's
//! ink and the chequered squares at its foot in a quieter tone painted over
//! the stem, both following the theme.
//!
//! GPUI draws an SVG as one mask tinted by the text colour, so the file is
//! served as two layers (`brand/pitwall-ink.svg`, `brand/pitwall-tail.svg`)
//! through the kit's [`Assets`](super::Assets). Both are cropped to the P
//! at 1.5 device px per unit (14 x 20 px at 2x), which puts every straight
//! edge of the 32-unit grid on a whole device pixel.

use gpui::{div, px, svg, Div, ParentElement, Rgba, SharedString, Styled};

use crate::theme::{Mode, Theme};

/// The mark as shipped (`docs/brand/pitwall-small-32.svg`).
pub const PITWALL_SVG: &str = include_str!("../../assets/brand/pitwall.svg");

/// The P's ink layer.
pub const INK_PATH: &str = "brand/pitwall-ink.svg";
/// The tail's squares.
pub const TAIL_PATH: &str = "brand/pitwall-tail.svg";

/// The mark's box in the top bar (px).
const W: f32 = 14.;
const H: f32 = 20.;

/// A layer of [`PITWALL_SVG`]: its `<path>` (ink) or its `<rect x=…>`
/// squares (tail), without the tile, cropped to the P.
pub fn layer(path: &str) -> Option<String> {
    let body: Vec<&str> = if path == INK_PATH {
        elements(PITWALL_SVG, "<path ")
    } else if path == TAIL_PATH {
        elements(PITWALL_SVG, "<rect x=")
    } else {
        return None;
    };
    // 7,3 is the P's top-left corner; 14 x 20 px at 1.5 device px a unit.
    let (vw, vh) = (W * 2. / 1.5, H * 2. / 1.5);
    Some(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="7 3 {vw:.4} {vh:.4}">{}</svg>"##,
        body.concat()
    ))
}

/// Every `<tag …/>` element of `svg` that starts with `start`.
fn elements<'a>(svg: &'a str, start: &str) -> Vec<&'a str> {
    let mut out = vec![];
    let mut rest = svg;
    while let Some(i) = rest.find(start) {
        let Some(end) = rest[i..].find("/>") else {
            break;
        };
        out.push(&rest[i..i + end + 2]);
        rest = &rest[i + end + 2..];
    }
    out
}

/// The website's `--mark-ink` and `--mark-b` for this theme.
fn colors(t: &Theme) -> (Rgba, Rgba) {
    match t.mode {
        Mode::Dark => (gpui::rgb(0xe8ebef), gpui::rgb(0x353c46)),
        Mode::Light => (gpui::rgb(0x111418), gpui::rgb(0xc5cbd3)),
    }
}

/// The mark, in the theme's ink.
pub fn brand_mark(t: &Theme) -> Div {
    let (ink, tail) = colors(t);
    let layer = |path: &'static str, color: Rgba| {
        svg()
            .path(SharedString::from(path))
            .absolute()
            .top_0()
            .left_0()
            .w(px(W))
            .h(px(H))
            .text_color(color)
    };
    div()
        .relative()
        .flex_none()
        .w(px(W))
        .h(px(H))
        .child(layer(INK_PATH, ink))
        .child(layer(TAIL_PATH, tail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layers_split_the_mark_without_its_tile() {
        let ink = layer(INK_PATH).unwrap();
        assert!(ink.contains("M7 3H17A8 8 0 0 1 17 19H11V29H7Z"));
        assert!(!ink.contains("<rect"));
        let tail = layer(TAIL_PATH).unwrap();
        assert_eq!(tail.matches("<rect").count(), 3);
        assert!(!tail.contains("rx=\"7\""));
        assert!(layer("brand/nope.svg").is_none());
    }
}
