//! The block finder: Mermaid fences in Markdown, with their positions, the
//! headings above them and stable ids (mermaid-lld.md §1).

use std::collections::HashMap;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// A heading above a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// 1 to 6.
    pub level: u8,
    pub text: String,
    /// The heading's first line in the file, 1-based.
    pub line: u32,
}

/// Where one line of a block's text starts in the Markdown file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineStart {
    line: u32,
    /// Characters before the block line's first character on that file line.
    column_offset: u32,
}

/// A fenced `mermaid` code block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// `<path>#<slug>`, see [`slug`].
    pub id: String,
    /// The block's text, without `\r`.
    pub text: String,
    /// The opening fence's line, 1-based.
    pub fence_line: u32,
    /// The closing fence's line (or the last line, for a fence left open).
    pub end_line: u32,
    /// Headings above the block, outermost first.
    pub headings: Vec<Heading>,
    lines: Vec<LineStart>,
}

impl Block {
    /// Maps a position inside the block's text (1-based line and column, in
    /// characters) to the Markdown file.
    pub fn file_position(&self, line: u32, column: u32) -> (u32, u32) {
        match self.lines.get(line.saturating_sub(1) as usize) {
            Some(start) => (start.line, start.column_offset + column),
            // Past the last line (an error at the end of the block): the
            // closing fence.
            None => (self.end_line, column),
        }
    }

    /// The nearest heading's text, if any.
    pub fn heading(&self) -> Option<&str> {
        self.headings.last().map(|h| h.text.as_str())
    }
}

/// A heading's slug: lowercase, letters, digits, `-` and `_` kept, spaces
/// turned into `-`, everything else dropped.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            out.extend(c.to_lowercase());
        } else if c.is_whitespace() {
            out.push('-');
        }
    }
    out
}

/// 1-based line starts of a text, for byte offset to (line, column).
struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Lines { text, starts }
    }

    fn line(&self, offset: usize) -> u32 {
        self.starts.partition_point(|&s| s <= offset) as u32
    }

    fn position(&self, offset: usize) -> (u32, u32) {
        let line = self.line(offset);
        let start = self.starts[line as usize - 1];
        let column = self.text.get(start..offset).map_or(0, |s| s.chars().count());
        (line, column as u32)
    }
}

/// Finds the Mermaid blocks of a Markdown file at `path` (used in ids).
pub fn find_blocks(path: &str, markdown: &str) -> Vec<Block> {
    let lines = Lines::new(markdown);
    let mut blocks = Vec::new();
    let mut headings: Vec<Heading> = Vec::new();
    let mut heading: Option<Heading> = None;
    let mut fence: Option<(usize, usize)> = None;
    let mut chunks: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    let mut seen: HashMap<String, u32> = HashMap::new();

    for (event, range) in Parser::new_ext(markdown, Options::empty()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let level = level as u8;
                let line = lines.line(range.start);
                heading = Some(Heading { level, text: String::new(), line });
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(mut h) = heading.take() {
                    h.text = h.text.split_whitespace().collect::<Vec<_>>().join(" ");
                    while headings.last().is_some_and(|last| last.level >= h.level) {
                        headings.pop();
                    }
                    headings.push(h);
                }
            }
            Event::Text(text) | Event::Code(text) if heading.is_some() => {
                if let Some(h) = heading.as_mut() {
                    h.text.push_str(&text);
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                let word = info.split_whitespace().next().unwrap_or("");
                if word.eq_ignore_ascii_case("mermaid") {
                    fence = Some((range.start, range.end));
                    chunks.clear();
                }
            }
            Event::Text(text) if fence.is_some() => chunks.push((text.into_string(), range)),
            Event::End(TagEnd::CodeBlock) => {
                if let Some((start, end)) = fence.take() {
                    let base = headings.last().map_or_else(|| "top".to_string(), |h| slug(&h.text));
                    let n = seen.entry(base.clone()).or_insert(0);
                    *n += 1;
                    let id = match *n {
                        1 => format!("{path}#{base}"),
                        n => format!("{path}#{base}-{n}"),
                    };
                    let (text, line_starts) = assemble(markdown, &lines, &chunks);
                    let end_line = lines.line(end.saturating_sub(1).max(start));
                    blocks.push(Block {
                        id,
                        text,
                        fence_line: lines.line(start),
                        end_line,
                        headings: headings.clone(),
                        lines: line_starts,
                    });
                }
            }
            _ => {}
        }
    }
    blocks
}

/// Joins a block's text chunks and records where each of its lines starts in
/// the file. A chunk that is a verbatim slice of the file maps exactly;
/// otherwise its lines count from the chunk's first line, at column 0.
fn assemble(
    markdown: &str,
    lines: &Lines,
    chunks: &[(String, std::ops::Range<usize>)],
) -> (String, Vec<LineStart>) {
    let mut text = String::new();
    let mut starts = Vec::new();
    let mut at_start = true;
    for (chunk, range) in chunks {
        let verbatim = markdown.get(range.clone()) == Some(chunk.as_str());
        let mut fallback_line = lines.line(range.start);
        for (i, c) in chunk.char_indices() {
            if c == '\r' {
                continue;
            }
            if at_start {
                let (line, column_offset) =
                    if verbatim { lines.position(range.start + i) } else { (fallback_line, 0) };
                starts.push(LineStart { line, column_offset });
                at_start = false;
            }
            text.push(c);
            if c == '\n' {
                at_start = true;
                fallback_line += 1;
            }
        }
    }
    (text, starts)
}
