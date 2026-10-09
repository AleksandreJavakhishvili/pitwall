//! Markdown as the React app's Review colours it (`editorSetup.ts`
//! `markdownSupport`, CodeMirror's Markdown with Monaco's colours):
//! headings in the keyword colour, block quotes in the comment colour, list
//! markers as keywords, fences and their info as strings, inline and
//! fenced code in the variable colour, link brackets and URLs as strings
//! (the link text stays plain), images as strings, entities as strings,
//! HTML as HTML, emphasis italic and strong bold. Parsed with
//! tree-sitter-md (MIT): the block grammar, then the inline grammar over
//! each paragraph's text.
//!
//! Where two colours cover the same text, the one declared later in
//! `baseSpecs` wins, as the CSS rules do (a link in a quote stays the
//! comment colour; a link in a heading turns string).

use std::ops::Range;

use tree_sitter::{Node, Parser};

use super::highlight::{token_spans, Lang, Style, Token};

/// Styled whole-text spans, in order and not overlapping, and the ranges
/// where brackets don't count (links, URLs, HTML comments).
pub struct Styled {
    pub spans: Vec<(Range<usize>, Style)>,
    pub quiet: Vec<Range<usize>>,
}

/// One layer of styling over a byte range.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Layer {
    Color(Token),
    Italic,
    Bold,
}

/// `baseSpecs` then `markupSpecs` order: a later colour beats an earlier.
fn rank(t: Token) -> u8 {
    match t {
        Token::Keyword => 0,
        Token::String => 1,
        Token::Number => 2,
        Token::Comment => 3,
        Token::Type => 4,
        Token::Delimiter => 5,
        Token::Regexp => 6,
        Token::Variable => 7,
        Token::Tag => 8,
        Token::Attr => 9,
        Token::Bracket(_) | Token::StrayBracket => 10,
    }
}

struct Walk<'a> {
    text: &'a str,
    layers: Vec<(Range<usize>, Layer)>,
    quiet: Vec<Range<usize>>,
}

impl Walk<'_> {
    fn color(&mut self, r: Range<usize>, t: Token) {
        if r.end > r.start {
            self.layers.push((r, Layer::Color(t)));
        }
    }

    /// A node's range without its trailing line break or spaces.
    fn trimmed(&self, n: Node) -> Range<usize> {
        let r = n.byte_range();
        let s = &self.text[r.clone()];
        r.start..r.start + s.trim_end().len()
    }

    /// Text in another grammar (HTML), its colours shifted into place.
    fn embed(&mut self, r: Range<usize>, lang: Lang) {
        if let Some((spans, quiet)) = token_spans(&self.text[r.clone()], lang) {
            for (s, t) in spans {
                self.color(r.start + s.start..r.start + s.end, t);
            }
            self.quiet.extend(quiet.into_iter().map(|q| r.start + q.start..r.start + q.end));
        }
    }

    fn block(&mut self, n: Node, inline: &mut Parser) {
        match n.kind() {
            "atx_heading" | "setext_heading" => self.color(self.trimmed(n), Token::Keyword),
            "block_quote" => self.color(self.trimmed(n), Token::Comment),
            k if k.starts_with("list_marker_") => self.color(self.trimmed(n), Token::Keyword),
            "fenced_code_block_delimiter" | "info_string" => {
                self.color(self.trimmed(n), Token::String)
            }
            "code_fence_content" | "indented_code_block" => {
                self.color(n.byte_range(), Token::Variable);
                return;
            }
            "html_block" => {
                self.embed(n.byte_range(), Lang::Html);
                return;
            }
            "link_destination" => {
                self.color(n.byte_range(), Token::String);
                self.quiet.push(n.byte_range());
            }
            "link_title" => self.color(n.byte_range(), Token::String),
            "inline" | "pipe_table_cell" => {
                self.inline(n, inline);
                return;
            }
            _ => {}
        }
        let mut c = n.walk();
        for child in n.children(&mut c) {
            self.block(child, inline);
        }
    }

    /// Parse one `inline` node's text (minus the block markers inside it,
    /// as tree-sitter-md's own parser does) with the inline grammar.
    fn inline(&mut self, n: Node, parser: &mut Parser) {
        let mut ranges = Vec::new();
        let mut range = n.range();
        let mut c = n.walk();
        for child in n.named_children(&mut c) {
            let cr = child.range();
            ranges.push(tree_sitter::Range {
                start_byte: range.start_byte,
                start_point: range.start_point,
                end_byte: cr.start_byte,
                end_point: cr.start_point,
            });
            range.start_byte = cr.end_byte;
            range.start_point = cr.end_point;
        }
        ranges.push(range);
        ranges.retain(|r| r.end_byte > r.start_byte);
        if ranges.is_empty() || parser.set_included_ranges(&ranges).is_err() {
            return;
        }
        let Some(tree) = parser.parse(self.text, None) else { return };
        self.inline_node(tree.root_node());
    }

    fn inline_node(&mut self, n: Node) {
        let r = n.byte_range();
        match n.kind() {
            "emphasis" => self.layers.push((r, Layer::Italic)),
            "strong_emphasis" => self.layers.push((r, Layer::Bold)),
            "code_span" => {
                self.color(r, Token::Variable);
                return;
            }
            "image" => {
                self.color(r, Token::String);
                return;
            }
            "uri_autolink" | "email_autolink" => {
                // The URL, not its angle brackets.
                if r.end - r.start >= 2 {
                    self.color(r.start + 1..r.end - 1, Token::String);
                }
                self.quiet.push(r);
                return;
            }
            "entity_reference" | "numeric_character_reference" => {
                self.color(r, Token::String);
                return;
            }
            "html_tag" => {
                self.embed(r, Lang::Html);
                return;
            }
            "inline_link" | "full_reference_link" | "collapsed_reference_link"
            | "shortcut_link" => {
                self.quiet.push(r.clone());
                let mut c = n.walk();
                // The first bracket pair (`LinkMark`s), the URL and title.
                let mut marks = 0;
                for child in n.children(&mut c) {
                    match child.kind() {
                        "[" | "]" if marks < 2 => {
                            marks += 1;
                            self.color(child.byte_range(), Token::String);
                        }
                        "(" | ")" | "link_destination" | "link_title" => {
                            self.color(child.byte_range(), Token::String)
                        }
                        _ => self.inline_node(child),
                    }
                }
                return;
            }
            _ => {}
        }
        let mut c = n.walk();
        for child in n.children(&mut c) {
            self.inline_node(child);
        }
    }

    /// Flatten the layers into ordered, non-overlapping spans (one sweep
    /// over the layer edges, counting what is open).
    fn finish(mut self) -> Styled {
        // (position, opens?, layer)
        let mut edges: Vec<(usize, bool, Layer)> = Vec::with_capacity(self.layers.len() * 2);
        for (r, l) in &self.layers {
            edges.push((r.start, true, *l));
            edges.push((r.end, false, *l));
        }
        edges.sort_by_key(|e| e.0);
        let mut open_colors: Vec<(Token, u32)> = Vec::new();
        let (mut italic, mut bold) = (0u32, 0u32);
        let mut spans: Vec<(Range<usize>, Style)> = Vec::new();
        let mut i = 0;
        while i < edges.len() {
            let at = edges[i].0;
            while i < edges.len() && edges[i].0 == at {
                let (_, opens, l) = edges[i];
                match l {
                    Layer::Italic if opens => italic += 1,
                    Layer::Italic => italic -= 1,
                    Layer::Bold if opens => bold += 1,
                    Layer::Bold => bold -= 1,
                    Layer::Color(t) => {
                        let slot = open_colors.iter_mut().find(|(k, _)| *k == t);
                        match (slot, opens) {
                            (Some(s), true) => s.1 += 1,
                            (Some(s), false) => s.1 -= 1,
                            (None, true) => open_colors.push((t, 1)),
                            (None, false) => {}
                        }
                    }
                }
                i += 1;
            }
            let Some(&(next, _, _)) = edges.get(i) else { break };
            let st = Style {
                tok: open_colors
                    .iter()
                    .filter(|(_, n)| *n > 0)
                    .map(|(t, _)| *t)
                    .max_by_key(|t| rank(*t)),
                italic: italic > 0,
                bold: bold > 0,
            };
            if st == Style::default() || next <= at {
                continue;
            }
            match spans.last_mut() {
                Some((last, s)) if *s == st && last.end == at => last.end = next,
                _ => spans.push((at..next, st)),
            }
        }
        self.quiet.sort_by_key(|r| (r.start, r.end));
        let mut quiet: Vec<Range<usize>> = Vec::new();
        for q in self.quiet {
            match quiet.last_mut() {
                Some(last) if q.start <= last.end => last.end = last.end.max(q.end),
                _ => quiet.push(q),
            }
        }
        Styled { spans, quiet }
    }
}

/// The styled spans of a Markdown text (`None`: it didn't parse).
pub fn styles(text: &str) -> Option<Styled> {
    let mut block = Parser::new();
    block.set_language(&tree_sitter_md::LANGUAGE.into()).ok()?;
    let tree = block.parse(text, None)?;
    let mut inline = Parser::new();
    inline
        .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
        .ok()?;
    let mut w = Walk {
        text,
        layers: Vec::new(),
        quiet: Vec::new(),
    };
    w.block(tree.root_node(), &mut inline);
    Some(w.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (text, colour, italic, bold) of each span.
    fn spans(src: &str) -> Vec<(String, Option<Token>, bool, bool)> {
        styles(src)
            .unwrap()
            .spans
            .into_iter()
            .map(|(r, s)| (src[r].to_string(), s.tok, s.italic, s.bold))
            .collect()
    }

    fn has(v: &[(String, Option<Token>, bool, bool)], text: &str, tok: Option<Token>) -> bool {
        v.iter().any(|(t, k, _, _)| t == text && *k == tok)
    }

    #[test]
    fn headings_quotes_and_list_markers() {
        let v = spans("# Title\n\n> quote\n\n- item\n1. one\n");
        assert!(has(&v, "# Title", Some(Token::Keyword)), "{v:?}");
        assert!(has(&v, "> quote", Some(Token::Comment)), "{v:?}");
        assert!(has(&v, "-", Some(Token::Keyword)), "{v:?}");
        assert!(has(&v, "1.", Some(Token::Keyword)), "{v:?}");
        assert!(!v.iter().any(|(t, ..)| t.contains("item")), "{v:?}");
    }

    #[test]
    fn code_is_the_variable_colour_and_fences_strings() {
        let v = spans("a `code` b\n\n```js\nlet a = 1;\n```\n");
        assert!(has(&v, "`code`", Some(Token::Variable)), "{v:?}");
        // The fence and its info, as one string run.
        assert!(has(&v, "```js", Some(Token::String)), "{v:?}");
        assert!(has(&v, "```", Some(Token::String)), "{v:?}");
        assert!(has(&v, "let a = 1;\n", Some(Token::Variable)), "{v:?}");
    }

    #[test]
    fn links_colour_their_marks_and_url_not_their_text() {
        let v = spans("see [text](http://x.io) end\n");
        assert!(has(&v, "[", Some(Token::String)), "{v:?}");
        assert!(has(&v, "](http://x.io)", Some(Token::String)), "{v:?}");
        assert!(!v.iter().any(|(t, ..)| t.contains("text")), "{v:?}");
    }

    #[test]
    fn emphasis_is_italic_and_keeps_the_block_colour() {
        let v = spans("plain *em* and **strong**\n\n# H *e*\n");
        assert!(v.contains(&("*em*".into(), None, true, false)), "{v:?}");
        assert!(v.contains(&("**strong**".into(), None, false, true)), "{v:?}");
        assert!(v.contains(&("*e*".into(), Some(Token::Keyword), true, false)), "{v:?}");
    }

    #[test]
    fn a_later_rule_wins_where_colours_meet() {
        // A link in a quote stays a comment; code in a heading is code.
        let v = spans("> [a](u)\n\n# H `c`\n");
        assert!(!v.iter().any(|(_, k, ..)| *k == Some(Token::String)), "{v:?}");
        assert!(has(&v, "`c`", Some(Token::Variable)), "{v:?}");
    }

    #[test]
    fn html_and_entities() {
        let v = spans("x &amp; y\n\n<div class=\"a\">t</div>\n");
        assert!(has(&v, "&amp;", Some(Token::String)), "{v:?}");
        assert!(has(&v, "div", Some(Token::Tag)), "{v:?}");
        assert!(has(&v, "class", Some(Token::Attr)), "{v:?}");
    }
}
