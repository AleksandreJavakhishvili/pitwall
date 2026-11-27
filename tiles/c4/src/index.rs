//! The diagram index and the Linker: which diagram details which box, and
//! before/after pairs (hla.md §3.4, FR-11 to FR-13, FR-17).

use std::collections::HashMap;

use crate::blocks::{self, Block, Heading};
use crate::c4::{self, Level, Structure};
use crate::mermaid::{self, Diagram, Problem};

/// Heading words that mark before and after (frd.md §3.4, decided by the
/// owner on 2026-11-27).
pub const BEFORE_AFTER: &[(&str, &str)] =
    &[("before", "after"), ("current", "proposed"), ("as is", "to be"), ("today", "target")];

/// One diagram of the index.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The file, relative to the project root, with `/`.
    pub path: String,
    pub block: Block,
    pub diagram: Diagram,
    /// The C4 reading, for flowcharts.
    pub structure: Option<Structure>,
    /// The level, for flowcharts (FR-11).
    pub level: Option<Level>,
}

impl Entry {
    pub fn id(&self) -> &str {
        &self.block.id
    }

    /// The frontmatter title, or else the nearest heading.
    pub fn title(&self) -> Option<&str> {
        self.diagram.title().or(self.block.heading())
    }

    /// The fallback's problems in Markdown file coordinates (FR-19).
    pub fn problems(&self) -> Vec<Problem> {
        let Diagram::Fallback(f) = &self.diagram else {
            return Vec::new();
        };
        f.problems
            .iter()
            .map(|p| {
                let (line, column) = self.block.file_position(p.line, p.column);
                Problem { line, column, message: p.message.clone() }
            })
            .collect()
    }

    fn folder(&self) -> &str {
        self.path.rfind('/').map_or("", |i| &self.path[..i])
    }
}

/// Where a box zooms to (FR-13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoomTarget {
    None,
    One(String),
    /// Several, for a menu, in the index's order.
    Several(Vec<String>),
}

/// A before/after pair of diagram ids (frd.md §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub before: String,
    pub after: String,
}

/// All diagrams of a project, by file, with zoom links and pairs.
#[derive(Debug, Default)]
pub struct Index {
    entries: Vec<Entry>,
    /// Name key → entries with a boundary of that name.
    details: HashMap<String, Vec<usize>>,
    pairs: Vec<Pair>,
}

/// Parses one block into an entry.
pub fn entry(path: &str, block: Block) -> Entry {
    let diagram = mermaid::parse(&block.text);
    let structure = match &diagram {
        Diagram::Flowchart(f) => Some(c4::read(f)),
        _ => None,
    };
    let headings: Vec<&str> = block.headings.iter().map(|h| h.text.as_str()).collect();
    let level = structure.as_ref().map(|s| c4::level(s, diagram.title(), &headings));
    Entry { path: path.to_string(), block, diagram, structure, level }
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a file's diagrams and re-links.
    pub fn set_file(&mut self, path: &str, markdown: &str) {
        self.entries.retain(|e| e.path != path);
        let blocks = blocks::find_blocks(path, markdown);
        self.entries.extend(blocks.into_iter().map(|b| entry(path, b)));
        self.entries.sort_by(|a, b| {
            a.path.cmp(&b.path).then(a.block.fence_line.cmp(&b.block.fence_line))
        });
        self.relink();
    }

    pub fn remove_file(&mut self, path: &str) {
        self.entries.retain(|e| e.path != path);
        self.relink();
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id() == id)
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.id() == id)
    }

    pub fn pairs(&self) -> &[Pair] {
        &self.pairs
    }

    /// The pair a diagram is part of, if any.
    pub fn pair_of(&self, id: &str) -> Option<&Pair> {
        self.pairs.iter().find(|p| p.before == id || p.after == id)
    }

    fn relink(&mut self) {
        self.details.clear();
        for (i, entry) in self.entries.iter().enumerate() {
            let Some(s) = &entry.structure else { continue };
            for b in &s.boundaries {
                let list = self.details.entry(c4::name_key(&b.name)).or_default();
                if !list.contains(&i) {
                    list.push(i);
                }
            }
        }
        self.pairs = self.find_pairs();
    }

    /// Where box `index` of diagram `from` zooms to, with `path` the zoom
    /// path above `from` (its diagrams are never targets).
    pub fn zoom_target(&self, from: &str, index: usize, path: &[&str]) -> ZoomTarget {
        let Some(at) = self.position(from) else {
            return ZoomTarget::None;
        };
        let source = &self.entries[at];
        let Some(b) = source.structure.as_ref().and_then(|s| s.boxes.get(index)) else {
            return ZoomTarget::None;
        };
        let candidates: Vec<&Entry> = self
            .details
            .get(&c4::name_key(&b.name))
            .into_iter()
            .flatten()
            .filter(|&&i| i != at)
            .map(|&i| &self.entries[i])
            .filter(|e| !path.contains(&e.id()))
            .collect();
        let same_file: Vec<&Entry> =
            candidates.iter().copied().filter(|e| e.path == source.path).collect();
        let same_folder: Vec<&Entry> =
            candidates.iter().copied().filter(|e| e.folder() == source.folder()).collect();
        let chosen = if !same_file.is_empty() {
            same_file
        } else if !same_folder.is_empty() {
            same_folder
        } else {
            candidates
        };
        match chosen.as_slice() {
            [] => ZoomTarget::None,
            [one] => ZoomTarget::One(one.id().to_string()),
            many => ZoomTarget::Several(many.iter().map(|e| e.id().to_string()).collect()),
        }
    }

    /// Whether a box shows the zoom mark (FR-12).
    pub fn is_detailed(&self, from: &str, index: usize) -> bool {
        self.zoom_target(from, index, &[]) != ZoomTarget::None
    }

    /// Sibling sections whose titles are a before/after word pair, with one
    /// structure diagram directly under each.
    fn find_pairs(&self) -> Vec<Pair> {
        // Grouped by (path, parent heading's line): each section's heading
        // and the structure diagrams directly under it.
        type Sections<'a> = Vec<(&'a Heading, Vec<&'a str>)>;
        let mut groups: Vec<((&str, Option<u32>), Sections)> = Vec::new();
        for entry in &self.entries {
            if entry.structure.is_none() {
                continue;
            }
            let headings = &entry.block.headings;
            let Some(section) = headings.last() else { continue };
            let parent = headings.len().checked_sub(2).map(|i| headings[i].line);
            let key = (entry.path.as_str(), parent);
            let group = match groups.iter().position(|g| g.0 == key) {
                Some(i) => &mut groups[i].1,
                None => {
                    groups.push((key, Vec::new()));
                    &mut groups.last_mut().expect("just pushed").1
                }
            };
            match group.iter_mut().find(|s| s.0.line == section.line) {
                Some(s) => s.1.push(entry.id()),
                None => group.push((section, vec![entry.id()])),
            }
        }
        let mut pairs = Vec::new();
        for (_, sections) in &groups {
            for &(before_word, after_word) in BEFORE_AFTER {
                let side = |word: &str| -> Vec<&Vec<&str>> {
                    sections
                        .iter()
                        .filter(|(h, _)| title_matches(&h.text, word))
                        .map(|(_, ids)| ids)
                        .collect()
                };
                let (befores, afters) = (side(before_word), side(after_word));
                if let ([before], [after]) = (befores.as_slice(), afters.as_slice()) {
                    if let ([b], [a]) = (before.as_slice(), after.as_slice()) {
                        pairs.push(Pair { before: b.to_string(), after: a.to_string() });
                    }
                }
            }
        }
        pairs
    }
}

/// Whether a heading's title is `word`: after an optional section number,
/// without case, the whole title or followed by a non-letter.
pub fn title_matches(title: &str, word: &str) -> bool {
    let title = title.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ');
    let title = title.to_lowercase();
    match title.strip_prefix(word) {
        Some(rest) => !rest.starts_with(char::is_alphabetic),
        None => false,
    }
}
