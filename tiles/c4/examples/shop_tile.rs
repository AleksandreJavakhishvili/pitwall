//! A demo of the C4 tile's phase B drawing (hla.md §3.8): the `shop`
//! fixture project's diagrams, parsed, linked and laid out by this crate's
//! core and drawn on a Tile SDK canvas. Wheel, pinch and drag zoom and pan;
//! a click on a box with a detail diagram opens it (click to zoom, FR-13),
//! on another box zooms to it; Back goes up. Reading the project's own
//! files and agents opening it come later.
//!
//! ```sh
//! cargo build --release --target wasm32-wasip2 --features tile --example shop_tile
//! ```

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use c4_tile::index::{Index, ZoomTarget};
use c4_tile::layout::{build, Dagre, Layout, Placed, SizedBox};
use c4_tile::paint::{self, HitBox};
use pitwall_extension::prelude::*;

/// The fixture project, by path.
const FILES: [(&str, &str); 4] = [
    ("docs/architecture.md", include_str!("../tests/fixtures/shop/docs/architecture.md")),
    ("docs/orders/hla.md", include_str!("../tests/fixtures/shop/docs/orders/hla.md")),
    ("docs/orders/cache-lld.md", include_str!("../tests/fixtures/shop/docs/orders/cache-lld.md")),
    ("docs/legacy/orders.md", include_str!("../tests/fixtures/shop/docs/legacy/orders.md")),
];

/// The bars above the canvas, for fitting.
const CHROME: f32 = 40.;

/// One shown diagram, laid out.
struct Shown {
    id: String,
    title: String,
    sized: Vec<SizedBox>,
    placed: Placed,
    detailed: Vec<bool>,
}

struct Shop {
    index: Index,
    /// The zoom path: the diagrams above the shown one, first the top.
    path: Vec<String>,
    shown: Option<Shown>,
    viewport: Viewport,
    size: (f32, f32),
    selected: Option<usize>,
}

impl Shop {
    fn show(&mut self, id: &str) {
        let Some(entry) = self.index.get(id) else { return };
        let Some(s) = &entry.structure else { return };
        let (graph, sized) = build(s);
        let placed = Dagre::default().layout(&graph);
        let path: Vec<&str> = self.path.iter().map(String::as_str).collect();
        let detailed = (0..s.boxes.len()).map(|i| self.index.zoom_target(id, i, &path) != ZoomTarget::None).collect();
        self.shown = Some(Shown { id: id.to_string(), title: entry.title().unwrap_or(id).to_string(), sized, placed, detailed });
        self.selected = None;
        self.fit();
    }

    fn fit(&mut self) {
        let Some(s) = &self.shown else { return };
        let (w, h) = if self.size.0 > 0. { self.size } else { (800., 600.) };
        let (x, y, bw, bh) = paint::bounds(&s.placed);
        self.viewport.fit(x, y, bw, bh, w, (h - CHROME).max(100.), 8.);
    }

    /// A click on box `i`: open its detail diagram, or zoom to it.
    fn clicked(&mut self, i: usize) {
        let Some(s) = &self.shown else { return };
        let from = s.id.clone();
        let path: Vec<&str> = self.path.iter().map(String::as_str).collect();
        match self.index.zoom_target(&from, i, &path) {
            ZoomTarget::One(to) => {
                self.path.push(from);
                self.show(&to);
            }
            // Several: the menu to pick one comes with the agent-open
            // work; the first for now.
            ZoomTarget::Several(to) => {
                self.path.push(from);
                self.show(&to[0]);
            }
            ZoomTarget::None => {
                let r = &s.placed.nodes[i];
                let (w, h) = self.size;
                let scale = (self.viewport.scale * 1.5).min(2.).max(self.viewport.scale);
                self.viewport.scale = scale;
                self.viewport.x = w / 2. - (r.x + r.w / 2.) as f32 * scale;
                self.viewport.y = (h - CHROME) / 2. - (r.y + r.h / 2.) as f32 * scale;
                self.selected = Some(i);
            }
        }
    }
}

impl Tile for Shop {
    fn new(_cx: &mut Cx<Self>) -> Self {
        let mut index = Index::new();
        for (path, text) in FILES {
            index.set_file(path, text);
        }
        let first = index.entries().iter().find(|e| e.structure.is_some()).map(|e| e.id().to_string());
        let mut shop = Shop { index, path: Vec::new(), shown: None, viewport: Viewport::default(), size: (0., 0.), selected: None };
        if let Some(id) = first {
            shop.show(&id);
        }
        shop
    }

    fn render(&mut self, _view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        let Some(s) = &self.shown else {
            return v_flex().size_full().p_4().child(text("No diagrams").text_color(Token::Text3)).into_any();
        };
        let entry = self.index.get(&s.id);
        let Some(structure) = entry.and_then(|e| e.structure.as_ref()) else {
            return v_flex().into_any();
        };
        let base = canvas().id("diagram").flex_1().bg(Token::Surface).transform(self.viewport).label(format!("Diagram: {}", s.title));
        let (mut c, hits) = paint::paint(base, structure, &s.sized, &s.placed, &s.detailed, self.selected);
        for HitBox { index, key, rect, label } in hits {
            let tip = if s.detailed[index] { format!("{label}: click to open its diagram") } else { label.clone() };
            c = c.region(
                Region::new(key, rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32, label).tooltip(tip).on_click(cx.listener(move |t: &mut Shop, _: &Click, cx| {
                    t.clicked(index);
                    cx.notify();
                })),
            );
        }
        let pan = |t: &mut Shop, p: &Pointer, cx: &mut Cx<Shop>| {
            if t.viewport.on_pointer(p) {
                cx.notify();
            }
        };
        let c = c.on_pointer(cx.listener(pan)).on_wheel(cx.listener(pan));
        let bar = h_flex()
            .id("bar")
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(Token::Line)
            .when(!self.path.is_empty(), |d| {
                d.child(icon_button(Icon::Back, "Back").id("back").on_click(cx.listener(|t: &mut Shop, _, cx| {
                    if let Some(up) = t.path.pop() {
                        t.show(&up);
                        cx.notify();
                    }
                })))
            })
            .child(text(s.title.clone()).id("title").font_semibold().truncate().flex_1())
            .child(text(format!("{} %", (self.viewport.scale * 100.).round())).id("zoom").text_sm().text_color(Token::Text3))
            .child(icon_button(Icon::Maximize, "Fit").id("fit").on_click(cx.listener(|t: &mut Shop, _, cx| {
                t.selected = None;
                t.fit();
                cx.notify();
            })));
        v_flex().size_full().bg(Token::Surface).child(bar).child(c).into_any()
    }

    fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
        if let Event::Resize { width, height, .. } = event {
            self.size = (*width, *height);
            self.fit();
            cx.notify();
        }
    }
}

// The component exports only link for wasm; natively (`cargo test`) the
// example just compiles.
#[cfg(target_arch = "wasm32")]
pitwall_extension::export!(Shop);
