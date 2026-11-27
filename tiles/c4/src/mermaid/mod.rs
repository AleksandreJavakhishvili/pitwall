//! The Mermaid subset of frd.md §6.1 (mermaid-lld.md §2–§4): the type line,
//! the flowchart and sequence parsers, and the fallback for the rest.

mod cursor;
pub mod flowchart;
pub mod label;
pub mod sequence;

pub use flowchart::Flowchart;
pub use sequence::Sequence;

use crate::limits;

/// A problem in a block: 1-based line and column (in characters) inside the
/// block's text. [`crate::blocks::Block::file_position`] maps it to the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl Problem {
    pub fn new(line: u32, column: u32, message: impl Into<String>) -> Self {
        Problem { line, column, message: message.into() }
    }
}

/// A part of a supported diagram that isn't drawn: "Not shown" (FR-20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub line: u32,
    pub what: String,
}

/// A parsed block.
#[derive(Debug, Clone, PartialEq)]
pub enum Diagram {
    Flowchart(Flowchart),
    Sequence(Sequence),
    Fallback(Fallback),
}

impl Diagram {
    /// The frontmatter title, if the block has one.
    pub fn title(&self) -> Option<&str> {
        match self {
            Diagram::Flowchart(f) => f.title.as_deref(),
            Diagram::Sequence(s) => s.title.as_deref(),
            Diagram::Fallback(f) => f.title.as_deref(),
        }
    }

    pub fn skipped(&self) -> &[Skipped] {
        match self {
            Diagram::Flowchart(f) => &f.skipped,
            Diagram::Sequence(s) => &s.skipped,
            Diagram::Fallback(_) => &[],
        }
    }
}

/// Why a block isn't drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    Empty,
    Unsupported,
    /// Mermaid's own `C4Context` family: not in v1 (frd.md §9, decision 3).
    MermaidC4,
    Problems,
    TooLarge,
}

/// A block shown as source (FR-18, FR-19, FR-21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallback {
    /// The type word, `""` for an empty block.
    pub type_word: String,
    pub title: Option<String>,
    pub reason: FallbackReason,
    pub problems: Vec<Problem>,
}

/// Known types' names, for the fallback note.
const TYPE_NAMES: &[(&str, &str)] = &[
    ("classDiagram", "Class diagrams"),
    ("classDiagram-v2", "Class diagrams"),
    ("stateDiagram", "State diagrams"),
    ("stateDiagram-v2", "State diagrams"),
    ("erDiagram", "Entity relationship diagrams"),
    ("gantt", "Gantt charts"),
    ("pie", "Pie charts"),
    ("mindmap", "Mindmaps"),
    ("timeline", "Timelines"),
    ("journey", "User journeys"),
    ("gitGraph", "Git graphs"),
    ("quadrantChart", "Quadrant charts"),
    ("requirementDiagram", "Requirement diagrams"),
    ("sankey-beta", "Sankey diagrams"),
    ("xychart-beta", "XY charts"),
    ("block-beta", "Block diagrams"),
    ("packet-beta", "Packet diagrams"),
    ("kanban", "Kanban boards"),
    ("architecture-beta", "Architecture diagrams"),
    ("radar-beta", "Radar charts"),
];

/// Mermaid's own C4 syntax.
const MERMAID_C4: &[&str] =
    &["C4Context", "C4Container", "C4Component", "C4Dynamic", "C4Deployment"];

impl Fallback {
    /// The short note the tile shows above the source.
    pub fn note(&self) -> String {
        match self.reason {
            FallbackReason::Empty => "The block is empty".into(),
            FallbackReason::MermaidC4 => {
                "Mermaid's C4 syntax isn't drawn here; write C4 diagrams as flowcharts".into()
            }
            FallbackReason::Problems => "This diagram has problems".into(),
            FallbackReason::TooLarge => "Too large to draw here".into(),
            FallbackReason::Unsupported => {
                match TYPE_NAMES.iter().find(|(word, _)| *word == self.type_word) {
                    Some((_, name)) => format!("{name} aren't drawn here"),
                    None => format!("'{}' diagrams aren't drawn here", self.type_word),
                }
            }
        }
    }
}

/// The preamble of a block: frontmatter title, directives, and where the
/// type line is.
struct Preamble {
    title: Option<String>,
    /// 0-based index of the type line in the block's lines.
    type_line: Option<usize>,
    skipped: Vec<Skipped>,
    problems: Vec<Problem>,
}

fn preamble(lines: &[&str]) -> Preamble {
    let mut p = Preamble { title: None, type_line: None, skipped: Vec::new(), problems: vec![] };
    let mut i = 0;
    if lines.first().is_some_and(|l| l.trim() == "---") {
        match lines.iter().skip(1).position(|l| l.trim() == "---") {
            Some(close) => {
                for line in &lines[1..=close] {
                    if let Some(value) = line.trim_start().strip_prefix("title:") {
                        let value = value.trim().trim_matches('"').trim_matches('\'');
                        p.title = Some(value.to_string());
                    }
                }
                i = close + 2;
            }
            None => {
                p.problems.push(Problem::new(1, 1, "frontmatter has no closing '---'"));
                return p;
            }
        }
    }
    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("%%{") {
            p.skipped.push(Skipped { line: i as u32 + 1, what: "init directive".into() });
        } else if !line.is_empty() && !line.starts_with("%%") {
            p.type_line = Some(i);
            break;
        }
        i += 1;
    }
    p
}

/// Parses a block's text.
pub fn parse(text: &str) -> Diagram {
    let lines: Vec<&str> = text.split('\n').collect();
    let pre = preamble(&lines);
    let fallback = |type_word: &str, reason, problems| {
        Diagram::Fallback(Fallback {
            type_word: type_word.to_string(),
            title: pre.title.clone(),
            reason,
            problems,
        })
    };
    if !pre.problems.is_empty() {
        return fallback("", FallbackReason::Problems, pre.problems.clone());
    }
    let Some(type_line) = pre.type_line else {
        return fallback("", FallbackReason::Empty, Vec::new());
    };
    let word: String = lines[type_line]
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let result = match word.as_str() {
        "flowchart" | "flowchart-elk" | "graph" => {
            flowchart::parse(&lines, type_line).map(Diagram::Flowchart)
        }
        "sequenceDiagram" => sequence::parse(&lines, type_line).map(Diagram::Sequence),
        w if MERMAID_C4.contains(&w) => return fallback(w, FallbackReason::MermaidC4, Vec::new()),
        w => return fallback(w, FallbackReason::Unsupported, Vec::new()),
    };
    match result {
        Ok(mut diagram) => {
            match &mut diagram {
                Diagram::Flowchart(f) => {
                    f.title = pre.title.clone();
                    f.skipped.splice(0..0, pre.skipped.iter().cloned());
                }
                Diagram::Sequence(s) => {
                    s.title = s.title.take().or(pre.title.clone());
                    s.skipped.splice(0..0, pre.skipped.iter().cloned());
                }
                Diagram::Fallback(_) => {}
            }
            diagram
        }
        Err(Failure::TooLarge(problems)) => fallback(&word, FallbackReason::TooLarge, problems),
        Err(Failure::Problems(problems)) => fallback(&word, FallbackReason::Problems, problems),
    }
}

/// Why a supported block didn't parse.
#[derive(Debug)]
pub(crate) enum Failure {
    Problems(Vec<Problem>),
    TooLarge(Vec<Problem>),
}

/// Applies the box and arrow limits (hla.md §8) to a parsed diagram.
pub(crate) fn check_size(boxes: usize, arrows: usize) -> Result<(), Failure> {
    if boxes > limits::MAX_BOXES || arrows > limits::MAX_ARROWS {
        Err(Failure::TooLarge(Vec::new()))
    } else {
        Ok(())
    }
}
