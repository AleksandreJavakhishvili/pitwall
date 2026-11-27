//! Label text: line breaks, HTML tags and entities (mermaid-lld.md §3.2).

/// A decoded label.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Decoded {
    /// Trimmed, non-empty lines.
    pub lines: Vec<String>,
    /// Parts dropped on purpose, for "Not shown".
    pub notes: Vec<&'static str>,
}

/// Decodes a label's raw text (quotes already removed).
pub fn decode(raw: &str) -> Decoded {
    let mut notes = Vec::new();
    let mut text = String::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            if let Some((name, len)) = tag_at(&chars[i..]) {
                match name.as_str() {
                    "br" => text.push('\n'),
                    "img" => notes.push("image in a label"),
                    _ => {}
                }
                i += len;
                continue;
            }
        }
        text.push(chars[i]);
        i += 1;
    }
    let text = entities(&text);
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let mut words = Vec::new();
        for word in line.split_whitespace() {
            if word.starts_with("fa:fa-") || word.starts_with("fab:fa-") {
                if !notes.contains(&"icon in a label") {
                    notes.push("icon in a label");
                }
            } else {
                words.push(word);
            }
        }
        if !words.is_empty() {
            lines.push(words.join(" "));
        }
    }
    Decoded { lines, notes }
}

/// An HTML tag at the start of `chars`: its lowercase name and length.
fn tag_at(chars: &[char]) -> Option<(String, usize)> {
    let mut i = 1;
    if chars.get(i) == Some(&'/') {
        i += 1;
    }
    if !chars.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let name_start = i;
    while chars.get(i).is_some_and(|c| c.is_ascii_alphanumeric()) {
        i += 1;
    }
    let name: String = chars[name_start..i].iter().collect::<String>().to_ascii_lowercase();
    while let Some(&c) = chars.get(i) {
        match c {
            '>' => return Some((name, i + 1)),
            '<' | '\n' => return None,
            _ => i += 1,
        }
    }
    None
}

/// Decodes `&name;`, `&#n;`, `&#xh;` and Mermaid's `#name;` and `#n;`.
fn entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(['&', '#']) {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let marker = if after.starts_with('&') { '&' } else { '#' };
        match entity(after) {
            Some((c, len)) => {
                out.push(c);
                rest = &after[len..];
            }
            None => {
                out.push(marker);
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The entity at the start of `text` (which starts with `&` or `#`): the
/// character and the entity's length.
fn entity(text: &str) -> Option<(char, usize)> {
    let numeric_amp = text.starts_with("&#");
    let start = if numeric_amp { 2 } else { 1 };
    let end = text[start..].find(';').filter(|&end| end > 0 && end <= 8)?;
    let body = &text[start..start + end];
    let numeric = numeric_amp || (text.starts_with('#') && body.starts_with(char::is_numeric));
    let c = if numeric {
        match body.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok().and_then(char::from_u32),
            None => body.parse().ok().and_then(char::from_u32),
        }
    } else {
        named(body)
    }?;
    Some((c, start + end + 1))
}

fn named(name: &str) -> Option<char> {
    Some(match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaks_tags_and_entities() {
        let d = decode("Scanner [Component]<br/>lists <b>Markdown</b> files");
        assert_eq!(d.lines, ["Scanner [Component]", "lists Markdown files"]);
        assert_eq!(decode("a<br>b<BR />c").lines, ["a", "b", "c"]);
        assert_eq!(decode("a\n  b ").lines, ["a", "b"]);
        assert_eq!(decode("&lt;data&gt;/x &amp; #quot;y#quot; &#39;z&#x27;").lines, [
            "<data>/x & \"y\" 'z'"
        ]);
        assert_eq!(decode("a < b and c#1").lines, ["a < b and c#1"]);
        let d = decode("fa:fa-car Car <img src='x.png'>");
        assert_eq!(d.lines, ["Car"]);
        assert_eq!(d.notes, ["image in a label", "icon in a label"]);
    }
}
