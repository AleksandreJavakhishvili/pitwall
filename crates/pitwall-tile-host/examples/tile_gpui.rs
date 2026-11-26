//! The host's half of "the guest describes, the host draws" (ui.md §3.3):
//! a tile's frame mapped to GPUI elements, laid out and painted in GPUI's
//! test platform with the system's real text system. No window appears on
//! screen; nothing is clicked or typed. Run in release:
//!
//!   cargo run --release -p pitwall-tile-host --example tile_gpui
//!
//! The tile runs on its worker thread; this (UI) thread only receives its
//! frames and posts its events (runtime.md §2.4). A real app maps tokens
//! through `theme.rs` and draws buttons with `kit::button`; this uses a
//! fixed dark palette and plain divs.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gpui::{
    div, px, relative, rems, rgb, size, AnyElement, Context, DefiniteLength, ElementId,
    InteractiveElement, IntoElement, ParentElement, Render, Rgba, SharedString,
    StatefulInteractiveElement, Styled, TestAppContext, TestDispatcher, VisualTestContext, Window,
};
use pitwall_tile_host::{
    engine, Configure, Element, EpochTicker, Event, Frame, FromWorker, Length, Runtime, Style,
    TextSize, Token, Worker,
};

const HELLO: &[u8] = include_bytes!("../tests/fixtures/hello-tile.wasm");

fn color(t: Token) -> Rgba {
    rgb(match t {
        Token::Bg => 0x0e0f11,
        Token::Surface => 0x16181b,
        Token::Surface2 => 0x1d2024,
        Token::Line => 0x2a2e33,
        Token::Text => 0xe8eaed,
        Token::Text2 => 0xb4b9bf,
        Token::Text3 => 0x80868d,
        Token::Focus => 0x5b9cff,
        Token::Good => 0x3fb950,
        Token::Warn => 0xd29922,
        Token::Bad => 0xf85149,
    })
}

fn definite(l: Length) -> DefiniteLength {
    match l {
        Length::Px(v) => px(v).into(),
        Length::Rem(v) => rems(v).into(),
        Length::Fraction(v) => relative(v),
        Length::Auto => px(0.).into(),
    }
}

fn length(l: Length) -> gpui::Length {
    match l {
        Length::Auto => gpui::Length::Auto,
        other => gpui::Length::Definite(definite(other)),
    }
}

fn apply<E: Styled>(mut e: E, styles: &[Style]) -> E {
    for s in styles {
        e = match *s {
            Style::FlexRow => e.flex().flex_row(),
            Style::FlexCol => e.flex().flex_col(),
            Style::Grow(g) => {
                e.style().flex_grow = Some(g);
                e
            }
            Style::Gap(l) => e.gap(definite(l)),
            Style::Padding(l) => e.p(definite(l)),
            Style::Width(l) => e.w(length(l)),
            Style::Height(l) => e.h(length(l)),
            Style::Background(t) => e.bg(color(t)),
            Style::Color(t) => e.text_color(color(t)),
            Style::BorderColor(t) => e.border_1().border_color(color(t)),
            Style::Radius(r) => e.rounded(px(r)),
            Style::TextSize(t) => e.text_size(px(match t {
                TextSize::Xs => 11.,
                TextSize::Sm => 12.,
                TextSize::Md => 13.,
                TextSize::Lg => 16.,
                TextSize::Xl => 20.,
            })),
            Style::Truncate => e.truncate(),
        };
    }
    e
}

/// The frame as one GPUI element tree. `post` sends a handler's event to
/// the tile's worker (never calls the guest from the handler).
fn map(frame: &Frame, post: Rc<dyn Fn(u32)>) -> AnyElement {
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); frame.nodes.len()];
    for (i, n) in frame.nodes.iter().enumerate() {
        if let Some(p) = n.parent {
            children[p as usize].push(i);
        }
    }
    build(frame, &children, 0, &post)
}

fn build(frame: &Frame, children: &[Vec<usize>], ix: usize, post: &Rc<dyn Fn(u32)>) -> AnyElement {
    let node = &frame.nodes[ix];
    let kids = children[ix].iter().map(|&c| build(frame, children, c, post));
    match &node.element {
        Element::Box => apply(div(), &node.style).children(kids).into_any_element(),
        Element::Text(t) => apply(div(), &node.style).child(SharedString::from(t.clone())).into_any_element(),
        Element::Button(b) => {
            let post = post.clone();
            let handler = b.on_click;
            apply(
                div()
                    .id(ElementId::NamedInteger("tile-btn".into(), ix as u64))
                    .px(px(8.))
                    .rounded(px(4.))
                    .bg(color(Token::Surface2))
                    .cursor_pointer(),
                &node.style,
            )
            .on_click(move |_, _, _| post(handler))
            .child(SharedString::from(b.label.clone()))
            .into_any_element()
        }
    }
}

/// The tile's view: draws the latest frame from its worker.
struct TileView {
    frame: Frame,
    post: Rc<dyn Fn(u32)>,
    /// How long the last `map` took, and how many renders ran.
    mapped: Rc<Cell<(Duration, usize)>>,
}

impl Render for TileView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = Instant::now();
        let e = map(&self.frame, self.post.clone());
        let (_, n) = self.mapped.get();
        self.mapped.set((t.elapsed(), n + 1));
        div().size_full().bg(color(Token::Bg)).child(e)
    }
}

fn main() {
    let rt = Runtime::new(engine(None).unwrap()).unwrap();
    let _ticker = EpochTicker::start(rt.engine()).unwrap();
    let component = rt.compile(HELLO).unwrap();
    let (tx, rx) = mpsc::channel();
    let worker = Rc::new(
        Worker::spawn("gpui", rt, component, move |m| {
            let _ = tx.send(m);
        })
        .unwrap(),
    );
    let next_frame = || loop {
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            FromWorker::Frame { frame, .. } => return frame,
            FromWorker::Log(..) => continue,
            FromWorker::Stopped { reason } => panic!("{reason}"),
        }
    };

    let text_system = gpui_platform::current_platform(true).text_system();
    let mut app = TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text_system);
    let w = worker.clone();
    let post: Rc<dyn Fn(u32)> = Rc::new(move |h| {
        w.send(Event::Click(h));
    });
    let mapped = Rc::new(Cell::new((Duration::ZERO, 0)));
    let hello = next_frame();
    let hello_len = hello.nodes.len();
    let (view, cx) = app.add_window_view({
        let (post, mapped) = (post.clone(), mapped.clone());
        move |_, _| TileView { frame: hello, post, mapped }
    });
    cx.simulate_resize(size(px(800.), px(600.)));
    let draw = |cx: &mut VisualTestContext| {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    };
    draw(cx);

    // The button's handler as a click would call it: it only posts.
    post(1);
    let after = next_frame();
    let count = after.nodes.iter().find(|n| n.key == "count").map(|n| n.element.clone());
    println!("hello tile: {hello_len} nodes drawn; after a click: {count:?}");

    // 2 000 nodes: map to elements, then layout + prepaint + paint.
    worker.send(Event::Configure(Configure { nodes: 2000, cached: false }));
    let big = next_frame();
    view.update(cx, |v, _| v.frame = big);
    draw(cx);
    let (mut mapping, mut drawing) = (Vec::new(), Vec::new());
    for _ in 0..100 {
        let (_, before) = mapped.get();
        let t = Instant::now();
        draw(cx);
        drawing.push(t.elapsed());
        let (m, after) = mapped.get();
        assert_eq!(after, before + 1, "one render per draw");
        mapping.push(m);
    }
    println!("| What | median | p95 |\n|---|---|---|");
    row("2000 nodes: frame → GPUI elements (map)", &mut mapping);
    row("2000 nodes: map + layout + prepaint + paint (all rows, no virtual list)", &mut drawing);
}

fn row(what: &str, s: &mut [Duration]) {
    s.sort();
    let at = |p: f64| s[((s.len() as f64 - 1.0) * p).round() as usize].as_secs_f64() * 1000.0;
    println!("| {what} | {:.3} ms | {:.3} ms |", at(0.5), at(0.95));
}
