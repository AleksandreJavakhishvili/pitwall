//! The view in a window: scrolling, layout, selecting and copying.

use gpui::{
    point, size, Bounds, Entity, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent,
    TestAppContext, VisualTestContext,
};

use super::*;
use model::Kind;
use crate::theme::Theme;

/// Mounted as Review mounts it: a flex column's `flex_1` child.
struct Host(Entity<MarkdownView>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h(px(30.)))
            .child(div().flex_1().min_h_0().child(self.0.clone()))
    }
}

fn setup<'a>(
    cx: &'a mut TestAppContext,
    width: f32,
    text: &str,
) -> (Entity<MarkdownView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(Theme::dark());
        crate::menu::register(cx);
        register(cx);
    });
    let (host, cx) = cx.add_window_view(|_, cx| Host(cx.new(MarkdownView::new)));
    let view = host.read_with(cx, |h, _| h.0.clone());
    cx.simulate_resize(size(px(width), px(600.)));
    view.update(cx, |v, cx| v.set("a.md", text.into(), None, None, cx));
    cx.run_until_parked();
    draw(cx);
    (view, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    // A keystroke nothing binds makes the window draw.
    cx.simulate_keystrokes("f12");
}

fn block(cx: &mut VisualTestContext, i: usize) -> Option<Bounds<Pixels>> {
    let s: &'static str = Box::leak(format!("md-b{i}").into_boxed_str());
    cx.debug_bounds(s)
}

/// Where byte `ix` of piece `k` is drawn (a little inside the line).
fn at(view: &Entity<MarkdownView>, cx: &mut VisualTestContext, k: usize, ix: usize) -> Point<Pixels> {
    view.read_with(cx, |v, _| v.text_position(k, ix))
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}

fn keys(k: &str) -> String {
    let m = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
    format!("{m}-{k}")
}

const SAMPLE: &str = "# A heading that is fairly long to see what happens to it\n\n\
A long paragraph that must wrap because it goes on and on with words that keep coming well \
past the width of any reasonable column of text in the preview, more than once, and then some \
more words after that to be sure it wraps. [A link](https://example.com) in it.\n\n\
## Lists\n\n\
- one\n- two with a long item that should wrap as well because it is long enough to go past \
the column edge when the window is narrow\n  - nested a\n  - nested b\n- three\n\n\
1. first\n2. second\n\n\
- [ ] todo\n- [x] done\n\n\
| Column one | Column two | Column three | Column four | Column five | Column six |\n\
|---|:---:|---:|---|---|---|\n\
| a fairly long cell value here | b | c | d | e | f |\n\n\
```rust\nfn main() { let a_very_long_line_of_code = \"that goes way past the width of the \
column so it has to scroll sideways\"; }\n```\n\n\
> A quote that is also long enough to wrap around in the column when it is narrow and keeps \
going on.\n>\n> > Nested quote.\n\n\
![alt text of a remote image](https://example.com/a.png)\n\n\
<div align=\"center\">html block</div>\n\n\
Text with a footnote.[^1]\n\n\
---\n\n\
Last paragraph.\n\n\
[^1]: The note.\n";

/// The column holds every block, each with a size, in order, none wider
/// than the column (wide code and tables scroll inside theirs), and long
/// text wraps.
fn check_layout(view: &Entity<MarkdownView>, cx: &mut VisualTestContext, what: &str) {
    let col = cx.debug_bounds("md-col").expect("drawn");
    let n = view.read_with(cx, |v, _| v.doc().unwrap().blocks.len().min(MAX_BLOCKS));
    let left = col.left() + px(PAD_X);
    let right = col.right() - px(PAD_X);
    assert!(col.size.width <= px(COLUMN_W + 2. * PAD_X), "{what}: {col:?}");
    let mut last_bottom = col.top();
    for i in 0..n {
        let b = block(cx, i).unwrap_or_else(|| panic!("{what}: block {i} drawn"));
        assert!(b.size.height > px(0.), "{what}: block {i} has a height: {b:?}");
        assert!(b.top() >= last_bottom, "{what}: block {i} under the one before");
        assert!(
            b.left() >= left - px(0.5) && b.right() <= right + px(0.5),
            "{what}: block {i} {b:?} in the column {left:?}..{right:?}"
        );
        last_bottom = b.bottom();
    }
    // The column grows with its text (the end isn't cut off).
    assert!(
        last_bottom <= col.bottom() - px(47.),
        "{what}: the column ({col:?}) holds its blocks (last ends at {last_bottom:?})"
    );
    // Every text fits its box (its lines were measured at the width they
    // are drawn at) and its box sits in a block; outside code blocks and
    // tables (they scroll sideways) it is no wider than the column.
    let blocks: Vec<(Bounds<Pixels>, bool)> = (0..n)
        .map(|i| {
            let b = block(cx, i).unwrap();
            let wide = view.read_with(cx, |v, _| {
                matches!(v.doc().unwrap().blocks[i].kind, Kind::Code { .. } | Kind::Table { .. })
            });
            (b, wide)
        })
        .collect();
    view.read_with(cx, |v, _| {
        for p in &v.pieces {
            let b = p.layout.bounds();
            let end = p.layout.position_for_index(p.text.len()).unwrap();
            assert!(b.size.height > px(0.), "{what}: {:?} has a height", p.text);
            assert!(
                end.y + p.layout.line_height() <= b.bottom() + px(0.5),
                "{what}: {:?} fits its box {b:?} (ends at {end:?})",
                p.text
            );
            let Some((outer, wide)) =
                blocks.iter().find(|(o, _)| b.top() >= o.top() && b.top() < o.bottom())
            else {
                panic!("{what}: {:?} at {b:?} is in a block", p.text);
            };
            assert!(b.bottom() <= outer.bottom() + px(0.5), "{what}: {:?} in its block", p.text);
            if !wide {
                assert!(b.right() <= right + px(0.5), "{what}: {:?} {b:?} in the column", p.text);
            }
        }
    });
}

#[gpui::test]
fn the_sample_lays_out_narrow_and_wide(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, 900., SAMPLE);
    check_layout(&view, cx, "900");
    // The long paragraph wraps: more than one line.
    let p = block(cx, 1).unwrap();
    assert!(p.size.height > px(BODY_SIZE * 1.6 * 2.), "{p:?}");
    let code = cx.debug_bounds("md-code").unwrap();
    let col = cx.debug_bounds("md-col").unwrap();
    assert!(code.right() <= col.right() - px(PAD_X) + px(0.5));
    for w in [420., 1600.] {
        cx.simulate_resize(size(px(w), px(600.)));
        draw(cx);
        check_layout(&view, cx, &format!("{w}"));
    }
    let narrow = block(cx, 1).unwrap();
    assert!(narrow.size.width <= px(COLUMN_W));
}

/// The repository's own Markdown (README, specs) draws without anomalies.
#[gpui::test]
fn real_files_lay_out(cx: &mut TestAppContext) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = vec![root.join("README.md")];
    let mut specs: Vec<PathBuf> = std::fs::read_dir(root.join("docs/spec"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    specs.sort();
    files.extend(specs);
    let (view, cx) = setup(cx, 900., "");
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        view.update(cx, |v, cx| v.set(&name, text.into(), None, None, cx));
        cx.run_until_parked();
        draw(cx);
        check_layout(&view, cx, &name);
    }
}

fn long_doc() -> String {
    (0..60)
        .map(|i| {
            format!(
                "Paragraph {i}: long enough to wrap onto a second line in the column, as most \
                 paragraphs of a real document do, with a few more words here.\n\n"
            )
        })
        .collect()
}

#[gpui::test]
fn long_documents_scroll_to_their_end(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, 900., &long_doc());
    check_layout(&view, cx, "long");
    let scroll = view.read_with(cx, |v, _| v.scroll.clone());
    let max = scroll.max_offset().height;
    assert!(max > px(1000.), "{max:?}");
    let at = point(px(400.), px(300.));
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(-200.))),
        ..Default::default()
    });
    assert_eq!(scroll.offset().y, px(-200.));
    // At the end, the last paragraph is in sight.
    cx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(-100000.))),
        ..Default::default()
    });
    draw(cx);
    assert_eq!(scroll.offset().y, -max);
    let last = block(cx, 59).unwrap();
    let viewport = scroll.bounds();
    assert!(last.bottom() <= viewport.bottom() && last.top() >= viewport.top(), "{last:?}");
}

const DOC: &str = "# Title\n\nFirst paragraph here.\n\nSecond paragraph there.\n\n\
- one\n- two\n  1. deep\n\n\
| a | b |\n|---|---|\n| 1 | 2 |\n\n\
```\ncode line\n```\n\n\
A [link](https://example.com/x) here.\n";

#[gpui::test]
fn drag_selects_across_blocks_and_copies(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, 900., DOC);
    let texts: Vec<String> =
        view.read_with(cx, |v, _| v.pieces.iter().map(|p| p.text.to_string()).collect());
    assert_eq!(
        texts,
        [
            "Title",
            "First paragraph here.",
            "Second paragraph there.",
            "one",
            "two",
            "deep",
            "a",
            "b",
            "1",
            "2",
            "code line",
            "A link here."
        ]
    );
    let from = at(&view, cx, 1, 6);
    let to = at(&view, cx, 2, 6);
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    let s = view.read_with(cx, |v, _| v.selection().unwrap());
    assert_eq!(s.range(), ((1, 6), (2, 6)));
    cx.simulate_keystrokes(&keys("c"));
    assert_eq!(clipboard(cx).as_deref(), Some("paragraph here.\n\nSecond"));
    draw(cx);

    // Shift-click extends into the list; items keep their markers.
    let item = at(&view, cx, 5, 4);
    cx.simulate_mouse_down(item, MouseButton::Left, Modifiers::shift());
    cx.simulate_mouse_up(item, MouseButton::Left, Modifiers::shift());
    cx.dispatch_action(crate::menu::Copy);
    assert_eq!(
        clipboard(cx).as_deref(),
        Some("paragraph here.\n\nSecond paragraph there.\n\n- one\n- two\n  1. deep")
    );

    // ⌘A: everything, tables by tabs.
    cx.simulate_keystrokes(&keys("a"));
    cx.simulate_keystrokes(&keys("c"));
    assert_eq!(
        clipboard(cx).as_deref(),
        Some(
            "Title\n\nFirst paragraph here.\n\nSecond paragraph there.\n\n- one\n- two\n  1. deep\
             \n\na\tb\n1\t2\n\ncode line\n\nA link here."
        )
    );
    // Esc drops it; then Esc goes on.
    cx.simulate_keystrokes("escape");
    assert_eq!(view.read_with(cx, |v, _| v.selected_text()), None);
}

#[gpui::test]
fn double_and_triple_click(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, 900., DOC);
    let p = at(&view, cx, 2, 9);
    let click = |n: usize, cx: &mut VisualTestContext| {
        cx.simulate_event(MouseDownEvent {
            position: p,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count: n,
            first_mouse: false,
        });
        cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
    };
    click(2, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected_text()).as_deref(), Some("paragraph"));
    click(3, cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.selected_text()).as_deref(),
        Some("Second paragraph there.")
    );
    // A click elsewhere drops it.
    click(1, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected_text()), None);
}

#[gpui::test]
fn a_click_on_a_link_opens_it(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, 900., DOC);
    let link = at(&view, cx, 11, 3);
    cx.simulate_mouse_move(link, None, Modifiers::none());
    cx.simulate_mouse_down(link, MouseButton::Left, Modifiers::none());
    draw(cx);
    cx.simulate_mouse_up(link, MouseButton::Left, Modifiers::none());
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/x"));
    assert_eq!(view.read_with(cx, |v, _| v.selected_text()), None, "nothing selected");
}

#[gpui::test]
fn the_context_menu_copies(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx, 900., DOC);
    let p = at(&view, cx, 1, 1);
    cx.simulate_event(MouseDownEvent {
        position: p,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_down(p, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(p, MouseButton::Right, Modifiers::none());
    draw(cx);
    assert!(view.read_with(cx, |v, _| v.menu.is_some()));
    let item = cx.debug_bounds("md-menu-copy").expect("the menu is drawn");
    cx.simulate_click(item.center(), Modifiers::none());
    assert_eq!(clipboard(cx).as_deref(), Some("First"));
    assert!(view.read_with(cx, |v, _| v.menu.is_none()));
}

/// HTML is drawn as formatting: a centred heading sits in the middle, a
/// row of badges side by side; centred text still selects where it is.
#[gpui::test]
fn html_and_centred_text(cx: &mut TestAppContext) {
    let text = "<h1 align=\"center\">Pitwall</h1>\n\n\
                <p align=\"center\">\n  <img alt=\"One\" src=\"https://x.dev/1.svg\" />\n  \
                <img alt=\"Two\" src=\"https://x.dev/2.svg\" />\n</p>\n\n\
                | Name | Centred value |\n|---|:---:|\n| a | middle word |\n";
    let (view, cx) = setup(cx, 900., text);
    check_layout(&view, cx, "html");
    let col = cx.debug_bounds("md-col").unwrap();
    let title = view.read_with(cx, |v, _| {
        assert_eq!(v.pieces[0].text.as_ref(), "Pitwall");
        v.pieces[0].layout.bounds()
    });
    assert!((title.center().x - col.center().x).abs() < px(2.), "{title:?} centred in {col:?}");
    // The two badges share a line.
    let badges = block(cx, 1).unwrap();
    assert!(badges.size.height < px(40.), "{badges:?}");
    // A double click on the centred cell's second word selects it.
    let k = view.read_with(cx, |v, _| {
        v.pieces.iter().position(|p| p.text.as_ref() == "middle word").unwrap()
    });
    let p = at(&view, cx, k, 8);
    cx.simulate_event(MouseDownEvent {
        position: p,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    assert_eq!(view.read_with(cx, |v, _| v.selected_text()).as_deref(), Some("word"));
}
