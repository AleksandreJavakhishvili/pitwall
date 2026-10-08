//! A unified diff as rows with old/new line numbers (port of
//! `src/lib/diff.ts`), for the read-only diff dialog opened from Changes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Hunk,
    Meta,
    Add,
    Del,
    Ctx,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
    pub old: Option<u32>,
    pub new: Option<u32>,
}

fn hunk_start(raw: &str) -> Option<(u32, u32)> {
    let rest = raw.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let num = |s: &str| s.split(',').next()?.parse::<u32>().ok();
    Some((num(old)?, num(new)?))
}

pub fn parse(diff: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let (mut o, mut n) = (0u32, 0u32);
    let mut in_hunk = false;
    let line = |kind, text: &str, old, new| Line {
        kind,
        text: text.to_string(),
        old,
        new,
    };
    for raw in diff.strip_suffix('\n').unwrap_or(diff).split('\n') {
        if let Some((a, b)) = hunk_start(raw) {
            (o, n) = (a, b);
            in_hunk = true;
            out.push(line(Kind::Hunk, raw, None, None));
            continue;
        }
        if !in_hunk {
            let header = ["diff --git", "index ", "--- ", "+++ "];
            if header.iter().any(|h| raw.starts_with(h)) {
                continue;
            }
            if !raw.trim().is_empty() {
                out.push(line(Kind::Meta, raw, None, None));
            }
            continue;
        }
        if let Some(t) = raw.strip_prefix('+') {
            out.push(line(Kind::Add, t, None, Some(n)));
            n += 1;
        } else if let Some(t) = raw.strip_prefix('-') {
            out.push(line(Kind::Del, t, Some(o), None));
            o += 1;
        } else if raw.starts_with('\\') {
            out.push(line(Kind::Meta, raw, None, None));
        } else if raw.starts_with("diff --git") {
            in_hunk = false;
        } else {
            let t = raw.get(1..).unwrap_or("");
            out.push(line(Kind::Ctx, t, Some(o), Some(n)));
            o += 1;
            n += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_lines_and_drops_headers() {
        let d = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n@@ -3,3 +3,3 @@ fn main\n keep\n-old\n+new\n\\ No newline at end of file\n";
        let lines = parse(d);
        let kinds: Vec<_> = lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [Kind::Hunk, Kind::Ctx, Kind::Del, Kind::Add, Kind::Meta]
        );
        assert_eq!((lines[1].old, lines[1].new), (Some(3), Some(3)));
        assert_eq!((lines[2].old, lines[2].new), (Some(4), None));
        assert_eq!((lines[3].old, lines[3].new), (None, Some(4)));
        assert_eq!(lines[3].text, "new");
        assert_eq!(
            parse("Binary files a/x and b/x differ\n")[0].kind,
            Kind::Meta
        );
        assert!(parse("").is_empty());
    }
}
