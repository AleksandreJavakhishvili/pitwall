//! The flowchart subset (mermaid-lld.md §3).

use std::collections::HashMap;

use super::cursor::{is_word_char, Cursor};
use super::label;
use super::{check_size, Failure, Problem, Skipped};
use crate::limits;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    TopDown,
    BottomUp,
    LeftRight,
    RightLeft,
}

impl Direction {
    fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "TB" | "TD" => Direction::TopDown,
            "BT" => Direction::BottomUp,
            "LR" => Direction::LeftRight,
            "RL" => Direction::RightLeft,
            _ => return None,
        })
    }
}

/// The Mermaid shape a node was written with: a hint (FR-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeShape {
    /// No shape written: the id is the label.
    Plain,
    Rect,
    Round,
    Stadium,
    Cylinder,
    Circle,
    Diamond,
    Hexagon,
    Parallelogram,
    ParallelogramAlt,
    Flag,
    /// Shapes outside the subset, drawn as plain boxes.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: String,
    pub lines: Vec<String>,
    pub shape: NodeShape,
    pub classes: Vec<String>,
    pub boundary: Option<usize>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boundary {
    pub id: String,
    pub title: String,
    pub parent: Option<usize>,
    pub direction: Option<Direction>,
    pub classes: Vec<String>,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Node(usize),
    Boundary(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    Solid,
    Dotted,
    Thick,
    Invisible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Head {
    None,
    Arrow,
    Cross,
    Circle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub from: End,
    pub to: End,
    pub label: Option<String>,
    pub style: LineStyle,
    pub start_head: Head,
    pub end_head: Head,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Flowchart {
    pub direction: Direction,
    pub title: Option<String>,
    pub nodes: Vec<Node>,
    pub boundaries: Vec<Boundary>,
    pub edges: Vec<Edge>,
    /// Names from `classDef`.
    pub class_defs: Vec<String>,
    pub skipped: Vec<Skipped>,
}

/// Shape delimiters, longest open first: (open, closes, shape).
const SHAPES: &[(&str, &[&str], NodeShape)] = &[
    ("(((", &[")))"], NodeShape::Other),
    ("([", &["])"], NodeShape::Stadium),
    ("[(", &[")]"], NodeShape::Cylinder),
    ("((", &["))"], NodeShape::Circle),
    ("{{", &["}}"], NodeShape::Hexagon),
    ("[[", &["]]"], NodeShape::Other),
    ("[/", &["/]", "\\]"], NodeShape::Parallelogram),
    ("[\\", &["\\]", "/]"], NodeShape::ParallelogramAlt),
    ("(", &[")"], NodeShape::Round),
    ("[", &["]"], NodeShape::Rect),
    ("{", &["}"], NodeShape::Diamond),
    (">", &["]"], NodeShape::Flag),
];

const SKIPPED_STATEMENTS: &[&str] = &["style", "linkStyle", "click", "accTitle", "accDescr"];

/// A node while parsing; ids resolve at the end.
struct Draft {
    id: String,
    lines: Vec<String>,
    shape: NodeShape,
    has_shape: bool,
    classes: Vec<String>,
    boundary: Option<usize>,
    line: u32,
}

struct DraftEdge {
    from: String,
    to: String,
    link: Link,
    line: u32,
}

#[derive(Clone)]
struct Link {
    label: Option<String>,
    style: LineStyle,
    start_head: Head,
    end_head: Head,
}

struct Vertex {
    id: String,
}

struct Parser {
    cur: Cursor,
    chart: Flowchart,
    drafts: Vec<Draft>,
    by_id: HashMap<String, usize>,
    edges: Vec<DraftEdge>,
    /// Open subgraphs (indices) and the column of their keyword.
    open: Vec<(usize, u32)>,
    class_uses: Vec<(Vec<String>, Vec<String>)>,
    problems: Vec<Problem>,
    too_deep: bool,
}

type Step<T> = Result<T, Problem>;

pub(crate) fn parse(lines: &[&str], type_line: usize) -> Result<Flowchart, Failure> {
    let mut p = Parser {
        cur: Cursor::new(lines, type_line),
        chart: Flowchart::default(),
        drafts: Vec::new(),
        by_id: HashMap::new(),
        edges: Vec::new(),
        open: Vec::new(),
        class_uses: Vec::new(),
        problems: Vec::new(),
        too_deep: false,
    };
    if let Err(problem) = p.header() {
        p.problems.push(problem);
        p.cur.skip_line();
    }
    while !p.cur.is_eof() {
        p.skip_blank();
        if p.cur.is_eof() {
            break;
        }
        if let Err(problem) = p.statement() {
            p.problems.push(problem);
            p.cur.skip_line();
        }
    }
    p.finish()
}

impl Parser {
    fn error(&self, message: impl Into<String>) -> Problem {
        let (line, column) = self.cur.position();
        Problem::new(line, column, message)
    }

    fn header(&mut self) -> Step<()> {
        self.cur.skip_spaces();
        for word in ["flowchart-elk", "flowchart", "graph"] {
            if self.cur.eat(word) {
                break;
            }
        }
        self.cur.skip_spaces();
        if !self.cur.at_statement_end() {
            self.chart.direction = self.direction()?;
        }
        self.end_statement()
    }

    fn direction(&mut self) -> Step<Direction> {
        let word: String =
            self.cur.rest_of_line().chars().take_while(|&c| is_word_char(c)).collect();
        match Direction::from_word(&word) {
            Some(d) => {
                self.cur.advance(word.chars().count());
                Ok(d)
            }
            None => Err(self.error(format!(
                "expected a direction (TB, TD, BT, LR or RL), found {}",
                self.cur.token()
            ))),
        }
    }

    /// Skips whitespace, `;` and comment lines.
    fn skip_blank(&mut self) {
        loop {
            match self.cur.peek() {
                Some(' ' | '\t' | '\n' | ';') => {
                    self.cur.bump();
                }
                Some('%') if self.cur.starts_with("%%") => {
                    if self.cur.starts_with("%%{") {
                        let line = self.cur.line();
                        self.chart.skipped.push(Skipped { line, what: "init directive".into() });
                    }
                    self.cur.skip_line();
                }
                _ => return,
            }
        }
    }

    /// After a statement: spaces, then the end of the statement.
    fn end_statement(&mut self) -> Step<()> {
        self.cur.skip_spaces();
        if self.cur.at_statement_end() || self.cur.starts_with("%%") {
            if self.cur.starts_with("%%") {
                self.cur.skip_line();
            }
            Ok(())
        } else {
            Err(self.error(format!(
                "expected an arrow, '&' or the end of the line, found {}",
                self.cur.token()
            )))
        }
    }

    fn statement(&mut self) -> Step<()> {
        let line = self.cur.line();
        if self.cur.starts_with_word("subgraph") {
            return self.subgraph();
        }
        if self.cur.starts_with_word("end") {
            let column = self.cur.position().1;
            self.cur.advance(3);
            if self.open.pop().is_none() {
                return Err(Problem::new(line, column, "'end' without an open subgraph"));
            }
            return self.end_statement();
        }
        if self.cur.starts_with_word("direction") {
            self.cur.advance("direction".len());
            self.cur.skip_spaces();
            let direction = self.direction()?;
            if let Some(&(b, _)) = self.open.last() {
                self.chart.boundaries[b].direction = Some(direction);
            }
            return self.end_statement();
        }
        if self.cur.starts_with_word("classDef") {
            self.cur.advance("classDef".len());
            self.cur.skip_spaces();
            let names = self.names()?;
            self.chart.class_defs.extend(names);
            self.cur.skip_line();
            return Ok(());
        }
        if self.cur.starts_with_word("class") {
            self.cur.advance("class".len());
            self.cur.skip_spaces();
            return self.class_statement();
        }
        for word in SKIPPED_STATEMENTS {
            let keyword = self.cur.starts_with_word(word)
                || (self.cur.starts_with(word) && self.cur.peek_at(word.len()) == Some(':'));
            if keyword {
                self.chart.skipped.push(Skipped { line, what: format!("'{word}' line") });
                self.cur.skip_line();
                return Ok(());
            }
        }
        self.chain()
    }

    /// Comma-separated class names.
    fn names(&mut self) -> Step<Vec<String>> {
        let mut names = Vec::new();
        loop {
            self.cur.skip_spaces();
            let name = self.word();
            if name.is_empty() {
                return Err(self.error("expected a class name"));
            }
            names.push(name);
            self.cur.skip_spaces();
            if !self.cur.eat(",") {
                return Ok(names);
            }
        }
    }

    /// A run of id characters.
    fn word(&mut self) -> String {
        let mut word = String::new();
        while let Some(c) = self.cur.peek() {
            let hyphen = c == '-' && self.cur.peek_at(1).is_some_and(|n| n.is_alphanumeric());
            if is_word_char(c) || c == '.' || hyphen {
                word.push(c);
                self.cur.bump();
            } else {
                break;
            }
        }
        word
    }

    fn class_statement(&mut self) -> Step<()> {
        let rest = self.cur.rest_of_line();
        let rest = rest.split(';').next().unwrap_or("").trim().to_string();
        let parts: Vec<&str> = match rest.rfind(char::is_whitespace) {
            Some(i) => vec![&rest[..i], &rest[i..]],
            None => vec![rest.as_str()],
        };
        if parts.len() < 2 || parts[1].trim().is_empty() {
            return Err(self.error("expected node ids and a class name"));
        }
        let split = |s: &str| -> Vec<String> {
            s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
        };
        let ids = split(parts[0]);
        let classes = split(parts[1]);
        if ids.is_empty() || classes.is_empty() {
            return Err(self.error("expected node ids and a class name"));
        }
        self.class_uses.push((ids, classes));
        self.cur.skip_line();
        Ok(())
    }

    fn subgraph(&mut self) -> Step<()> {
        let (line, column) = self.cur.position();
        self.cur.advance("subgraph".len());
        self.cur.skip_spaces();
        let rest = self.cur.rest_of_line();
        let raw = rest.trim_end();
        let (id, title) = if let Some(quoted) = raw.strip_prefix('"') {
            let Some(end) = quoted.find('"') else {
                return Err(self.error("unclosed quote"));
            };
            (quoted[..end].to_string(), quoted[..end].to_string())
        } else if let Some(open) = raw.find('[') {
            let Some(close) = raw.rfind(']') else {
                return Err(self.error("unclosed label: expected ']'"));
            };
            let inner = raw[open + 1..close].trim();
            let inner = inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(inner);
            (raw[..open].trim().to_string(), inner.to_string())
        } else {
            (raw.to_string(), raw.to_string())
        };
        if id.is_empty() {
            return Err(self.error(format!("expected a node id, found {}", self.cur.token())));
        }
        let title = label::decode(&title).lines.join(" ");
        if self.open.len() >= limits::MAX_DEPTH {
            self.too_deep = true;
            return Err(Problem::new(line, column, "nested deeper than 16 subgraphs"));
        }
        let index = self.chart.boundaries.len();
        self.chart.boundaries.push(Boundary {
            id: id.clone(),
            title,
            parent: self.open.last().map(|o| o.0),
            direction: None,
            classes: Vec::new(),
            line,
        });
        self.open.push((index, column));
        self.cur.skip_line();
        Ok(())
    }

    /// `group (link group)*`
    fn chain(&mut self) -> Step<()> {
        let line = self.cur.line();
        let mut left = self.group()?;
        loop {
            self.cur.skip_spaces();
            let Some(link) = self.link()? else {
                break;
            };
            self.cur.skip_spaces();
            let right = self.group()?;
            for from in &left {
                for to in &right {
                    self.edges.push(DraftEdge {
                        from: from.id.clone(),
                        to: to.id.clone(),
                        link: link.clone(),
                        line,
                    });
                }
            }
            left = right;
        }
        self.end_statement()
    }

    /// `vertex ("&" vertex)*`
    fn group(&mut self) -> Step<Vec<Vertex>> {
        let mut group = vec![self.vertex()?];
        loop {
            self.cur.skip_spaces();
            if !self.cur.eat("&") {
                return Ok(group);
            }
            self.cur.skip_spaces();
            group.push(self.vertex()?);
        }
    }

    fn vertex(&mut self) -> Step<Vertex> {
        let line = self.cur.line();
        let id = self.word();
        if id.is_empty() {
            return Err(self.error(format!("expected a node id, found {}", self.cur.token())));
        }
        let mut shape = None;
        let mut spaces = 0;
        while matches!(self.cur.peek_at(spaces), Some(' ' | '\t')) {
            spaces += 1;
        }
        if spaces > 0 && matches!(self.cur.peek_at(spaces), Some('[' | '(' | '{')) {
            self.cur.advance(spaces);
        }
        if self.cur.starts_with("@{") {
            self.skip_braces()?;
            self.chart.skipped.push(Skipped { line, what: "shape syntax '@{…}'".into() });
        } else if let Some(&(open, closes, kind)) =
            SHAPES.iter().find(|(open, _, _)| self.cur.starts_with(open))
        {
            self.cur.advance(open.chars().count());
            let raw = self.shape_text(closes)?;
            if kind == NodeShape::Other {
                self.chart.skipped.push(Skipped {
                    line,
                    what: format!("shape '{open}…' drawn as a box"),
                });
            }
            shape = Some((kind, raw));
        }
        let mut classes = Vec::new();
        if self.cur.eat(":::") {
            classes = self.names()?;
        }
        self.mention(&id, line, shape, classes);
        Ok(Vertex { id })
    }

    /// Skips a balanced `{ … }` after `@`.
    fn skip_braces(&mut self) -> Step<()> {
        self.cur.bump();
        let mut depth = 0;
        while let Some(c) = self.cur.peek() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.cur.bump();
                        return Ok(());
                    }
                }
                '\n' => break,
                _ => {}
            }
            self.cur.bump();
        }
        Err(self.error("unclosed label: expected '}'"))
    }

    /// A shape's label up to one of `closes`, quoted or not.
    fn shape_text(&mut self, closes: &[&str]) -> Step<String> {
        self.cur.skip_spaces();
        let expected = || format!("unclosed label: expected '{}'", closes[0]);
        if self.cur.peek() == Some('"') {
            let text = self.quoted()?;
            self.cur.skip_spaces();
            return match closes.iter().find(|c| self.cur.starts_with(c)) {
                Some(close) => {
                    self.cur.advance(close.chars().count());
                    Ok(text)
                }
                None => Err(self.error(expected())),
            };
        }
        let mut text = String::new();
        loop {
            if let Some(close) = closes.iter().find(|c| self.cur.starts_with(c)) {
                self.cur.advance(close.chars().count());
                return Ok(text);
            }
            match self.cur.peek() {
                None | Some('\n') => return Err(self.error(expected())),
                Some(c) => {
                    text.push(c);
                    self.cur.bump();
                }
            }
        }
    }

    /// A `"…"` string, which may span lines. Markdown strings lose their
    /// backticks and are noted.
    fn quoted(&mut self) -> Step<String> {
        let (line, column) = self.cur.position();
        self.cur.bump();
        let mut text = String::new();
        loop {
            match self.cur.bump() {
                None => return Err(Problem::new(line, column, "unclosed quote")),
                Some('"') => break,
                Some(c) => text.push(c),
            }
        }
        if text.len() >= 2 && text.starts_with('`') && text.ends_with('`') {
            self.chart.skipped.push(Skipped { line, what: "Markdown string in a label".into() });
            text = text[1..text.len() - 1].to_string();
        }
        Ok(text)
    }

    /// A label's lines, noting what decoding dropped.
    fn label_lines(&mut self, raw: &str, line: u32) -> Vec<String> {
        let decoded = label::decode(raw);
        for note in decoded.notes {
            self.chart.skipped.push(Skipped { line, what: note.into() });
        }
        decoded.lines
    }

    fn mention(
        &mut self,
        id: &str,
        line: u32,
        shape: Option<(NodeShape, String)>,
        classes: Vec<String>,
    ) {
        let current = self.open.last().map(|o| o.0);
        let index = match self.by_id.get(id) {
            Some(&i) => i,
            None => {
                self.drafts.push(Draft {
                    id: id.to_string(),
                    lines: vec![id.to_string()],
                    shape: NodeShape::Plain,
                    has_shape: false,
                    classes: Vec::new(),
                    boundary: current,
                    line,
                });
                self.by_id.insert(id.to_string(), self.drafts.len() - 1);
                self.drafts.len() - 1
            }
        };
        if self.drafts[index].boundary.is_none() {
            self.drafts[index].boundary = current;
        }
        if let Some((kind, raw)) = shape {
            let lines = self.label_lines(&raw, line);
            let draft = &mut self.drafts[index];
            draft.lines = lines;
            draft.shape = kind;
            draft.has_shape = true;
        }
        self.drafts[index].classes.extend(classes);
    }

    /// A link at the cursor, or `None` when there is none.
    fn link(&mut self) -> Step<Option<Link>> {
        let start_pos = self.cur.position();
        let mut n = 0;
        let start_head = match self.cur.peek() {
            Some('<') => Head::Arrow,
            Some(c @ ('x' | 'o')) if matches!(self.cur.peek_at(1), Some('-' | '=')) => {
                if c == 'x' {
                    Head::Cross
                } else {
                    Head::Circle
                }
            }
            _ => Head::None,
        };
        if start_head != Head::None {
            n = 1;
        }
        let body_char = self.cur.peek_at(n);
        let run = |cur: &Cursor, from: usize, c: char| {
            let mut k = 0;
            while cur.peek_at(from + k) == Some(c) {
                k += 1;
            }
            k
        };
        let (style, len, open_label) = match body_char {
            Some('~') => {
                let k = run(&self.cur, n, '~');
                if k < 3 {
                    return Ok(None);
                }
                (LineStyle::Invisible, k, false)
            }
            Some('=') => {
                let k = run(&self.cur, n, '=');
                if k < 2 {
                    return Ok(None);
                }
                (LineStyle::Thick, k, k == 2)
            }
            Some('-') => {
                let k = run(&self.cur, n, '-');
                if k == 1 && self.cur.peek_at(n + 1) == Some('.') {
                    let dots = run(&self.cur, n + 1, '.');
                    let dashes = run(&self.cur, n + 1 + dots, '-');
                    if dashes == 0 {
                        (LineStyle::Dotted, 1 + dots, true)
                    } else {
                        (LineStyle::Dotted, 1 + dots + dashes, false)
                    }
                } else if k >= 2 {
                    (LineStyle::Solid, k, k == 2)
                } else {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        };
        self.cur.advance(n + len);
        let end_head = self.end_head();
        let mut link = Link { label: None, style, start_head, end_head };
        let label_follows = open_label
            && end_head == Head::None
            && matches!(self.cur.peek(), Some(' ' | '\t' | '"'));
        if label_follows {
            link.label = Some(self.inline_label(style, start_pos)?);
            link.end_head = self.end_head();
        }
        self.cur.skip_spaces();
        if self.cur.peek() == Some('|') {
            link.label = Some(self.pipe_label()?);
        }
        if let Some(raw) = link.label.take() {
            let lines = self.label_lines(&raw, start_pos.0);
            link.label = (!lines.is_empty()).then(|| lines.join(" "));
        }
        Ok(Some(link))
    }

    /// `|text|` or `|"text"|`, the cursor on the first `|`.
    fn pipe_label(&mut self) -> Step<String> {
        self.cur.bump();
        self.cur.skip_spaces();
        let unclosed = |p: &Self| p.error("unclosed edge label: expected '|'");
        if self.cur.peek() == Some('"') {
            let text = self.quoted()?;
            self.cur.skip_spaces();
            return if self.cur.eat("|") { Ok(text) } else { Err(unclosed(self)) };
        }
        let mut text = String::new();
        loop {
            match self.cur.peek() {
                None | Some('\n') => return Err(unclosed(self)),
                Some('|') => {
                    self.cur.bump();
                    return Ok(text.trim().to_string());
                }
                Some(c) => {
                    text.push(c);
                    self.cur.bump();
                }
            }
        }
    }

    /// `>`, or `x` / `o` when no id character follows.
    fn end_head(&mut self) -> Head {
        let head = match self.cur.peek() {
            Some('>') => Head::Arrow,
            Some('x') => Head::Cross,
            Some('o') => Head::Circle,
            _ => return Head::None,
        };
        if head != Head::Arrow && self.cur.peek_at(1).is_some_and(is_word_char) {
            return Head::None;
        }
        self.cur.bump();
        head
    }

    /// The text of `-- text -->`, `== text ==>` or `-. text .->`, leaving the
    /// cursor on the closing arrow's head (if any).
    fn inline_label(&mut self, style: LineStyle, start: (u32, u32)) -> Step<String> {
        self.cur.skip_spaces();
        let (close, closing) = match style {
            LineStyle::Thick => ("==", "==>"),
            LineStyle::Dotted => (".-", ".->"),
            _ => ("--", "-->"),
        };
        let mut text = String::new();
        if self.cur.peek() == Some('"') {
            text = self.quoted()?;
            self.cur.skip_spaces();
        }
        loop {
            if self.cur.starts_with(close) {
                let c = close.chars().next().unwrap_or('-');
                while self.cur.peek() == Some(c) {
                    self.cur.bump();
                }
                while self.cur.peek() == Some('-') {
                    self.cur.bump();
                }
                return Ok(text.trim().to_string());
            }
            match self.cur.peek() {
                None | Some('\n') => {
                    return Err(Problem::new(
                        start.0,
                        start.1,
                        format!("edge label with no arrow: expected '{closing}'"),
                    ));
                }
                Some(c) => {
                    text.push(c);
                    self.cur.bump();
                }
            }
        }
    }

    fn finish(mut self) -> Result<Flowchart, Failure> {
        while let Some((b, column)) = self.open.pop() {
            let boundary = &self.chart.boundaries[b];
            self.problems.push(Problem::new(
                boundary.line,
                column,
                format!("subgraph '{}' has no matching 'end'", boundary.title),
            ));
        }
        if !self.problems.is_empty() {
            self.problems.sort_by_key(|p| (p.line, p.column));
            return Err(if self.too_deep {
                Failure::TooLarge(self.problems)
            } else {
                Failure::Problems(self.problems)
            });
        }
        let boundary_ids: HashMap<String, usize> =
            self.chart.boundaries.iter().enumerate().map(|(i, b)| (b.id.clone(), i)).collect();
        let mut node_index: HashMap<String, usize> = HashMap::new();
        for draft in self.drafts {
            if !draft.has_shape && boundary_ids.contains_key(&draft.id) {
                continue;
            }
            node_index.insert(draft.id.clone(), self.chart.nodes.len());
            self.chart.nodes.push(Node {
                id: draft.id,
                lines: draft.lines,
                shape: draft.shape,
                classes: draft.classes,
                boundary: draft.boundary,
                line: draft.line,
            });
        }
        let end = |id: &str| match node_index.get(id) {
            Some(&n) => End::Node(n),
            None => End::Boundary(boundary_ids[id]),
        };
        for e in self.edges {
            self.chart.edges.push(Edge {
                from: end(&e.from),
                to: end(&e.to),
                label: e.link.label,
                style: e.link.style,
                start_head: e.link.start_head,
                end_head: e.link.end_head,
                line: e.line,
            });
        }
        for (ids, classes) in self.class_uses {
            for id in ids {
                if let Some(&n) = node_index.get(&id) {
                    self.chart.nodes[n].classes.extend(classes.iter().cloned());
                } else if let Some(&b) = boundary_ids.get(&id) {
                    self.chart.boundaries[b].classes.extend(classes.iter().cloned());
                }
            }
        }
        check_size(self.chart.nodes.len(), self.chart.edges.len())?;
        Ok(self.chart)
    }
}
