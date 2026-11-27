//! Checks and sizes of the canvas, code view and diff view elements and of
//! drag styles (ui.md §3.6–3.7, native-blocks.md §16.2), for
//! [`crate::frame::check`].

use crate::limits::{MAX_CANVAS_OPS, MAX_CANVAS_REGIONS, MAX_CODE_BYTES, MAX_DASH, MAX_DRAG_DATA, MAX_KEY_BYTES};
use crate::ui::{Canvas, DiffSource, DrawOp, Element, Node, PathStep, Stroke, Style};

/// What is wrong with a node's canvas, code, diff or drag, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A canvas with more than [`MAX_CANVAS_OPS`] ops.
    Ops(usize),
    /// A canvas with more than [`MAX_CANVAS_REGIONS`] regions.
    Regions(usize),
    /// A canvas region's key is longer than [`MAX_KEY_BYTES`] or used twice.
    RegionKey,
    /// A dash pattern longer than [`MAX_DASH`].
    Dash,
    /// A code or diff text over [`MAX_CODE_BYTES`].
    Text,
    /// A drag's data over [`MAX_DRAG_DATA`].
    DragData,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::Ops(n) => write!(f, "draws {n} ops (at most {MAX_CANVAS_OPS})"),
            Problem::Regions(n) => write!(f, "has {n} regions (at most {MAX_CANVAS_REGIONS})"),
            Problem::RegionKey => write!(f, "has a region key used twice or longer than {MAX_KEY_BYTES} bytes"),
            Problem::Dash => write!(f, "has a dash pattern of more than {MAX_DASH} lengths"),
            Problem::Text => write!(f, "shows a text of more than {} KiB", MAX_CODE_BYTES / 1024),
            Problem::DragData => write!(f, "drags more than {} KiB of data", MAX_DRAG_DATA / 1024),
        }
    }
}

/// Check node `n`'s canvas, code view, diff view and drag style.
pub fn check(n: &Node) -> Result<(), Problem> {
    match &n.element {
        Element::Canvas(c) => check_canvas(c)?,
        Element::CodeView(c) if c.text.len() > MAX_CODE_BYTES => return Err(Problem::Text),
        Element::DiffView(d) => {
            let long = match &d.source {
                DiffSource::Texts(t) => t.old.len().max(t.new.len()),
                DiffSource::Unified(u) => u.len(),
            };
            if long > MAX_CODE_BYTES {
                return Err(Problem::Text);
            }
        }
        _ => {}
    }
    for s in &n.style {
        if let Style::Drag(d) = s {
            if d.data.len() > MAX_DRAG_DATA {
                return Err(Problem::DragData);
            }
        }
    }
    Ok(())
}

fn check_canvas(c: &Canvas) -> Result<(), Problem> {
    if c.ops.len() > MAX_CANVAS_OPS {
        return Err(Problem::Ops(c.ops.len()));
    }
    if c.regions.len() > MAX_CANVAS_REGIONS {
        return Err(Problem::Regions(c.regions.len()));
    }
    let mut keys = std::collections::HashSet::with_capacity(c.regions.len());
    for r in &c.regions {
        if r.key.len() > MAX_KEY_BYTES || !keys.insert(r.key.as_str()) {
            return Err(Problem::RegionKey);
        }
    }
    let dash = |s: &Option<Stroke>| s.as_ref().is_some_and(|s| s.dash.len() > MAX_DASH);
    for op in &c.ops {
        let long = match op {
            DrawOp::Rect(r) => dash(&r.stroke),
            DrawOp::Path(p) => dash(&p.stroke),
            _ => false,
        };
        if long {
            return Err(Problem::Dash);
        }
    }
    Ok(())
}

/// About how many bytes node `n`'s canvas, code view or diff view and its
/// drag styles take through the canonical ABI (0 for other elements).
pub fn bytes(n: &Node) -> usize {
    let element = match &n.element {
        Element::Canvas(c) => {
            let stroke = |s: &Option<Stroke>| s.as_ref().map_or(0, |s| 16 + s.dash.len() * 4);
            let ops: usize = c
                .ops
                .iter()
                .map(|op| match op {
                    DrawOp::Rect(r) => 40 + stroke(&r.stroke),
                    DrawOp::Path(p) => 24 + stroke(&p.stroke) + p.steps.iter().map(|s| if matches!(s, PathStep::CubicTo(_)) { 28 } else { 12 }).sum::<usize>(),
                    DrawOp::Text(t) => 48 + t.text.len(),
                    DrawOp::ClipPush(_) => 20,
                    DrawOp::ClipPop => 4,
                })
                .sum();
            let regions: usize = c.regions.iter().map(|r| 40 + r.key.len() + r.label.len() + r.tooltip.as_ref().map_or(0, String::len)).sum();
            16 + ops + regions
        }
        Element::CodeView(c) => c.text.len() + c.language.as_ref().map_or(0, String::len) + c.marks.iter().map(|m| 16 + m.message.as_ref().map_or(0, String::len)).sum::<usize>() + 16,
        Element::DiffView(d) => {
            let source = match &d.source {
                DiffSource::Texts(t) => t.old.len() + t.new.len(),
                DiffSource::Unified(u) => u.len(),
            };
            source + d.language.as_ref().map_or(0, String::len) + 16
        }
        _ => 0,
    };
    let drag: usize = n
        .style
        .iter()
        .map(|s| match s {
            Style::Drag(d) => d.kind.len() + d.data.len() + d.label.len(),
            Style::DropTarget(t) => t.accepts.iter().map(String::len).sum::<usize>() + 8,
            _ => 0,
        })
        .sum();
    element + drag
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{CodeView, DrawRect, HitRegion, Paint, Rect, Token, Transform};

    fn node(element: Element) -> Node {
        Node { parent: None, key: "root".into(), element, style: vec![], on: vec![], focusable: false, tooltip: None, label: None }
    }

    fn rect(stroke: Option<Stroke>) -> DrawOp {
        DrawOp::Rect(DrawRect { rect: Rect { x: 0., y: 0., width: 1., height: 1. }, fill: Some(Paint::Token(Token::Surface2)), stroke, radius: 0. })
    }

    fn region(key: &str) -> HitRegion {
        HitRegion { key: key.into(), rect: Rect { x: 0., y: 0., width: 1., height: 1. }, label: "Box".into(), tooltip: None, click: Some(1), hover: None }
    }

    fn canvas(ops: Vec<DrawOp>, regions: Vec<HitRegion>) -> Node {
        node(Element::Canvas(Canvas { ops, transform: Transform { scale: 1., dx: 0., dy: 0. }, regions }))
    }

    #[test]
    fn canvases_and_texts_keep_to_their_limits() {
        assert_eq!(check(&canvas(vec![rect(None); 3], vec![region("a"), region("b")])), Ok(()));
        assert_eq!(check(&canvas(vec![rect(None); MAX_CANVAS_OPS + 1], vec![])), Err(Problem::Ops(MAX_CANVAS_OPS + 1)));
        assert_eq!(check(&canvas(vec![], vec![region("a"), region("a")])), Err(Problem::RegionKey));
        let long = Stroke { color: Paint::Rgba(0xff0000ff), width: 1., dash: vec![1.; MAX_DASH + 1] };
        assert_eq!(check(&canvas(vec![rect(Some(long))], vec![])), Err(Problem::Dash));
        let code = |n| node(Element::CodeView(CodeView { text: "x".repeat(n), language: None, marks: vec![], reveal: None }));
        assert_eq!(check(&code(MAX_CODE_BYTES)), Ok(()));
        assert_eq!(check(&code(MAX_CODE_BYTES + 1)), Err(Problem::Text));
        assert!(bytes(&code(1000)) > 1000);
        assert!(bytes(&canvas(vec![rect(None); 100], vec![])) >= 4000);
    }
}
