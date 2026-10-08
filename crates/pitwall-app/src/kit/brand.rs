//! The Pitwall mark, the Apex P, drawn exactly as the website header and
//! the React top bar draw it (`website/index.html` `.brand .mark`,
//! `TopBar.tsx`): the racing-line P stroked in the theme's ink, its
//! chequered tail masked to the stroke in `--mark-squares`, no tile.
//!
//! GPUI draws an SVG as one mask tinted by the text colour, so the mark is
//! two assets served by the kit's [`Assets`](super::Assets): the line
//! (`brand/mark-line.svg`) and the squares, already masked to the line's
//! stroke inside their own file (`brand/mark-squares.svg`), each tinted by
//! the theme.

use gpui::{div, px, svg, Div, Hsla, ParentElement, SharedString, Styled};

use crate::theme::{Mode, Theme};

/// The racing line.
pub const LINE_PATH: &str = "brand/mark-line.svg";
/// The chequer squares, masked to the line.
pub const SQUARES_PATH: &str = "brand/mark-squares.svg";

const LINE_SVG: &str = include_str!("../../assets/brand/mark-line.svg");
const SQUARES_SVG: &str = include_str!("../../assets/brand/mark-squares.svg");

/// `.wordmark-mark`: 20 x 20 px.
const SIZE: f32 = 20.;

/// The asset at `path`, if it is one of the mark's layers.
pub fn layer(path: &str) -> Option<&'static str> {
    match path {
        LINE_PATH => Some(LINE_SVG),
        SQUARES_PATH => Some(SQUARES_SVG),
        _ => None,
    }
}

/// `--mark-squares`.
fn squares(t: &Theme) -> Hsla {
    match t.mode {
        Mode::Dark => gpui::rgb(0x353c46).into(),
        Mode::Light => gpui::rgb(0xc5cbd3).into(),
    }
}

/// The mark, in the theme's ink (`--text`).
pub fn brand_mark(t: &Theme) -> Div {
    let layer = |path: &'static str, color: Hsla| {
        svg()
            .path(SharedString::from(path))
            .absolute()
            .top_0()
            .left_0()
            .size(px(SIZE))
            .text_color(color)
    };
    div()
        .relative()
        .flex_none()
        .size(px(SIZE))
        .child(layer(LINE_PATH, t.text))
        .child(layer(SQUARES_PATH, squares(t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layers_are_the_website_mark() {
        let line = layer(LINE_PATH).unwrap();
        assert!(line.contains(r#"viewBox="7 7 50 50""#));
        assert!(line.contains("M1 52H10C19 52 24 48 24 40V13H33A12.5 12.5 0 0 1 33 38H24"));
        let sq = layer(SQUARES_PATH).unwrap();
        assert!(sq.contains(r#"mask="url(#m)""#));
        assert!(sq.contains("M1 47.5h4.5V52H1z"));
        assert!(layer("brand/nope.svg").is_none());
    }
}
