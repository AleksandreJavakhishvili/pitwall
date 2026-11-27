//! C4 reading of a flowchart: Names, Kinds, purposes, shapes, classes and
//! levels (mermaid-lld.md §5, frd.md §3.2, FR-5, FR-11).

use crate::mermaid::flowchart::{self, Direction, Flowchart, Head, LineStyle, NodeShape};

/// A box's Kind: `[Container: Rust]` has the base `container` and the
/// technology `Rust`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kind {
    /// As written, without the brackets.
    pub text: String,
    /// Lowercase, before an optional `:`.
    pub base: String,
    pub technology: Option<String>,
}

impl Kind {
    pub fn parse(text: &str) -> Self {
        let text = text.trim().to_string();
        let (base, technology) = match text.split_once(':') {
            Some((b, t)) => (b, Some(t.trim().to_string()).filter(|t| !t.is_empty())),
            None => (text.as_str(), None),
        };
        Kind { base: base.trim().to_lowercase(), technology, text }
    }
}

/// How a box is drawn (FR-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Box,
    Person,
    Cylinder,
    Pipe,
}

/// The classes with meaning (FR-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Marks {
    pub new: bool,
    pub changed: bool,
    pub external: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct C4Box {
    /// The Mermaid node id.
    pub id: String,
    pub name: String,
    pub kind: Option<Kind>,
    pub purpose: Option<String>,
    pub shape: Shape,
    pub classes: Vec<String>,
    pub marks: Marks,
    pub boundary: Option<usize>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct C4Boundary {
    pub id: String,
    pub name: String,
    pub kind: Option<Kind>,
    pub parent: Option<usize>,
    pub classes: Vec<String>,
    pub line: u32,
}

/// An arrow end: a box or a boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Box(usize),
    Boundary(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct C4Arrow {
    pub from: End,
    pub to: End,
    pub label: Option<String>,
    pub style: LineStyle,
    pub start_head: Head,
    pub end_head: Head,
    pub line: u32,
}

/// A structure diagram: a flowchart read with C4 meaning. Indices match the
/// flowchart's nodes, boundaries and edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structure {
    pub direction: Direction,
    pub boxes: Vec<C4Box>,
    pub boundaries: Vec<C4Boundary>,
    pub arrows: Vec<C4Arrow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Context,
    Container,
    Component,
}

/// The name key that zoom links and the Changes view compare: the Name
/// without `[Kind]`, spaces collapsed, trimmed, lowercased.
pub fn name_key(text: &str) -> String {
    let (name, _) = split_kind(text);
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Splits `Name [Kind]` into the Name and the Kind's text.
pub fn split_kind(line: &str) -> (&str, Option<&str>) {
    let line = line.trim();
    if let Some(stripped) = line.strip_suffix(']') {
        if let Some(open) = stripped.rfind('[') {
            let name = stripped[..open].trim();
            if !name.is_empty() {
                return (name, Some(stripped[open + 1..].trim()));
            }
        }
    }
    (line, None)
}

/// Reads a label's lines as Name, Kind and purpose.
pub fn read_label(lines: &[String]) -> (String, Option<Kind>, Option<String>) {
    let Some(first) = lines.first() else {
        return (String::new(), None, None);
    };
    let (name, kind) = split_kind(first);
    let mut rest = &lines[1..];
    let mut kind = kind.map(Kind::parse);
    if kind.is_none() {
        let second = rest.first().map(|l| l.trim());
        if let Some(inner) = second.and_then(|l| l.strip_prefix('[')?.strip_suffix(']')) {
            kind = Some(Kind::parse(inner));
            rest = &rest[1..];
        }
    }
    let purpose = (!rest.is_empty()).then(|| rest.join(" "));
    (name.to_string(), kind, purpose)
}

fn shape(kind: Option<&Kind>, hint: NodeShape) -> Shape {
    if let Some(kind) = kind {
        let text = kind.text.to_lowercase();
        let words: Vec<&str> =
            text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
        let has = |w: &str| words.contains(&w);
        if has("person") || has("actor") {
            return Shape::Person;
        }
        if has("database") || has("store") || has("db") {
            return Shape::Cylinder;
        }
        if has("queue") {
            return Shape::Pipe;
        }
    }
    if hint == NodeShape::Cylinder {
        Shape::Cylinder
    } else {
        Shape::Box
    }
}

fn has_class(classes: &[String], name: &str) -> bool {
    classes.iter().any(|c| c.eq_ignore_ascii_case(name))
}

/// Reads a flowchart as a structure diagram.
pub fn read(chart: &Flowchart) -> Structure {
    let boxes = chart
        .nodes
        .iter()
        .map(|node| {
            let (name, kind, purpose) = read_label(&node.lines);
            let external = has_class(&node.classes, "external")
                || kind.as_ref().is_some_and(|k| k.base.starts_with("external"));
            C4Box {
                id: node.id.clone(),
                shape: shape(kind.as_ref(), node.shape),
                marks: Marks {
                    new: has_class(&node.classes, "new"),
                    changed: has_class(&node.classes, "changed"),
                    external,
                },
                name,
                kind,
                purpose,
                classes: node.classes.clone(),
                boundary: node.boundary,
                line: node.line,
            }
        })
        .collect();
    let boundaries = chart
        .boundaries
        .iter()
        .map(|b| {
            let (name, kind) = split_kind(&b.title);
            C4Boundary {
                id: b.id.clone(),
                name: name.to_string(),
                kind: kind.map(Kind::parse),
                parent: b.parent,
                classes: b.classes.clone(),
                line: b.line,
            }
        })
        .collect();
    let end = |e: flowchart::End| match e {
        flowchart::End::Node(i) => End::Box(i),
        flowchart::End::Boundary(i) => End::Boundary(i),
    };
    let arrows = chart
        .edges
        .iter()
        .map(|e| C4Arrow {
            from: end(e.from),
            to: end(e.to),
            label: e.label.clone(),
            style: e.style,
            start_head: e.start_head,
            end_head: e.end_head,
            line: e.line,
        })
        .collect();
    Structure { direction: chart.direction, boxes, boundaries, arrows }
}

/// The level a heading or title names, if any: the first word starting
/// with `context`, `container` or `component`.
fn level_word(text: &str) -> Option<Level> {
    text.split(|c: char| !c.is_alphanumeric()).find_map(|word| {
        let word = word.to_lowercase();
        if word.starts_with("context") {
            Some(Level::Context)
        } else if word.starts_with("container") {
            Some(Level::Container)
        } else if word.starts_with("component") {
            Some(Level::Component)
        } else {
            None
        }
    })
}

fn kind_is(kind: &Kind, word: &str) -> bool {
    kind.base == word || kind.base.ends_with(&format!(" {word}"))
}

/// FR-11: the title, then the headings from the nearest outwards, then the
/// Kinds.
pub fn level(structure: &Structure, title: Option<&str>, headings: &[&str]) -> Level {
    let named = title.into_iter().chain(headings.iter().rev().copied()).find_map(level_word);
    if let Some(level) = named {
        return level;
    }
    let kinds = || structure.boxes.iter().filter_map(|b| b.kind.as_ref());
    if kinds().any(|k| kind_is(k, "component")) {
        Level::Component
    } else if kinds().any(|k| kind_is(k, "container")) {
        Level::Container
    } else {
        Level::Context
    }
}

impl Structure {
    /// The Name of an arrow end.
    pub fn end_name(&self, end: End) -> &str {
        match end {
            End::Box(i) => &self.boxes[i].name,
            End::Boundary(i) => &self.boundaries[i].name,
        }
    }

    /// How deep a boundary is nested (0 for a top-level one).
    pub fn depth(&self, boundary: usize) -> usize {
        let mut depth = 0;
        let mut at = self.boundaries[boundary].parent;
        while let Some(p) = at {
            depth += 1;
            at = self.boundaries[p].parent;
        }
        depth
    }
}
