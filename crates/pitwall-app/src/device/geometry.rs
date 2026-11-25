//! Where a point in the tile is on the device's screen.
//!
//! The screen is drawn whole and centred in the tile (letterboxed), turned
//! by the tile's own [`Turn`] (the user turning the picture, not the
//! device). A point is mapped back to a ratio of the frame as the device
//! sent it (0..1 from its top left), which is what `pitwall-simhost`
//! takes; [`to_pixel`] gives scrcpy's pixel position in the frame's size,
//! which scrcpy requires to be the current video size.

/// How the picture is turned in the tile, clockwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Turn {
    #[default]
    None,
    Right,
    Half,
    Left,
}

impl Turn {
    /// A quarter turn further clockwise.
    pub fn next(self) -> Turn {
        match self {
            Turn::None => Turn::Right,
            Turn::Right => Turn::Half,
            Turn::Half => Turn::Left,
            Turn::Left => Turn::None,
        }
    }

    /// Whether width and height swap.
    pub fn sideways(self) -> bool {
        matches!(self, Turn::Right | Turn::Left)
    }
}

/// A rectangle: origin and size, in the tile's points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Where the picture is drawn in a tile of `tile` (w, h) points: as large
/// as fits, centred, its aspect kept. `None` for an empty tile or frame.
pub fn fit(tile: (f32, f32), frame: (u32, u32), turn: Turn) -> Option<Rect> {
    let (fw, fh) = if turn.sideways() { (frame.1 as f32, frame.0 as f32) } else { (frame.0 as f32, frame.1 as f32) };
    if tile.0 <= 0. || tile.1 <= 0. || fw <= 0. || fh <= 0. {
        return None;
    }
    let scale = (tile.0 / fw).min(tile.1 / fh);
    let (w, h) = (fw * scale, fh * scale);
    Some(Rect { x: (tile.0 - w) / 2., y: (tile.1 - h) / 2., w, h })
}

/// The point (`x`, `y`) of the tile as a ratio of the frame the device
/// sent; `None` outside the picture (in the letterbox).
pub fn to_screen(p: (f32, f32), tile: (f32, f32), frame: (u32, u32), turn: Turn) -> Option<(f64, f64)> {
    let r = fit(tile, frame, turn)?;
    let u = ((p.0 - r.x) / r.w) as f64;
    let v = ((p.1 - r.y) / r.h) as f64;
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return None;
    }
    // (u, v) is in the turned picture; undo the turn.
    Some(match turn {
        Turn::None => (u, v),
        Turn::Right => (v, 1. - u),
        Turn::Half => (1. - u, 1. - v),
        Turn::Left => (1. - v, u),
    })
}

/// A ratio of the frame as a pixel of it (clamped inside).
pub fn to_pixel(ratio: (f64, f64), frame: (u32, u32)) -> (i32, i32) {
    let px = |r: f64, n: u32| ((r * n as f64).floor() as i64).clamp(0, n.saturating_sub(1) as i64) as i32;
    (px(ratio.0, frame.0), px(ratio.1, frame.1))
}
