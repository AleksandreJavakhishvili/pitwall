//! How the viewer draws an image: fitted to the pane (never enlarged past
//! its own size) or at a zoom, over a quiet checkerboard so transparent
//! parts read as transparent, not as the pane's colour. The zoom steps and
//! the fitted scale are free of GPUI, so they can be tested.

use gpui::{canvas, fill, point, px, size, Bounds, Hsla, IntoElement, Pixels, Styled};

/// How an image is shown.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Zoom {
    /// The whole image in the pane (small images at their own size).
    #[default]
    Fit,
    /// One image pixel per `f32` points.
    Scale(f32),
}

/// The zoom steps − and + walk through.
pub const STEPS: [f32; 12] = [0.1, 0.25, 0.5, 0.75, 1., 1.5, 2., 3., 4., 6., 8., 16.];

/// Room around an image in the pane.
pub const PAD: f32 = 16.;

/// The scale that shows all of an image `img` (w, h px) in `area`
/// (w, h pt): smaller when it doesn't fit, never larger than 100 %.
pub fn fit_scale(img: (f32, f32), area: (f32, f32)) -> f32 {
    if img.0 <= 0. || img.1 <= 0. {
        return 1.;
    }
    let s = ((area.0 - 2. * PAD) / img.0).min((area.1 - 2. * PAD) / img.1);
    s.clamp(0.01, 1.)
}

/// The scale `zoom` draws at.
pub fn scale(zoom: Zoom, img: (f32, f32), area: (f32, f32)) -> f32 {
    match zoom {
        Zoom::Fit => fit_scale(img, area),
        Zoom::Scale(s) => s,
    }
}

/// The next step up from `current` (the last one stays).
pub fn zoom_in(current: f32) -> f32 {
    STEPS.iter().copied().find(|s| *s > current + 1e-3).unwrap_or(STEPS[STEPS.len() - 1])
}

/// The next step down from `current` (below the first one: stays).
pub fn zoom_out(current: f32) -> f32 {
    STEPS.iter().rev().copied().find(|s| *s < current - 1e-3).unwrap_or(current.min(STEPS[0]))
}

/// The range a pinch or ⌘-scroll zooms in: the steps' (and down to the
/// fitted scale, when that is smaller still).
pub fn limits(fitted: f32) -> (f32, f32) {
    (STEPS[0].min(fitted), STEPS[STEPS.len() - 1])
}

/// "100%", "33%".
pub fn percent(scale: f32) -> String {
    format!("{}%", (scale * 100.).round() as i64)
}

/// A checkerboard of 8 pt squares in `a` and `b`, painted only where it
/// is seen (a large image at 1600 % would be millions of squares).
pub fn checkerboard(a: Hsla, b: Hsla) -> impl IntoElement + Styled {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            const SQ: f32 = 8.;
            window.paint_quad(fill(bounds, a));
            let seen = bounds.intersect(&window.content_mask().bounds);
            if seen.size.width <= px(0.) || seen.size.height <= px(0.) {
                return;
            }
            let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let first_col = ((f32::from(seen.origin.x) - x0) / SQ).floor() as i64;
            let last_col = ((f32::from(seen.right()) - x0) / SQ).ceil() as i64;
            let first_row = ((f32::from(seen.origin.y) - y0) / SQ).floor() as i64;
            let last_row = ((f32::from(seen.bottom()) - y0) / SQ).ceil() as i64;
            for r in first_row..last_row {
                for c in first_col..last_col {
                    if (r + c) % 2 == 0 {
                        continue;
                    }
                    let sq = Bounds::new(point(px(x0 + c as f32 * SQ), px(y0 + r as f32 * SQ)), size(px(SQ), px(SQ)));
                    window.paint_quad(fill(sq.intersect(&bounds), b));
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shrinks_large_images_and_leaves_small_ones() {
        // 96 px in a big pane: its own size.
        assert_eq!(fit_scale((96., 96.), (800., 600.)), 1.);
        // 2000 × 1000 in 1032 × 632 (16 pt around): half.
        assert!((fit_scale((2000., 1000.), (1032., 632.)) - 0.5).abs() < 1e-6);
        // The taller side decides.
        assert!((fit_scale((100., 1000.), (1032., 532.)) - 0.5).abs() < 1e-6);
        assert_eq!(fit_scale((0., 10.), (100., 100.)), 1.);
        assert_eq!(scale(Zoom::Scale(2.), (10., 10.), (100., 100.)), 2.);
        assert_eq!(scale(Zoom::Fit, (96., 96.), (800., 600.)), 1.);
    }

    #[test]
    fn zoom_walks_the_steps_from_anywhere() {
        assert_eq!(zoom_in(1.), 1.5);
        assert_eq!(zoom_out(1.), 0.75);
        // From a fitted 33 %: the steps around it.
        assert_eq!(zoom_in(0.33), 0.5);
        assert_eq!(zoom_out(0.33), 0.25);
        // The ends stay.
        assert_eq!(zoom_in(16.), 16.);
        assert_eq!(zoom_out(0.1), 0.1);
        assert_eq!(zoom_out(0.05), 0.05);
        assert_eq!(percent(1.), "100%");
        assert_eq!(percent(0.333), "33%");
        // A pinch's range: the steps', or down to a smaller fitted scale.
        assert_eq!(limits(0.5), (0.1, 16.));
        assert_eq!(limits(0.04), (0.04, 16.));
    }
}
