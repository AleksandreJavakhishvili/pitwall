//! Canvases, drag and drop, and code and diff views in the harness: what
//! the user does to them, sent as Pitwall sends it, and how the text
//! snapshot shows them.

use pitwall_tile_host::ui::{self, Canvas, CodeView, DiffSource, DiffView, Element, Event, Listen, Modifiers, MouseButton, Node, Point, PointerKind, Style};

use crate::{key_paths, Found, Session};

/// A canvas in the text snapshot: how much it draws and its zoom.
pub(crate) fn describe(c: &Canvas) -> String {
    let t = &c.transform;
    let zoom = if (t.scale - 1.).abs() > f32::EPSILON || t.dx != 0. || t.dy != 0. { format!(" zoom={:.2} at=({:.0}, {:.0})", t.scale, t.dx, t.dy) } else { String::new() };
    let texts: Vec<&str> = c
        .ops
        .iter()
        .filter_map(|o| match o {
            ui::DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    format!("canvas ops={}{zoom} texts={texts:?}", c.ops.len())
}

/// A canvas's regions, one a line under it.
pub(crate) fn describe_regions(c: &Canvas, depth: usize) -> String {
    c.regions
        .iter()
        .map(|r| {
            let mut on = Vec::new();
            if r.click.is_some() {
                on.push("click");
            }
            if r.hover.is_some() {
                on.push("hover");
            }
            let on = if on.is_empty() { String::new() } else { format!(" [{}]", on.join(", ")) };
            format!("{}region {:?} #{}{on}\n", "  ".repeat(depth), r.label, r.key)
        })
        .collect()
}

fn lang(l: &Option<String>) -> String {
    l.as_ref().map(|l| format!(" language={l}")).unwrap_or_default()
}

/// A code view in the text snapshot.
pub(crate) fn describe_code(c: &CodeView) -> String {
    format!("code-view{} lines={}{}", lang(&c.language), c.text.lines().count().max(1), if c.marks.is_empty() { String::new() } else { format!(" marks={}", c.marks.len()) })
}

/// A diff view in the text snapshot.
pub(crate) fn describe_diff(d: &DiffView) -> String {
    let source = match &d.source {
        DiffSource::Texts(_) => "texts",
        DiffSource::Unified(_) => "unified",
    };
    let layout = match d.layout {
        ui::DiffLayout::SideBySide => "side-by-side",
        ui::DiffLayout::Inline => "inline",
    };
    format!("diff-view {source} {layout}{}", lang(&d.language))
}

/// A node's drag source and drop target in the text snapshot.
pub(crate) fn describe_dnd(n: &Node) -> String {
    let mut out = String::new();
    for s in &n.style {
        match s {
            Style::Drag(d) => out.push_str(&format!(" drag={}", d.kind)),
            Style::DropTarget(t) => out.push_str(&format!(" drop={}", t.accepts.join("|"))),
            _ => {}
        }
    }
    out
}

fn drag_of(n: &Node) -> Option<&ui::DragSource> {
    n.style.iter().rev().find_map(|s| match s {
        Style::Drag(d) => Some(d),
        _ => None,
    })
}

fn drop_of(n: &Node) -> Option<&ui::DropTarget> {
    n.style.iter().rev().find_map(|s| match s {
        Style::DropTarget(t) => Some(t),
        _ => None,
    })
}

impl Session {
    /// The canvas at key path `key_path` (or with that key) in the main
    /// view: its ops, transform and regions. Panics when it isn't one.
    pub fn canvas(&self, key_path: &str) -> Canvas {
        match self.find_key(key_path).map(|f| f.node.element) {
            Some(Element::Canvas(c)) => c,
            Some(_) => panic!("“{key_path}” isn't a canvas"),
            None => panic!("there is no canvas “{key_path}”; it shows:\n{}", self.text_snapshot()),
        }
    }

    /// The code view at `key_path` in the main view, as the tile sent it.
    pub fn code_view(&self, key_path: &str) -> CodeView {
        match self.find_key(key_path).map(|f| f.node.element) {
            Some(Element::CodeView(c)) => c,
            _ => panic!("there is no code view “{key_path}”; it shows:\n{}", self.text_snapshot()),
        }
    }

    /// The diff view at `key_path` in the main view, as the tile sent it.
    pub fn diff_view(&self, key_path: &str) -> DiffView {
        match self.find_key(key_path).map(|f| f.node.element) {
            Some(Element::DiffView(d)) => d,
            _ => panic!("there is no diff view “{key_path}”; it shows:\n{}", self.text_snapshot()),
        }
    }

    /// The canvas region labelled `label` in the main view: the canvas
    /// node and the region.
    fn region(&self, label: &str) -> (Found, ui::HitRegion) {
        let f = self.frame();
        let paths = key_paths(f);
        for (i, n) in f.nodes.iter().enumerate() {
            if let Element::Canvas(c) = &n.element {
                if let Some(r) = c.regions.iter().rev().find(|r| r.label == label) {
                    return (Found { view: self.main.clone(), index: i, key_path: paths[i].clone(), node: n.clone() }, r.clone());
                }
            }
        }
        panic!("no canvas has a region labelled “{label}”; it shows:\n{}", self.text_snapshot())
    }

    /// Where a region's centre is drawn, from its canvas node's corner.
    fn region_centre(c: &Node, r: &ui::HitRegion) -> Point {
        let t = match &c.element {
            Element::Canvas(c) => c.transform,
            _ => ui::Transform { scale: 1., dx: 0., dy: 0. },
        };
        Point { x: (r.rect.x + r.rect.width / 2.) * t.scale + t.dx, y: (r.rect.y + r.rect.height / 2.) * t.scale + t.dy }
    }

    /// Click the canvas region labelled `label` (its accessible name) at
    /// its centre, as the user would.
    pub fn click_region(&mut self, label: &str) -> &mut Self {
        let (canvas, r) = self.region(label);
        let Some(handler) = r.click else { panic!("the region “{label}” doesn't listen for clicks") };
        let at = Session::region_centre(&canvas.node, &r);
        self.take_focus();
        self.send(Event::Click(ui::Click { handler, button: MouseButton::Left, modifiers: Modifiers::empty(), count: 1, at }));
        self.settle()
    }

    /// Move the pointer into (`true`) or out of the region labelled
    /// `label`.
    pub fn hover_region(&mut self, label: &str, inside: bool) -> &mut Self {
        let (canvas, r) = self.region(label);
        let Some(handler) = r.hover else { panic!("the region “{label}” doesn't listen for hover") };
        let at = Session::region_centre(&canvas.node, &r);
        self.send(Event::Hover(ui::Hover { handler, inside, at }));
        self.settle()
    }

    /// Send pointer event `kind` at (x, y) in node `key_path` with
    /// `modifiers`: to its `on_pointer` listener, or its `on_wheel` one for
    /// a wheel or a pinch.
    pub fn pointer_with(&mut self, key_path: &str, kind: PointerKind, x: f32, y: f32, modifiers: Modifiers) -> &mut Self {
        let f = match self.find_key(key_path) {
            Some(f) => f,
            None => panic!("there is no node “{key_path}”; it shows:\n{}", self.text_snapshot()),
        };
        let on = if matches!(kind, PointerKind::Wheel(_) | PointerKind::Pinch(_)) { Listen::Wheel } else { Listen::Pointer };
        let Some(l) = f.node.on.iter().find(|l| l.on == on) else { panic!("“{key_path}” doesn't listen for {on:?}") };
        let handler = l.handler;
        if matches!(kind, PointerKind::Down(_)) {
            self.take_focus();
        }
        self.send(Event::Pointer(ui::PointerEvent { handler, kind, at: Point { x, y }, modifiers }));
        self.settle()
    }

    /// [`Session::pointer_with`] with no keys held.
    pub fn pointer(&mut self, key_path: &str, kind: PointerKind, x: f32, y: f32) -> &mut Self {
        self.pointer_with(key_path, kind, x, y, Modifiers::empty())
    }

    /// Press the main button at `from`, move to `to` and let go, in node
    /// `key_path` (panning a canvas).
    pub fn drag_pointer(&mut self, key_path: &str, from: (f32, f32), to: (f32, f32)) -> &mut Self {
        self.pointer(key_path, PointerKind::Down(MouseButton::Left), from.0, from.1)
            .pointer(key_path, PointerKind::Move, (from.0 + to.0) / 2., (from.1 + to.1) / 2.)
            .pointer(key_path, PointerKind::Move, to.0, to.1)
            .pointer(key_path, PointerKind::Up(MouseButton::Left), to.0, to.1)
    }

    /// Scroll the wheel over node `key_path` at (x, y) by (dx, dy) pixels
    /// (positive `dy`: scrolling down).
    pub fn wheel(&mut self, key_path: &str, dx: f32, dy: f32, x: f32, y: f32) -> &mut Self {
        self.pointer(key_path, PointerKind::Wheel(ui::Delta { x: dx, y: dy }), x, y)
    }

    /// Pinch over node `key_path` at (x, y): zoom by `delta` (0.1: 10 %
    /// larger).
    pub fn pinch(&mut self, key_path: &str, delta: f32, x: f32, y: f32) -> &mut Self {
        self.pointer(key_path, PointerKind::Pinch(delta), x, y)
    }

    /// Drag node `from` (or the node around it that can be dragged) onto
    /// node `to` (or the drop target around it), as the user would. Panics
    /// when there is no drag source or no target that accepts its kind
    /// ([`Session::try_drag`] to test that).
    pub fn drag(&mut self, from: &str, to: &str) -> &mut Self {
        if let Err(why) = self.try_drag(from, to) {
            panic!("{why}");
        }
        self
    }

    /// [`Session::drag`], saying why nothing was dropped instead of
    /// panicking.
    pub fn try_drag(&mut self, from: &str, to: &str) -> Result<&mut Self, String> {
        let nodes = self.frame().nodes.clone();
        let up = |key: &str| -> Result<Vec<usize>, String> {
            let f = self.find_key(key).ok_or_else(|| format!("there is no node “{key}”"))?;
            let mut chain = Vec::new();
            let mut i = Some(f.index);
            while let Some(at) = i {
                chain.push(at);
                i = nodes[at].parent.map(|p| p as usize);
            }
            Ok(chain)
        };
        let src = up(from)?.into_iter().find_map(|i| drag_of(&nodes[i]).cloned()).ok_or_else(|| format!("“{from}” (and the nodes around it) can't be dragged"))?;
        let target = up(to)?
            .into_iter()
            .find_map(|i| drop_of(&nodes[i]).cloned())
            .ok_or_else(|| format!("“{to}” (and the nodes around it) take no drops"))?;
        if !target.accepts.contains(&src.kind) {
            return Err(format!("“{to}” doesn't take a “{}” (it takes {:?})", src.kind, target.accepts));
        }
        self.take_focus();
        self.send(Event::Dropped(ui::DropEvent { handler: target.handler, kind: src.kind, data: src.data, at: Point { x: 1., y: 1. } }));
        Ok(self.settle())
    }

    /// Click line number `line` (0-based) of `side` in the code or diff
    /// view at `key_path` (a code view's lines are [`Side::New`](ui::DiffSide::New)).
    pub fn click_line(&mut self, key_path: &str, side: ui::DiffSide, line: usize) -> &mut Self {
        let f = match self.find_key(key_path) {
            Some(f) if matches!(f.node.element, Element::CodeView(_) | Element::DiffView(_)) => f,
            _ => panic!("there is no code or diff view “{key_path}”; it shows:\n{}", self.text_snapshot()),
        };
        let Some(l) = f.node.on.iter().find(|l| l.on == Listen::LineClick) else { panic!("“{key_path}” doesn't listen for line clicks") };
        let handler = l.handler;
        self.take_focus();
        self.send(Event::LineClicked(ui::LineClick { handler, side, line: line as u32 }));
        self.settle()
    }

    /// Select from `start` to `end` ((line, column), 0-based, columns in
    /// UTF-8 bytes) in `side` of the code or diff view at `key_path`; the
    /// tile hears the selected text when it gave the texts.
    pub fn select_code(&mut self, key_path: &str, side: ui::DiffSide, start: (usize, usize), end: (usize, usize)) -> &mut Self {
        let f = match self.find_key(key_path) {
            Some(f) => f,
            None => panic!("there is no code or diff view “{key_path}”; it shows:\n{}", self.text_snapshot()),
        };
        let text = match (&f.node.element, side) {
            (Element::CodeView(c), _) => c.text.clone(),
            (Element::DiffView(DiffView { source: DiffSource::Texts(t), .. }), ui::DiffSide::Old) => t.old.clone(),
            (Element::DiffView(DiffView { source: DiffSource::Texts(t), .. }), ui::DiffSide::New) => t.new.clone(),
            (Element::DiffView(_), _) => String::new(),
            _ => panic!("“{key_path}” isn't a code or diff view"),
        };
        let Some(l) = f.node.on.iter().find(|l| l.on == Listen::Select) else { panic!("“{key_path}” doesn't listen for selection changes") };
        let handler = l.handler;
        let lines: Vec<&str> = text.split('\n').collect();
        let offset = |(line, col): (usize, usize)| lines.iter().take(line).map(|l| l.len() + 1).sum::<usize>() + col;
        let (a, b) = (offset(start).min(text.len()), offset(end).min(text.len()));
        let selected = text.get(a.min(b)..a.max(b)).unwrap_or_default().to_string();
        let pos = |(line, column): (usize, usize)| ui::TextPos { line: line as u32, column: column as u32 };
        self.take_focus();
        self.send(Event::CodeSelected(ui::CodeSelection { handler, side, start: pos(start), end: pos(end), text: selected }));
        self.settle()
    }
}
