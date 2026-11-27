//! The Changes view model (FR-17) on the before/after fixture.

mod common;

use c4_tile::c4::Structure;
use c4_tile::changes::{compare, Change, ChangeView};

fn pair() -> (Structure, Structure) {
    let index = common::index_of(&["shop/docs/orders/cache-lld.md"]);
    let s = |id: &str| index.get(id).unwrap().structure.clone().unwrap();
    (s("shop/docs/orders/cache-lld.md#before"), s("shop/docs/orders/cache-lld.md#after"))
}

#[test]
fn boxes_added_changed_kept_and_removed() {
    let (before, after) = pair();
    let view = compare(&before, &after);
    let named: Vec<(&str, Change)> =
        after.boxes.iter().zip(&view.boxes).map(|(b, c)| (b.name.as_str(), *c)).collect();
    assert_eq!(named, [
        ("Web app", Change::Kept),
        // `changed` class and a new purpose.
        ("Orders API", Change::Changed),
        ("Price cache", Change::Added),
        ("Orders DB", Change::Kept),
    ]);
    let removed: Vec<&str> =
        view.removed_boxes.iter().map(|&i| before.boxes[i].name.as_str()).collect();
    assert_eq!(removed, ["Reports"]);
}

#[test]
fn arrows_added_changed_kept_and_removed() {
    let (before, after) = pair();
    let view = compare(&before, &after);
    let label = |s: &Structure, i: usize| {
        let a = &s.arrows[i];
        let label = a.label.as_deref().unwrap_or("");
        format!("{} -> {}: {label}", s.end_name(a.from), s.end_name(a.to))
    };
    let got: Vec<(String, Change)> =
        (0..after.arrows.len()).map(|i| (label(&after, i), view.arrows[i])).collect();
    assert_eq!(got, [
        ("Web app -> Orders API: JSON over HTTPS".to_string(), Change::Kept),
        ("Orders API -> Price cache: reads prices".to_string(), Change::Added),
        ("Price cache -> Orders DB: fills from".to_string(), Change::Added),
        // Same ends and label; now dotted.
        ("Orders API -> Orders DB: reads prices".to_string(), Change::Changed),
    ]);
    let removed: Vec<String> = view.removed_arrows.iter().map(|&i| label(&before, i)).collect();
    assert_eq!(removed, ["Reports -> Orders DB: reads"]);
}

#[test]
fn a_diagram_against_itself_is_all_kept() {
    let (before, _) = pair();
    let view = compare(&before, &before);
    assert_eq!(view, ChangeView {
        boxes: vec![Change::Kept; before.boxes.len()],
        arrows: vec![Change::Kept; before.arrows.len()],
        removed_boxes: vec![],
        removed_arrows: vec![],
    });
}

#[test]
fn boxes_match_by_name_without_kind_or_case() {
    let index = common::index_of(&["shop/docs/orders/cache-lld.md"]);
    let s = |id: &str| index.get(id).unwrap().structure.clone().unwrap();
    let current = s("shop/docs/orders/cache-lld.md#1-current");
    let proposed = s("shop/docs/orders/cache-lld.md#2-proposed-a-replica");
    let view = compare(&current, &proposed);
    assert_eq!(view.boxes, [Change::Kept, Change::Added]);
    assert_eq!(view.arrows, [Change::Added]);
}
