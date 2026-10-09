//! Text with CSS `letter-spacing` and the label faces (`.label`,
//! `.status-word`, chips). GPUI 0.2.2 text runs have no tracking, so
//! [`Tracked`] places each character at its measured advance plus the gap,
//! as browsers do: the box is exactly as wide as the browser's. Font, size
//! and colour come from the inherited text style, so the usual `Div`
//! styling (`text_color`, `font_family`, group hover) applies to it.

use gpui::{
    div, point, prelude::*, px, App, Bounds, Div, Element, ElementId, FontWeight, GlobalElementId,
    Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, SharedString, Style, Styled, TextRun,
    Window,
};

use super::fonts::LABEL_FONT;

/// A box whose text stays on one line and is clipped (gpui's `truncate()`
/// without its broken ellipsis). Put the text itself in [`one_line`] to get
/// the "…".
pub trait Ellipsis: Styled + Sized {
    fn ellipsis(self) -> Self {
        // A flex box: its [`one_line`] child is sized as a flex item (a
        // block box measures it at its smallest).
        self.overflow_hidden().whitespace_nowrap().flex().items_center()
    }
}

impl<E: Styled + Sized> Ellipsis for E {}

/// One line of text cut with "…" to the width it gets (CSS `white-space:
/// nowrap; overflow: hidden; text-overflow: ellipsis`). gpui 0.2.2's
/// `truncate()` measures a text once, before a flex row shrinks it, and
/// never cuts it; a one-line clamp keeps the smallest width it was ever
/// given. This measures the whole line, shrinks freely, and cuts at paint
/// time. Font, size and colour come from the inherited text style.
pub fn one_line(text: impl Into<SharedString>) -> OneLine {
    OneLine { text: text.into() }
}

pub struct OneLine {
    text: SharedString,
}

impl IntoElement for OneLine {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for OneLine {
    type RequestLayoutState = ();
    type PrepaintState = Option<gpui::ShapedLine>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = window.text_style();
        let rem = window.rem_size();
        let size = style.font_size.to_pixels(rem);
        let line_h = style.line_height_in_pixels(rem);
        let run = style.to_run(self.text.len());
        let full = window
            .text_system()
            .shape_line(self.text.clone(), size, &[run], None)
            .width;
        let mut layout = Style::default();
        layout.min_size.width = px(0.).into();
        layout.flex_shrink = 1.;
        let id = window.request_measured_layout(layout, move |known, available, _, _| {
            let w = known.width.unwrap_or(match available.width {
                gpui::AvailableSpace::Definite(w) => full.min(w),
                gpui::AvailableSpace::MinContent => px(0.),
                gpui::AvailableSpace::MaxContent => full,
            });
            gpui::size(w, line_h)
        });
        let _ = cx;
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) -> Option<gpui::ShapedLine> {
        None
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut Option<gpui::ShapedLine>,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Shaped here, with the colour the text has now (hover included).
        let style = window.text_style();
        let rem = window.rem_size();
        let size = style.font_size.to_pixels(rem);
        let mut runs = vec![style.to_run(self.text.len())];
        let ts = window.text_system().clone();
        let full = ts.shape_line(self.text.clone(), size, &runs, None);
        let line = if full.width > bounds.size.width + px(0.5) {
            let cut = cx
                .text_system()
                .line_wrapper(style.font(), size)
                .truncate_line(self.text.clone(), bounds.size.width, "…", &mut runs);
            ts.shape_line(cut, size, &runs, None)
        } else {
            full
        };
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            let _ = line.paint(bounds.origin, bounds.size.height, window, cx);
        });
    }
}

/// A wrapping paragraph (CSS `white-space: normal`) that breaks lines only
/// where a browser does: after spaces, after a hyphen or a dash between
/// words, and inside a word only when the word alone is wider than the
/// line. gpui 0.2.2's wrapper also breaks before any punctuation, which
/// leaves a lone ")" or "." on the next line. Font, size, line height and
/// colour come from the inherited text style; [`rich`] sets runs.
pub fn para(text: impl Into<SharedString>) -> Para {
    Para {
        text: text.into(),
        runs: None,
        scales: Vec::new(),
    }
}

/// How a part of [`rich`] text is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Text,
    /// `.mono`
    Mono,
    /// `<b>`
    Bold,
}

/// A paragraph with inline `.mono` and bold parts, in `color`.
pub fn rich(parts: &[(&str, Span)], color: Hsla) -> Para {
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut scales = Vec::new();
    for (s, span) in parts {
        // `.mono { font-size: 0.94em }`.
        scales.push(if *span == Span::Mono { 0.94 } else { 1. });
        text.push_str(s);
        let mut f = gpui::font(if *span == Span::Mono {
            super::fonts::MONO_FONT
        } else {
            super::fonts::UI_FONT
        });
        f.weight = if *span == Span::Bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        };
        runs.push(TextRun {
            len: s.len(),
            font: f,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    distinct_fonts(&mut runs);
    Para {
        text: text.into(),
        runs: Some(runs),
        scales,
    }
}

/// gpui 0.2.2's `layout_line` merges a run into the one before it when
/// only the font differs (it compares the colour and decorations, not the
/// font), so a mono or bold part of a line came out in the first part's
/// font. Neighbours with different fonts get a colour one 10 000th of
/// lightness apart, which no screen shows but keeps their fonts.
fn distinct_fonts(runs: &mut [TextRun]) {
    for i in 1..runs.len() {
        let (a, b) = runs.split_at_mut(i);
        let (prev, run) = (&a[i - 1], &mut b[0]);
        if prev.font != run.font && prev.color == run.color {
            let l = run.color.l;
            run.color.l = if l > 0.5 { l - 1e-4 } else { l + 1e-4 };
        }
    }
}

pub struct Para {
    text: SharedString,
    runs: Option<Vec<TextRun>>,
    /// Each run's size against the inherited one (empty: all 1).
    scales: Vec<f32>,
}

/// Where lines may break in `text` (byte offsets where a new line may
/// start), as UAX #14 does for the app's texts.
pub fn break_points(text: &str) -> Vec<usize> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let Some(&(j, next)) = chars.get(k + 1) else {
            break;
        };
        let _ = i;
        let after_space = c == ' ' && next != ' ';
        // A hyphen or a dash between letters (`read-only`, `this—that`).
        let after_dash = matches!(c, '-' | '–' | '—')
            && next.is_alphabetic()
            && k > 0
            && chars[k - 1].1.is_alphanumeric();
        if after_space || after_dash {
            out.push(j);
        }
    }
    out
}

/// Lines (byte ranges) of `text` no wider than `max`; `width(a..b)` measures.
pub fn wrap_lines(
    text: &str,
    max: f32,
    width: &mut dyn FnMut(std::ops::Range<usize>) -> f32,
) -> Vec<std::ops::Range<usize>> {
    let mut lines = Vec::new();
    if text.is_empty() {
        #[allow(clippy::single_range_in_vec_init)]
        return vec![0..0];
    }
    for (start, end) in text
        .split_inclusive('\n')
        .scan(0, |at, part| {
            let s = *at;
            *at += part.len();
            Some((s, s + part.trim_end_matches('\n').len()))
        })
    {
        let piece = &text[start..end];
        let mut breaks: Vec<usize> = break_points(piece).into_iter().map(|b| b + start).collect();
        breaks.push(end);
        let mut line_start = start;
        let mut last_fit: Option<usize> = None;
        let mut i = 0;
        while i < breaks.len() {
            let b = breaks[i];
            // The line's width without trailing spaces.
            let trimmed = line_start + text[line_start..b].trim_end().len();
            if width(line_start..trimmed) <= max + 0.5 {
                last_fit = Some(b);
                i += 1;
                continue;
            }
            match last_fit {
                Some(f) if f > line_start => {
                    lines.push(line_start..line_start + text[line_start..f].trim_end().len());
                    line_start = f;
                    last_fit = None;
                }
                _ => {
                    // One word wider than the line: break inside it.
                    let mut cut = line_start;
                    for (o, c) in text[line_start..b].char_indices() {
                        let next = line_start + o + c.len_utf8();
                        if width(line_start..next) > max + 0.5 && cut > line_start {
                            break;
                        }
                        cut = next;
                    }
                    lines.push(line_start..cut);
                    line_start = cut;
                    last_fit = None;
                }
            }
        }
        if line_start < end || lines.is_empty() || start == end {
            lines.push(line_start..end);
        }
    }
    lines
}

/// The runs of `runs` (covering the whole text) cut to `range`, each with
/// its scale.
fn slice_runs(
    runs: &[TextRun],
    scales: &[f32],
    range: std::ops::Range<usize>,
) -> Vec<(TextRun, f32)> {
    let mut out = Vec::new();
    let mut at = 0;
    for (i, r) in runs.iter().enumerate() {
        let (a, b) = (at, at + r.len);
        at = b;
        let (s, e) = (a.max(range.start), b.min(range.end));
        if s < e {
            let mut r = r.clone();
            r.len = e - s;
            out.push((r, scales.get(i).copied().unwrap_or(1.)));
        }
    }
    out
}

pub struct ParaLayout {
    lines: std::rc::Rc<std::cell::RefCell<Vec<std::ops::Range<usize>>>>,
}

impl IntoElement for Para {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Para {
    type RequestLayoutState = ParaLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ParaLayout) {
        let style = window.text_style();
        let rem = window.rem_size();
        let size = style.font_size.to_pixels(rem);
        let line_h = f32::from(style.line_height_in_pixels(rem));
        let runs = self
            .runs
            .clone()
            .unwrap_or_else(|| vec![style.to_run(self.text.len())]);
        // Each byte's x on one shaped line (per size group), so the
        // measure sees the shaper's kerning and fallback faces (⌘, ✓…).
        let ts = window.text_system().clone();
        let mut prefix = vec![0f32; self.text.len() + 1];
        let (mut x0, mut at, mut k) = (0f32, 0usize, 0usize);
        while k < runs.len() {
            let scale = self.scales.get(k).copied().unwrap_or(1.);
            let mut group = Vec::new();
            while k < runs.len() && self.scales.get(k).copied().unwrap_or(1.) == scale {
                group.push(runs[k].clone());
                k += 1;
            }
            let len: usize = group.iter().map(|g| g.len).sum();
            let line = ts.layout_line(&self.text[at..at + len], size * scale, &group, None);
            let mut xs: Vec<(usize, f32)> = line
                .runs
                .iter()
                .flat_map(|r| r.glyphs.iter().map(|g| (g.index, f32::from(g.position.x))))
                .collect();
            xs.sort_by_key(|g| g.0);
            let width = f32::from(line.width);
            let mut g = 0;
            for b in 0..=len {
                while g < xs.len() && xs[g].0 < b {
                    g += 1;
                }
                prefix[at + b] = x0 + if g < xs.len() { xs[g].1 } else { width };
            }
            x0 += width;
            at += len;
        }
        let text = self.text.clone();
        let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let out = lines.clone();
        let full = prefix.last().copied().unwrap_or(0.);
        let id = window.request_measured_layout(Style::default(), move |known, available, _, _| {
            let max = known.width.map(f32::from).unwrap_or(match available.width {
                gpui::AvailableSpace::Definite(w) => f32::from(w),
                gpui::AvailableSpace::MinContent => 0.,
                gpui::AvailableSpace::MaxContent => f32::INFINITY,
            });
            let mut width = |r: std::ops::Range<usize>| prefix[r.end] - prefix[r.start];
            let l = wrap_lines(&text, max, &mut width);
            let w = l.iter().map(|r| width(r.clone())).fold(0., f32::max);
            let n = l.len().max(1);
            *out.borrow_mut() = l;
            let _ = full;
            gpui::size(px(w), px(line_h * n as f32))
        });
        let _ = cx;
        (id, ParaLayout { lines })
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut ParaLayout,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut ParaLayout,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let style = window.text_style();
        let rem = window.rem_size();
        let size = style.font_size.to_pixels(rem);
        let line_h = style.line_height_in_pixels(rem);
        let runs = self
            .runs
            .clone()
            .unwrap_or_else(|| vec![style.to_run(self.text.len())]);
        let ts = window.text_system().clone();
        let lines = layout.lines.borrow().clone();
        for (i, r) in lines.into_iter().enumerate() {
            if r.is_empty() {
                continue;
            }
            // One shaped piece per size, side by side.
            let mut x = bounds.left();
            let mut at = r.start;
            let pieces = slice_runs(&runs, &self.scales, r);
            let mut k = 0;
            while k < pieces.len() {
                let scale = pieces[k].1;
                let mut group = Vec::new();
                while k < pieces.len() && pieces[k].1 == scale {
                    group.push(pieces[k].0.clone());
                    k += 1;
                }
                let len: usize = group.iter().map(|g| g.len).sum();
                let line = ts.shape_line(
                    SharedString::from(self.text[at..at + len].to_string()),
                    size * scale,
                    &group,
                    None,
                );
                let origin = point(x, bounds.top() + line_h * i as f32);
                x += line.width;
                let _ = line.paint(origin, line_h, window, cx);
                at += len;
            }
        }
    }
}

/// One line of text with `em` letter spacing (see the module docs).
pub struct Tracked {
    text: SharedString,
    em: f32,
}

impl Tracked {
    pub fn new(text: impl Into<SharedString>, em: f32) -> Tracked {
        Tracked {
            text: text.into(),
            em,
        }
    }
}

/// The measured characters (advance of each, in px) and the gap.
pub struct TrackedLayout {
    chars: Vec<(char, f32)>,
    gap: f32,
}

impl IntoElement for Tracked {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Tracked {
    type RequestLayoutState = TrackedLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, TrackedLayout) {
        let style = window.text_style();
        let rem = window.rem_size();
        let size = style.font_size.to_pixels(rem);
        let gap = f32::from(size) * self.em;
        // Advances from the shaped line, so kerning pairs (`AV`, `LT`)
        // close up as in the browser; the gap goes after every character.
        let text: SharedString = self.text.clone();
        let run = style.to_run(text.len());
        let line = window
            .text_system()
            .layout_line(&text, size, &[run], None);
        let mut starts: Vec<(usize, f32)> = Vec::new();
        for r in &line.runs {
            for g in &r.glyphs {
                if starts.last().is_none_or(|(ix, _)| *ix != g.index) {
                    starts.push((g.index, f32::from(g.position.x)));
                }
            }
        }
        let total = f32::from(line.width);
        let x_at = |byte: usize| -> Option<f32> {
            starts.iter().find(|(ix, _)| *ix == byte).map(|(_, x)| *x)
        };
        let idx: Vec<(usize, char)> = text.char_indices().collect();
        let chars: Vec<(char, f32)> = idx
            .iter()
            .enumerate()
            .map(|(k, &(b, c))| {
                let x0 = x_at(b).unwrap_or(0.);
                let x1 = idx
                    .get(k + 1)
                    .and_then(|&(nb, _)| x_at(nb))
                    .unwrap_or(total);
                (c, (x1 - x0).max(0.))
            })
            .collect();
        let width: f32 = chars.iter().map(|(_, w)| w + gap).sum();
        let mut layout = Style::default();
        layout.size.width = px(width).into();
        layout.size.height = style.line_height_in_pixels(rem).into();
        layout.flex_shrink = 0.;
        (
            window.request_layout(layout, [], cx),
            TrackedLayout { chars, gap },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut TrackedLayout,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut TrackedLayout,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let style = window.text_style();
        let size = style.font_size.to_pixels(window.rem_size());
        let font = style.font();
        let mut x = bounds.left();
        for (c, w) in &layout.chars {
            if !c.is_whitespace() {
                let s: SharedString = c.to_string().into();
                let run = TextRun {
                    len: s.len(),
                    font: font.clone(),
                    color: style.color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(s, size, &[run], None);
                let _ = line.paint(point(x, bounds.top()), bounds.size.height, window, cx);
            }
            x += px(w + layout.gap);
        }
    }
}

/// `text` with `tracking` em between characters (CSS `letter-spacing`),
/// at `size` px; the line height is inherited, as CSS labels' is (the
/// body's 1.45 of the label's own size).
pub fn tracked(text: &str, size: f32, tracking: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .text_size(px(size))
        .whitespace_nowrap()
        .child(Tracked::new(text.to_string(), tracking))
}

/// `.label`: Barlow Condensed 600, upper case, 0.12em letter spacing.
pub fn label(text: impl Into<SharedString>, color: Hsla, size: f32) -> Div {
    label_t(text, color, size, 0.12)
}

/// A label with its own letter spacing (`em`).
pub fn label_t(text: impl Into<SharedString>, color: Hsla, size: f32, tracking: f32) -> Div {
    let text: SharedString = text.into();
    tracked(&text.to_uppercase(), size, tracking)
        .font_family(LABEL_FONT)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
}

/// A label that may not fit (project names, paths): tracked when short,
/// else one text run that ends with an ellipsis.
pub fn label_fit(text: impl Into<SharedString>, color: Hsla, size: f32, max_chars: usize) -> Div {
    let text: SharedString = text.into();
    if text.chars().count() <= max_chars {
        return label(text, color, size);
    }
    div()
        .min_w_0()
        .ellipsis()
        .font_family(LABEL_FONT)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(size))
        .text_color(color)
        .child(crate::kit::one_line(SharedString::from(text.to_uppercase())))
}

/// `.muted`: body text in text-2 (a [`para`]).
pub fn muted(text: impl Into<SharedString>, t: &crate::theme::Theme) -> Div {
    div().text_color(t.text_2).child(para(text))
}

/// `.hint` / `.muted-sm`: 12 px text-3 (a [`para`]).
pub fn hint(text: impl Into<SharedString>, t: &crate::theme::Theme) -> Div {
    div()
        .text_size(px(12.))
        .text_color(t.text_3)
        .child(para(text))
}

/// `.hint-error` / a field's error: 12 px red (a [`para`]).
pub fn error_text(text: impl Into<SharedString>, t: &crate::theme::Theme) -> Div {
    div()
        .text_size(px(12.))
        .text_color(t.red)
        .child(para(text))
}

/// `.mono` at 12 px text-3 (paths, commands).
pub fn mono_text(text: impl Into<SharedString>, t: &crate::theme::Theme) -> Div {
    div()
        .font_family(super::fonts::MONO_FONT)
        .text_size(px(12.))
        .text_color(t.text_3)
        .child(text.into())
}

/// The project's name for headings: the last part of its folder (the
/// engine gives `~/code/checkout-web`; the heading says `checkout-web`).
pub fn project_name(display: &str) -> &str {
    let t = display.trim_end_matches(['/', '\\']);
    match t.rfind(['/', '\\']) {
        Some(i) if i + 1 < t.len() => &t[i + 1..],
        _ if t.is_empty() => display,
        _ => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character 1 px wide.
    fn lines(text: &str, max: f32) -> Vec<&str> {
        let mut w = |r: std::ops::Range<usize>| text[r].chars().count() as f32;
        wrap_lines(text, max, &mut w)
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    #[test]
    fn paragraphs_break_at_spaces_not_before_punctuation() {
        // gpui's own wrapper would put the ")" alone on the next line.
        assert_eq!(lines("reads (projects)", 15.), ["reads", "(projects)"]);
        assert_eq!(lines("one two three", 7.), ["one two", "three"]);
        assert_eq!(lines("read-only files", 10.), ["read-only", "files"]);
        assert_eq!(lines("a supercalifragilistic word", 8.), ["a", "supercal", "ifragili", "stic", "word"]);
        assert_eq!(lines("line one\nline two", 40.), ["line one", "line two"]);
        assert_eq!(lines("", 10.), [""]);
        assert!(break_points("x (y).").iter().all(|&b| !"x (y)."[b..].starts_with([')', '.'])));
    }

    #[test]
    fn headings_use_the_folder_name() {
        assert_eq!(project_name("~/code/checkout-web"), "checkout-web");
        assert_eq!(project_name("/srv/work/api/"), "api");
        assert_eq!(project_name(r"C:\work\beta"), "beta");
        assert_eq!(project_name("handbook"), "handbook");
        assert_eq!(project_name("~"), "~");
        assert_eq!(project_name("/"), "/");
    }

    #[gpui::test]
    fn tracked_text_is_as_wide_as_its_advances_plus_the_gaps(cx: &mut gpui::TestAppContext) {
        use gpui::{Render, VisualTestContext};
        struct V;
        impl Render for V {
            fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
                div().child(label("ab", gpui::black(), 10.))
            }
        }
        cx.update(super::super::fonts::register);
        let (_, vcx) = cx.add_window_view(|_, _| V);
        let vcx: &mut VisualTestContext = vcx;
        vcx.run_until_parked();
    }
}
