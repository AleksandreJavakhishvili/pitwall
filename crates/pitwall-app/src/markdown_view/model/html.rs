//! Raw HTML in Markdown drawn the way GitHub shows it rather than as
//! source: tags become formatting (paragraphs, headings, `align="center"`,
//! line breaks, rules, list items, links, images, bold, italic, code,
//! strikethrough, preformatted text); comments, scripts and styles vanish;
//! entities are decoded; other tags keep only their text. Tolerant: any
//! input gives something.

use std::ops::Range;

use super::{highlight, Block, Inline, Kind, Seg, Style, Text};

pub enum Token<'a> {
    Text(&'a str),
    Open {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Close(String),
}

impl Token<'_> {
    /// An attribute of an opening tag (decoded).
    pub fn attr(&self, key: &str) -> Option<String> {
        match self {
            Token::Open { attrs, .. } => attrs.iter().find(|(k, _)| k == key).map(|(_, v)| decode(v)),
            _ => None,
        }
    }
}

/// `s` as text and tags (comments and `<!…>` / `<?…>` dropped).
pub fn tokens(s: &str) -> Vec<Token<'_>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text = 0;
    let skip_to = |from: usize, pat: &str| s[from..].find(pat).map_or(s.len(), |e| from + e + pat.len());
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &s[i..];
        let end = if rest.starts_with("<!--") {
            Some((skip_to(i + 4, "-->"), None))
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            Some((skip_to(i, ">"), None))
        } else if rest.starts_with("</") && b.get(i + 2).is_some_and(u8::is_ascii_alphabetic) {
            let e = skip_to(i, ">");
            let name: String = s[i + 2..e]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            Some((e, Some(Token::Close(name.to_ascii_lowercase()))))
        } else if b.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
            let (e, tok) = open_tag(s, i);
            Some((e, Some(tok)))
        } else {
            None
        };
        match end {
            Some((e, tok)) => {
                if text < i {
                    out.push(Token::Text(&s[text..i]));
                }
                out.extend(tok);
                i = e;
                text = e;
            }
            None => i += 1,
        }
    }
    if text < s.len() {
        out.push(Token::Text(&s[text..]));
    }
    out
}

/// The opening tag at `at` (`<name attr="v" …>`): where it ends and it.
fn open_tag(s: &str, at: usize) -> (usize, Token<'_>) {
    let b = s.as_bytes();
    let mut i = at + 1;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') {
        i += 1;
    }
    let name = s[at + 1..i].to_ascii_lowercase();
    let mut attrs = Vec::new();
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == b'>' {
            i += 1;
            break;
        }
        let k = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let key = s[k..i].to_ascii_lowercase();
        let mut value = String::new();
        if b.get(i) == Some(&b'=') {
            i += 1;
            match b.get(i) {
                Some(&q) if q == b'"' || q == b'\'' => {
                    let e = s[i + 1..].find(q as char).map_or(s.len(), |e| i + 1 + e);
                    value = s[i + 1..e].to_string();
                    i = (e + 1).min(s.len());
                }
                _ => {
                    let v = i;
                    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                        i += 1;
                    }
                    value = s[v..i].to_string();
                }
            }
        }
        if key.is_empty() {
            i += 1;
        } else {
            attrs.push((key, value));
        }
    }
    (i, Token::Open { name, attrs })
}

/// `&amp;`, `&#39;`, `&#x2014;` and the common named entities.
pub fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|e| *e <= 10);
        let ch = end.and_then(|e| {
            let name = &rest[1..e];
            let named = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                "middot" => Some('·'),
                "copy" => Some('©'),
                "reg" => Some('®'),
                "trade" => Some('™'),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "hellip" => Some('…'),
                "rarr" => Some('→'),
                "larr" => Some('←'),
                "times" => Some('×'),
                "bull" => Some('•'),
                _ => None,
            };
            named.or_else(|| {
                let n = name.strip_prefix('#')?;
                let code = match n.strip_prefix(['x', 'X']) {
                    Some(h) => u32::from_str_radix(h, 16).ok()?,
                    None => n.parse().ok()?,
                };
                char::from_u32(code)
            })
        });
        match (ch, end) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Tags that start and end a block of their own.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "center"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "nav"
            | "aside"
            | "figure"
            | "figcaption"
            | "blockquote"
            | "details"
            | "summary"
            | "ul"
            | "ol"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "address"
    )
}

fn heading(name: &str) -> Option<u8> {
    match name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

/// An `<img>`'s image: its source, alt text and `width` (pixels).
pub fn image(tok: &Token) -> Seg {
    Seg::Image {
        src: tok.attr("src").unwrap_or_default(),
        alt: tok.attr("alt").unwrap_or_default(),
        width: tok
            .attr("width")
            .and_then(|w| w.trim_end_matches("px").trim().parse::<f32>().ok())
            .filter(|w| *w > 0.),
    }
}

/// Builds blocks from an HTML block.
#[derive(Default)]
struct Conv {
    out: Vec<(Block, bool)>,
    lines: Range<usize>,
    segs: Inline,
    text: Text,
    bold: u32,
    italic: u32,
    code: u32,
    strike: u32,
    links: Vec<String>,
    /// Open block tags and whether each centres its content.
    open: Vec<(String, bool)>,
    heading: Option<u8>,
    /// Inside `<pre>`: its text so far.
    pre: Option<String>,
    /// Inside `<script>`, `<style>`…: dropped up to this end tag.
    skip: Option<String>,
    /// Numbering of the open lists (`None`: bullets).
    lists: Vec<Option<u64>>,
}

impl Conv {
    fn center(&self) -> bool {
        self.open.iter().any(|(_, c)| *c)
    }

    fn style(&mut self) -> Style {
        let link = self.links.last().map(|u| {
            if self.text.links.last() != Some(u) {
                self.text.links.push(u.clone());
            }
            self.text.links.len() - 1
        });
        Style {
            bold: self.bold > 0,
            italic: self.italic > 0,
            code: self.code > 0,
            strike: self.strike > 0,
            note: false,
            link,
        }
    }

    /// Text with its white space collapsed, as HTML shows it.
    fn push_text(&mut self, raw: &str) {
        let s = decode(raw);
        let mut out = String::with_capacity(s.len());
        let mut space = self.text.text.is_empty() && self.segs.is_empty()
            || self.text.text.ends_with([' ', '\n']);
        for c in s.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                if !space {
                    out.push(' ');
                    space = true;
                }
            } else {
                out.push(c);
                space = false;
            }
        }
        let style = self.style();
        self.text.push(&out, style);
    }

    /// The text so far, trailing white space dropped (`None`: none).
    fn take_text(&mut self) -> Option<Text> {
        let mut t = std::mem::take(&mut self.text);
        let keep = t.text.trim_end_matches(' ').len();
        t.text.truncate(keep);
        t.runs.retain_mut(|(r, _)| {
            r.end = r.end.min(keep);
            r.start < r.end
        });
        (!t.text.trim().is_empty()).then_some(t)
    }

    fn flush(&mut self) {
        if let Some(t) = self.take_text() {
            self.segs.push(Seg::Text(t));
        }
        if self.segs.is_empty() {
            return;
        }
        let inline = std::mem::take(&mut self.segs);
        let kind = match self.heading {
            Some(n) => Kind::Heading(n, inline),
            None => Kind::Paragraph(inline),
        };
        self.push(kind);
    }

    fn push(&mut self, kind: Kind) {
        let b = Block {
            kind,
            lines: self.lines.clone(),
            change: None,
        };
        let center = self.center();
        self.out.push((b, center));
    }

    fn open(&mut self, tok: &Token, name: &str) {
        if self.skip.is_some() {
            return;
        }
        if self.pre.is_some() {
            if name == "br" {
                self.pre.as_mut().expect("pre").push('\n');
            }
            return;
        }
        let center = tok
            .attr("align")
            .is_some_and(|a| a.eq_ignore_ascii_case("center"))
            || name == "center";
        match name {
            "br" => {
                let style = self.style();
                self.text.push("\n", style);
            }
            "hr" => {
                self.flush();
                self.push(Kind::Rule);
            }
            "img" => {
                if let Some(t) = self.take_text() {
                    self.segs.push(Seg::Text(t));
                }
                self.segs.push(image(tok));
            }
            "a" => self.links.push(tok.attr("href").unwrap_or_default()),
            "b" | "strong" => self.bold += 1,
            "i" | "em" | "cite" => self.italic += 1,
            "code" | "kbd" | "tt" | "samp" => self.code += 1,
            "s" | "del" | "strike" => self.strike += 1,
            "pre" => {
                self.flush();
                self.pre = Some(String::new());
            }
            "script" | "style" | "title" | "head" | "template" | "noscript" => {
                self.skip = Some(name.to_string())
            }
            "td" | "th" => {
                if !self.text.text.is_empty() {
                    let style = self.style();
                    self.text.push(" · ", style);
                }
                if name == "th" {
                    self.bold += 1;
                }
            }
            _ => {
                if let Some(n) = heading(name) {
                    self.flush();
                    self.heading = Some(n);
                    self.open.push((name.to_string(), center));
                } else if is_block(name) {
                    self.flush();
                    match name {
                        "ul" => self.lists.push(None),
                        "ol" => self.lists.push(Some(
                            tok.attr("start").and_then(|s| s.parse().ok()).unwrap_or(1),
                        )),
                        "li" => {
                            let marker = match self.lists.last_mut() {
                                Some(Some(n)) => {
                                    *n += 1;
                                    format!("{}. ", *n - 1)
                                }
                                _ => "• ".to_string(),
                            };
                            self.text.push(&marker, Style::default());
                        }
                        _ => {}
                    }
                    self.open.push((name.to_string(), center));
                }
            }
        }
    }

    fn close(&mut self, name: &str) {
        if let Some(s) = &self.skip {
            if s == name {
                self.skip = None;
            }
            return;
        }
        if name == "pre" {
            if let Some(text) = self.pre.take() {
                let text = text.trim_matches('\n').to_string();
                let spans = highlight::highlight(&text, None);
                self.push(Kind::Code {
                    lang: None,
                    text,
                    spans,
                });
            }
            return;
        }
        if self.pre.is_some() {
            return;
        }
        let dec = |n: &mut u32| *n = n.saturating_sub(1);
        match name {
            "a" => {
                self.links.pop();
            }
            "b" | "strong" => dec(&mut self.bold),
            "i" | "em" | "cite" => dec(&mut self.italic),
            "code" | "kbd" | "tt" | "samp" => dec(&mut self.code),
            "s" | "del" | "strike" => dec(&mut self.strike),
            "th" => dec(&mut self.bold),
            _ => {
                let h = heading(name).is_some();
                if !(h || is_block(name)) {
                    return;
                }
                self.flush();
                if h {
                    self.heading = None;
                }
                if matches!(name, "ul" | "ol") {
                    self.lists.pop();
                }
                if let Some(at) = self.open.iter().rposition(|(n, _)| n == name) {
                    self.open.truncate(at);
                }
            }
        }
    }
}

/// An HTML block as blocks (centred runs grouped), all on `lines`.
pub fn blocks(raw: &str, lines: Range<usize>) -> Vec<Block> {
    let mut c = Conv {
        lines: lines.clone(),
        ..Conv::default()
    };
    for tok in tokens(raw) {
        match &tok {
            Token::Text(s) => {
                if c.skip.is_some() {
                } else if let Some(p) = c.pre.as_mut() {
                    p.push_str(&decode(s));
                } else {
                    c.push_text(s);
                }
            }
            Token::Open { name, .. } => c.open(&tok, &name.clone()),
            Token::Close(name) => c.close(name),
        }
    }
    if c.pre.is_some() {
        c.close("pre");
    }
    c.flush();
    // Runs of centred blocks share a group.
    let mut out: Vec<Block> = Vec::new();
    for (b, center) in c.out {
        if !center {
            out.push(b);
            continue;
        }
        match out.last_mut() {
            Some(Block {
                kind: Kind::Group { center: true, blocks },
                ..
            }) => blocks.push(b),
            _ => out.push(Block {
                kind: Kind::Group {
                    center: true,
                    blocks: vec![b],
                },
                lines: lines.clone(),
                change: None,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown_view::model::plain;

    fn kinds(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| match &b.kind {
                Kind::Paragraph(i) => format!("p:{}", plain(i)),
                Kind::Heading(n, i) => format!("h{n}:{}", plain(i)),
                Kind::Group { center, blocks } => format!("group{center}{:?}", kinds(blocks)),
                Kind::Code { text, .. } => format!("pre:{text}"),
                Kind::Rule => "hr".into(),
                _ => "?".into(),
            })
            .collect()
    }

    #[test]
    fn entities_and_tags() {
        assert_eq!(decode("a &amp; b &lt;c&gt; &#39;d&#x27; &middot; &bogus; &"), "a & b <c> 'd' · &bogus; &");
        let t = tokens("x<!-- c --><a href=\"u?a=1&amp;b\" class=k>y</a><br/>< z");
        let names: Vec<String> = t
            .iter()
            .map(|t| match t {
                Token::Text(s) => format!("t:{s}"),
                Token::Open { name, .. } => format!("<{name}"),
                Token::Close(n) => format!("/{n}"),
            })
            .collect();
        assert_eq!(names, ["t:x", "<a", "t:y", "/a", "<br", "t:< z"]);
        assert_eq!(t[1].attr("href").as_deref(), Some("u?a=1&b"));
        assert_eq!(t[1].attr("class").as_deref(), Some("k"));
    }

    #[test]
    fn a_readme_header() {
        let raw = "<p align=\"center\">\n  <img src=\"icon.svg\" alt=\"Icon\" width=\"112\" />\n</p>\n\
                   <h1 align=\"center\">Pitwall</h1>\n\
                   <p align=\"center\">\n  <strong>Bold.</strong><br />\n  Next   line,<br />\n  end.\n</p>\n\
                   <p>Plain <a href=\"https://x.dev\">link</a> &amp; <code>code</code>.</p>\n\
                   <!-- hidden --><script>no()</script><hr>\n<pre>  a\n  b</pre>\n\
                   <ul><li>one</li><li>two</li></ul><ol start=\"3\"><li>three</li></ol>";
        let b = blocks(raw, 0..12);
        assert_eq!(
            kinds(&b),
            [
                "grouptrue[\"p:Icon\", \"h1:Pitwall\", \"p:Bold.\\nNext line,\\nend.\"]",
                "p:Plain link & code.",
                "hr",
                "pre:  a\n  b",
                "p:• one",
                "p:• two",
                "p:3. three",
            ]
        );
        let Kind::Group { blocks: g, .. } = &b[0].kind else { panic!() };
        let Kind::Paragraph(i) = &g[0].kind else { panic!() };
        assert_eq!(
            i[0],
            Seg::Image {
                src: "icon.svg".into(),
                alt: "Icon".into(),
                width: Some(112.)
            }
        );
        let Kind::Paragraph(i) = &b[1].kind else { panic!() };
        let Seg::Text(t) = &i[0] else { panic!() };
        assert_eq!(t.links, ["https://x.dev"]);
        let styles: Vec<(&str, Style)> =
            t.runs.iter().map(|(r, s)| (&t.text[r.clone()], *s)).collect();
        assert_eq!(styles[1].0, "link");
        assert_eq!(styles[1].1.link, Some(0));
        assert!(styles[3].1.code);
        assert!(b.iter().all(|b| b.lines == (0..12)));
    }

    #[test]
    fn nothing_visible_is_nothing() {
        assert!(blocks("<!-- just a comment -->", 0..1).is_empty());
        assert!(blocks("<div>\n</div>", 0..2).is_empty());
    }
}
