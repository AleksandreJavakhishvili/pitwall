//! Box-drawing (U+2500–U+257F) and block elements (U+2580–U+259F), drawn as
//! rectangles snapped to device pixels instead of font glyphs, so lines
//! meet across cells with no gaps at any line height or font.
//!
//! Each character is described by its four arms (up, down, left, right) and
//! their weight; geometry is computed in device pixels from the cell's
//! device-pixel rectangle. Diagonals (U+2571–U+2573) are left to the font.

/// Weight of one arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum W {
    None,
    Light,
    Heavy,
    Double,
}

/// Something to fill, in device pixels relative to the cell's top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prim {
    /// A filled rectangle `[x0, x1) × [y0, y1)`; `alpha` scales the colour (shades).
    Rect { x0: i32, y0: i32, x1: i32, y1: i32, alpha: f32 },
    /// A quarter circle drawn as the rounded corner of a bordered box whose
    /// `corner` is at the cell centre; the box extends past the cell, so the
    /// caller clips to the cell. `x0..x1 × y0..y1` is the whole box.
    Arc { x0: i32, y0: i32, x1: i32, y1: i32, corner: Corner, thickness: i32, radius: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Whether `ch` is drawn by [`prims`] rather than by the font.
pub fn is_drawn(ch: char) -> bool {
    matches!(ch as u32, 0x2500..=0x2570 | 0x2574..=0x259f)
}

/// (up, down, left, right) for the line characters.
fn arms(ch: char) -> Option<[W; 4]> {
    use W::{Double as D, Heavy as H, Light as L, None as N};
    let c = ch as u32;
    Some(match c {
        0x2500 | 0x2504 | 0x2508 | 0x254c => [N, N, L, L],
        0x2501 | 0x2505 | 0x2509 | 0x254d => [N, N, H, H],
        0x2502 | 0x2506 | 0x250a | 0x254e => [L, L, N, N],
        0x2503 | 0x2507 | 0x250b | 0x254f => [H, H, N, N],
        0x250c => [N, L, N, L],
        0x250d => [N, L, N, H],
        0x250e => [N, H, N, L],
        0x250f => [N, H, N, H],
        0x2510 => [N, L, L, N],
        0x2511 => [N, L, H, N],
        0x2512 => [N, H, L, N],
        0x2513 => [N, H, H, N],
        0x2514 => [L, N, N, L],
        0x2515 => [L, N, N, H],
        0x2516 => [H, N, N, L],
        0x2517 => [H, N, N, H],
        0x2518 => [L, N, L, N],
        0x2519 => [L, N, H, N],
        0x251a => [H, N, L, N],
        0x251b => [H, N, H, N],
        0x251c => [L, L, N, L],
        0x251d => [L, L, N, H],
        0x251e => [H, L, N, L],
        0x251f => [L, H, N, L],
        0x2520 => [H, H, N, L],
        0x2521 => [H, L, N, H],
        0x2522 => [L, H, N, H],
        0x2523 => [H, H, N, H],
        0x2524 => [L, L, L, N],
        0x2525 => [L, L, H, N],
        0x2526 => [H, L, L, N],
        0x2527 => [L, H, L, N],
        0x2528 => [H, H, L, N],
        0x2529 => [H, L, H, N],
        0x252a => [L, H, H, N],
        0x252b => [H, H, H, N],
        0x252c => [N, L, L, L],
        0x252d => [N, L, H, L],
        0x252e => [N, L, L, H],
        0x252f => [N, L, H, H],
        0x2530 => [N, H, L, L],
        0x2531 => [N, H, H, L],
        0x2532 => [N, H, L, H],
        0x2533 => [N, H, H, H],
        0x2534 => [L, N, L, L],
        0x2535 => [L, N, H, L],
        0x2536 => [L, N, L, H],
        0x2537 => [L, N, H, H],
        0x2538 => [H, N, L, L],
        0x2539 => [H, N, H, L],
        0x253a => [H, N, L, H],
        0x253b => [H, N, H, H],
        0x253c => [L, L, L, L],
        0x253d => [L, L, H, L],
        0x253e => [L, L, L, H],
        0x253f => [L, L, H, H],
        0x2540 => [H, L, L, L],
        0x2541 => [L, H, L, L],
        0x2542 => [H, H, L, L],
        0x2543 => [H, L, H, L],
        0x2544 => [H, L, L, H],
        0x2545 => [L, H, H, L],
        0x2546 => [L, H, L, H],
        0x2547 => [H, L, H, H],
        0x2548 => [L, H, H, H],
        0x2549 => [H, H, H, L],
        0x254a => [H, H, L, H],
        0x254b => [H, H, H, H],
        0x2550 => [N, N, D, D],
        0x2551 => [D, D, N, N],
        0x2552 => [N, L, N, D],
        0x2553 => [N, D, N, L],
        0x2554 => [N, D, N, D],
        0x2555 => [N, L, D, N],
        0x2556 => [N, D, L, N],
        0x2557 => [N, D, D, N],
        0x2558 => [L, N, N, D],
        0x2559 => [D, N, N, L],
        0x255a => [D, N, N, D],
        0x255b => [L, N, D, N],
        0x255c => [D, N, L, N],
        0x255d => [D, N, D, N],
        0x255e => [L, L, N, D],
        0x255f => [D, D, N, L],
        0x2560 => [D, D, N, D],
        0x2561 => [L, L, D, N],
        0x2562 => [D, D, L, N],
        0x2563 => [D, D, D, N],
        0x2564 => [N, L, D, D],
        0x2565 => [N, D, L, L],
        0x2566 => [N, D, D, D],
        0x2567 => [L, N, D, D],
        0x2568 => [D, N, L, L],
        0x2569 => [D, N, D, D],
        0x256a => [L, L, D, D],
        0x256b => [D, D, L, L],
        0x256c => [D, D, D, D],
        0x2574 => [N, N, L, N],
        0x2575 => [L, N, N, N],
        0x2576 => [N, N, N, L],
        0x2577 => [N, L, N, N],
        0x2578 => [N, N, H, N],
        0x2579 => [H, N, N, N],
        0x257a => [N, N, N, H],
        0x257b => [N, H, N, N],
        0x257c => [N, N, L, H],
        0x257d => [L, H, N, N],
        0x257e => [N, N, H, L],
        0x257f => [H, L, N, N],
        _ => return None,
    })
}

/// Dash count for the dashed lines (0 = solid).
fn dashes(ch: char) -> u32 {
    match ch as u32 {
        0x2504..=0x2507 => 3,
        0x2508..=0x250b => 4,
        0x254c..=0x254f => 2,
        _ => 0,
    }
}

/// The shapes for `ch` in a cell of `w × h` device pixels, with light lines
/// `light` device pixels thick. `None` = not drawn here.
pub fn prims(ch: char, w: i32, h: i32, light: i32) -> Option<Vec<Prim>> {
    let c = ch as u32;
    if (0x2580..=0x259f).contains(&c) {
        return Some(block(c, w, h));
    }
    if (0x256d..=0x2570).contains(&c) {
        return Some(vec![arc(c, w, h, light)]);
    }
    let a = arms(ch)?;
    let n = dashes(ch);
    if n > 0 {
        return Some(dashed(a, n, w, h, light));
    }
    Some(lines(a, w, h, light))
}

fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Prim {
    Prim::Rect { x0, y0, x1, y1, alpha: 1.0 }
}

fn thickness(wt: W, light: i32) -> i32 {
    match wt {
        W::Heavy => light * 2,
        _ => light,
    }
}

/// Start of a centred band of thickness `t` in `0..len`.
fn centred(len: i32, t: i32) -> i32 {
    (len - t) / 2
}

fn lines(a: [W; 4], w: i32, h: i32, t: i32) -> Vec<Prim> {
    let [up, down, left, right] = a;
    let mut out = Vec::with_capacity(8);
    // Double lines: two bands of `t` with a gap of `t`.
    let dv = centred(w, 3 * t); // left edge of a double vertical
    let dh = centred(h, 3 * t); // top edge of a double horizontal
    let is_d = |x: W| x == W::Double;
    let single = |x: W| matches!(x, W::Light | W::Heavy);
    // The widest single vertical / horizontal, for where single arms meet.
    let vt = thickness(if up == W::Heavy || down == W::Heavy { W::Heavy } else { W::Light }, t);
    let ht = thickness(if left == W::Heavy || right == W::Heavy { W::Heavy } else { W::Light }, t);
    let has_v = up != W::None || down != W::None;
    let has_h = left != W::None || right != W::None;
    let v_double = is_d(up) || is_d(down);
    let h_double = is_d(left) || is_d(right);

    // ── horizontal arms ──
    for (arm, is_right) in [(left, false), (right, true)] {
        match arm {
            W::None => {}
            W::Double => {
                for (upper, y) in [(true, dh), (false, dh + 2 * t)] {
                    // Which vertical this line stops at.
                    let (near, far) = if upper { (up, down) } else { (down, up) };
                    let stop = if is_d(near) {
                        if is_right {
                            dv + 2 * t
                        } else {
                            dv + t
                        }
                    } else if is_d(far) {
                        if is_right {
                            dv
                        } else {
                            dv + 3 * t
                        }
                    } else if single(near) || single(far) {
                        let x = centred(w, vt);
                        if is_right {
                            x
                        } else {
                            x + vt
                        }
                    } else if is_right {
                        dv
                    } else {
                        dv + 3 * t
                    };
                    out.push(if is_right { rect(stop, y, w, y + t) } else { rect(0, y, stop, y + t) });
                }
            }
            wt => {
                let th = thickness(wt, t);
                let y = centred(h, th);
                let stop = if v_double {
                    if is_right {
                        dv + 2 * t
                    } else {
                        dv + t
                    }
                } else if has_v {
                    let x = centred(w, vt);
                    if is_right {
                        x
                    } else {
                        x + vt
                    }
                } else {
                    let x = centred(w, th);
                    if is_right {
                        x
                    } else {
                        x + th
                    }
                };
                out.push(if is_right { rect(stop, y, w, y + th) } else { rect(0, y, stop, y + th) });
            }
        }
    }

    // ── vertical arms ──
    for (arm, is_down) in [(up, false), (down, true)] {
        match arm {
            W::None => {}
            W::Double => {
                for (is_left_line, x) in [(true, dv), (false, dv + 2 * t)] {
                    let (near, far) = if is_left_line { (left, right) } else { (right, left) };
                    let stop = if is_d(near) {
                        if is_down {
                            dh + 2 * t
                        } else {
                            dh + t
                        }
                    } else if is_d(far) {
                        if is_down {
                            dh
                        } else {
                            dh + 3 * t
                        }
                    } else if single(near) || single(far) {
                        let y = centred(h, ht);
                        if is_down {
                            y
                        } else {
                            y + ht
                        }
                    } else if is_down {
                        dh
                    } else {
                        dh + 3 * t
                    };
                    out.push(if is_down { rect(x, stop, x + t, h) } else { rect(x, 0, x + t, stop) });
                }
            }
            wt => {
                let tv = thickness(wt, t);
                let x = centred(w, tv);
                let stop = if h_double {
                    if is_down {
                        dh + 2 * t
                    } else {
                        dh + t
                    }
                } else if has_h {
                    let y = centred(h, ht);
                    if is_down {
                        y
                    } else {
                        y + ht
                    }
                } else {
                    let y = centred(h, tv);
                    if is_down {
                        y
                    } else {
                        y + tv
                    }
                };
                out.push(if is_down { rect(x, stop, x + tv, h) } else { rect(x, 0, x + tv, stop) });
            }
        }
    }
    out
}

fn dashed(a: [W; 4], n: u32, w: i32, h: i32, t: i32) -> Vec<Prim> {
    let n = n as i32;
    let horizontal = a[2] != W::None;
    let th = thickness(if a.contains(&W::Heavy) { W::Heavy } else { W::Light }, t);
    let len = if horizontal { w } else { h };
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let s = i * len / n;
        let e = (i + 1) * len / n;
        // Dash takes ~60 % of its slot, centred, so neighbours keep a rhythm.
        let gap = ((e - s) * 2 / 5).max(1);
        let (s, e) = (s + gap / 2, e - (gap - gap / 2));
        if horizontal {
            let y = centred(h, th);
            out.push(rect(s, y, e, y + th));
        } else {
            let x = centred(w, th);
            out.push(rect(x, s, x + th, e));
        }
    }
    out
}

fn arc(c: u32, w: i32, h: i32, t: i32) -> Prim {
    let x = centred(w, t);
    let y = centred(h, t);
    let radius = (w.min(h) / 2).max(t);
    // The box is big enough that its other corners are outside the cell.
    let big = w.max(h) * 2;
    let (x0, y0, x1, y1, corner) = match c {
        0x256d => (x, y, x + big, y + big, Corner::TopLeft), // ╭ down and right
        0x256e => (x + t - big, y, x + t, y + big, Corner::TopRight), // ╮ down and left
        0x256f => (x + t - big, y + t - big, x + t, y + t, Corner::BottomRight), // ╯ up and left
        _ => (x, y + t - big, x + big, y + t, Corner::BottomLeft), // ╰ up and right
    };
    Prim::Arc { x0, y0, x1, y1, corner, thickness: t, radius }
}

fn block(c: u32, w: i32, h: i32) -> Vec<Prim> {
    let frac_h = |n: i32| h * n / 8;
    let frac_w = |n: i32| w * n / 8;
    let (hw, hh) = (w / 2, h / 2);
    let shade = |alpha: f32| vec![Prim::Rect { x0: 0, y0: 0, x1: w, y1: h, alpha }];
    // Quadrants: upper-left, upper-right, lower-left, lower-right.
    let quads = |ul: bool, ur: bool, ll: bool, lr: bool| {
        let mut v = Vec::new();
        if ul {
            v.push(rect(0, 0, hw, hh));
        }
        if ur {
            v.push(rect(hw, 0, w, hh));
        }
        if ll {
            v.push(rect(0, hh, hw, h));
        }
        if lr {
            v.push(rect(hw, hh, w, h));
        }
        v
    };
    match c {
        0x2580 => vec![rect(0, 0, w, hh)],
        0x2581..=0x2587 => vec![rect(0, h - frac_h((c - 0x2580) as i32), w, h)],
        0x2588 => vec![rect(0, 0, w, h)],
        0x2589..=0x258f => vec![rect(0, 0, frac_w(8 - (c - 0x2588) as i32), h)],
        0x2590 => vec![rect(hw, 0, w, h)],
        0x2591 => shade(0.25),
        0x2592 => shade(0.5),
        0x2593 => shade(0.75),
        0x2594 => vec![rect(0, 0, w, frac_h(1))],
        0x2595 => vec![rect(w - frac_w(1), 0, w, h)],
        0x2596 => quads(false, false, true, false),
        0x2597 => quads(false, false, false, true),
        0x2598 => quads(true, false, false, false),
        0x2599 => quads(true, false, true, true),
        0x259a => quads(true, false, false, true),
        0x259b => quads(true, true, true, false),
        0x259c => quads(true, true, false, true),
        0x259d => quads(false, true, false, false),
        0x259e => quads(false, true, true, false),
        _ => quads(false, true, true, true), // 0x259f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(ch: char) -> Vec<(i32, i32, i32, i32)> {
        prims(ch, 16, 34, 2)
            .unwrap()
            .into_iter()
            .map(|p| match p {
                Prim::Rect { x0, y0, x1, y1, .. } => (x0, y0, x1, y1),
                Prim::Arc { .. } => panic!("arc"),
            })
            .collect()
    }

    #[test]
    fn horizontal_and_vertical_lines_span_the_cell() {
        // ─: two arms that together cover the full width, centred vertically.
        let r = rects('─');
        assert_eq!(r.iter().map(|r| r.0).min(), Some(0));
        assert_eq!(r.iter().map(|r| r.2).max(), Some(16));
        assert!(r.iter().all(|r| r.1 == 16 && r.3 == 18));
        // │ covers the full height, so rows join with no gap whatever the line height.
        let r = rects('│');
        assert_eq!(r.iter().map(|r| r.1).min(), Some(0));
        assert_eq!(r.iter().map(|r| r.3).max(), Some(34));
        // ┼ joins: arms meet in the middle.
        let r = rects('┼');
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn corners_reach_their_edges_only() {
        let r = rects('┌');
        // Right arm reaches x = 16, down arm reaches y = 34; nothing at x = 0 or y = 0.
        assert!(r.iter().any(|r| r.2 == 16));
        assert!(r.iter().any(|r| r.3 == 34));
        assert!(r.iter().all(|r| r.0 > 0 && r.1 > 0));
    }

    #[test]
    fn double_lines_have_two_bands() {
        let r = rects('═');
        assert_eq!(r.len(), 4); // two arms × two lines
        let ys: std::collections::BTreeSet<_> = r.iter().map(|r| r.1).collect();
        assert_eq!(ys.len(), 2);
        // ╔: outer lines start at the outer edges, inner ones further in.
        assert_eq!(rects('╔').len(), 4);
    }

    #[test]
    fn blocks_and_arcs() {
        assert_eq!(rects('█'), vec![(0, 0, 16, 34)]);
        assert_eq!(rects('▀'), vec![(0, 0, 16, 17)]);
        assert_eq!(rects('▄'), vec![(0, 17, 16, 34)]);
        assert_eq!(rects('▌'), vec![(0, 0, 8, 34)]);
        match prims('╭', 16, 34, 2).unwrap()[0] {
            Prim::Arc { corner, x0, y0, .. } => {
                assert_eq!(corner, Corner::TopLeft);
                assert_eq!((x0, y0), (7, 16));
            }
            _ => panic!(),
        }
        assert!(is_drawn('╰'));
        assert!(!is_drawn('╱'));
        assert!(!is_drawn('a'));
    }
}
