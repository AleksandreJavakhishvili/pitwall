//! How the viewer lays out a PDF (macOS: `crate::platform::pdf`): its
//! pages one under the other on the pane's ground, as white paper, fitted
//! to the pane's width or at a zoom; only the pages in view (and a page or
//! so around them) are drawn, at the zoom and the display's scale, and the
//! drawn pages kept stay under a memory budget. Free of GPUI, so it can be
//! tested.

use std::ops::Range;

use super::image_view::Zoom;

/// Room around the pages.
pub const PAD: f32 = 16.;
/// Room between two pages.
pub const GAP: f32 = 12.;
/// The zoom steps − and + walk through.
pub const STEPS: [f32; 13] = [0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1., 1.1, 1.25, 1.5, 2., 3., 4.];
/// Pages drawn beyond the view, above and below (in points of the pane).
pub const MARGIN: f32 = 600.;
/// The drawn pages one document keeps, in bytes (about 30 A4 pages on a
/// Retina display at 100 %).
pub const BUDGET: usize = 192 * 1024 * 1024;
/// A drawn page is never larger than this many pixels (64 MiB): beyond it
/// a page is drawn smaller and scaled up.
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;
/// …nor longer than this a side (GPUI's textures stop at 16 384).
pub const MAX_SIDE: u32 = 8192;
/// At most this many pages are being drawn at a time.
pub const IN_FLIGHT: usize = 3;

/// The scale that fits the widest page in `area_w` points.
pub fn fit_width(pages: &[(f32, f32)], area_w: f32) -> f32 {
    let widest = pages.iter().map(|p| p.0).fold(0., f32::max);
    if widest <= 0. || area_w <= 0. {
        return 1.;
    }
    ((area_w - 2. * PAD) / widest).clamp(STEPS[0], STEPS[STEPS.len() - 1])
}

/// The scale `zoom` shows the pages at.
pub fn scale(zoom: Zoom, pages: &[(f32, f32)], area_w: f32) -> f32 {
    match zoom {
        Zoom::Fit => fit_width(pages, area_w),
        Zoom::Scale(s) => s,
    }
}

/// The next step up from `current` (the last one stays).
pub fn zoom_in(current: f32) -> f32 {
    STEPS.iter().copied().find(|s| *s > current + 1e-3).unwrap_or(STEPS[STEPS.len() - 1])
}

/// The next step down from `current` (the first one stays).
pub fn zoom_out(current: f32) -> f32 {
    STEPS.iter().rev().copied().find(|s| *s < current - 1e-3).unwrap_or(STEPS[0])
}

/// Where each page is at a scale (points of the pane).
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Each page's top.
    pub tops: Vec<f32>,
    /// Each page's size at the scale.
    pub sizes: Vec<(f32, f32)>,
    /// All of it, with the room around.
    pub width: f32,
    pub height: f32,
}

impl Layout {
    pub fn new(pages: &[(f32, f32)], s: f32) -> Layout {
        let mut tops = Vec::with_capacity(pages.len());
        let mut sizes = Vec::with_capacity(pages.len());
        let mut y = PAD;
        let mut widest: f32 = 0.;
        for (w, h) in pages {
            let (w, h) = (w * s, h * s);
            tops.push(y);
            sizes.push((w, h));
            y += h + GAP;
            widest = widest.max(w);
        }
        let height = if pages.is_empty() { 2. * PAD } else { y - GAP + PAD };
        Layout { tops, sizes, width: widest + 2. * PAD, height }
    }

    pub fn len(&self) -> usize {
        self.tops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tops.is_empty()
    }

    /// Page `i`'s left in a pane `area_w` wide (centred).
    pub fn left(&self, i: usize, area_w: f32) -> f32 {
        let w = self.width.max(area_w);
        ((w - self.sizes[i].0) / 2.).max(PAD)
    }

    /// The pages any of `top`..`top + view_h` (and `margin` around) shows.
    pub fn visible(&self, top: f32, view_h: f32, margin: f32) -> Range<usize> {
        let (from, to) = (top - margin, top + view_h + margin);
        // The first page whose bottom is below `from`, the first whose top
        // is past `to`.
        let first = self.tops.iter().zip(&self.sizes).position(|(t, s)| t + s.1 > from).unwrap_or(self.len());
        let end = first + self.tops[first..].iter().take_while(|t| **t < to).count();
        first..end.max(first)
    }

    /// The page being read: the one under the line a third down the view
    /// (or the last one above it, in a gap).
    pub fn current(&self, top: f32, view_h: f32) -> usize {
        let y = top + view_h / 3.;
        self.tops.iter().rposition(|t| *t <= y).unwrap_or(0)
    }

    /// What `top` shows at its top: a page and how far into it (0..1, or
    /// past 1 in the gap below it).
    pub fn anchor(&self, top: f32) -> (usize, f32) {
        let i = self.tops.iter().rposition(|t| *t <= top).unwrap_or(0);
        let Some(h) = self.sizes.get(i).map(|s| s.1) else { return (0, 0.) };
        (i, if h > 0. { (top - self.tops[i]) / h } else { 0. })
    }

    /// The top that shows `anchor` at the top (the same place, at another
    /// scale).
    pub fn top_of(&self, anchor: (usize, f32)) -> f32 {
        match (self.tops.get(anchor.0), self.sizes.get(anchor.0)) {
            (Some(t), Some(s)) => (t + anchor.1 * s.1).max(0.),
            _ => 0.,
        }
    }

    /// The top that shows `y` of page `i` (points of the page at the
    /// scale) a third down a view `view_h` high.
    pub fn top_showing(&self, i: usize, y: f32, view_h: f32) -> f32 {
        self.tops.get(i).map_or(0., |t| (t + y - view_h / 3.).max(0.))
    }
}

/// The pixels to draw a page `page` (points) at scale `s` on a display of
/// `dpr` pixels a point: never more than [`MAX_PIXELS`] or [`MAX_SIDE`] a
/// side (then smaller, and scaled up when shown).
pub fn render_px(page: (f32, f32), s: f32, dpr: f32) -> (u32, u32) {
    let (mut w, mut h) = ((page.0 * s * dpr).max(1.), (page.1 * s * dpr).max(1.));
    let over = ((w as f64 * h as f64) / MAX_PIXELS as f64).sqrt().max(w.max(h) as f64 / MAX_SIDE as f64);
    if over > 1. {
        w /= over as f32;
        h /= over as f32;
    }
    (w.round().max(1.) as u32, h.round().max(1.) as u32)
}

/// A page drawn `have` pixels wide needs drawing again for `want`: it
/// would look blurred (under 90 %) or wastes memory (over 160 %).
pub fn needs_render(have: Option<u32>, want: u32) -> bool {
    match have {
        None => true,
        Some(h) => (h as f32) < want as f32 * 0.9 || (h as f32) > want as f32 * 1.6,
    }
}

/// A page drawn `have` pixels wide is drawn (again) for `want` now: not
/// while a pinch or ⌘-scroll is going on (`zooming`), when the pages drawn
/// are scaled as they are and drawn again once it settles; a page not
/// drawn yet is drawn at once.
pub fn render_now(have: Option<u32>, want: u32, zooming: bool) -> bool {
    match have {
        None => true,
        Some(_) if zooming => false,
        _ => needs_render(have, want),
    }
}

/// The top of the view after the pages go from `old`'s scale to `new`'s
/// that keeps the place at `at` points down a view `view_h` high (shown
/// from `top`) where it is, as far as the pages can be scrolled.
pub fn anchored_top(old: &Layout, new: &Layout, top: f32, at: f32, view_h: f32) -> f32 {
    let y = new.top_of(old.anchor(top + at));
    (y - at).clamp(0., (new.height - view_h).max(0.))
}

/// The widest page's width at scale 1 (what a pinch moves horizontally).
pub fn widest(pages: &[(f32, f32)]) -> f32 {
    pages.iter().map(|p| p.0).fold(0., f32::max)
}

/// Which drawn pages to let go (`kept`: page and bytes) to stay under
/// `budget`: the farthest from `keep` first, never those in it.
pub fn evict(kept: &[(usize, usize)], keep: Range<usize>, budget: usize) -> Vec<usize> {
    let mut total: usize = kept.iter().map(|k| k.1).sum();
    if total <= budget {
        return Vec::new();
    }
    let mid = (keep.start + keep.end) as f32 / 2.;
    let mut far: Vec<(usize, usize)> = kept.iter().copied().filter(|(p, _)| !keep.contains(p)).collect();
    far.sort_by(|a, b| (b.0 as f32 - mid).abs().total_cmp(&(a.0 as f32 - mid).abs()));
    let mut out = Vec::new();
    for (p, bytes) in far {
        if total <= budget {
            break;
        }
        total -= bytes;
        out.push(p);
    }
    out
}

/// The pages to draw next, nearest the page being read first.
pub fn wanted(range: Range<usize>, current: usize) -> Vec<usize> {
    let mut v: Vec<usize> = range.collect();
    v.sort_by_key(|p| p.abs_diff(current));
    v
}

/// The hit after (`forward`) or before `current` of `n`, round the ends.
pub fn step(current: Option<usize>, n: usize, forward: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(match (current, forward) {
        (None, true) => 0,
        (None, false) => n - 1,
        (Some(c), true) => (c + 1) % n,
        (Some(c), false) => (c + n - 1) % n,
    })
}

/// The first hit on page `page` or after it (else the first): where a new
/// search starts.
pub fn first_from(pages: &[usize], page: usize) -> Option<usize> {
    if pages.is_empty() {
        return None;
    }
    Some(pages.iter().position(|p| *p >= page).unwrap_or(0))
}

/// "Page 3 of 120".
pub fn page_label(current: usize, n: usize) -> String {
    format!("Page {} of {n}", (current + 1).min(n.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A4: (f32, f32) = (595., 842.);

    #[test]
    fn fit_width_fills_the_pane_within_the_steps() {
        // 595 pt pages in a 627 pt pane (16 pt each side): 100 %.
        assert!((fit_width(&[A4, A4], 627.) - 1.).abs() < 1e-6);
        // The widest page decides.
        assert!((fit_width(&[A4, (1190., 842.)], 627.) - 0.5).abs() < 1e-6);
        assert_eq!(fit_width(&[A4], 50.), STEPS[0], "never below the first step");
        assert_eq!(fit_width(&[A4], 10_000.), 4., "nor above the last");
        assert_eq!(fit_width(&[], 600.), 1.);
        assert_eq!(scale(Zoom::Scale(2.), &[A4], 600.), 2.);
    }

    #[test]
    fn zoom_walks_the_steps() {
        assert_eq!(zoom_in(1.), 1.1);
        assert_eq!(zoom_out(1.), 0.9);
        // From a fitted 0.8: the steps around it.
        assert_eq!(zoom_in(0.8), 0.9);
        assert_eq!(zoom_out(0.8), 0.75);
        assert_eq!(zoom_in(4.), 4.);
        assert_eq!(zoom_out(0.25), 0.25);
        assert_eq!(zoom_out(0.1), 0.25);
    }

    #[test]
    fn pages_stack_with_room_between() {
        let l = Layout::new(&[(100., 200.), (300., 100.), (100., 200.)], 2.);
        assert_eq!(l.tops, vec![PAD, PAD + 400. + GAP, PAD + 400. + GAP + 200. + GAP]);
        assert_eq!(l.sizes[1], (600., 200.));
        assert_eq!(l.width, 600. + 2. * PAD);
        assert_eq!(l.height, PAD + 400. + GAP + 200. + GAP + 400. + PAD);
        // Centred in a wider pane; at the room's edge in a narrower one.
        assert_eq!(l.left(0, 1000.), 400.);
        assert_eq!(l.left(1, 100.), PAD);
        assert_eq!(Layout::new(&[], 1.).height, 2. * PAD);
    }

    #[test]
    fn only_pages_in_view_are_drawn() {
        // 100 pages 100 pt high: each starts every 112 pt from 16.
        let l = Layout::new(&vec![(80., 100.); 100], 1.);
        assert_eq!(l.visible(0., 300., 0.), 0..3);
        assert_eq!(l.visible(1000., 300., 0.), 8..12);
        // With a margin: one page more each side here.
        assert_eq!(l.visible(1000., 300., 112.), 7..13);
        // Past the end, or a view in a gap.
        assert_eq!(l.visible(20_000., 300., 0.), 100..100);
        assert_eq!(l.visible(117., 10., 0.), 1..1, "between pages 1 and 2");
        assert_eq!(wanted(7..13, 9), vec![9, 8, 10, 7, 11, 12]);
    }

    #[test]
    fn the_page_being_read_and_zoom_keeps_the_place() {
        let l = Layout::new(&[(80., 100.); 10], 1.);
        assert_eq!(l.current(0., 300.), 0);
        assert_eq!(l.current(300., 300.), 3, "a third down the view: 400");
        assert_eq!(page_label(l.current(300., 300.), l.len()), "Page 4 of 10");
        let at = l.anchor(16. + 112. * 4. + 50.);
        assert_eq!(at, (4, 0.5));
        // Twice the size: half way down page 5 still.
        let big = Layout::new(&[(80., 100.); 10], 2.);
        assert_eq!(big.top_of(at), 16. + 212. * 4. + 100.);
        assert_eq!(big.top_of((99, 0.)), 0.);
        assert_eq!(l.top_showing(2, 30., 300.), 16. + 224. + 30. - 100.);
        assert_eq!(l.top_showing(0, 0., 300.), 0.);
        assert_eq!(page_label(0, 0), "Page 1 of 0");
    }

    #[test]
    fn drawn_pages_stay_in_bounds() {
        // A4 at 100 % on a Retina display.
        assert_eq!(render_px(A4, 1., 2.), (1190, 1684));
        // At 400 %: capped by pixels, the shape kept.
        let (w, h) = render_px(A4, 4., 2.);
        assert!((w as u64 * h as u64) <= MAX_PIXELS + 8192, "{w}×{h}");
        assert!(((w as f32 / h as f32) - 595. / 842.).abs() < 0.01);
        // A very long strip: capped by its side.
        let (w, h) = render_px((100., 20_000.), 1., 2.);
        assert!(h <= MAX_SIDE && w >= 1, "{w}×{h}");
        assert_eq!(render_px((0., 0.), 1., 2.), (1, 1));
    }

    #[test]
    fn redrawn_only_when_it_shows() {
        assert!(needs_render(None, 1000));
        assert!(!needs_render(Some(1000), 1000));
        assert!(!needs_render(Some(950), 1000), "a little smaller is fine (resizing)");
        assert!(needs_render(Some(800), 1000), "blurred");
        assert!(!needs_render(Some(1500), 1000));
        assert!(needs_render(Some(1700), 1000), "too large");
    }

    #[test]
    fn while_zooming_pages_are_scaled_and_drawn_again_once_it_settles() {
        // Mid-pinch at twice the size: the drawn page is scaled, not drawn.
        assert!(!render_now(Some(1000), 2000, true));
        // A page not drawn yet is drawn even then (blank paper otherwise).
        assert!(render_now(None, 2000, true));
        // Settled: drawn again at the new scale…
        assert!(render_now(Some(1000), 2000, false));
        // …unless it still looks right.
        assert!(!render_now(Some(1000), 1050, false));
    }

    #[test]
    fn zooming_a_pdf_keeps_the_place_under_the_pointer() {
        let pages = [(400., 600.); 10];
        let (old, new) = (Layout::new(&pages, 1.), Layout::new(&pages, 2.));
        // Scrolled to 2000, the pointer 300 down: 2300 is page 3 (612 a
        // page from 16), 464 into it.
        let top = anchored_top(&old, &new, 2000., 300., 700.);
        let (page, into) = old.anchor(2300.);
        assert_eq!(page, 3);
        let y = top + 300.;
        assert!((new.tops[3] + into * new.sizes[3].1 - y).abs() < 1e-3, "{top}");
        assert!((y - new.tops[3] - 2. * (2300. - old.tops[3])).abs() < 1e-3, "twice as far into the page");
        // In the room above the first page, zoomed in: it stops at the
        // top; at the end: at the end.
        assert_eq!(anchored_top(&old, &new, 0., 5., 700.), 0.);
        assert_eq!(anchored_top(&old, &new, old.height - 700., 699., 700.), new.height - 700.);
        // Shorter than the view: from the top.
        let one = Layout::new(&[(400., 600.)], 0.5);
        assert_eq!(anchored_top(&Layout::new(&[(400., 600.)], 1.), &one, 0., 300., 700.), 0.);
        assert_eq!(widest(&[(100., 5.), (300., 1.)]), 300.);
    }

    #[test]
    fn eviction_lets_far_pages_go_first() {
        let kept: Vec<(usize, usize)> = (0..10).map(|p| (p, 10)).collect();
        assert!(evict(&kept, 4..6, 100).is_empty(), "under the budget");
        assert_eq!(evict(&kept, 4..6, 70), vec![0, 1, 9]);
        // Never what is in view, even over the budget.
        let out = evict(&kept, 0..10, 10);
        assert!(out.is_empty());
    }

    #[test]
    fn hits_go_round() {
        assert_eq!(step(None, 3, true), Some(0));
        assert_eq!(step(None, 3, false), Some(2));
        assert_eq!(step(Some(2), 3, true), Some(0));
        assert_eq!(step(Some(0), 3, false), Some(2));
        assert_eq!(step(Some(0), 0, true), None);
        assert_eq!(first_from(&[0, 2, 2, 5], 2), Some(1));
        assert_eq!(first_from(&[0, 2, 2, 5], 6), Some(0), "none after: the first");
        assert_eq!(first_from(&[], 0), None);
    }
}
