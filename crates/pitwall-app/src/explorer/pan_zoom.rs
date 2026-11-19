//! Moving around a picture larger than its pane (an image, a PDF's pages)
//! and zooming it at the pointer: one axis at a time, free of GPUI, so it
//! can be tested.
//!
//! An axis: the picture is `len` points long at its scale, with `pad`
//! points of room each side, in a pane `view` points long. The offset is
//! how far the picture and its room are scrolled (0: their start at the
//! pane's). While it fits with its room the picture is centred and can't
//! be moved.

/// A press travels this far (points) before it moves the picture: less
/// is a click (and a double click zooms).
pub const DRAG_THRESHOLD: f32 = 3.;

/// ⌘ (Ctrl) + scroll: the scale grows by e^(this × points scrolled), about
/// 10 % a wheel's notch.
pub const WHEEL_ZOOM: f32 = 0.005;

/// Points a wheel's line scrolls (wheels that count lines, not points).
pub const LINE: f32 = 20.;

/// A pinch or a ⌘-scroll is over once nothing more came for this long
/// (ms): a PDF's pages are drawn again at the new scale then.
pub const SETTLE_MS: u64 = 200;

/// How far the offset can go.
pub fn max_offset(len: f32, pad: f32, view: f32) -> f32 {
    (len + 2. * pad - view).max(0.)
}

/// `off` kept in reach: the picture never leaves the pane.
pub fn clamp(off: f32, len: f32, pad: f32, view: f32) -> f32 {
    off.clamp(0., max_offset(len, pad, view))
}

/// Where the picture starts in the pane: centred while it fits with its
/// room, else at its room's edge less the offset.
pub fn start(off: f32, len: f32, pad: f32, view: f32) -> f32 {
    if len + 2. * pad <= view {
        (view - len) / 2.
    } else {
        pad - clamp(off, len, pad, view)
    }
}

/// The offset after the picture grows from `old` to `new` points long
/// that keeps the point of it at `at` (pane points) at `at`, as far as
/// the picture can be moved (fitting, it is centred).
pub fn anchored(off: f32, old: f32, new: f32, pad: f32, view: f32, at: f32) -> f32 {
    let u = if old > 0. { (at - start(off, old, pad, view)) / old } else { 0.5 };
    clamp(pad - (at - u * new), new, pad, view)
}

/// A press from `from` to `to` (points) has become a drag.
pub fn dragged(from: (f32, f32), to: (f32, f32)) -> bool {
    (to.0 - from.0).hypot(to.1 - from.1) > DRAG_THRESHOLD
}

/// The scale after a pinch of `delta` (0.1: 10 % larger), within `min`
/// and `max`.
pub fn pinch(scale: f32, delta: f32, min: f32, max: f32) -> f32 {
    (scale * (1. + delta).max(0.01)).clamp(min, max)
}

/// The scale after ⌘-scrolling `dy` points (up, positive: larger),
/// within `min` and `max`.
pub fn wheel(scale: f32, dy: f32, min: f32, max: f32) -> f32 {
    (scale * (dy * WHEEL_ZOOM).exp()).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the picture (0..1 along it) at `at` in the pane.
    fn under(off: f32, len: f32, at: f32) -> f32 {
        (at - start(off, len, 16., 500.)) / len
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        // 1000 pt long in a 500 pt pane, scrolled 300: at 120 is (120 - (16 - 300)) / 1000.
        let (off, at) = (300., 120.);
        let before = under(off, 1000., at);
        assert!((before - 0.404).abs() < 1e-6);
        for new in [1500., 2000., 3000., 800.] {
            let o = anchored(off, 1000., new, 16., 500., at);
            assert!((under(o, new, at) - before).abs() < 1e-5, "{new}: {o}");
        }
        // Twice the size twice over and back: where it was.
        let o = anchored(off, 1000., 2000., 16., 500., at);
        let back = anchored(o, 2000., 1000., 16., 500., at);
        assert!((back - off).abs() < 1e-3);
    }

    #[test]
    fn zooming_near_an_edge_stops_at_it() {
        // The pointer in the room before the start, zoomed in: the start
        // can't come past its room, so the offset is 0 (the point drifts).
        assert_eq!(anchored(0., 1000., 2000., 16., 500., 5.), 0.);
        // Its end, scrolled to the end and zoomed out: still at the end.
        let o = anchored(max_offset(1000., 16., 500.), 1000., 600., 16., 500., 484.);
        assert_eq!(o, max_offset(600., 16., 500.));
        // Small enough to fit: centred, whatever the pointer.
        assert_eq!(anchored(300., 1000., 400., 16., 500., 450.), 0.);
        assert_eq!(start(0., 400., 16., 500.), 50.);
    }

    #[test]
    fn the_offset_stays_in_reach_at_every_edge() {
        // 1000 pt long in 500 with 16 around: 0..532.
        assert_eq!(max_offset(1000., 16., 500.), 532.);
        assert_eq!(clamp(-40., 1000., 16., 500.), 0.);
        assert_eq!(clamp(9000., 1000., 16., 500.), 532.);
        assert_eq!(clamp(200., 1000., 16., 500.), 200.);
        // The picture's ends at the offset's ends: its room at either side.
        assert_eq!(start(0., 1000., 16., 500.), 16.);
        assert_eq!(start(532., 1000., 16., 500.) + 1000., 500. - 16.);
        // Past the ends: held at them.
        assert_eq!(start(-100., 1000., 16., 500.), 16.);
        assert_eq!(start(5000., 1000., 16., 500.), start(532., 1000., 16., 500.));
    }

    #[test]
    fn a_picture_smaller_than_its_pane_is_centred_and_does_not_move() {
        for off in [0., 100., -100.] {
            assert_eq!(clamp(off, 300., 16., 500.), 0.);
            assert_eq!(start(off, 300., 16., 500.), 100.);
        }
        // Exactly fitting with its room: still nothing to move.
        assert_eq!(max_offset(468., 16., 500.), 0.);
        // Fits, but not with its room: moves by the room it lacks.
        assert_eq!(max_offset(480., 16., 500.), 12.);
    }

    #[test]
    fn a_press_drags_once_it_travels() {
        assert!(!dragged((10., 10.), (10., 10.)));
        assert!(!dragged((10., 10.), (12., 12.)), "under 3 pt: a click");
        assert!(!dragged((10., 10.), (13., 10.)), "3 pt exactly: a click");
        assert!(dragged((10., 10.), (13.5, 10.)));
        assert!(dragged((10., 10.), (8., 7.)), "any way");
    }

    #[test]
    fn pinching_and_wheeling_stay_within_the_limits() {
        assert!((pinch(1., 0.1, 0.1, 16.) - 1.1).abs() < 1e-6);
        assert!((pinch(2., -0.5, 0.1, 16.) - 1.).abs() < 1e-6);
        assert_eq!(pinch(15., 0.5, 0.1, 16.), 16.);
        assert_eq!(pinch(0.12, -0.5, 0.1, 16.), 0.1);
        assert_eq!(pinch(1., -5., 0.1, 16.), 0.1, "a wild delta: still in range");
        // Up: larger; down: smaller; a notch about 10 %.
        assert!(wheel(1., LINE, 0.1, 16.) > 1.1 && wheel(1., LINE, 0.1, 16.) < 1.11);
        assert!(wheel(1., -LINE, 0.1, 16.) < 1.);
        assert_eq!(wheel(16., 400., 0.1, 16.), 16.);
        assert_eq!(wheel(0.1, -400., 0.1, 16.), 0.1);
        // Up then down as far: where it was.
        assert!((wheel(wheel(2., 37., 0.1, 16.), -37., 0.1, 16.) - 2.).abs() < 1e-5);
    }
}
