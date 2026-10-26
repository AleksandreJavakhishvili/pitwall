//! Markdown as blocks to draw: pulldown-cmark (MIT, CommonMark with GFM
//! tables, task lists, strikethrough and footnotes) folded into headings,
//! paragraphs, code, quotes, lists, tables, rules and raw HTML (kept as
//! text), each with the source lines it came from. [`mark`] colours the
//! blocks a diff's hunks touch. Pure.

use std::ops::Range;

use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, HeadingLevel, MetadataBlockKind, Options, Parser, Tag,
    TagEnd,
};

use crate::code_view::diff::Hunk;
use crate::code_view::highlight::{self, Lang, LineSpans};

/// Where a markdown file is shown as Preview: `.md`, `.markdown`, `.mdx`
/// (its JSX stays text).
pub fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".md", ".markdown", ".mdx", ".mkd"].iter().any(|e| lower.ends_with(e))
}

/// Inline style of a run of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    /// A footnote reference (`[1]`).
    pub note: bool,
    /// Index into [`Text::links`].
    pub link: Option<usize>,
}

/// Styled text: the runs cover `text` in order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text {
    pub text: String,
    pub runs: Vec<(Range<usize>, Style)>,
    pub links: Vec<String>,
}

impl Text {
    fn push(&mut self, s: &str, style: Style) {
        if s.is_empty() {
            return;
        }
        let at = self.text.len();
        self.text.push_str(s);
        match self.runs.last_mut() {
            Some((r, st)) if *st == style && r.end == at => r.end = self.text.len(),
            _ => self.runs.push((at..self.text.len(), style)),
        }
    }
}

/// A piece of inline content: text, or an image (drawn on its own line).
#[derive(Debug, Clone, PartialEq)]
pub enum Seg {
    Text(Text),
    Image { src: String, alt: String },
}

pub type Inline = Vec<Seg>;

/// The plain text of inline content (images as their alt text).
pub fn plain(inline: &Inline) -> String {
    inline
        .iter()
        .map(|s| match s {
            Seg::Text(t) => t.text.as_str(),
            Seg::Image { alt, .. } => alt.as_str(),
        })
        .collect()
}

/// What a diff did to a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// `- [ ]` / `- [x]`.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
    pub lines: Range<usize>,
    pub change: Option<Change>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Heading(u8, Inline),
    Paragraph(Inline),
    /// A fenced or indented block (and front matter), highlighted when its
    /// language is known: `spans` per line, as [`highlight::highlight`].
    Code {
        lang: Option<String>,
        text: String,
        spans: Vec<LineSpans>,
    },
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Table {
        aligns: Vec<Align>,
        head: Vec<Inline>,
        rows: Vec<Vec<Inline>>,
    },
    Rule,
    /// Raw HTML, shown as text.
    Html(String),
    Footnote(String, Vec<Block>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: Kind,
    /// 0-based source lines, half-open.
    pub lines: Range<usize>,
    /// Set by [`mark`] on top-level blocks other than lists (their items
    /// carry it).
    pub change: Option<Change>,
}

/// Lines a diff removed, shown before top-level block `before` (the block
/// count: at the end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removed {
    pub before: usize,
    pub lines: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Doc {
    pub blocks: Vec<Block>,
    pub removed: Vec<Removed>,
}

/// The language of a fence's info string (`rust`, `ts`, `sh`…).
pub fn fence_lang(info: &str) -> Option<Lang> {
    let word = info.split([' ', ',', '{']).next().unwrap_or("").to_ascii_lowercase();
    let ext = match word.as_str() {
        "" => return None,
        "rust" => "rs",
        "javascript" | "node" => "js",
        "typescript" => "ts",
        "python" | "python3" => "py",
        "golang" => "go",
        "shell" | "console" | "shellscript" => "sh",
        "c++" => "cpp",
        "csharp" | "c#" => "cs",
        "kotlin" => "kt",
        "ruby" => "rb",
        "markdown" => "md",
        "powershell" | "pwsh" => "ps1",
        "haskell" => "hs",
        "elixir" => "ex",
        "ocaml" => "ml",
        "objc" | "objective-c" => "c",
        w => w,
    };
    Lang::for_path(&format!("x.{ext}"))
}

/// `text` as blocks.
pub fn parse(text: &str) -> Doc {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
    let mut b = Builder {
        it: Parser::new_ext(text, opts).into_offset_iter().peekable(),
        starts: line_starts(text),
        item_depth: 0,
        task: None,
        last: 0,
    };
    Doc {
        blocks: b.blocks(),
        removed: Vec::new(),
    }
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

fn is_inline(tag: &Tag) -> bool {
    matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

fn is_inline_end(end: &TagEnd) -> bool {
    matches!(
        end,
        TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image
    )
}

struct Builder<'a, I: Iterator<Item = (Event<'a>, Range<usize>)>> {
    it: std::iter::Peekable<I>,
    starts: Vec<usize>,
    /// Nesting of list items being read.
    item_depth: usize,
    /// A task marker and the item depth it belongs to.
    task: Option<(usize, bool)>,
    /// The end of the last inline event read.
    last: usize,
}

impl<'a, I: Iterator<Item = (Event<'a>, Range<usize>)>> Builder<'a, I> {
    fn lines(&self, r: &Range<usize>) -> Range<usize> {
        let line = |at: usize| self.starts.partition_point(|s| *s <= at).saturating_sub(1);
        let start = line(r.start);
        start..(line(r.end.saturating_sub(1).max(r.start)) + 1).max(start + 1)
    }

    /// Blocks up to the enclosing End (consumed) or the end of the text.
    fn blocks(&mut self) -> Vec<Block> {
        let mut out = Vec::new();
        while let Some((ev, range)) = self.it.peek().cloned() {
            match ev {
                Event::End(_) => {
                    self.it.next();
                    break;
                }
                Event::Start(tag) if !is_inline(&tag) => {
                    self.it.next();
                    if let Some(b) = self.block(tag, &range) {
                        out.push(b);
                    }
                }
                Event::Rule => {
                    self.it.next();
                    out.push(self.mk(Kind::Rule, &range));
                }
                Event::Html(h) | Event::DisplayMath(h) => {
                    // Outside an HTML block only in odd nesting: as text.
                    self.it.next();
                    out.push(self.mk(Kind::Html(h.trim_end().to_string()), &range));
                }
                _ => {
                    // A tight list item's text: an implicit paragraph.
                    let inline = self.inline();
                    if !inline.is_empty() {
                        let end = self.last.max(range.start);
                        out.push(self.mk(Kind::Paragraph(inline), &(range.start..end)));
                    }
                }
            }
        }
        out
    }

    fn mk(&self, kind: Kind, range: &Range<usize>) -> Block {
        Block {
            kind,
            lines: self.lines(range),
            change: None,
        }
    }

    /// Raw text up to the enclosing End (consumed).
    fn raw(&mut self) -> String {
        let mut s = String::new();
        for (ev, _) in self.it.by_ref() {
            match ev {
                Event::End(_) => break,
                Event::Text(t) | Event::Html(t) | Event::Code(t) | Event::InlineHtml(t) => {
                    s.push_str(&t)
                }
                _ => {}
            }
        }
        s
    }

    fn block(&mut self, tag: Tag<'a>, range: &Range<usize>) -> Option<Block> {
        let kind = match tag {
            Tag::Paragraph => {
                let i = self.inline();
                self.it.next();
                Kind::Paragraph(i)
            }
            Tag::Heading { level, .. } => {
                let i = self.inline();
                self.it.next();
                let n = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                Kind::Heading(n, i)
            }
            Tag::BlockQuote(_) => Kind::Quote(self.blocks()),
            Tag::CodeBlock(k) => {
                let lang = match k {
                    CodeBlockKind::Fenced(info) => Some(info.to_string()).filter(|s| !s.is_empty()),
                    CodeBlockKind::Indented => None,
                };
                let text = self.raw();
                self.code(lang, text)
            }
            Tag::MetadataBlock(k) => {
                let lang = match k {
                    MetadataBlockKind::YamlStyle => "yaml",
                    MetadataBlockKind::PlusesStyle => "toml",
                };
                let text = self.raw();
                self.code(Some(lang.into()), text)
            }
            Tag::HtmlBlock => Kind::Html(self.raw().trim_end().to_string()),
            Tag::List(start) => Kind::List {
                start,
                items: self.items(),
            },
            Tag::FootnoteDefinition(label) => Kind::Footnote(label.to_string(), self.blocks()),
            Tag::Table(aligns) => self.table(aligns),
            // Not enabled (definition lists) or stray: their content.
            _ => {
                let mut inner = self.blocks();
                return (!inner.is_empty()).then(|| inner.remove(0));
            }
        };
        Some(self.mk(kind, range))
    }

    fn code(&self, lang: Option<String>, mut text: String) -> Kind {
        if text.ends_with('\n') {
            text.pop();
        }
        let spans = highlight::highlight(&text, lang.as_deref().and_then(fence_lang));
        Kind::Code { lang, text, spans }
    }

    fn items(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while let Some((ev, range)) = self.it.next() {
            match ev {
                Event::Start(Tag::Item) => {
                    self.item_depth += 1;
                    let blocks = self.blocks();
                    let task = match self.task {
                        Some((d, on)) if d == self.item_depth => {
                            self.task = None;
                            Some(on)
                        }
                        _ => None,
                    };
                    self.item_depth -= 1;
                    items.push(Item {
                        task,
                        blocks,
                        lines: self.lines(&range),
                        change: None,
                    });
                }
                Event::End(_) => break,
                _ => {}
            }
        }
        items
    }

    fn table(&mut self, aligns: Vec<Alignment>) -> Kind {
        let aligns = aligns
            .into_iter()
            .map(|a| match a {
                Alignment::None => Align::None,
                Alignment::Left => Align::Left,
                Alignment::Center => Align::Center,
                Alignment::Right => Align::Right,
            })
            .collect();
        let mut head = Vec::new();
        let mut rows: Vec<Vec<Inline>> = Vec::new();
        let mut in_head = false;
        while let Some((ev, _)) = self.it.next() {
            match ev {
                Event::Start(Tag::TableHead) => in_head = true,
                Event::End(TagEnd::TableHead) => in_head = false,
                Event::Start(Tag::TableRow) => rows.push(Vec::new()),
                Event::Start(Tag::TableCell) => {
                    let cell = self.inline();
                    self.it.next();
                    if in_head {
                        head.push(cell);
                    } else if let Some(r) = rows.last_mut() {
                        r.push(cell);
                    }
                }
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }
        Kind::Table { aligns, head, rows }
    }

    /// Inline content up to the enclosing End (left unread) or a block
    /// start.
    fn inline(&mut self) -> Inline {
        let mut segs: Inline = Vec::new();
        let mut text = Text::default();
        let mut style = Style::default();
        // Open inline tags: what to undo at their End.
        let mut open: Vec<TagEnd> = Vec::new();
        let mut links: Vec<Option<usize>> = Vec::new();
        while let Some((ev, _)) = self.it.peek() {
            match ev {
                Event::End(e) if !is_inline_end(e) || open.is_empty() => break,
                Event::Start(t) if !is_inline(t) => break,
                Event::Rule | Event::Html(_) | Event::DisplayMath(_) if open.is_empty() => break,
                _ => {}
            }
            let (ev, r) = self.it.next().expect("peeked");
            self.last = r.end;
            match ev {
                Event::Text(s) => text.push(&s, style),
                Event::Code(s) | Event::InlineMath(s) | Event::DisplayMath(s) => {
                    text.push(&s, Style { code: true, ..style })
                }
                Event::Html(s) | Event::InlineHtml(s) => text.push(&s, style),
                Event::SoftBreak => text.push(" ", style),
                Event::HardBreak => text.push("\n", style),
                Event::FootnoteReference(l) => {
                    text.push(&format!("[{l}]"), Style { note: true, ..style })
                }
                Event::TaskListMarker(on) => self.task = Some((self.item_depth, on)),
                Event::Start(Tag::Image { dest_url, .. }) => {
                    let mut alt = String::new();
                    let mut depth = 1;
                    for (ev, r) in self.it.by_ref() {
                        self.last = r.end;
                        match ev {
                            Event::Start(_) => depth += 1,
                            Event::End(_) => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Event::Text(s) | Event::Code(s) => alt.push_str(&s),
                            _ => {}
                        }
                    }
                    if !text.text.is_empty() {
                        segs.push(Seg::Text(std::mem::take(&mut text)));
                    }
                    segs.push(Seg::Image {
                        src: dest_url.to_string(),
                        alt,
                    });
                }
                Event::Start(t) => {
                    match &t {
                        Tag::Emphasis => style.italic = true,
                        Tag::Strong => style.bold = true,
                        Tag::Strikethrough => style.strike = true,
                        Tag::Link { dest_url, .. } => {
                            links.push(style.link);
                            text.links.push(dest_url.to_string());
                            style.link = Some(text.links.len() - 1);
                        }
                        _ => {}
                    }
                    open.push(t.to_end());
                }
                Event::End(e) => {
                    open.pop();
                    match e {
                        TagEnd::Emphasis => style.italic = open.contains(&TagEnd::Emphasis),
                        TagEnd::Strong => style.bold = open.contains(&TagEnd::Strong),
                        TagEnd::Strikethrough => {
                            style.strike = open.contains(&TagEnd::Strikethrough)
                        }
                        TagEnd::Link => style.link = links.pop().flatten(),
                        _ => {}
                    }
                }
                Event::Rule => {}
            }
        }
        if !text.text.is_empty() {
            segs.push(Seg::Text(text));
        }
        segs
    }
}

/// Colour the blocks `hunks` (lines of the old text → lines of `doc`'s)
/// touch: a block wholly inside an insertion is added, any other touched
/// block modified (list items one by one). A deletion between blocks is a
/// [`Removed`] marker; inside a block it modifies that block.
pub fn mark(doc: &mut Doc, hunks: &[Hunk]) {
    for h in hunks {
        if h.new.is_empty() {
            let p = h.new.start;
            if let Some(b) = doc
                .blocks
                .iter_mut()
                .find(|b| b.lines.start < p && p < b.lines.end)
            {
                touch(b, p - 1..p, Change::Modified);
                continue;
            }
            let before = doc.blocks.partition_point(|b| b.lines.start < p);
            match doc.removed.last_mut() {
                Some(r) if r.before == before => r.lines += h.old.len(),
                _ => doc.removed.push(Removed {
                    before,
                    lines: h.old.len(),
                }),
            }
            continue;
        }
        let kind = if h.old.is_empty() {
            Change::Added
        } else {
            Change::Modified
        };
        for b in doc.blocks.iter_mut().filter(|b| overlaps(&b.lines, &h.new)) {
            touch(b, h.new.clone(), kind);
        }
    }
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// Added when the inserted lines cover all of `lines`.
fn change_for(lines: &Range<usize>, r: &Range<usize>, kind: Change, was: Option<Change>) -> Change {
    let covered = r.start <= lines.start && lines.end <= r.end;
    if kind == Change::Added && covered && was.is_none() {
        Change::Added
    } else {
        Change::Modified
    }
}

fn touch(b: &mut Block, r: Range<usize>, kind: Change) {
    if let Kind::List { items, .. } = &mut b.kind {
        for it in items.iter_mut().filter(|it| overlaps(&it.lines, &r)) {
            it.change = Some(change_for(&it.lines, &r, kind, it.change));
        }
        return;
    }
    b.change = Some(change_for(&b.lines, &r, kind, b.change));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(d: &Doc) -> Vec<&'static str> {
        d.blocks
            .iter()
            .map(|b| match b.kind {
                Kind::Heading(..) => "h",
                Kind::Paragraph(_) => "p",
                Kind::Code { .. } => "code",
                Kind::Quote(_) => "quote",
                Kind::List { .. } => "list",
                Kind::Table { .. } => "table",
                Kind::Rule => "hr",
                Kind::Html(_) => "html",
                Kind::Footnote(..) => "note",
            })
            .collect()
    }

    #[test]
    fn headings_paragraphs_and_their_lines() {
        let d = parse("# Title\n\nSome *em* and **strong**\nnext line.\n\n###### Six\n---\n");
        assert_eq!(kinds(&d), ["h", "p", "h", "hr"]);
        assert_eq!(d.blocks[0].lines, 0..1);
        assert_eq!(d.blocks[1].lines, 2..4);
        let Kind::Heading(6, ref i) = d.blocks[2].kind else { panic!() };
        assert_eq!(plain(i), "Six");
        let Kind::Paragraph(ref p) = d.blocks[1].kind else { panic!() };
        let Seg::Text(t) = &p[0] else { panic!() };
        assert_eq!(t.text, "Some em and strong next line.");
        let em = t.runs.iter().find(|(_, s)| s.italic).unwrap();
        assert_eq!(&t.text[em.0.clone()], "em");
        let strong = t.runs.iter().find(|(_, s)| s.bold).unwrap();
        assert_eq!(&t.text[strong.0.clone()], "strong");
    }

    #[test]
    fn inline_code_links_strike_and_images() {
        let d = parse("Run `cargo` see [docs](https://x.dev) ~~old~~\n\n![logo](img/a.png)\n");
        let Kind::Paragraph(ref p) = d.blocks[0].kind else { panic!() };
        let Seg::Text(t) = &p[0] else { panic!() };
        let code = t.runs.iter().find(|(_, s)| s.code).unwrap();
        assert_eq!(&t.text[code.0.clone()], "cargo");
        let link = t.runs.iter().find(|(_, s)| s.link.is_some()).unwrap();
        assert_eq!(&t.text[link.0.clone()], "docs");
        assert_eq!(t.links, ["https://x.dev"]);
        assert!(t.runs.iter().any(|(r, s)| s.strike && &t.text[r.clone()] == "old"));
        let Kind::Paragraph(ref p) = d.blocks[1].kind else { panic!() };
        assert_eq!(
            p[0],
            Seg::Image {
                src: "img/a.png".into(),
                alt: "logo".into()
            }
        );
    }

    #[test]
    fn nested_and_task_lists() {
        let d = parse("- [x] done\n- [ ] todo\n  1. one\n  2. two\n- plain\n\n3. three\n");
        assert_eq!(kinds(&d), ["list", "list"]);
        let Kind::List { start: None, ref items } = d.blocks[0].kind else { panic!() };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].task, Some(true));
        assert_eq!(items[1].task, Some(false));
        assert_eq!(items[2].task, None);
        assert_eq!(items[1].lines, 1..4);
        let Kind::List { start: Some(1), items: ref inner } = items[1].blocks[1].kind else {
            panic!("{:?}", items[1].blocks)
        };
        assert_eq!(inner.len(), 2);
        assert_eq!(inner[0].task, None);
        let Kind::List { start: Some(3), .. } = d.blocks[1].kind else { panic!() };
    }

    #[test]
    fn tables_with_alignment() {
        let d = parse("| a | b | c |\n|:--|:-:|--:|\n| 1 | **2** | 3 |\n| 4 | 5 | 6 |\n");
        let Kind::Table { ref aligns, ref head, ref rows } = d.blocks[0].kind else { panic!() };
        assert_eq!(aligns, &[Align::Left, Align::Center, Align::Right]);
        assert_eq!(head.iter().map(plain).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(plain(&rows[0][1]), "2");
        assert_eq!(d.blocks[0].lines, 0..4);
    }

    #[test]
    fn code_quotes_html_and_front_matter() {
        let d = parse(
            "---\ntitle: x\n---\n\n```rust\nfn a() {}\n```\n\n> quoted\n> more\n\n<div>hi</div>\n",
        );
        assert_eq!(kinds(&d), ["code", "code", "quote", "html"]);
        let Kind::Code { ref lang, ref text, ref spans } = d.blocks[1].kind else { panic!() };
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(text, "fn a() {}");
        assert!(!spans[0].is_empty(), "highlighted");
        assert_eq!(d.blocks[1].lines, 4..7);
        let Kind::Html(ref h) = d.blocks[3].kind else { panic!() };
        assert_eq!(h, "<div>hi</div>");
        assert_eq!(fence_lang("ts {1,3}"), Some(Lang::TypeScript));
        assert_eq!(fence_lang("python"), Some(Lang::Python));
        assert_eq!(fence_lang("nope"), None);
    }

    fn hunk(old: Range<usize>, new: Range<usize>) -> Hunk {
        Hunk { old, new }
    }

    #[test]
    fn hunks_mark_blocks_items_and_removals() {
        let new = "# T\n\npara one\n\nnew para\n\n- a\n- b changed\n- c\n\nend\n";
        let mut d = parse(new);
        assert_eq!(kinds(&d), ["h", "p", "p", "list", "p"]);
        mark(
            &mut d,
            &[
                // "new para" and its blank line inserted.
                hunk(3..3, 4..6),
                // "- b" → "- b changed".
                hunk(5..6, 7..8),
                // Two lines removed before "end".
                hunk(8..10, 10..10),
            ],
        );
        assert_eq!(d.blocks[0].change, None);
        assert_eq!(d.blocks[1].change, None);
        assert_eq!(d.blocks[2].change, Some(Change::Added));
        let Kind::List { ref items, .. } = d.blocks[3].kind else { panic!() };
        let ch: Vec<_> = items.iter().map(|i| i.change).collect();
        assert_eq!(ch, [None, Some(Change::Modified), None]);
        assert_eq!(d.blocks[3].change, None);
        assert_eq!(d.removed, [Removed { before: 4, lines: 2 }]);
        assert_eq!(d.blocks[4].change, None);
    }

    #[test]
    fn a_partial_insertion_or_inner_deletion_modifies() {
        let mut d = parse("line one\nline two\nline three\n\nafter\n");
        mark(&mut d, &[hunk(0..0, 1..2), hunk(3..4, 5..5)]);
        assert_eq!(d.blocks[0].change, Some(Change::Modified));
        // At the very end: a marker after the last block.
        assert_eq!(d.removed, [Removed { before: 2, lines: 1 }]);
        let mut d = parse("a\nb\nc\n");
        mark(&mut d, &[hunk(1..2, 1..1)]);
        assert_eq!(d.blocks[0].change, Some(Change::Modified));
        assert!(d.removed.is_empty());
    }

    #[test]
    fn markdown_paths() {
        assert!(is_markdown("docs/README.md"));
        assert!(is_markdown("a.MARKDOWN"));
        assert!(is_markdown("page.mdx"));
        assert!(!is_markdown("a.rs"));
    }
}
