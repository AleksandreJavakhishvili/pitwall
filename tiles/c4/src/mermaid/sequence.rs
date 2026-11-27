//! The sequence subset (mermaid-lld.md §4).

use std::collections::HashMap;

use super::{check_size, label, Failure, Problem, Skipped};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub id: String,
    /// The alias, or the id.
    pub label: String,
    pub actor: bool,
    pub box_group: Option<usize>,
    pub line: u32,
}

/// A `box` grouping participants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxGroup {
    pub label: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageHead {
    /// `->`, `-->`
    None,
    /// `->>`, `-->>`
    Arrow,
    /// `-x`, `--x`
    Cross,
    /// `-)`, `--)`
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Loop,
    Alt,
    Opt,
    Par,
    Critical,
    Break,
    Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    Else,
    And,
    Option,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotePlacement {
    LeftOf,
    RightOf,
    Over,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Message {
        from: usize,
        to: usize,
        text: String,
        dotted: bool,
        head: MessageHead,
        activate_target: bool,
        deactivate_source: bool,
        line: u32,
    },
    Note { placement: NotePlacement, first: usize, second: Option<usize>, text: String, line: u32 },
    Activate { participant: usize, line: u32 },
    Deactivate { participant: usize, line: u32 },
    FrameStart { kind: FrameKind, label: String, line: u32 },
    FrameBranch { kind: BranchKind, label: String, line: u32 },
    FrameEnd { line: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sequence {
    pub title: Option<String>,
    pub participants: Vec<Participant>,
    pub boxes: Vec<BoxGroup>,
    pub steps: Vec<Step>,
    pub autonumber: bool,
    pub skipped: Vec<Skipped>,
}

/// Message arrows, longest first.
const ARROWS: &[(&str, bool, MessageHead)] = &[
    ("-->>", true, MessageHead::Arrow),
    ("--x", true, MessageHead::Cross),
    ("--)", true, MessageHead::Async),
    ("-->", true, MessageHead::None),
    ("->>", false, MessageHead::Arrow),
    ("-x", false, MessageHead::Cross),
    ("-)", false, MessageHead::Async),
    ("->", false, MessageHead::None),
];

const FRAMES: &[(&str, FrameKind)] = &[
    ("loop", FrameKind::Loop),
    ("alt", FrameKind::Alt),
    ("opt", FrameKind::Opt),
    ("par", FrameKind::Par),
    ("critical", FrameKind::Critical),
    ("break", FrameKind::Break),
    ("rect", FrameKind::Rect),
];

const BRANCHES: &[(&str, BranchKind, FrameKind)] = &[
    ("else", BranchKind::Else, FrameKind::Alt),
    ("and", BranchKind::And, FrameKind::Par),
    ("option", BranchKind::Option, FrameKind::Critical),
];

const SKIPPED: &[&str] =
    &["accTitle", "accDescr", "link", "links", "properties", "details", "destroy"];

enum Open {
    Frame { kind: FrameKind, word: &'static str, line: u32, column: u32 },
    Box { line: u32, column: u32 },
}

struct Parser {
    seq: Sequence,
    by_id: HashMap<String, usize>,
    open: Vec<Open>,
    active: Vec<u32>,
    problems: Vec<Problem>,
}

pub(crate) fn parse(lines: &[&str], type_line: usize) -> Result<Sequence, Failure> {
    let mut p = Parser {
        seq: Sequence::default(),
        by_id: HashMap::new(),
        open: Vec::new(),
        active: Vec::new(),
        problems: Vec::new(),
    };
    for (i, raw) in lines.iter().enumerate().skip(type_line + 1) {
        let line = i as u32 + 1;
        let trimmed = raw.trim();
        let column = (raw.chars().count() - raw.trim_start().chars().count()) as u32 + 1;
        let statement = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
        if statement.is_empty() {
            continue;
        }
        if statement.starts_with("%%") {
            if statement.starts_with("%%{") {
                p.seq.skipped.push(Skipped { line, what: "init directive".into() });
            }
            continue;
        }
        if let Err(problem) = p.statement(statement, line, column) {
            p.problems.push(problem);
        }
    }
    while let Some(open) = p.open.pop() {
        let (word, line, column) = match open {
            Open::Frame { word, line, column, .. } => (word, line, column),
            Open::Box { line, column } => ("box", line, column),
        };
        p.problems.push(Problem::new(line, column, format!("'{word}' has no matching 'end'")));
    }
    if !p.problems.is_empty() {
        p.problems.sort_by_key(|p| (p.line, p.column));
        return Err(Failure::Problems(p.problems));
    }
    check_size(p.seq.participants.len(), p.seq.steps.len())?;
    Ok(p.seq)
}

/// Splits off the first word, case kept; the rest is trimmed.
fn first_word(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim()),
        None => (s, ""),
    }
}

fn text(raw: &str) -> String {
    label::decode(raw).lines.join(" ")
}

impl Parser {
    fn participant(&mut self, id: &str, line: u32) -> usize {
        if let Some(&i) = self.by_id.get(id) {
            return i;
        }
        let box_group = self.current_box();
        self.seq.participants.push(Participant {
            id: id.to_string(),
            label: id.to_string(),
            actor: false,
            box_group,
            line,
        });
        self.active.push(0);
        self.by_id.insert(id.to_string(), self.seq.participants.len() - 1);
        self.seq.participants.len() - 1
    }

    fn current_box(&self) -> Option<usize> {
        self.open
            .iter()
            .any(|o| matches!(o, Open::Box { .. }))
            .then(|| self.seq.boxes.len().saturating_sub(1))
    }

    fn statement(&mut self, statement: &str, line: u32, column: u32) -> Result<(), Problem> {
        let (word, rest) = first_word(statement);
        let keyword = word.trim_end_matches(':').to_ascii_lowercase();
        let err = |message: String| Problem::new(line, column, message);
        match keyword.as_str() {
            "create" => {
                self.seq.skipped.push(Skipped { line, what: "'create'".into() });
                let offset = (statement.len() - rest.len()) as u32;
                return self.statement(rest, line, column + offset);
            }
            "participant" | "actor" => {
                let (id, alias) = match rest.find(" as ") {
                    Some(i) => (rest[..i].trim(), Some(rest[i + 4..].trim())),
                    None => (rest, None),
                };
                if id.is_empty() {
                    return Err(err("expected a participant name".into()));
                }
                let p = self.participant(id, line);
                let participant = &mut self.seq.participants[p];
                participant.actor = keyword == "actor";
                participant.line = line;
                if let Some(alias) = alias {
                    participant.label = text(alias);
                }
                return Ok(());
            }
            "activate" | "deactivate" => {
                if rest.is_empty() {
                    return Err(err("expected a participant name".into()));
                }
                let p = self.participant(rest, line);
                if keyword == "activate" {
                    self.active[p] += 1;
                    self.seq.steps.push(Step::Activate { participant: p, line });
                } else {
                    self.deactivate(p, line, column)?;
                    self.seq.steps.push(Step::Deactivate { participant: p, line });
                }
                return Ok(());
            }
            "note" => return self.note(rest, line, column),
            "end" => {
                return match self.open.pop() {
                    Some(Open::Frame { .. }) => {
                        self.seq.steps.push(Step::FrameEnd { line });
                        Ok(())
                    }
                    Some(Open::Box { .. }) => Ok(()),
                    None => Err(err("'end' without an open frame".into())),
                };
            }
            "autonumber" => {
                self.seq.autonumber = true;
                return Ok(());
            }
            "title" => {
                self.seq.title = Some(text(rest.trim_start_matches(':')));
                return Ok(());
            }
            "box" => {
                let label = match rest.find(')') {
                    Some(i) if rest.starts_with("rgb") => rest[i + 1..].trim(),
                    _ => rest.strip_prefix("transparent").unwrap_or(rest).trim(),
                };
                self.seq.boxes.push(BoxGroup { label: text(label), line });
                self.open.push(Open::Box { line, column });
                return Ok(());
            }
            _ => {}
        }
        if let Some(&(name, kind)) = FRAMES.iter().find(|(name, _)| *name == keyword) {
            if kind == FrameKind::Rect {
                self.seq.skipped.push(Skipped { line, what: "'rect' colour".into() });
            }
            let label = if kind == FrameKind::Rect { String::new() } else { text(rest) };
            self.open.push(Open::Frame { kind, word: name, line, column });
            self.seq.steps.push(Step::FrameStart { kind, label, line });
            return Ok(());
        }
        if let Some(&(name, kind, parent)) = BRANCHES.iter().find(|(name, ..)| *name == keyword) {
            let inside = self
                .open
                .last()
                .is_some_and(|o| matches!(o, Open::Frame { kind, .. } if *kind == parent));
            if !inside {
                let parent_word = FRAMES.iter().find(|f| f.1 == parent).map_or("", |f| f.0);
                return Err(err(format!("'{name}' outside '{parent_word}'")));
            }
            self.seq.steps.push(Step::FrameBranch { kind, label: text(rest), line });
            return Ok(());
        }
        if SKIPPED.iter().any(|s| s.eq_ignore_ascii_case(&keyword)) {
            let word = word.trim_end_matches(':');
            self.seq.skipped.push(Skipped { line, what: format!("'{word}' line") });
            return Ok(());
        }
        self.message(statement, line, column)
    }

    fn deactivate(&mut self, p: usize, line: u32, column: u32) -> Result<(), Problem> {
        if self.active[p] == 0 {
            let name = &self.seq.participants[p].id;
            return Err(Problem::new(line, column, format!("'{name}' is not active")));
        }
        self.active[p] -= 1;
        Ok(())
    }

    fn note(&mut self, rest: &str, line: u32, column: u32) -> Result<(), Problem> {
        let lower = rest.to_ascii_lowercase();
        let (placement, after) = if lower.starts_with("left of ") {
            (NotePlacement::LeftOf, &rest[8..])
        } else if lower.starts_with("right of ") {
            (NotePlacement::RightOf, &rest[9..])
        } else if lower.starts_with("over ") {
            (NotePlacement::Over, &rest[5..])
        } else {
            return Err(Problem::new(line, column, "expected 'left of', 'right of' or 'over'"));
        };
        let Some(colon) = after.find(':') else {
            return Err(Problem::new(line, column, "expected ':' and the note text"));
        };
        let names: Vec<&str> = after[..colon].split(',').map(str::trim).collect();
        if names.iter().any(|n| n.is_empty()) || names.len() > 2 {
            return Err(Problem::new(line, column, "expected a participant name"));
        }
        let first = self.participant(names[0], line);
        let second = names.get(1).map(|n| self.participant(n, line));
        let text = text(&after[colon + 1..]);
        self.seq.steps.push(Step::Note { placement, first, second, text, line });
        Ok(())
    }

    fn message(&mut self, statement: &str, line: u32, column: u32) -> Result<(), Problem> {
        let found = statement.char_indices().find_map(|(i, _)| {
            ARROWS.iter().find(|(token, ..)| statement[i..].starts_with(token)).map(|a| (i, a))
        });
        let Some((at, &(token, dotted, head))) = found else {
            let (word, _) = first_word(statement);
            return Err(Problem::new(line, column, format!("not a sequence statement: '{word}'")));
        };
        let from = statement[..at].trim();
        let mut rest = &statement[at + token.len()..];
        let mut activate_target = false;
        let mut deactivate_source = false;
        if let Some(r) = rest.strip_prefix('+') {
            activate_target = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix('-') {
            deactivate_source = true;
            rest = r;
        }
        let Some(colon) = rest.find(':') else {
            let col = column + statement.chars().count() as u32;
            return Err(Problem::new(line, col, "expected ':' and the message text"));
        };
        let to = rest[..colon].trim();
        if from.is_empty() || to.is_empty() {
            return Err(Problem::new(line, column, "expected a participant name"));
        }
        let from = self.participant(from, line);
        let to = self.participant(to, line);
        if deactivate_source {
            self.deactivate(from, line, column)?;
        }
        if activate_target {
            self.active[to] += 1;
        }
        self.seq.steps.push(Step::Message {
            from,
            to,
            text: text(&rest[colon + 1..]),
            dotted,
            head,
            activate_target,
            deactivate_source,
            line,
        });
        Ok(())
    }
}
