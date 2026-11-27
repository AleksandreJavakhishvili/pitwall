//! The blocks tile: the test fixture for canvases, drag and drop inside a
//! tile, and the code and diff views, written with the guest SDK.
//!
//! - `diagram`: three boxes and their links on a canvas, with clickable
//!   regions (a click selects a box and zooms to it), zoom and pan by
//!   wheel, pinch and drag, zoom buttons, and box names to drag onto a
//!   "Pinned" list. A right-click on a box opens its menu under it (a
//!   popover anchored to a point of the canvas), which says how wide
//!   the box's name is (`ui.measure-text`).
//! - `code`: a Rust file in the code view, with margin marks, line clicks
//!   and the selection; Check adds an error and a warning mark.
//! - `diff`: two versions of it in the diff view, side by side or inline,
//!   and a unified diff.

use pitwall_extension::events::MouseButton;
use pitwall_extension::prelude::*;

/// A box of the diagram: key, name, what it is, where.
struct Box4 {
    key: &'static str,
    name: &'static str,
    what: &'static str,
    x: f32,
    y: f32,
}

const W: f32 = 168.;
const H: f32 = 72.;

const BOXES: [Box4; 3] = [
    Box4 { key: "web", name: "Web app", what: "TypeScript, React", x: 0., y: 0. },
    Box4 { key: "api", name: "API", what: "Rust, axum", x: 248., y: 0. },
    Box4 { key: "db", name: "Database", what: "PostgreSQL", x: 248., y: 152. },
];

const OLD: &str = "fn total(items: &[Item]) -> u32 {\n    let mut sum = 0;\n    for i in items {\n        sum += i.price;\n    }\n    sum\n}\n\nfn main() {\n    println!(\"{}\", total(&[]));\n}\n";
const NEW: &str = "/// The sum of the items' prices, in cents.\nfn total(items: &[Item]) -> u64 {\n    items.iter().map(|i| u64::from(i.price)).sum()\n}\n\nfn main() {\n    println!(\"{}\", total(&[]));\n}\n";
const UNIFIED: &str = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -12,4 +12,5 @@ impl Cart {\n     pub fn add(&mut self, item: Item) {\n-        self.items.push(item);\n+        self.total += u64::from(item.price);\n+        self.items.push(item);\n     }\n }\n";

#[derive(Default)]
struct Blocks {
    viewport: Viewport,
    size: (f32, f32),
    fitted: bool,
    selected: Option<&'static str>,
    hovered: Option<&'static str>,
    pinned: Vec<String>,
    line: Option<usize>,
    selection: String,
    inline: bool,
    unified: bool,
    /// The box whose menu is open (a right-click).
    menu: Option<&'static str>,
    /// The code view shows its problems.
    checked: bool,
}

impl Blocks {
    fn fit(&mut self) {
        let (w, h) = if self.size.0 > 0. { self.size } else { (640., 360.) };
        // The canvas is the view less the bars above and below.
        self.viewport.fit(-16., -16., 248. + W + 32., 152. + H + 32., w, (h - 120.).max(120.), 24.);
    }

    fn zoom_to(&mut self, key: &str) {
        let Some(b) = BOXES.iter().find(|b| b.key == key) else { return };
        let (w, h) = if self.size.0 > 0. { self.size } else { (640., 360.) };
        // Closer, at most twice the size, with the box in the middle.
        let s = (self.viewport.scale * 1.6).min(2.).max(self.viewport.scale);
        self.viewport.scale = s;
        self.viewport.x = w / 2. - (b.x + W / 2.) * s;
        self.viewport.y = (h - 120.).max(120.) / 2. - (b.y + H / 2.) * s;
    }

    fn diagram(&mut self, cx: &mut Cx<Self>) -> AnyElement {
        if !self.fitted {
            self.fitted = true;
            self.fit();
        }
        let mut c = canvas().id("diagram").flex_1().min_h(px(120.)).bg(Token::Surface).transform(self.viewport);
        // The links first, under the boxes.
        c = c
            .draw(Shape::line(W, H / 2., 248., H / 2.).stroke(Token::Text3, 1.5))
            .draw(arrow(248., H / 2., 1., 0.))
            .draw(Label::new("HTTPS", (W + 248.) / 2., H / 2. - 6.).size(11.).color(Token::Text3).center().bottom())
            .draw(Shape::line(248. + W / 2., H, 248. + W / 2., 152.).stroke(Token::Text3, 1.5).dash([5., 4.]))
            .draw(arrow(248. + W / 2., 152., 0., 1.))
            .draw(Label::new("SQL", 248. + W / 2. + 8., 112.).size(11.).color(Token::Text3).middle());
        // A dashed boundary around the system's own boxes.
        c = c
            .draw(Shape::rect(-16., -16., 248. + W + 32., 152. + H + 32.).stroke(Token::LineStrong, 1.).dash([6., 4.]).radius(10.))
            .draw(Label::new("Shop", -8., 152. + H + 12.).size(12.).color(Token::Text3).medium().bottom());
        for b in &BOXES {
            let on = self.selected == Some(b.key);
            let hover = self.hovered == Some(b.key);
            c = c
                .draw(Shape::rect(b.x, b.y, W, H).fill(if on { Token::DoneSoft } else if hover { Token::Surface3 } else { Token::Surface2 }).stroke(if on { Token::Done } else { Token::Line }, if on { 2. } else { 1. }).radius(8.))
                .draw(Label::new(b.name, b.x + W / 2., b.y + 26.).size(14.).semibold().center().middle().max_width(W - 16.))
                .draw(Label::new(b.what, b.x + W / 2., b.y + 48.).size(12.).color(Token::Text2).center().middle().max_width(W - 16.));
            let key = b.key;
            c = c.region(
                Region::new(b.key, b.x, b.y, W, H, b.name)
                    .tooltip(format!("{}: {}", b.name, b.what))
                    .on_click(cx.listener(move |t: &mut Blocks, e: &Click, cx| {
                        if e.button == MouseButton::Right {
                            t.menu = Some(key);
                        } else {
                            t.selected = Some(key);
                            t.zoom_to(key);
                        }
                        cx.notify();
                    }))
                    .on_hover(cx.listener(move |t: &mut Blocks, h: &Hover, cx| {
                        t.hovered = if h.inside { Some(key) } else { None };
                        cx.notify();
                    })),
            );
        }
        // A box's menu, under the box.
        if let Some(b) = self.menu.and_then(|k| BOXES.iter().find(|b| b.key == k)) {
            let (x, y) = self.viewport.to_view(b.x, b.y + H);
            let wide = Label::new(b.name, 0., 0.).size(14.).semibold().width();
            let key = b.key;
            c = c.popover(
                popover()
                    .id("box-menu")
                    .at(x, y)
                    .width(220.)
                    .on_dismiss(cx.listener(|t: &mut Blocks, _: &Dismissed, cx| {
                        t.menu = None;
                        cx.notify();
                    }))
                    .child(
                        v_flex()
                            .id("items")
                            .p_1()
                            .gap_0p5()
                            .child(text(format!("{} · its name is {} px wide", b.name, wide.round())).id("about").text_xs().text_color(Token::Text3).px_2().py_1())
                            .child(button("Pin").id("pin").small().on_click(cx.listener(move |t: &mut Blocks, _, cx| {
                                if !t.pinned.iter().any(|p| p == key) {
                                    t.pinned.push(key.to_string());
                                }
                                t.menu = None;
                                cx.notify();
                            }))),
                    ),
            );
        }
        let c = c
            .on_pointer(cx.listener(|t: &mut Blocks, p: &Pointer, cx| {
                if t.viewport.on_pointer(p) {
                    cx.notify();
                }
            }))
            .on_wheel(cx.listener(|t: &mut Blocks, p: &Pointer, cx| {
                if t.viewport.on_pointer(p) {
                    cx.notify();
                }
            }))
            .label("Diagram of the shop: Web app, API and Database");
        let zoom = |f: f32| {
            move |t: &mut Blocks, _: &Click, cx: &mut Cx<Blocks>| {
                let (w, h) = t.size;
                t.viewport.zoom_at(f, w / 2., (h - 120.) / 2.);
                cx.notify();
            }
        };
        let bar = h_flex()
            .id("bar")
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(Token::Line)
            .child(text(match self.selected.and_then(|k| BOXES.iter().find(|b| b.key == k)) {
                Some(b) => format!("Selected: {}", b.name),
                None => "Click a box to zoom to it".to_string(),
            }).id("status").text_sm().text_color(Token::Text2).flex_1())
            .child(text(format!("{} %", (self.viewport.scale * 100.).round())).id("zoom").text_sm().text_color(Token::Text3))
            .child(icon_button(Icon::Minus, "Zoom out").id("out").on_click(cx.listener(zoom(1. / 1.25))))
            .child(icon_button(Icon::Plus, "Zoom in").id("in").on_click(cx.listener(zoom(1.25))))
            .child(icon_button(Icon::Maximize, "Fit").id("fit").on_click(cx.listener(|t: &mut Blocks, _, cx| {
                t.selected = None;
                t.fit();
                cx.notify();
            })));
        let chips = h_flex().id("names").gap_1().children(BOXES.iter().map(|b| {
            text(b.name).id(b.key).text_sm().px_2().py_1().rounded_md().bg(Token::Surface2).border_1().border_color(Token::Line).draggable("box", b.key, b.name)
        }));
        let pinned = h_flex()
            .id("pinned")
            .gap_2()
            .min_h(px(32.))
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(Token::Line)
            .child(text("Pinned:").text_sm().text_color(Token::Text3))
            .children(self.pinned.iter().map(|k| text(BOXES.iter().find(|b| b.key == k).map_or(k.as_str(), |b| b.name)).id(k.clone()).text_sm()))
            .when(self.pinned.is_empty(), |d| d.child(text("drag a name here").text_sm().text_color(Token::Text4)))
            .on_drop(["box"], cx.listener(|t: &mut Blocks, d: &Dropped, cx| {
                if !t.pinned.contains(&d.data) {
                    t.pinned.push(d.data.clone());
                    cx.notify();
                }
            }));
        let foot = v_flex().id("foot").gap_2().p_2().border_t_1().border_color(Token::Line).child(chips).child(pinned);
        v_flex().size_full().bg(Token::Surface).child(bar).child(c).child(foot).into_any()
    }

    fn code(&mut self, cx: &mut Cx<Self>) -> AnyElement {
        let status = match (self.line, self.selection.is_empty()) {
            (_, false) => format!("Selected: {}", self.selection.lines().next().unwrap_or_default()),
            (Some(l), true) => format!("Line {} clicked", l + 1),
            (None, true) => "Click a line number".to_string(),
        };
        v_flex()
            .size_full()
            .bg(Token::Surface)
            .child(
                h_flex()
                    .id("bar")
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(Token::Line)
                    .child(text(status).id("status").text_sm().text_color(Token::Text2).flex_1())
                    .child(button(if self.checked { "Clear" } else { "Check" }).id("check").small().on_click(cx.listener(|t: &mut Blocks, _, cx| {
                        t.checked = !t.checked;
                        cx.notify();
                    }))),
            )
            .child(
                code_view(NEW)
                    .id("code")
                    .language("rust")
                    .flex_1()
                    .mark(0..1, Mark::Added)
                    .mark(2..3, Mark::Changed)
                    .mark(3..3, Mark::Deleted)
                    .when(self.checked, |c| c.mark_with(6..7, Mark::Error, "mismatched types: expected `&[Item]`, found `&[_; 0]`").mark_with(1..2, Mark::Warning, "function `total` is never used"))
                    .on_line_click(cx.listener(|t: &mut Blocks, l: &LineClick, cx| {
                        t.line = Some(l.line);
                        t.selection.clear();
                        cx.notify();
                    }))
                    .on_select(cx.listener(|t: &mut Blocks, s: &CodeSelection, cx| {
                        t.selection = s.text.clone();
                        cx.notify();
                    })),
            )
            .into_any()
    }

    fn diff(&mut self, cx: &mut Cx<Self>) -> AnyElement {
        let view = if self.unified { diff_view_unified(UNIFIED) } else { diff_view(OLD, NEW) };
        let view = view.id("diff").language("rust").flex_1().when(self.inline, |d| d.inline()).on_line_click(cx.listener(|t: &mut Blocks, l: &LineClick, cx| {
            t.line = Some(l.line);
            cx.notify();
        }));
        let status = match self.line {
            Some(l) => format!("Line {} clicked", l + 1),
            None => "Click a line number".to_string(),
        };
        v_flex()
            .size_full()
            .bg(Token::Surface)
            .child(
                h_flex()
                    .id("bar")
                    .gap_1()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(Token::Line)
                    .child(text(status).id("status").text_sm().text_color(Token::Text2).flex_1())
                    .child(button(if self.unified { "Two texts" } else { "Unified diff" }).id("source").small().on_click(cx.listener(|t: &mut Blocks, _, cx| {
                        t.unified = !t.unified;
                        cx.notify();
                    })))
                    .child(button(if self.inline { "Side by side" } else { "Inline" }).id("layout").small().on_click(cx.listener(|t: &mut Blocks, _, cx| {
                        t.inline = !t.inline;
                        cx.notify();
                    }))),
            )
            .child(view)
            .into_any()
    }
}

/// An arrowhead at (x, y) pointing along (dx, dy).
fn arrow(x: f32, y: f32, dx: f32, dy: f32) -> Shape {
    let (px_, py_) = (-dy, dx);
    let (l, w) = (9., 4.5);
    Shape::path()
        .move_to(x, y)
        .line_to(x - dx * l + px_ * w, y - dy * l + py_ * w)
        .line_to(x - dx * l - px_ * w, y - dy * l - py_ * w)
        .close()
        .fill(Token::Text3)
}

impl Tile for Blocks {
    fn new(_cx: &mut Cx<Self>) -> Self {
        Blocks::default()
    }

    fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        match cx.view(view).map(|v| v.tile.clone()).as_deref() {
            Some("code") => self.code(cx),
            Some("diff") => self.diff(cx),
            _ => self.diagram(cx),
        }
    }

    fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
        if let Event::Resize { width, height, .. } = event {
            let first = self.size.0 == 0.;
            self.size = (*width, *height);
            if first || self.selected.is_none() {
                self.fit();
            }
            cx.notify();
        }
    }
}

pitwall_extension::export!(Blocks);
