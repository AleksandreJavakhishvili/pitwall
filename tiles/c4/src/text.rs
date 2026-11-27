//! Text widths from Inter's advance table (layout-lld.md §1).

mod table {
    include!(concat!(env!("OUT_DIR"), "/inter_widths.rs"));
}

/// The two weights the tile draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    SemiBold,
}

/// Advance of one character in font units.
fn advance(c: char, weight: Weight) -> u16 {
    let (rows, fallback) = match weight {
        Weight::Regular => (table::REGULAR, table::REGULAR_FALLBACK),
        Weight::SemiBold => (table::SEMIBOLD, table::SEMIBOLD_FALLBACK),
    };
    match rows.binary_search_by_key(&c, |r| r.0) {
        Ok(i) => rows[i].1,
        Err(_) => fallback,
    }
}

/// Kerning between two characters in font units.
fn kern(a: char, b: char, weight: Weight) -> i16 {
    let pairs = match weight {
        Weight::Regular => table::REGULAR_KERN,
        Weight::SemiBold => table::SEMIBOLD_KERN,
    };
    match pairs.binary_search_by_key(&(a, b), |p| (p.0, p.1)) {
        Ok(i) => pairs[i].2,
        Err(_) => 0,
    }
}

/// Width of `text` on one line at `px` pixels, kerned, without slack.
pub fn text_width(text: &str, px: f64, weight: Weight) -> f64 {
    let mut units: i64 = 0;
    let mut previous = None;
    for c in text.chars() {
        units += advance(c, weight) as i64;
        if let Some(p) = previous {
            units += kern(p, c, weight) as i64;
        }
        previous = Some(c);
    }
    units.max(0) as f64 * px / table::UNITS_PER_EM as f64
}

/// Wraps `text` at spaces into at most `max_lines` lines no wider than
/// `max_width`. A word wider than the line is cut; the last line ends in
/// `…` when text is left over.
pub fn wrap(text: &str, px: f64, weight: Weight, max_width: f64, max_lines: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() && lines.len() < max_lines {
        let mut line = words[i].to_string();
        i += 1;
        while i < words.len() {
            let candidate = format!("{line} {}", words[i]);
            if text_width(&candidate, px, weight) > max_width {
                break;
            }
            line = candidate;
            i += 1;
        }
        lines.push(line);
    }
    let truncated = i < words.len();
    let count = lines.len();
    for (n, line) in lines.iter_mut().enumerate() {
        if text_width(line, px, weight) > max_width || (truncated && n + 1 == count) {
            *line = ellipsize(line, px, weight, max_width);
        }
    }
    lines
}

/// Cuts `line` so that it plus `…` fits `max_width`.
fn ellipsize(line: &str, px: f64, weight: Weight, max_width: f64) -> String {
    let mut out: String = line.to_string();
    loop {
        let candidate = format!("{}…", out.trim_end());
        if text_width(&candidate, px, weight) <= max_width || out.is_empty() {
            return candidate;
        }
        out.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_scale_with_size() {
        let a = text_width("Scanner", 12.0, Weight::Regular);
        let b = text_width("Scanner", 24.0, Weight::Regular);
        assert!((b - 2.0 * a).abs() < 1e-9);
        assert!(text_width("Scanner", 12.0, Weight::SemiBold) > a);
    }

    #[test]
    fn wrap_fits_and_cuts() {
        let lines = wrap("finds Mermaid blocks in Markdown files", 12.0, Weight::Regular, 100.0, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with('…'));
        for l in &lines {
            assert!(text_width(l, 12.0, Weight::Regular) <= 100.0, "{l}");
        }
        assert_eq!(wrap("short", 12.0, Weight::Regular, 100.0, 2), ["short"]);
        assert!(wrap("", 12.0, Weight::Regular, 100.0, 2).is_empty());
    }
}
