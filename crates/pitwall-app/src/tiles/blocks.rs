//! Native building blocks in a tile (docs/spec/tile-sdk/native-blocks.md
//! §16.2): Pitwall's code view ([`crate::code_view`]) showing a tile's
//! `code-view` or `diff-view` node, kept by key path across frames (scroll
//! position and selection stay while the text does), with line number
//! clicks and selection changes sent back.

use std::hash::{Hash, Hasher};
use std::time::Duration;

use gpui::{AppContext, Context, Entity, Subscription};
use pitwall_tile_host::ui::{self, Element, Event, Listen};

use crate::code_view::{CodeView, CodeViewEvent, Layout, LineMark, Side};

use super::view::TileView;

/// How often selection changes reach the tile at most.
const SELECT_EVERY: Duration = Duration::from_millis(100);
/// The selected text a `code-selected` carries at most.
const SELECTED_MAX: usize = 64 << 10;

/// A selection as told: the side, and its start and end (line, column).
type Told = (Side, (usize, usize), (usize, usize));

/// A code or diff view Pitwall keeps for a node.
pub struct Block {
    pub view: Entity<CodeView>,
    /// A diff (else one text).
    diff: bool,
    /// The content last applied (hash of texts and language), its marks,
    /// its layout and its reveal.
    content: u64,
    marks: Vec<ui::LineMark>,
    layout: Option<ui::DiffLayout>,
    reveal: Option<u32>,
    /// The selection last told, and whether a tell waits.
    told: Option<Told>,
    waiting: bool,
    _subs: Vec<Subscription>,
}

/// A path whose name gives the code view `language`: a language name
/// (`rust`, `typescript`) as a file name with its extension, or a file name
/// as it is.
pub fn language_path(language: Option<&str>) -> String {
    let Some(l) = language.map(str::trim).filter(|l| !l.is_empty()) else { return "text".into() };
    if l.contains('.') || l.contains('/') {
        return l.rsplit(['/', '\\']).next().unwrap_or(l).to_string();
    }
    let ext = match l.to_ascii_lowercase().as_str() {
        "rust" | "rs" => "rs",
        "javascript" | "js" | "jsx" => "js",
        "typescript" | "ts" => "ts",
        "tsx" => "tsx",
        "python" | "py" => "py",
        "go" | "golang" => "go",
        "bash" | "sh" | "shell" | "zsh" => "sh",
        "css" => "css",
        "html" => "html",
        "c" => "c",
        "cpp" | "c++" => "cpp",
        "java" => "java",
        "ruby" | "rb" => "rb",
        "yaml" | "yml" => "yaml",
        "php" => "php",
        "csharp" | "c#" | "cs" => "cs",
        "lua" => "lua",
        "swift" => "swift",
        "kotlin" | "kt" => "kt",
        "scala" => "scala",
        "sql" => "sql",
        "xml" | "svg" => "xml",
        "haskell" | "hs" => "hs",
        "elixir" | "ex" => "ex",
        "zig" => "zig",
        "r" => "r",
        "powershell" | "ps1" => "ps1",
        "scss" | "less" => "scss",
        "ocaml" | "ml" => "ml",
        "nix" => "nix",
        "markdown" | "md" => "md",
        _ => return "text".into(),
    };
    format!("text.{ext}")
}

/// The two texts of a unified diff of one file: the lines each hunk shows,
/// at their numbers from its header, with blank lines standing for what it
/// doesn't show (the code view folds them as unchanged). Header lines
/// (`diff`, `index`, `---`, `+++`) and `\ No newline` are skipped.
pub fn unified_texts(diff: &str) -> (String, String) {
    let (mut old, mut new): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    let mut in_hunk = false;
    for line in diff.lines() {
        if let Some(h) = line.strip_prefix("@@") {
            if let Some((o, n)) = hunk_starts(h) {
                // Pad up to the hunk's first lines with blank ones, keeping
                // both sides as long so the blanks pair as unchanged.
                let pad = (o.saturating_sub(old.len())).min(n.saturating_sub(new.len()));
                old.extend(std::iter::repeat_n("", pad));
                new.extend(std::iter::repeat_n("", pad));
                in_hunk = true;
                continue;
            }
        }
        if !in_hunk {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => new.push(&line[1..]),
            Some(b'-') => old.push(&line[1..]),
            Some(b' ') => {
                old.push(&line[1..]);
                new.push(&line[1..]);
            }
            Some(b'\\') => {}
            // A blank context line some tools write without its space.
            None => {
                old.push("");
                new.push("");
            }
            // Another file's header: this diff is of one file.
            _ => in_hunk = false,
        }
    }
    (old.join("\n"), new.join("\n"))
}

/// A hunk header's first line numbers, 0-based (`-12,3 +14,5 @@`).
fn hunk_starts(h: &str) -> Option<(usize, usize)> {
    let mut parts = h.split_whitespace();
    let num = |s: &str| s.split(',').next().and_then(|n| n.parse::<usize>().ok()).map(|n| n.saturating_sub(1));
    let o = num(parts.next()?.strip_prefix('-')?)?;
    let n = num(parts.next()?.strip_prefix('+')?)?;
    Some((o, n))
}

fn hash(parts: &[&str], language: Option<&str>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    language.hash(&mut h);
    h.finish()
}

fn mark(m: &ui::LineMark) -> (std::ops::Range<usize>, LineMark) {
    let kind = match m.kind {
        ui::LineMarkKind::Added => LineMark::Added,
        ui::LineMarkKind::Changed => LineMark::Changed,
        ui::LineMarkKind::Deleted => LineMark::Deleted,
    };
    (m.lines.start as usize..(m.lines.end.max(m.lines.start)) as usize, kind)
}

fn side(s: Side) -> ui::DiffSide {
    match s {
        Side::Old => ui::DiffSide::Old,
        Side::New => ui::DiffSide::New,
    }
}

impl Block {
    /// The block of node `path` (`element` a code or diff view), made when
    /// it has none or one of the other kind, and brought up to `element`.
    pub fn sync(slot: &mut Option<Block>, path: &str, element: &Element, cx: &mut Context<TileView>) {
        let diff = match element {
            Element::CodeView(_) => false,
            Element::DiffView(_) => true,
            _ => {
                *slot = None;
                return;
            }
        };
        if slot.as_ref().is_some_and(|b| b.diff != diff) {
            *slot = None;
        }
        let b = slot.get_or_insert_with(|| {
            let view = cx.new(|cx| {
                let mut v = CodeView::new(cx);
                v.set_line_clicks(true);
                v
            });
            let p = path.to_string();
            let subs = vec![
                cx.subscribe(&view, {
                    let p = p.clone();
                    move |this: &mut TileView, _, e: &CodeViewEvent, cx| this.block_event(&p, e, cx)
                }),
                cx.observe(&view, move |this: &mut TileView, _, cx| this.block_changed(&p, cx)),
            ];
            Block { view, diff, content: 0, marks: Vec::new(), layout: None, reveal: None, told: None, waiting: false, _subs: subs }
        });
        let view = b.view.clone();
        match element {
            Element::CodeView(c) => {
                let h = hash(&[&c.text], c.language.as_deref());
                if h != b.content {
                    b.content = h;
                    let path = language_path(c.language.as_deref());
                    let text = c.text.clone();
                    view.update(cx, |v, cx| v.set_file(&path, text, cx));
                }
                if c.marks != b.marks {
                    b.marks = c.marks.clone();
                    let marks = c.marks.iter().map(mark).collect();
                    view.update(cx, |v, cx| v.set_line_marks(marks, cx));
                }
                if c.reveal != b.reveal {
                    b.reveal = c.reveal;
                    if let Some(line) = c.reveal {
                        view.update(cx, |v, cx| v.reveal(Side::New, line as usize, None, cx));
                    }
                }
            }
            Element::DiffView(d) => {
                let h = match &d.source {
                    ui::DiffSource::Texts(t) => hash(&["texts", &t.old, &t.new], d.language.as_deref()),
                    ui::DiffSource::Unified(u) => hash(&["unified", u], d.language.as_deref()),
                };
                if h != b.content {
                    b.content = h;
                    let (old, new) = match &d.source {
                        ui::DiffSource::Texts(t) => (t.old.clone(), t.new.clone()),
                        ui::DiffSource::Unified(u) => unified_texts(u),
                    };
                    let path = language_path(d.language.as_deref());
                    view.update(cx, |v, cx| v.set_diff(&path, old, new, cx));
                }
                if b.layout != Some(d.layout) {
                    b.layout = Some(d.layout);
                    let l = match d.layout {
                        ui::DiffLayout::SideBySide => Layout::Split,
                        ui::DiffLayout::Inline => Layout::Unified,
                    };
                    view.update(cx, |v, cx| v.set_layout(l, cx));
                }
                if d.reveal != b.reveal {
                    b.reveal = d.reveal;
                    if let Some(line) = d.reveal {
                        view.update(cx, |v, cx| v.reveal(Side::New, line as usize, None, cx));
                    }
                }
            }
            _ => {}
        }
    }

    /// The selection now, when it differs from what the tile was told.
    pub fn selection_news(&mut self, cx: &gpui::App) -> Option<ui::CodeSelection> {
        let (s, a, b, text) = self.view.read(cx).selection_source(SELECTED_MAX)?;
        let now = (s, (a.line, a.col), (b.line, b.col));
        if self.told == Some(now) {
            return None;
        }
        self.told = Some(now);
        let pos = |p: (usize, usize)| ui::TextPos { line: p.0 as u32, column: p.1 as u32 };
        Some(ui::CodeSelection { handler: 0, side: side(s), start: pos(now.1), end: pos(now.2), text })
    }
}

impl TileView {
    /// A code or diff view of node `path` said something.
    fn block_event(&mut self, path: &str, e: &CodeViewEvent, cx: &mut Context<Self>) {
        if let CodeViewEvent::LineClicked { side: s, line } = e {
            if let Some(handler) = self.listener_of(path, Listen::LineClick) {
                self.send_event(Event::LineClicked(ui::LineClick { handler, side: side(*s), line: *line as u32 }), cx);
            }
        }
    }

    /// A code or diff view of node `path` changed (its selection, maybe):
    /// told at most every 100 ms, the newest.
    fn block_changed(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.listener_of(path, Listen::Select).is_none() {
            return;
        }
        let Some(b) = self.block_mut(path) else { return };
        if b.waiting {
            return;
        }
        b.waiting = true;
        let p = path.to_string();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SELECT_EVERY).await;
            let _ = this.update(cx, |this, cx| {
                let h = this.listener_of(&p, Listen::Select);
                let Some(b) = this.block_mut(&p) else { return };
                b.waiting = false;
                if let (Some(handler), Some(sel)) = (h, b.selection_news(cx)) {
                    this.send_event(Event::CodeSelected(ui::CodeSelection { handler, ..sel }), cx);
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_name_a_file_the_highlighter_knows() {
        assert_eq!(language_path(Some("Rust")), "text.rs");
        assert_eq!(language_path(Some("typescript")), "text.ts");
        assert_eq!(language_path(Some("src/main.go")), "main.go");
        assert_eq!(language_path(Some("klingon")), "text");
        assert_eq!(language_path(None), "text");
        for l in ["rust", "python", "markdown", "yaml", "c++", "c#"] {
            assert!(crate::code_view::highlight::Lang::for_path(&language_path(Some(l))).is_some(), "{l}");
        }
    }

    #[test]
    fn a_unified_diff_keeps_its_line_numbers() {
        let diff = "diff --git a/x.rs b/x.rs\nindex 1..2 100644\n--- a/x.rs\n+++ b/x.rs\n@@ -3,3 +3,4 @@ fn main() {\n a\n-b\n+B\n+C\n c\n\\ No newline at end of file\n@@ -10,2 +11,2 @@\n x\n-y\n+Y\n";
        let (old, new) = unified_texts(diff);
        let old: Vec<&str> = old.split('\n').collect();
        let new: Vec<&str> = new.split('\n').collect();
        // Line 3 (index 2) is the hunk's first.
        assert_eq!(&old[..2], ["", ""]);
        assert_eq!(&old[2..5], ["a", "b", "c"]);
        assert_eq!(&new[2..6], ["a", "B", "C", "c"]);
        // The second hunk starts at old line 10, new line 11.
        assert_eq!(old[9], "x");
        assert_eq!(old[10], "y");
        assert_eq!(new[10], "x");
        assert_eq!(new[11], "Y");
        assert_eq!(unified_texts("not a diff"), (String::new(), String::new()));
    }
}
