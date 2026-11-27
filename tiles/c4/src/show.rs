//! Shown diagrams (frd.md §5.1, hla.md §3.10): the Mermaid an agent sends
//! with `c4__show`, read all or nothing (FR-37), and the reply in words
//! (FR-38). Plain Rust: the tile (`src/tile.rs`) keeps the items.

use crate::c4;
use crate::mermaid::{self, Diagram, FallbackReason, Problem, Skipped};

/// The most characters of Mermaid one call takes (FR-34; the tool's
/// schema says the same, so Pitwall refuses longer input first).
pub const MAX_SOURCE: usize = 200_000;

/// What a call shows, once it reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Ready {
    /// The Mermaid, without a ```` ```mermaid ```` fence around it.
    pub source: String,
    /// The `title` given, else the frontmatter title, else the first
    /// box's (or participant's) Name.
    pub title: String,
    pub diagram: Diagram,
    /// Parts not drawn, with lines in the agent's text.
    pub skipped: Vec<Skipped>,
}

/// Why nothing was shown: a message and the problems, with lines and
/// columns counted in the agent's `mermaid` text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub message: String,
    pub problems: Vec<Problem>,
}

impl Refused {
    fn plain(message: impl Into<String>) -> Refused {
        Refused { message: message.into(), problems: Vec::new() }
    }
}

/// The text inside a ```` ```mermaid ```` fence when the whole input is
/// one (agents often send it so), and how many lines come before it.
pub fn unfence(text: &str) -> (&str, u32) {
    let trimmed = text.trim();
    let Some(first_end) = trimmed.find('\n') else { return (text, 0) };
    let first = trimmed[..first_end].trim();
    let fence = if first.starts_with("```") { "```" } else if first.starts_with("~~~") { "~~~" } else { return (text, 0) };
    let word = first.trim_start_matches(fence).trim();
    if !(word.is_empty() || word.eq_ignore_ascii_case("mermaid")) {
        return (text, 0);
    }
    let body = &trimmed[first_end + 1..];
    let Some(body) = body.trim_end().strip_suffix(fence) else { return (text, 0) };
    // Lines before the fence's first line: blank ones the trim took off.
    let before = text[..text.len() - text.trim_start().len()].matches('\n').count() as u32;
    (body.trim_end_matches([' ', '\t']), before + 1)
}

/// Reads an agent's call: the diagram, or why nothing changes (FR-37).
pub fn read(mermaid_text: &str, title: Option<&str>) -> Result<Ready, Refused> {
    let (source, offset) = unfence(mermaid_text);
    let source = source.replace('\r', "");
    if source.trim().is_empty() {
        return Err(Refused::plain("`mermaid` has no diagram in it. Nothing was changed."));
    }
    if source.chars().count() > MAX_SOURCE {
        return Err(Refused::plain(format!("`mermaid` is longer than {MAX_SOURCE} characters. Nothing was changed.")));
    }
    let shift = |mut p: Problem| {
        p.line += offset;
        p
    };
    let diagram = mermaid::parse(&source);
    if let Diagram::Fallback(f) = &diagram {
        return Err(match f.reason {
            FallbackReason::Problems => Refused { message: "The diagram has problems; nothing was changed.".into(), problems: f.problems.iter().cloned().map(shift).collect() },
            FallbackReason::TooLarge => Refused::plain(format!(
                "Too large to draw here: at most {} boxes and {} arrows. Nothing was changed; split it into a context diagram and detail diagrams.",
                crate::limits::MAX_BOXES,
                crate::limits::MAX_ARROWS
            )),
            FallbackReason::Empty => Refused::plain("`mermaid` has no diagram in it. Nothing was changed."),
            FallbackReason::MermaidC4 => Refused::plain(format!(
                "{}. Nothing was changed: send a `flowchart` with boxes labelled \"Name [Kind]<br/>purpose\" and a `subgraph` per boundary.",
                f.note()
            )),
            FallbackReason::Unsupported => Refused::plain(format!("{}: the C4 tile draws `flowchart` (or `graph`) and `sequenceDiagram`. Nothing was changed.", f.note())),
        });
    }
    let skipped = diagram.skipped().iter().cloned().map(|mut s| {
        s.line += offset;
        s
    }).collect();
    let title = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .or_else(|| diagram.title().map(String::from))
        .or_else(|| first_name(&diagram))
        .unwrap_or_else(|| "Diagram".into());
    Ok(Ready { source, title, diagram, skipped })
}

fn first_name(d: &Diagram) -> Option<String> {
    match d {
        Diagram::Flowchart(f) => c4::read(f).boxes.first().map(|b| b.name.clone()).filter(|n| !n.is_empty()),
        Diagram::Sequence(s) => s.participants.first().map(|p| p.label.clone()),
        Diagram::Fallback(_) => None,
    }
}

/// An item id from a title, for a call without one: its slug, then `-2`,
/// `-3`… past the ones `taken`.
pub fn item_for(title: &str, taken: impl Fn(&str) -> bool) -> String {
    let base = crate::blocks::slug(title);
    let base = if base.is_empty() { "diagram".to_string() } else { base.chars().take(60).collect() };
    if !taken(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|c| !taken(c)).unwrap_or(base)
}

/// What the diagram holds, in words: "7 boxes, 6 arrows, 2 boundaries" or
/// "4 participants, 9 messages".
pub fn counts(d: &Diagram) -> String {
    fn n(count: usize, one: &str, many: &str) -> String {
        format!("{count} {}", if count == 1 { one } else { many })
    }
    match d {
        Diagram::Flowchart(f) => {
            let s = c4::read(f);
            let mut parts = vec![n(s.boxes.len(), "box", "boxes"), n(s.arrows.len(), "arrow", "arrows")];
            if !s.boundaries.is_empty() {
                parts.push(n(s.boundaries.len(), "boundary", "boundaries"));
            }
            parts.join(", ")
        }
        Diagram::Sequence(s) => {
            let messages = s.steps.iter().filter(|x| matches!(x, mermaid::sequence::Step::Message { .. })).count();
            format!("{}, {}", n(s.participants.len(), "participant", "participants"), n(messages, "message", "messages"))
        }
        Diagram::Fallback(_) => String::new(),
    }
}

/// The skipped parts in a reply: "Not shown: line 9: a click line; …".
pub fn skipped_words(skipped: &[Skipped]) -> Option<String> {
    if skipped.is_empty() {
        return None;
    }
    let list: Vec<String> = skipped.iter().take(10).map(|s| format!("line {}: {}", s.line, s.what)).collect();
    let more = if skipped.len() > 10 { format!(" and {} more", skipped.len() - 10) } else { String::new() };
    Some(format!("Not shown: {}{more}.", list.join("; ")))
}
