//! Find in a file (Review's Monaco-like find widget): plain text or a
//! regular expression, Match Case, Match Whole Word, optionally only inside
//! a selection. Matches are per line, in shown columns. Pure.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

use super::text::{Document, Pos};

/// Monaco stops counting here ("19999+").
pub const MAX_MATCHES: usize = 19_999;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn compile(&self) -> Result<Regex, String> {
        let pat = if self.regex {
            self.text.clone()
        } else {
            regex::escape(&self.text)
        };
        let pat = if self.word {
            format!(r"\b(?:{pat})\b")
        } else {
            pat
        };
        RegexBuilder::new(&pat)
            .case_insensitive(!self.case)
            .multi_line(true)
            .build()
            .map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub cols: Range<usize>,
}

impl Match {
    pub fn start(&self) -> Pos {
        Pos::new(self.line, self.cols.start)
    }

    pub fn end(&self) -> Pos {
        Pos::new(self.line, self.cols.end)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub matches: Vec<Match>,
    /// More than [`MAX_MATCHES`].
    pub capped: bool,
    /// The query is not a valid regular expression.
    pub error: Option<String>,
}

/// Every match of `q` in `doc` (inside `within` when given).
pub fn find_all(doc: &Document, q: &Query, within: Option<(Pos, Pos)>) -> Found {
    if q.is_empty() {
        return Found::default();
    }
    let re = match q.compile() {
        Ok(re) => re,
        Err(e) => {
            return Found {
                error: Some(e),
                ..Found::default()
            }
        }
    };
    let mut out = Found::default();
    for (ix, line) in doc.lines.iter().enumerate() {
        for m in re.find_iter(&line.text) {
            if m.start() == m.end() {
                continue;
            }
            if let Some((a, b)) = within {
                if Pos::new(ix, m.start()) < a || Pos::new(ix, m.end()) > b {
                    continue;
                }
            }
            if out.matches.len() >= MAX_MATCHES {
                out.capped = true;
                return out;
            }
            out.matches.push(Match {
                line: ix,
                cols: m.start()..m.end(),
            });
        }
    }
    out
}

/// The first match at or after `from` (wrapping).
pub fn next_from(matches: &[Match], from: Pos) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(matches.iter().position(|m| m.start() >= from).unwrap_or(0))
}

/// The match after the one ending at `cur_end` (wrapping).
pub fn next_after(matches: &[Match], cur_end: Pos) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(
        matches
            .iter()
            .position(|m| m.start() >= cur_end && m.end() > cur_end)
            .unwrap_or(0),
    )
}

/// The match before the one starting at `cur_start` (wrapping).
pub fn prev_before(matches: &[Match], cur_start: Pos) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(
        matches
            .iter()
            .rposition(|m| m.end() <= cur_start && m.start() < cur_start)
            .unwrap_or(matches.len() - 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str) -> Query {
        Query {
            text: text.into(),
            ..Query::default()
        }
    }

    #[test]
    fn plain_text_is_case_insensitive_by_default() {
        let d = Document::new("Foo foo\nbar FOO");
        let f = find_all(&d, &q("foo"), None);
        assert_eq!(f.matches.len(), 3);
        let f = find_all(
            &d,
            &Query {
                case: true,
                ..q("foo")
            },
            None,
        );
        assert_eq!(
            f.matches,
            [Match {
                line: 0,
                cols: 4..7
            }]
        );
    }

    #[test]
    fn whole_words_and_regexes() {
        let d = Document::new("cat concat cat_x cat");
        let f = find_all(
            &d,
            &Query {
                word: true,
                ..q("cat")
            },
            None,
        );
        assert_eq!(f.matches.len(), 2);
        let f = find_all(
            &d,
            &Query {
                regex: true,
                ..q(r"c\w+t")
            },
            None,
        );
        assert_eq!(
            f.matches.len(),
            4,
            "cat, concat, cat (of cat_x), cat: {f:?}"
        );
        let f = find_all(
            &d,
            &Query {
                regex: true,
                ..q("(")
            },
            None,
        );
        assert!(f.error.is_some());
        assert!(f.matches.is_empty());
    }

    #[test]
    fn special_characters_are_literal_without_regex() {
        let d = Document::new("a.b axb (x)");
        assert_eq!(find_all(&d, &q("a.b"), None).matches.len(), 1);
        assert_eq!(find_all(&d, &q("(x)"), None).matches.len(), 1);
    }

    #[test]
    fn georgian_and_whole_words() {
        let d = Document::new("გამარჯობა მეგობარო, გამარჯობა!");
        let f = find_all(
            &d,
            &Query {
                word: true,
                ..q("გამარჯობა")
            },
            None,
        );
        assert_eq!(f.matches.len(), 2);
        let m = &f.matches[1];
        assert_eq!(&d.lines[0].text[m.cols.clone()], "გამარჯობა");
        // Case folding works for Georgian Mtavruli too.
        let d = Document::new("ᲒᲐᲛᲐᲠᲯᲝᲑᲐ");
        assert_eq!(find_all(&d, &q("გამარჯობა"), None).matches.len(), 1);
    }

    #[test]
    fn find_in_selection_and_navigation() {
        let d = Document::new("x x\nx x\nx");
        let f = find_all(&d, &q("x"), Some((Pos::new(0, 2), Pos::new(1, 1))));
        assert_eq!(f.matches.len(), 2);
        let all = find_all(&d, &q("x"), None).matches;
        assert_eq!(next_from(&all, Pos::new(1, 0)), Some(2));
        assert_eq!(next_after(&all, all[4].end()), Some(0), "wraps");
        assert_eq!(prev_before(&all, all[0].start()), Some(4), "wraps back");
        assert_eq!(prev_before(&all, all[2].start()), Some(1));
    }
}
