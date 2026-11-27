//! The block finder (plan.md phase A; mermaid-lld.md §1).

mod common;

use c4_tile::blocks::{find_blocks, slug, Block};

fn summary(blocks: &[Block]) -> Vec<(&str, u32, u32)> {
    blocks.iter().map(|b| (b.id.as_str(), b.fence_line, b.end_line)).collect()
}

#[test]
fn finds_every_mermaid_fence_with_lines_and_ids() {
    let blocks = find_blocks("docs/fences.md", &common::fixture("markdown/fences.md"));
    assert_eq!(summary(&blocks), [
        ("docs/fences.md#top", 1, 4),
        ("docs/fences.md#tildes", 16, 19),
        ("docs/fences.md#in-a-list", 25, 28),
        ("docs/fences.md#in-a-list-2", 32, 35),
        ("docs/fences.md#repeats", 39, 42),
        ("docs/fences.md#repeats-2", 44, 47),
        ("docs/fences.md#repeats-3", 49, 52),
        ("docs/fences.md#repeats-4", 56, 59),
        ("docs/fences.md#setext-heading", 64, 67),
    ]);
}

#[test]
fn tilde_fences_keep_their_text() {
    let blocks = find_blocks("f.md", &common::fixture("markdown/fences.md"));
    assert_eq!(blocks[1].text, "flowchart TB\n  x[\"X [Container]\"] --> y\n");
    assert_eq!(blocks[1].file_position(2, 3), (18, 3));
}

#[test]
fn indented_fences_lose_the_indent_and_map_back_to_the_file() {
    let blocks = find_blocks("f.md", &common::fixture("markdown/fences.md"));
    assert_eq!(blocks[2].text, "flowchart LR\n  one --> two\n");
    // "     one --> two" in the file: 'o' at column 6.
    assert_eq!(blocks[2].file_position(2, 3), (27, 6));
    assert_eq!(blocks[3].text, "flowchart LR\n  three --> four\n");
    assert_eq!(blocks[3].file_position(1, 1), (33, 7));
    assert_eq!(blocks[3].file_position(2, 3), (34, 9));
}

#[test]
fn heading_paths_are_outermost_first() {
    let blocks = find_blocks("f.md", &common::fixture("markdown/fences.md"));
    assert!(blocks[0].headings.is_empty());
    let path = |b: &Block| -> Vec<(u8, String, u32)> {
        b.headings.iter().map(|h| (h.level, h.text.clone(), h.line)).collect()
    };
    assert_eq!(path(&blocks[1]), [(1, "Fences".into(), 6), (2, "Tildes".into(), 14)]);
    assert_eq!(path(&blocks[7]), [(1, "Fences".into(), 6), (2, "Repeats".into(), 54)]);
    assert_eq!(path(&blocks[8]), [(1, "Fences".into(), 6), (2, "Setext heading".into(), 61)]);
}

#[test]
fn crlf_files_give_the_same_blocks() {
    let lf = common::fixture("markdown/fences.md");
    let crlf = lf.replace('\n', "\r\n");
    let a = find_blocks("f.md", &lf);
    let b = find_blocks("f.md", &crlf);
    assert_eq!(a, b);
    assert!(b.iter().all(|block| !block.text.contains('\r')));
}

#[test]
fn ids_survive_edits_elsewhere() {
    let text = common::fixture("markdown/fences.md");
    let edited = text.replace("Some prose", "Much more prose,\nover two lines,");
    let ids =
        |t: &str| -> Vec<String> { find_blocks("f.md", t).into_iter().map(|b| b.id).collect() };
    assert_eq!(ids(&text), ids(&edited));
}

#[test]
fn slugs() {
    assert_eq!(slug("2. Containers"), "2-containers");
    assert_eq!(slug("3.9 File change to redraw"), "39-file-change-to-redraw");
    assert_eq!(slug("Components of tile.wasm"), "components-of-tilewasm");
    assert_eq!(slug("Before / after"), "before--after");
    assert_eq!(slug("Übersicht"), "übersicht");
}

#[test]
fn an_unclosed_fence_runs_to_the_end() {
    let blocks = find_blocks("f.md", "# A\n\n```mermaid\nflowchart LR\n  a --> b\n");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "flowchart LR\n  a --> b\n");
    assert_eq!(blocks[0].fence_line, 3);
}
