//! Syntax highlighting with tree-sitter (MIT) and MIT-licensed grammars,
//! coloured like Review's Monaco look: keywords, strings, numbers,
//! comments, types, delimiters, regexps, and (outside JavaScript/TypeScript,
//! as Monaco did) tags and attributes. Bracket pairs get Monaco's three
//! rotating colours. JSON, TOML and diffs stay plain, as in the React app.
//!
//! Runs off the main thread (`highlight` is blocking); the result is per
//! line, in source byte offsets.

use std::ops::Range;
use std::sync::OnceLock;

use tree_sitter::Language;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Files larger than this are shown plain.
pub const MAX_HIGHLIGHT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Token {
    Keyword,
    String,
    Number,
    Comment,
    Type,
    Delimiter,
    Regexp,
    Tag,
    Attr,
    /// A bracket at nesting level 0, 1 or 2 (mod 3).
    Bracket(u8),
    /// A closing bracket without its opener.
    StrayBracket,
    /// `--rv-tk-variable`: Markdown's inline and fenced code (`t.monospace`).
    Variable,
}

/// How a span is drawn: a token colour (none: the text colour), italic
/// (`t.emphasis`) and bold (`t.strong`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Style {
    pub tok: Option<Token>,
    pub italic: bool,
    pub bold: bool,
}

impl From<Token> for Style {
    fn from(tok: Token) -> Style {
        Style {
            tok: Some(tok),
            italic: false,
            bold: false,
        }
    }
}

/// The languages Pitwall highlights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    JavaScript,
    TypeScript,
    Tsx,
    Python,
    Go,
    Bash,
    Css,
    Html,
    C,
    Cpp,
    Java,
    Ruby,
    Yaml,
    Php,
    CSharp,
    Lua,
    Swift,
    Kotlin,
    Scala,
    Sql,
    Xml,
    Haskell,
    Elixir,
    Zig,
    R,
    PowerShell,
    Scss,
    OCaml,
    Nix,
    /// Markers only, as Monaco: see [`super::markdown`].
    Markdown,
}

const ALL: [Lang; 30] = [
    Lang::Rust,
    Lang::JavaScript,
    Lang::TypeScript,
    Lang::Tsx,
    Lang::Python,
    Lang::Go,
    Lang::Bash,
    Lang::Css,
    Lang::Html,
    Lang::C,
    Lang::Cpp,
    Lang::Java,
    Lang::Ruby,
    Lang::Yaml,
    Lang::Php,
    Lang::CSharp,
    Lang::Lua,
    Lang::Swift,
    Lang::Kotlin,
    Lang::Scala,
    Lang::Sql,
    Lang::Xml,
    Lang::Haskell,
    Lang::Elixir,
    Lang::Zig,
    Lang::R,
    Lang::PowerShell,
    Lang::Scss,
    Lang::OCaml,
    Lang::Nix,
];

impl Lang {
    /// The language of a path, by file name then extension (`None`: plain).
    pub fn for_path(path: &str) -> Option<Lang> {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            ".bashrc" | ".bash_profile" | ".zshrc" | ".profile" | ".bash_logout" | "pkgbuild" => {
                return Some(Lang::Bash)
            }
            "gemfile" | "rakefile" | "podfile" | "vagrantfile" => return Some(Lang::Ruby),
            "build.sbt" => return Some(Lang::Scala),
            _ => {}
        }
        let ext = lower.rsplit_once('.').map(|(_, e)| e)?;
        Some(match ext {
            "rs" => Lang::Rust,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "py" | "pyw" | "pyi" => Lang::Python,
            "go" => Lang::Go,
            "sh" | "bash" | "zsh" | "ksh" => Lang::Bash,
            "css" => Lang::Css,
            "html" | "htm" | "xhtml" => Lang::Html,
            "c" | "h" => Lang::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "c++" | "h++" | "ino" => Lang::Cpp,
            "java" => Lang::Java,
            "rb" | "gemspec" | "rake" => Lang::Ruby,
            "yml" | "yaml" => Lang::Yaml,
            "php" | "phtml" | "php3" | "php4" | "php5" | "php7" | "phps" => Lang::Php,
            "cs" | "csx" => Lang::CSharp,
            "lua" => Lang::Lua,
            "swift" => Lang::Swift,
            "kt" | "kts" => Lang::Kotlin,
            "scala" | "sc" | "sbt" => Lang::Scala,
            "sql" => Lang::Sql,
            "xml" | "xsl" | "xsd" | "xslt" | "svg" | "plist" | "csproj" | "fsproj" | "vbproj"
            | "props" | "targets" | "rss" | "atom" | "wsdl" | "xaml" => Lang::Xml,
            "hs" | "lhs" => Lang::Haskell,
            "ex" | "exs" => Lang::Elixir,
            "zig" => Lang::Zig,
            "r" => Lang::R,
            "ps1" | "psm1" | "psd1" => Lang::PowerShell,
            "scss" | "less" => Lang::Scss,
            "ml" | "mli" => Lang::OCaml,
            "nix" => Lang::Nix,
            "md" | "markdown" | "mkd" => Lang::Markdown,
            _ => return None,
        })
    }

    fn markup(self) -> bool {
        !matches!(self, Lang::JavaScript | Lang::TypeScript | Lang::Tsx)
    }

    fn index(self) -> usize {
        // Markdown has its own walker and no config.
        ALL.iter().position(|l| *l == self).unwrap_or(0)
    }

    fn grammar(self) -> (Language, String) {
        use tree_sitter_javascript as js;
        let (lang, q): (Language, String) = match self {
            Lang::Rust => (
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::JavaScript => (js::LANGUAGE.into(), js::HIGHLIGHT_QUERY.into()),
            Lang::TypeScript => (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                format!(
                    "{}\n{}",
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    js::HIGHLIGHT_QUERY
                ),
            ),
            Lang::Tsx => (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                format!(
                    "{}\n{}",
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    js::HIGHLIGHT_QUERY
                ),
            ),
            Lang::Python => (
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Go => (
                tree_sitter_go::LANGUAGE.into(),
                tree_sitter_go::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Bash => (
                tree_sitter_bash::LANGUAGE.into(),
                tree_sitter_bash::HIGHLIGHT_QUERY.into(),
            ),
            Lang::Css => (
                tree_sitter_css::LANGUAGE.into(),
                tree_sitter_css::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Html => (
                tree_sitter_html::LANGUAGE.into(),
                tree_sitter_html::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::C => (
                tree_sitter_c::LANGUAGE.into(),
                tree_sitter_c::HIGHLIGHT_QUERY.into(),
            ),
            Lang::Cpp => (
                tree_sitter_cpp::LANGUAGE.into(),
                format!(
                    "{}\n{}",
                    tree_sitter_cpp::HIGHLIGHT_QUERY,
                    tree_sitter_c::HIGHLIGHT_QUERY
                ),
            ),
            Lang::Java => (
                tree_sitter_java::LANGUAGE.into(),
                tree_sitter_java::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Ruby => (
                tree_sitter_ruby::LANGUAGE.into(),
                tree_sitter_ruby::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Yaml => (
                tree_sitter_yaml::LANGUAGE.into(),
                tree_sitter_yaml::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Php => (
                tree_sitter_php::LANGUAGE_PHP.into(),
                tree_sitter_php::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::CSharp => (
                tree_sitter_c_sharp::LANGUAGE.into(),
                tree_sitter_c_sharp::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Lua => (
                tree_sitter_lua::LANGUAGE.into(),
                tree_sitter_lua::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Swift => (
                tree_sitter_swift::LANGUAGE.into(),
                tree_sitter_swift::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Kotlin => (
                tree_sitter_kotlin_ng::LANGUAGE.into(),
                include_str!("kotlin_highlights.scm").into(),
            ),
            Lang::Scala => (
                tree_sitter_scala::LANGUAGE.into(),
                tree_sitter_scala::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Sql => (
                tree_sitter_sequel::LANGUAGE.into(),
                tree_sitter_sequel::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Xml => (
                tree_sitter_xml::LANGUAGE_XML.into(),
                tree_sitter_xml::XML_HIGHLIGHT_QUERY.into(),
            ),
            Lang::Haskell => (
                tree_sitter_haskell::LANGUAGE.into(),
                tree_sitter_haskell::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Elixir => (
                tree_sitter_elixir::LANGUAGE.into(),
                tree_sitter_elixir::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Zig => (
                tree_sitter_zig::LANGUAGE.into(),
                tree_sitter_zig::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::R => (
                tree_sitter_r::LANGUAGE.into(),
                tree_sitter_r::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::PowerShell => (
                tree_sitter_powershell::LANGUAGE.into(),
                tree_sitter_powershell::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Scss => (
                tree_sitter_scss::language(),
                tree_sitter_scss::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::OCaml => (
                tree_sitter_ocaml::LANGUAGE_OCAML.into(),
                tree_sitter_ocaml::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Nix => (
                tree_sitter_nix::LANGUAGE.into(),
                tree_sitter_nix::HIGHLIGHTS_QUERY.into(),
            ),
            Lang::Markdown => unreachable!("Markdown is walked, not queried"),
        };
        (lang, q)
    }
}

/// Capture names Pitwall colours, and the token each becomes. The longest
/// matching name wins (`punctuation.bracket` → `punctuation`).
const NAMES: [(&str, Token); 20] = [
    ("keyword", Token::Keyword),
    ("conditional", Token::Keyword),
    ("repeat", Token::Keyword),
    ("include", Token::Keyword),
    ("boolean", Token::Keyword),
    ("constant.builtin", Token::Keyword),
    ("variable.builtin", Token::Keyword),
    ("string", Token::String),
    ("character", Token::String),
    ("escape", Token::String),
    ("string.special.regex", Token::Regexp),
    ("string.regex", Token::Regexp),
    ("number", Token::Number),
    ("float", Token::Number),
    ("comment", Token::Comment),
    ("type", Token::Type),
    ("punctuation", Token::Delimiter),
    ("operator", Token::Delimiter),
    ("tag", Token::Tag),
    ("attribute", Token::Attr),
];

fn config(lang: Lang) -> Option<&'static HighlightConfiguration> {
    static CONFIGS: [OnceLock<Option<HighlightConfiguration>>; ALL.len()] =
        [const { OnceLock::new() }; ALL.len()];
    CONFIGS[lang.index()]
        .get_or_init(|| {
            let (language, query) = lang.grammar();
            let mut c =
                HighlightConfiguration::new(language, format!("{lang:?}"), &query, "", "").ok()?;
            let names: Vec<&str> = NAMES.iter().map(|(n, _)| *n).collect();
            c.configure(&names);
            Some(c)
        })
        .as_ref()
}

/// Highlighted spans of one line: source byte ranges, in order, not
/// overlapping.
pub type LineSpans = Vec<(Range<usize>, Style)>;

/// Per-line spans of a whole text (empty when the language is unknown, the
/// file too large or the parse failed).
pub fn highlight(text: &str, lang: Option<Lang>) -> Vec<LineSpans> {
    let line_starts = line_starts(text);
    let mut out: Vec<LineSpans> = vec![Vec::new(); line_starts.len()];
    let Some(lang) = lang else { return out };
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return out;
    }
    if lang == Lang::Markdown {
        let Some(md) = super::markdown::styles(text) else { return out };
        for (r, st) in md.spans {
            push_span(&mut out, &line_starts, text, r, st);
        }
        brackets(text, &line_starts, &md.quiet, &mut out);
        return out;
    }
    let Some((spans, quiet)) = token_spans(text, lang) else {
        return out;
    };
    for (r, tok) in spans {
        push_span(&mut out, &line_starts, text, r, tok.into());
    }
    brackets(text, &line_starts, &quiet, &mut out);
    out
}

/// Token spans, and the ranges where brackets don't count.
pub(super) type Spans = (Vec<(Range<usize>, Token)>, Vec<Range<usize>>);

/// Whole-text token spans of a queried language, and the ranges where
/// brackets don't count (strings, comments, regexps). `None`: no parse.
pub(super) fn token_spans(text: &str, lang: Lang) -> Option<Spans> {
    let cfg = config(lang)?;
    let mut hl = Highlighter::new();
    let events = hl.highlight(cfg, text.as_bytes(), None, |_| None).ok()?;
    let mut stack: Vec<Option<Token>> = Vec::new();
    let mut spans = Vec::new();
    let mut quiet: Vec<Range<usize>> = Vec::new();
    for ev in events {
        match ev.ok()? {
            HighlightEvent::HighlightStart(h) => stack.push(NAMES.get(h.0).map(|(_, t)| *t)),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let Some(tok) = stack.iter().rev().find_map(|t| *t) else {
                    continue;
                };
                let tok = match tok {
                    Token::Tag | Token::Attr if !lang.markup() => continue,
                    t => t,
                };
                if matches!(tok, Token::String | Token::Comment | Token::Regexp) {
                    quiet.push(start..end);
                }
                spans.push((start..end, tok));
            }
        }
    }
    Some((spans, quiet))
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut v = vec![0];
    v.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    v
}

/// Split a whole-text span over the lines it covers.
fn push_span(out: &mut [LineSpans], starts: &[usize], text: &str, r: Range<usize>, tok: Style) {
    let mut line = starts.partition_point(|s| *s <= r.start).saturating_sub(1);
    let mut from = r.start;
    while from < r.end && line < starts.len() {
        let line_end = starts.get(line + 1).map(|s| s - 1).unwrap_or(text.len());
        let to = r.end.min(line_end);
        if to > from {
            let (a, b) = (from - starts[line], to - starts[line]);
            let spans = &mut out[line];
            match spans.last_mut() {
                Some((last, t)) if *t == tok && last.end == a => last.end = b,
                _ => spans.push((a..b, tok)),
            }
        }
        line += 1;
        from = starts.get(line).copied().unwrap_or(r.end);
    }
}

/// Bracket pair colours over the existing spans.
fn brackets(text: &str, starts: &[usize], quiet: &[Range<usize>], out: &mut [LineSpans]) {
    const OPEN: &[u8] = b"([{";
    const CLOSE: &[u8] = b")]}";
    let mut stack: Vec<u8> = Vec::new();
    let mut q = 0;
    let bytes = text.as_bytes();
    let mut found: Vec<(usize, Token)> = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        while q < quiet.len() && quiet[q].end <= i {
            q += 1;
        }
        if q < quiet.len() && quiet[q].start <= i {
            continue;
        }
        if let Some(o) = OPEN.iter().position(|c| *c == b) {
            found.push((i, Token::Bracket((stack.len() % 3) as u8)));
            stack.push(OPEN[o]);
        } else if let Some(c) = CLOSE.iter().position(|c| *c == b) {
            if stack.last() == Some(&OPEN[c]) {
                stack.pop();
                found.push((i, Token::Bracket((stack.len() % 3) as u8)));
            } else {
                found.push((i, Token::StrayBracket));
            }
        }
    }
    for (pos, tok) in found {
        let line = starts.partition_point(|s| *s <= pos).saturating_sub(1);
        let col = pos - starts[line];
        overlay(&mut out[line], col..col + 1, tok);
    }
}

/// Put bracket colour `tok` over `r` in a line's spans, splitting what was
/// there (italic and bold stay, as a CSS colour class over them would).
fn overlay(spans: &mut LineSpans, r: Range<usize>, tok: Token) {
    let mut next: LineSpans = Vec::with_capacity(spans.len() + 2);
    let mut placed = false;
    let under = spans
        .iter()
        .find(|(s, _)| s.start < r.end && s.end > r.start)
        .map(|(_, st)| *st)
        .unwrap_or_default();
    let tok = Style {
        tok: Some(tok),
        ..under
    };
    for (s, t) in spans.drain(..) {
        if s.end <= r.start || s.start >= r.end {
            if !placed && s.start >= r.end {
                next.push((r.clone(), tok));
                placed = true;
            }
            next.push((s, t));
            continue;
        }
        if s.start < r.start {
            next.push((s.start..r.start, t));
        }
        if !placed {
            next.push((r.clone(), tok));
            placed = true;
        }
        if s.end > r.end {
            next.push((r.end..s.end, t));
        }
    }
    if !placed {
        next.push((r, tok));
    }
    *spans = next;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(line: &str, spans: &LineSpans) -> Vec<(String, Token)> {
        spans
            .iter()
            .filter_map(|(r, t)| Some((line[r.clone()].to_string(), t.tok?)))
            .collect()
    }

    #[test]
    fn languages_by_name_and_extension() {
        assert_eq!(Lang::for_path("src/main.rs"), Some(Lang::Rust));
        assert_eq!(Lang::for_path("App.TSX"), Some(Lang::Tsx));
        assert_eq!(Lang::for_path("x/Gemfile"), Some(Lang::Ruby));
        assert_eq!(Lang::for_path("data.json"), None, "JSON stays plain");
        assert_eq!(Lang::for_path("Cargo.toml"), None);
        assert_eq!(Lang::for_path("README"), None);
        assert_eq!(Lang::for_path("Main.kt"), Some(Lang::Kotlin));
        assert_eq!(Lang::for_path("icon.svg"), Some(Lang::Xml));
        assert_eq!(Lang::for_path("theme.less"), Some(Lang::Scss));
        assert_eq!(Lang::for_path("analysis.R"), Some(Lang::R));
    }

    #[test]
    fn kotlin_query_colours_keywords_and_strings() {
        let src = "fun main() { val s = \"hi\" }\n";
        let out = highlight(src, Some(Lang::Kotlin));
        let l0 = tokens(src.trim_end(), &out[0]);
        assert!(l0.contains(&("fun".into(), Token::Keyword)), "{l0:?}");
        assert!(l0.iter().any(|(_, t)| *t == Token::String), "{l0:?}");
    }

    #[test]
    fn every_grammar_loads() {
        let mut bad = Vec::new();
        for l in ALL {
            if config(l).is_none() {
                let (language, query) = l.grammar();
                let e = HighlightConfiguration::new(language, "x", &query, "", "").err();
                bad.push(format!("{l:?}: {e:?}"));
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }

    #[test]
    fn rust_keywords_strings_comments() {
        let src = "fn main() {\n    let s = \"hi (x)\"; // note\n}\n";
        let out = highlight(src, Some(Lang::Rust));
        let l0 = tokens("fn main() {", &out[0]);
        assert!(l0.contains(&("fn".into(), Token::Keyword)), "{l0:?}");
        assert!(
            l0.contains(&("(".into(), Token::Bracket(1)))
                || l0.contains(&("(".into(), Token::Bracket(0)))
        );
        let line1 = "    let s = \"hi (x)\"; // note";
        let l1 = tokens(line1, &out[1]);
        assert!(
            l1.contains(&("\"hi (x)\"".into(), Token::String)),
            "brackets in strings stay: {l1:?}"
        );
        assert!(l1.contains(&("// note".into(), Token::Comment)), "{l1:?}");
        let l2 = tokens("}", &out[2]);
        assert_eq!(l2, [("}".into(), Token::Bracket(0))]);
    }

    #[test]
    fn nesting_levels_rotate_and_strays_show() {
        let out = highlight("a([{x}]) )", Some(Lang::JavaScript));
        let toks: Vec<Token> = out[0]
            .iter()
            .filter_map(|(_, t)| t.tok)
            .filter(|t| matches!(t, Token::Bracket(_) | Token::StrayBracket))
            .collect();
        assert_eq!(
            toks,
            [
                Token::Bracket(0),
                Token::Bracket(1),
                Token::Bracket(2),
                Token::Bracket(2),
                Token::Bracket(1),
                Token::Bracket(0),
                Token::StrayBracket
            ]
        );
    }

    #[test]
    fn multi_line_comments_split_per_line_and_unicode_offsets_hold() {
        let src = "/* გამარჯობა\n   end */ x";
        let out = highlight(src, Some(Lang::C));
        assert_eq!(out[0], vec![(0.."/* გამარჯობა".len(), Token::Comment.into())]);
        assert_eq!(out[1][0], (0..9, Token::Comment.into()));
    }

    #[test]
    fn plain_without_a_language() {
        let out = highlight("a\nb", None);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|l| l.is_empty()));
    }
}
