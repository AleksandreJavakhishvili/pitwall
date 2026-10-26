//! A Markdown file drawn as formatted text: Review's and the file viewer's
//! Preview (docs/spec/review.md "Markdown preview"). [`model`] turns the
//! text into blocks; [`MarkdownView`] draws them in a centred column with
//! the app's fonts and theme: headings, paragraphs, emphasis, inline and
//! fenced code (highlighted by [`crate::code_view::highlight`]), quotes,
//! lists and task lists, tables (scrolling sideways when wide), rules,
//! footnotes, raw HTML as text. Links to http(s) and mailto open in the
//! browser; other links are only styled. Images next to the file are
//! shown; remote ones are never fetched (alt text and an icon instead).
//! Given a diff's hunks, changed blocks get a bar on their left (added
//! green, modified amber) and removed lines a thin marker. Text is not
//! selectable (GPUI's text elements aren't; the Diff is).

pub mod model;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    div, img, prelude::*, px, relative, AnyElement, Context, Div, ElementId, FontStyle,
    FontWeight, Hsla, InteractiveText, ObjectFit, ScrollHandle, SharedString, StrikethroughStyle,
    StyledText, Task, TextRun, UnderlineStyle, Window,
};

use crate::code_view::diff::Hunk;
use crate::code_view::highlight::LineSpans;
use crate::code_view::style::Colors;
use crate::kit::{self, MONO_FONT, UI_FONT};
use crate::theme::{theme, Theme, RADIUS, RADIUS_SM};
use model::{Align, Block, Change, Doc, Inline, Item, Kind, Seg, Style, Text};

/// Files larger than this get no Preview.
pub const MAX_PREVIEW_BYTES: usize = 1024 * 1024;
/// Top-level blocks drawn; the rest are counted.
const MAX_BLOCKS: usize = 2000;
/// The text column (readable line length).
const COLUMN_W: f32 = 760.;
const PAD_X: f32 = 28.;
const BODY_SIZE: f32 = 14.;

/// What a Preview shows: the text, the hunks that made it (Review) and the
/// folder relative images are read from.
#[derive(Clone, PartialEq)]
struct Source {
    path: SharedString,
    text: Arc<str>,
    hunks: Option<Vec<Hunk>>,
    base: Option<PathBuf>,
}

pub struct MarkdownView {
    src: Option<Source>,
    doc: Option<Arc<Doc>>,
    scroll: ScrollHandle,
    /// Code blocks' and tables' sideways scrolling, in drawing order.
    hscroll: Vec<ScrollHandle>,
    generation: u64,
    _load: Option<Task<()>>,
}

impl MarkdownView {
    pub fn new(_: &mut Context<Self>) -> Self {
        MarkdownView {
            src: None,
            doc: None,
            scroll: ScrollHandle::new(),
            hscroll: Vec::new(),
            generation: 0,
            _load: None,
        }
    }

    /// Show `text` of `path`, marking what `hunks` changed (lines of the
    /// old text → lines of `text`); relative images are read from `base`.
    /// The same again is ignored; the same path keeps the scroll position.
    pub fn set(
        &mut self,
        path: &str,
        text: Arc<str>,
        hunks: Option<Vec<Hunk>>,
        base: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let src = Source {
            path: path.to_string().into(),
            text,
            hunks,
            base,
        };
        if self.src.as_ref() == Some(&src) {
            return;
        }
        if self.src.as_ref().is_none_or(|s| s.path != src.path) {
            self.doc = None;
            self.hscroll.clear();
            self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        }
        self.src = Some(src.clone());
        self.generation += 1;
        let generation = self.generation;
        let bg = cx.background_executor().clone();
        self._load = Some(cx.spawn(async move |this, cx| {
            let doc = bg
                .spawn(async move {
                    let mut d = model::parse(&src.text);
                    if let Some(h) = &src.hunks {
                        model::mark(&mut d, h);
                    }
                    d
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                if v.generation == generation {
                    v.doc = Some(Arc::new(doc));
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// The blocks shown (`None` while parsing).
    pub fn doc(&self) -> Option<&Doc> {
        self.doc.as_deref()
    }
}

impl Render for MarkdownView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let Some(doc) = self.doc.clone() else {
            return div()
                .size_full()
                .child(kit::hint("Loading…", &t).p(px(16.)))
                .into_any_element();
        };
        let base = self.src.as_ref().and_then(|s| s.base.clone());
        let mut d = Draw {
            t: &t,
            colors: Colors::for_theme(&t),
            base: base.as_deref(),
            color: t.text,
            hs: &mut self.hscroll,
            next_h: 0,
            ids: 0,
        };
        let mut col: Vec<AnyElement> = Vec::new();
        let mut removed = doc.removed.iter().peekable();
        for (i, b) in doc.blocks.iter().enumerate().take(MAX_BLOCKS) {
            while let Some(r) = removed.next_if(|r| r.before <= i) {
                col.push(d.removed(r.lines));
            }
            col.push(d.top(b));
        }
        if doc.blocks.len() > MAX_BLOCKS {
            col.push(
                kit::hint(
                    format!(
                        "Preview stops here: {} more blocks.",
                        doc.blocks.len() - MAX_BLOCKS
                    ),
                    &t,
                )
                .into_any_element(),
            );
        } else {
            for r in removed {
                col.push(d.removed(r.lines));
            }
        }
        if doc.blocks.is_empty() && doc.removed.is_empty() {
            col.push(kit::hint("Empty file", &t).into_any_element());
        }
        let body = div()
            .id("md-body")
            .track_scroll(&self.scroll)
            .size_full()
            .overflow_y_scroll()
            .child(
                div().w_full().flex().justify_center().child(
                    div()
                        .w_full()
                        .max_w(px(COLUMN_W + 2. * PAD_X))
                        .px(px(PAD_X))
                        .pt(px(20.))
                        .pb(px(48.))
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .font_family(UI_FONT)
                        .text_size(px(BODY_SIZE))
                        .line_height(relative(1.6))
                        .text_color(t.text)
                        .children(col),
                ),
            );
        kit::vscroll_fill("md-bar", &self.scroll, &t, body)
            .size_full()
            .debug_selector(|| "md-preview".into())
            .into_any_element()
    }
}

/// Drawing state for one frame.
struct Draw<'a> {
    t: &'a Theme,
    colors: Colors,
    base: Option<&'a Path>,
    /// The text colour here (quotes are dimmer).
    color: Hsla,
    hs: &'a mut Vec<ScrollHandle>,
    next_h: usize,
    ids: usize,
}

fn change_color(c: Change, t: &Theme) -> Hsla {
    match c {
        Change::Added => t.green,
        Change::Modified => t.amber,
    }
}

/// `el` with a change bar in the gutter on its left.
fn marked(el: impl IntoElement, change: Option<Change>, t: &Theme) -> Div {
    div().relative().child(el).when_some(change, |d, c| {
        d.child(
            div()
                .absolute()
                .top(px(2.))
                .bottom(px(2.))
                .left(px(-16.))
                .w(px(3.))
                .rounded(px(1.5))
                .bg(change_color(c, t)),
        )
    })
}

/// Whether a link opens in the browser (the rest are only styled).
fn opens(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

/// A local image's path next to the file (`None`: remote, absolute or
/// missing, shown as its alt text).
fn local_image(base: Option<&Path>, src: &str) -> Option<PathBuf> {
    let lower = src.to_ascii_lowercase();
    if src.is_empty() || src.starts_with('/') || src.starts_with('\\') || lower.contains("://")
        || src.starts_with("//") || lower.starts_with("data:")
    {
        return None;
    }
    let path = src.split(['?', '#']).next().unwrap_or(src).replace("%20", " ");
    let p = base?.join(path.trim_start_matches("./"));
    p.is_file().then_some(p)
}

impl Draw<'_> {
    fn id(&mut self) -> ElementId {
        self.ids += 1;
        ElementId::NamedInteger("md-text".into(), self.ids as u64)
    }

    fn hscroll(&mut self) -> ScrollHandle {
        if self.next_h == self.hs.len() {
            self.hs.push(ScrollHandle::new());
        }
        self.next_h += 1;
        self.hs[self.next_h - 1].clone()
    }

    fn top(&mut self, b: &Block) -> AnyElement {
        let el = self.block(b, 0);
        marked(el, b.change, self.t).into_any_element()
    }

    /// "N lines removed" where a diff took lines out.
    fn removed(&self, lines: usize) -> AnyElement {
        let t = self.t;
        let line = || div().flex_1().h(px(1.)).bg(t.red.opacity(0.35));
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(line())
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .line_height(px(14.))
                    .text_color(t.red)
                    .child(if lines == 1 {
                        "1 line removed".to_string()
                    } else {
                        format!("{lines} lines removed")
                    }),
            )
            .child(line())
            .into_any_element()
    }

    fn block(&mut self, b: &Block, depth: usize) -> AnyElement {
        let t = self.t;
        match &b.kind {
            Kind::Heading(level, inline) => {
                let (size, weight) = match level {
                    1 => (24., FontWeight::BOLD),
                    2 => (20., FontWeight::SEMIBOLD),
                    3 => (17., FontWeight::SEMIBOLD),
                    4 => (15., FontWeight::SEMIBOLD),
                    5 => (14., FontWeight::SEMIBOLD),
                    _ => (13., FontWeight::SEMIBOLD),
                };
                let color = if *level == 6 { t.text_2 } else { self.color };
                div()
                    .text_size(px(size))
                    .line_height(relative(1.3))
                    .when(*level == 1, |d| d.mt(px(6.)))
                    .when(*level <= 2, |d| d.pb(px(6.)).border_b_1().border_color(t.line))
                    .child(self.inline(inline, weight, color))
                    .into_any_element()
            }
            Kind::Paragraph(inline) => self.inline(inline, FontWeight::NORMAL, self.color),
            Kind::Code { text, spans, .. } => self.code(text, spans),
            Kind::Quote(blocks) => {
                let was = self.color;
                self.color = t.text_2;
                let kids: Vec<AnyElement> =
                    blocks.iter().map(|b| self.block(b, depth)).collect();
                self.color = was;
                div()
                    .border_l(px(3.))
                    .border_color(t.line_strong)
                    .pl(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .children(kids)
                    .into_any_element()
            }
            Kind::List { start, items } => {
                let rows: Vec<AnyElement> = items
                    .iter()
                    .enumerate()
                    .map(|(i, it)| {
                        let n = start.map(|s| s + i as u64);
                        let row = self.item(it, n, depth);
                        // Only a top-level list's items carry changes.
                        marked(row, it.change, t).into_any_element()
                    })
                    .collect();
                div().flex().flex_col().gap(px(4.)).children(rows).into_any_element()
            }
            Kind::Table { aligns, head, rows } => self.table(aligns, head, rows),
            Kind::Rule => div().h(px(1.)).my(px(6.)).bg(t.line_strong).into_any_element(),
            Kind::Html(h) => div()
                .font_family(MONO_FONT)
                .text_size(px(12.5))
                .line_height(relative(1.5))
                .text_color(t.text_3)
                .child(h.clone())
                .into_any_element(),
            Kind::Footnote(label, blocks) => {
                let kids: Vec<AnyElement> =
                    blocks.iter().map(|b| self.block(b, depth)).collect();
                div()
                    .flex()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .child(div().flex_none().text_color(t.text_3).child(format!("[{label}]")))
                    .child(div().flex_1().min_w_0().flex().flex_col().gap(px(6.)).children(kids))
                    .into_any_element()
            }
        }
    }

    fn item(&mut self, it: &Item, n: Option<u64>, depth: usize) -> Div {
        let t = self.t;
        let marker: AnyElement = match (it.task, n) {
            (Some(on), _) => div()
                .mt(px(5.))
                .size(px(13.))
                .rounded(px(3.))
                .border_1()
                .border_color(if on { t.text_2 } else { t.line_strong })
                .when(on, |d| d.bg(t.text_2))
                .flex()
                .items_center()
                .justify_center()
                .when(on, |d| d.child(kit::icon("check", 11., t.bg)))
                .into_any_element(),
            (None, Some(n)) => div()
                .text_color(t.text_3)
                .child(format!("{n}."))
                .into_any_element(),
            (None, None) => div()
                .text_color(t.text_3)
                .child(["•", "◦", "▪"][depth % 3])
                .into_any_element(),
        };
        let kids: Vec<AnyElement> = it.blocks.iter().map(|b| self.block(b, depth + 1)).collect();
        div()
            .flex()
            .gap(px(6.))
            .child(div().flex_none().w(px(22.)).flex().justify_end().child(marker))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(6.)).children(kids))
    }

    /// Inline content: text with images on their own lines.
    fn inline(&mut self, inline: &Inline, weight: FontWeight, color: Hsla) -> AnyElement {
        let mut parts: Vec<AnyElement> = Vec::new();
        for seg in inline {
            match seg {
                Seg::Text(text) => parts.push(self.text(text, weight, color)),
                Seg::Image { src, alt } => parts.push(self.image(src, alt)),
            }
        }
        if parts.len() == 1 {
            return parts.pop().expect("one");
        }
        div().flex().flex_col().gap(px(6.)).children(parts).into_any_element()
    }

    fn text(&mut self, text: &Text, weight: FontWeight, color: Hsla) -> AnyElement {
        let runs: Vec<TextRun> = text
            .runs
            .iter()
            .map(|(r, st)| self.run(r.len(), *st, weight, color, text))
            .collect();
        let styled = StyledText::new(SharedString::from(text.text.clone())).with_runs(runs);
        let clicks: Vec<(std::ops::Range<usize>, String)> = text
            .runs
            .iter()
            .filter_map(|(r, st)| {
                let url = &text.links[st.link?];
                opens(url).then(|| (r.clone(), url.clone()))
            })
            .collect();
        if text.links.is_empty() {
            return div().w_full().child(styled).into_any_element();
        }
        let tips: Vec<(std::ops::Range<usize>, String)> = text
            .runs
            .iter()
            .filter_map(|(r, st)| Some((r.clone(), text.links[st.link?].clone())))
            .collect();
        let urls: Vec<String> = clicks.iter().map(|(_, u)| u.clone()).collect();
        div()
            .w_full()
            .child(
                InteractiveText::new(self.id(), styled)
                    .on_click(clicks.into_iter().map(|(r, _)| r).collect(), move |ix, _, cx| {
                        if let Some(u) = urls.get(ix) {
                            crate::platform::files::open_url(cx, u);
                        }
                    })
                    .tooltip(move |ix, _, cx| {
                        tips.iter()
                            .find(|(r, _)| r.contains(&ix))
                            .map(|(_, u)| kit::tooltip_view(u.clone(), cx))
                    }),
            )
            .into_any_element()
    }

    fn run(&self, len: usize, st: Style, weight: FontWeight, color: Hsla, text: &Text) -> TextRun {
        let t = self.t;
        let mut font = gpui::font(if st.code { MONO_FONT } else { UI_FONT });
        font.weight = if st.bold { FontWeight::BOLD } else { weight };
        if st.italic {
            font.style = FontStyle::Italic;
        }
        let link = st.link.map(|i| &text.links[i]);
        let color = if st.note || link.is_some_and(|u| !opens(u)) {
            t.text_2
        } else {
            color
        };
        TextRun {
            len,
            font,
            color,
            background_color: st.code.then_some(t.surface_3),
            underline: link.filter(|u| opens(u)).map(|_| UnderlineStyle {
                thickness: px(1.),
                color: Some(t.text_3),
                wavy: false,
            }),
            strikethrough: st.strike.then_some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(color),
            }),
        }
    }

    fn image(&self, src: &str, alt: &str) -> AnyElement {
        let t = self.t;
        let alt: SharedString = if alt.is_empty() { "image".into() } else { alt.to_string().into() };
        let muted = t.text_3;
        let chip = move || {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.5))
                .text_color(muted)
                .child(kit::icon("image", 14., muted))
                .child(alt.clone())
                .into_any_element()
        };
        match local_image(self.base, src) {
            Some(p) => div()
                .rounded(RADIUS_SM)
                .overflow_hidden()
                .child(
                    img(p)
                        .max_w_full()
                        .max_h(px(560.))
                        .object_fit(ObjectFit::Contain)
                        .with_fallback(chip),
                )
                .into_any_element(),
            None => chip(),
        }
    }

    fn code(&mut self, text: &str, spans: &[LineSpans]) -> AnyElement {
        let t = self.t;
        let mut runs = Vec::new();
        let font = gpui::font(MONO_FONT);
        let plain = |len: usize| TextRun {
            len,
            font: font.clone(),
            color: self.colors.text,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let lines: Vec<&str> = text.split('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            let mut at = 0;
            for (r, st) in spans.get(i).map(|s| s.as_slice()).unwrap_or(&[]) {
                if r.start > at {
                    runs.push(plain(r.start - at));
                }
                let mut f = font.clone();
                if st.bold {
                    f.weight = FontWeight::BOLD;
                }
                if st.italic {
                    f.style = FontStyle::Italic;
                }
                runs.push(TextRun {
                    len: r.len(),
                    font: f,
                    color: st.tok.map_or(self.colors.text, |k| self.colors.token(k)),
                    ..plain(0)
                });
                at = r.end;
            }
            let rest = line.len() - at.min(line.len()) + usize::from(i + 1 < lines.len());
            if rest > 0 {
                runs.push(plain(rest));
            }
        }
        let h = self.hscroll();
        let id = self.id();
        div()
            .rounded(RADIUS)
            .border_1()
            .border_color(t.line)
            .bg(t.surface)
            .overflow_hidden()
            .child(kit::hscroll(
                id.clone(),
                &h,
                t,
                div()
                    .id(id)
                    .track_scroll(&h)
                    .overflow_x_scroll()
                    .w_full()
                    .px(px(14.))
                    .py(px(10.))
                    .font_family(MONO_FONT)
                    .text_size(px(12.5))
                    .line_height(relative(1.5))
                    .whitespace_nowrap()
                    .child(StyledText::new(SharedString::from(text.to_string())).with_runs(runs)),
            ))
            .into_any_element()
    }

    fn table(&mut self, aligns: &[Align], head: &[Inline], rows: &[Vec<Inline>]) -> AnyElement {
        let t = self.t;
        let cols = aligns.len().max(head.len());
        // Column widths from the longest cell (no table layout in GPUI).
        let widths: Vec<f32> = (0..cols)
            .map(|c| {
                let chars = std::iter::once(head.get(c))
                    .chain(rows.iter().map(|r| r.get(c)))
                    .flatten()
                    .map(|i| model::plain(i).chars().count())
                    .max()
                    .unwrap_or(0);
                (chars as f32 * 7.6 + 24.).clamp(56., 360.)
            })
            .collect();
        let row = |d: &mut Self, cells: &[Inline], is_head: bool| {
            let mut r = div().flex().border_b_1().border_color(t.line).when(is_head, |r| r.bg(t.surface_2));
            for (c, w) in widths.iter().enumerate() {
                let weight = if is_head { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
                let content = cells.get(c).map(|i| d.inline(i, weight, d.color));
                let cell = div()
                    .flex_none()
                    .w(px(*w))
                    .px(px(10.))
                    .py(px(5.))
                    .when(c + 1 < cols, |x| x.border_r_1().border_color(t.line))
                    .map(|x| match aligns.get(c) {
                        Some(Align::Center) => x.text_center(),
                        Some(Align::Right) => x.text_right(),
                        _ => x,
                    })
                    .children(content);
                r = r.child(cell);
            }
            r.into_any_element()
        };
        let mut lines = vec![row(self, head, true)];
        for r in rows {
            lines.push(row(self, r, false));
        }
        let h = self.hscroll();
        let id = self.id();
        div()
            .max_w_full()
            .child(kit::hscroll(
                id.clone(),
                &h,
                t,
                div().id(id).track_scroll(&h).overflow_x_scroll().max_w_full().child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .border_1()
                        .border_b_0()
                        .border_color(t.line)
                        .rounded(RADIUS_SM)
                        .text_size(px(13.))
                        .line_height(relative(1.5))
                        .children(lines),
                ),
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_images_stay_local() {
        assert!(opens("https://x.dev") && opens("mailto:a@b.c"));
        assert!(!opens("docs/a.md") && !opens("#top"));
        let dir = std::env::temp_dir().join(format!("pw-md-img-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("img")).unwrap();
        std::fs::write(dir.join("img/a b.png"), b"x").unwrap();
        assert_eq!(
            local_image(Some(&dir), "./img/a%20b.png"),
            Some(dir.join("img/a b.png"))
        );
        assert_eq!(local_image(Some(&dir), "https://x.dev/a.png"), None);
        assert_eq!(local_image(Some(&dir), "//x.dev/a.png"), None);
        assert_eq!(local_image(Some(&dir), "img/missing.png"), None);
        assert_eq!(local_image(None, "img/a b.png"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
