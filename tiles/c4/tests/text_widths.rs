//! The Inter advance-width table against the font itself, shaped with
//! rustybuzz (kerning included), for 100 sample labels: within 1 %
//! (layout-lld.md §1, plan.md phase A).

mod common;

use c4_tile::text::{text_width, Weight};

fn shaped_width(face: &rustybuzz::Face, text: &str, px: f64) -> f64 {
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    let glyphs = rustybuzz::shape(face, &[], buffer);
    let units: i64 = glyphs.glyph_positions().iter().map(|p| p.x_advance as i64).sum();
    units as f64 * px / face.units_per_em() as f64
}

/// 100 labels: the fixtures' Names, Kinds, purposes and arrow labels, then
/// hand-written ones with the punctuation labels use. Labels with a
/// character Pitwall's Inter lacks (arrows, `⌘`: the host falls back to
/// another font for those) can't be checked against Inter and are left out.
fn samples(face: &rustybuzz::Face) -> Vec<String> {
    let paths = common::all_fixtures();
    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
    let index = common::index_of(&paths);
    let mut out: Vec<String> = Vec::new();
    for e in index.entries() {
        let Some(s) = &e.structure else { continue };
        for b in &s.boxes {
            out.push(b.name.clone());
            out.extend(b.kind.as_ref().map(|k| format!("[{}]", k.text)));
            out.extend(b.purpose.clone());
        }
        out.extend(s.arrows.iter().filter_map(|a| a.label.clone()));
    }
    out.extend(
        [
            "Engineer’s computer — “quoted”",
            "files.read, files.watch; v2 files.write",
            "Before › After › Changes",
            "Café • naïve — Ångström…",
            "WAVE AVATAR Typography",
            "To: Yvonne, “Tea” at 7 o’clock",
        ]
        .map(String::from),
    );
    out.sort();
    out.dedup();
    out.retain(|s| s.chars().count() >= 3 && s.chars().all(|c| face.glyph_index(c).is_some()));
    assert!(out.len() >= 100, "only {} samples", out.len());
    out.truncate(100);
    out
}

#[test]
fn table_matches_the_font_within_one_percent() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
    let fonts = [("Inter-Regular.ttf", Weight::Regular), ("Inter-SemiBold.ttf", Weight::SemiBold)];
    for (file, weight) in fonts {
        let data = std::fs::read(dir.join(file)).unwrap();
        let face = rustybuzz::Face::from_slice(&data, 0).unwrap();
        let mut worst = (0.0, String::new());
        for label in samples(&face) {
            let shaped = shaped_width(&face, &label, 14.0);
            let table = text_width(&label, 14.0, weight);
            let error = (table - shaped).abs() / shaped;
            if error > worst.0 {
                worst = (error, label.clone());
            }
        }
        eprintln!("{file}: worst {:.3} % for {:?}", worst.0 * 100.0, worst.1);
        assert!(worst.0 <= 0.01,"{file}: {:.2} % off for {:?}", worst.0 * 100.0, worst.1);
    }
}

/// Gap G2: Pitwall now measures text for tiles (`ui.measure-text`), from
/// the same font files with the same shaping. The tile keeps its own table
/// (layout-lld.md §1: its core lays out without the SDK, and a layout
/// measures thousands of strings, more than a tile's host calls allow);
/// this pins the two together, so a font Pitwall changes fails here.
#[test]
fn table_is_what_pitwall_measures() {
    use pitwall_extension_test::{text_width as pitwall, Font, Weight as W};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
    let data = std::fs::read(dir.join("Inter-Regular.ttf")).unwrap();
    let face = rustybuzz::Face::from_slice(&data, 0).unwrap();
    let mut worst = (0.0f64, String::new());
    for label in samples(&face) {
        for (ours, theirs, px) in [(Weight::Regular, W::Normal, 11.0), (Weight::SemiBold, W::Semibold, 14.0)] {
            let table = text_width(&label, px, ours);
            let measured = f64::from(pitwall(&label, Font::Ui, px as f32, theirs));
            let error = (table - measured).abs() / measured.max(1.0);
            if error > worst.0 {
                worst = (error, label.clone());
            }
        }
    }
    eprintln!("table against Pitwall's measure: worst {:.4} % for {:?}", worst.0 * 100.0, worst.1);
    assert!(worst.0 < 1e-4, "{:.3} % off for {:?}", worst.0 * 100.0, worst.1);
}
