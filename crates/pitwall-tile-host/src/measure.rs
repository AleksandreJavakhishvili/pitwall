//! `ui.measure-text`: how wide a tile's text is in Pitwall's fonts, so a
//! canvas can size boxes around labels (C4 tile gap G2). The widths come
//! from tables build.rs makes from the app's own font files (Inter and
//! JetBrains Mono, each weight Pitwall draws), shaped with kerning as
//! Pitwall draws text, so the host needs no GPUI and the test harness
//! gives the same numbers.

use crate::ui::{Font, Weight};

mod table {
    include!(concat!(env!("OUT_DIR"), "/text_widths.rs"));
}

/// At most this many texts a call.
pub const MAX_TEXTS: usize = 1000;
/// At most this many bytes of text a call.
pub const MAX_BYTES: usize = 64 * 1024;
/// The sizes measured, in logical pixels (others are clamped).
pub const SIZES: std::ops::RangeInclusive<f32> = 4.0..=200.0;

struct Table {
    advances: &'static [(char, u16)],
    fallback: u16,
    kern: &'static [(char, char, i16)],
    upem: u16,
}

macro_rules! t {
    ($name:ident, $f:ident, $k:ident, $u:ident) => {
        Table { advances: table::$name, fallback: table::$f, kern: table::$k, upem: table::$u }
    };
}

/// The font file Pitwall draws `font` at `weight` with.
fn table_of(font: Font, weight: Weight) -> Table {
    match (font, weight) {
        (Font::Ui, Weight::Normal) => t!(INTER_REGULAR, INTER_REGULAR_FALLBACK, INTER_REGULAR_KERN, INTER_REGULAR_UPEM),
        (Font::Ui, Weight::Medium) => t!(INTER_MEDIUM, INTER_MEDIUM_FALLBACK, INTER_MEDIUM_KERN, INTER_MEDIUM_UPEM),
        (Font::Ui, Weight::Semibold) => t!(INTER_SEMIBOLD, INTER_SEMIBOLD_FALLBACK, INTER_SEMIBOLD_KERN, INTER_SEMIBOLD_UPEM),
        (Font::Ui, Weight::Bold) => t!(INTER_BOLD, INTER_BOLD_FALLBACK, INTER_BOLD_KERN, INTER_BOLD_UPEM),
        // JetBrains Mono has no Medium: Pitwall draws it Regular.
        (Font::Mono, Weight::Normal | Weight::Medium) => t!(MONO_REGULAR, MONO_REGULAR_FALLBACK, MONO_REGULAR_KERN, MONO_REGULAR_UPEM),
        (Font::Mono, Weight::Semibold) => t!(MONO_SEMIBOLD, MONO_SEMIBOLD_FALLBACK, MONO_SEMIBOLD_KERN, MONO_SEMIBOLD_UPEM),
        (Font::Mono, Weight::Bold) => t!(MONO_BOLD, MONO_BOLD_FALLBACK, MONO_BOLD_KERN, MONO_BOLD_UPEM),
    }
}

impl Table {
    fn advance(&self, c: char) -> u16 {
        match self.advances.binary_search_by_key(&c, |r| r.0) {
            Ok(i) => self.advances[i].1,
            Err(_) => self.fallback,
        }
    }

    fn kern(&self, a: char, b: char) -> i16 {
        match self.kern.binary_search_by_key(&(a, b), |p| (p.0, p.1)) {
            Ok(i) => self.kern[i].2,
            Err(_) => 0,
        }
    }

    /// `text`'s width on one line, in font units.
    fn units(&self, text: &str) -> i64 {
        let mut units: i64 = 0;
        let mut previous = None;
        for c in text.chars().filter(|c| *c != '\n' && *c != '\r') {
            units += self.advance(c) as i64;
            if let Some(p) = previous {
                units += self.kern(p, c) as i64;
            }
            previous = Some(c);
        }
        units.max(0)
    }
}

/// How wide `text` is on one line in `font` at `size` logical pixels and
/// `weight`, in logical pixels.
pub fn width(text: &str, font: Font, size: f32, weight: Weight) -> f32 {
    let size = if size.is_finite() { size.clamp(*SIZES.start(), *SIZES.end()) } else { 13.0 };
    let t = table_of(font, weight);
    t.units(text) as f32 * size / t.upem as f32
}

/// The widths of `texts`, or why there are none (too many, too large).
pub fn widths(texts: &[String], font: Font, size: f32, weight: Weight) -> Result<Vec<f32>, String> {
    if texts.len() > MAX_TEXTS {
        return Err(format!("measure-text: at most {MAX_TEXTS} texts a call, not {}", texts.len()));
    }
    let bytes: usize = texts.iter().map(String::len).sum();
    if bytes > MAX_BYTES {
        return Err(format!("measure-text: at most {} KiB of text a call, not {} bytes", MAX_BYTES / 1024, bytes));
    }
    Ok(texts.iter().map(|t| width(t, font, size, weight)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inter_is_kerned_and_wider_when_bolder() {
        let regular = width("Payments API", Font::Ui, 13.0, Weight::Normal);
        let bold = width("Payments API", Font::Ui, 13.0, Weight::Bold);
        assert!(regular > 60.0 && regular < 100.0, "{regular}");
        assert!(bold > regular, "{bold} > {regular}");
        // "AV" is kerned tighter than its letters apart.
        let pair = width("AV", Font::Ui, 100.0, Weight::Normal);
        let apart = width("A", Font::Ui, 100.0, Weight::Normal) + width("V", Font::Ui, 100.0, Weight::Normal);
        assert!(pair < apart, "{pair} < {apart}");
        // Twice the size, twice the width.
        assert!((width("Payments API", Font::Ui, 26.0, Weight::Normal) - 2.0 * regular).abs() < 0.01);
    }

    #[test]
    fn mono_is_one_width_a_character() {
        let one = width("m", Font::Mono, 12.0, Weight::Normal);
        assert!((one - 7.2).abs() < 0.01, "{one}");
        assert!((width("i->j", Font::Mono, 12.0, Weight::Normal) - 4.0 * one).abs() < 0.01);
        assert_eq!(width("abc", Font::Mono, 12.0, Weight::Medium), width("abc", Font::Mono, 12.0, Weight::Normal));
    }

    #[test]
    fn unknown_characters_count_as_an_average_letter_and_limits_hold() {
        let x = width("字", Font::Ui, 13.0, Weight::Normal);
        assert!(x > 4.0 && x < 10.0, "{x}");
        assert_eq!(width("", Font::Ui, 13.0, Weight::Normal), 0.0);
        assert!(widths(&vec![String::new(); MAX_TEXTS + 1], Font::Ui, 13.0, Weight::Normal).is_err());
        assert!(widths(&[String::from_utf8(vec![b'a'; MAX_BYTES + 1]).unwrap()], Font::Ui, 13.0, Weight::Normal).is_err());
        assert_eq!(widths(&["a".into(), "ab".into()], Font::Ui, 13.0, Weight::Normal).unwrap().len(), 2);
    }
}
