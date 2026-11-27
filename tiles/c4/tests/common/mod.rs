//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::Path;

use c4_tile::index::Index;
use c4_tile::mermaid::{self, Diagram, Fallback, Flowchart, Sequence};

/// A fixture's text, by its path under `tests/fixtures/`.
pub fn fixture(path: &str) -> String {
    let full = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(path);
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
}

/// Every Markdown fixture, by path under `tests/fixtures/`.
pub fn all_fixtures() -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "md") {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.push(rel);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out
}

/// An index of the given fixtures.
pub fn index_of(paths: &[&str]) -> Index {
    let mut index = Index::new();
    for path in paths {
        index.set_file(path, &fixture(path));
    }
    index
}

/// The C4 structure of a diagram in an index.
pub fn structure(index: &Index, id: &str) -> c4_tile::c4::Structure {
    let entry = index.get(id).unwrap_or_else(|| panic!("no diagram {id}"));
    entry.structure.clone().unwrap_or_else(|| panic!("{id} is not a structure diagram"))
}

/// Mermaid text from its lines.
pub fn lines(lines: &[&str]) -> String {
    lines.join("\n")
}

/// Parses Mermaid text that must be a flowchart.
pub fn flowchart(text: &str) -> Flowchart {
    match mermaid::parse(text) {
        Diagram::Flowchart(f) => f,
        other => panic!("not a flowchart: {other:?}"),
    }
}

/// Parses Mermaid text that must be a sequence diagram.
pub fn sequence(text: &str) -> Sequence {
    match mermaid::parse(text) {
        Diagram::Sequence(s) => s,
        other => panic!("not a sequence diagram: {other:?}"),
    }
}

/// Parses Mermaid text that must fall back.
pub fn fallback(text: &str) -> Fallback {
    match mermaid::parse(text) {
        Diagram::Fallback(f) => f,
        other => panic!("not a fallback: {other:?}"),
    }
}

/// The problems of the single Mermaid block in `markdown`, in file
/// coordinates, as (line, column, message).
pub fn problems(markdown: &str) -> Vec<(u32, u32, String)> {
    let mut index = Index::new();
    index.set_file("doc.md", markdown);
    assert_eq!(index.entries().len(), 1, "one block expected");
    index.entries()[0].problems().into_iter().map(|p| (p.line, p.column, p.message)).collect()
}
