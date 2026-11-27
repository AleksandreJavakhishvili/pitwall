//! A character cursor over a block's lines that knows each character's line
//! and column (1-based, in characters), for errors.

pub(crate) struct Cursor {
    chars: Vec<(char, u32, u32)>,
    pos: usize,
    /// Position just past the last character.
    end: (u32, u32),
}

impl Cursor {
    /// A cursor over `lines[from..]`, joined with `\n`.
    pub fn new(lines: &[&str], from: usize) -> Self {
        let mut chars = Vec::new();
        let mut end = (from as u32 + 1, 1);
        for (i, line) in lines.iter().enumerate().skip(from) {
            let number = i as u32 + 1;
            let mut column = 1;
            for c in line.chars() {
                chars.push((c, number, column));
                column += 1;
            }
            end = (number, column);
            if i + 1 < lines.len() {
                chars.push(('\n', number, column));
            }
        }
        Cursor { chars, pos: 0, end }
    }

    pub fn peek(&self) -> Option<char> {
        self.peek_at(0)
    }

    pub fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).map(|c| c.0)
    }

    pub fn starts_with(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(i, c)| self.peek_at(i) == Some(c))
    }

    /// Like [`Self::starts_with`], ignoring ASCII case.
    pub fn starts_with_word(&self, word: &str) -> bool {
        let n = word.chars().count();
        word.chars()
            .enumerate()
            .all(|(i, c)| self.peek_at(i).is_some_and(|p| p.eq_ignore_ascii_case(&c)))
            && !self.peek_at(n).is_some_and(is_word_char)
    }

    pub fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    pub fn advance(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.chars.len());
    }

    pub fn eat(&mut self, s: &str) -> bool {
        if self.starts_with(s) {
            self.advance(s.chars().count());
            true
        } else {
            false
        }
    }

    pub fn is_eof(&self) -> bool {
        self.pos >= self.chars.len()
    }

    /// Line and column of the next character (or just past the end).
    pub fn position(&self) -> (u32, u32) {
        self.chars.get(self.pos).map_or(self.end, |c| (c.1, c.2))
    }

    pub fn line(&self) -> u32 {
        self.position().0
    }

    /// Skips spaces and tabs, not newlines.
    pub fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.pos += 1;
        }
    }

    /// At the end of a statement: the end, a newline or `;`.
    pub fn at_statement_end(&self) -> bool {
        matches!(self.peek(), None | Some('\n' | ';'))
    }

    /// Skips to just after the next newline.
    pub fn skip_line(&mut self) {
        while let Some(c) = self.bump() {
            if c == '\n' {
                break;
            }
        }
    }

    /// The rest of the current line, not consumed.
    pub fn rest_of_line(&self) -> String {
        self.chars[self.pos..].iter().map(|c| c.0).take_while(|&c| c != '\n').collect()
    }

    /// The next whitespace-free run (at most 24 characters), for messages.
    pub fn token(&self) -> String {
        let token: String = self.chars[self.pos.min(self.chars.len())..]
            .iter()
            .map(|c| c.0)
            .take_while(|c| !c.is_whitespace())
            .take(24)
            .collect();
        if token.is_empty() {
            "the end of the line".to_string()
        } else {
            format!("'{token}'")
        }
    }
}

/// Characters of keywords and plain words.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
