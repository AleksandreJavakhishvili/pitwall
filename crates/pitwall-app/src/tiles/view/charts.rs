//! A tile's charts (docs/spec/tile-sdk/ui.md §3.10), drawn by Pitwall
//! from data alone (series and options, no drawing ops) in the theme's
//! colours: a sparkline, a bar chart, a line chart with axes, and a
//! status timeline. Hovering a bar, a point's column or a segment shows
//! its values; a chart's label (or, without one, its summary) is its
//! tooltip's heading for the whole of it.

use gpui::{canvas, div, point, prelude::*, px, relative, AnyElement, Context, Hsla, PathBuilder, Pixels, SharedString};
use pitwall_tile_host::ui::{self, ControlValue, Element, Listen, Node};
use pitwall_tile_host::widgets::{self as hw, Outcome};

use super::{handler, TileView};
use crate::kit::{self, feedback as fb};
use crate::theme::Theme;
use crate::tiles::render;

/// The y axis's labels' width.
const AXIS_W: f32 = 40.;

fn has(node: &Node, f: impl Fn(&ui::Style) -> bool) -> bool {
    node.style.iter().any(f)
}

/// A polyline through `pts`.
fn line(pts: &[gpui::Point<Pixels>], width: f32) -> Option<gpui::Path<Pixels>> {
    let mut p = PathBuilder::stroke(px(width));
    for (k, pt) in pts.iter().enumerate() {
        if k == 0 {
            p.move_to(*pt);
        } else {
            p.line_to(*pt);
        }
    }
    p.build().ok()
}

/// The area under `pts` down to `bottom`.
fn area(pts: &[gpui::Point<Pixels>], bottom: Pixels) -> Option<gpui::Path<Pixels>> {
    let (first, last) = (pts.first()?, pts.last()?);
    let mut p = PathBuilder::fill();
    p.move_to(point(first.x, bottom));
    for pt in pts {
        p.line_to(*pt);
    }
    p.line_to(point(last.x, bottom));
    p.close();
    p.build().ok()
}

/// The legend: a dot and a name per series.
fn legend(series: &[ui::ChartSeries], t: &Theme) -> Option<AnyElement> {
    (series.len() > 1).then(|| {
        div()
            .flex()
            .flex_wrap()
            .gap(px(12.))
            .text_size(px(11.))
            .text_color(t.text_3)
            .children(series.iter().enumerate().map(|(i, s)| div().flex().items_center().gap(px(5.)).child(div().size(px(8.)).rounded(px(2.)).bg(render::color(hw::series_token(s, i), t))).child(s.name.clone())))
            .into_any_element()
    })
}

/// The y axis: its values from the top down.
fn y_axis(lo: f64, hi: f64, step: f64, unit: Option<&str>, t: &Theme) -> AnyElement {
    let mut labels = Vec::new();
    let mut v = hi;
    while v >= lo - step * 0.01 && labels.len() < 12 {
        labels.push(hw::number(v, unit));
        v -= step;
    }
    div().flex_none().w(px(AXIS_W)).h_full().flex().flex_col().justify_between().items_end().pr(px(6.)).text_size(px(10.5)).text_color(t.text_3).children(labels.into_iter().map(|l| div().h(px(0.)).flex().items_center().child(l))).into_any_element()
}

/// Lines across the plot at each step.
fn grid(lo: f64, hi: f64, step: f64, color: Hsla) -> AnyElement {
    let n = (((hi - lo) / step).round() as usize).clamp(1, 12);
    div().absolute().inset_0().flex().flex_col().justify_between().children((0..=n).map(move |_| div().h(px(1.)).w_full().bg(color))).into_any_element()
}

/// The x axis's labels, thinned out to at most `most`.
fn x_axis(labels: &[String], most: usize, t: &Theme) -> AnyElement {
    let every = labels.len().div_ceil(most.max(1)).max(1);
    div()
        .flex()
        .pl(px(AXIS_W))
        .pt(px(4.))
        .text_size(px(10.5))
        .text_color(t.text_3)
        .children(labels.iter().enumerate().map(|(k, l)| div().flex_1().min_w_0().flex().justify_center().overflow_hidden().whitespace_nowrap().child(if k % every == 0 { l.clone() } else { String::new() })))
        .into_any_element()
}

impl TileView {
    /// Chart node `node` at `path`, in its box `d`.
    pub(super) fn build_chart(&self, d: gpui::Stateful<gpui::Div>, node: &Node, path: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = format!("tile-{}-{}-{path}", self.ext, self.tile);
        let (w, h) = (has(node, |s| matches!(s, ui::Style::Width(_) | ui::Style::Grow(_))), has(node, |s| matches!(s, ui::Style::Height(_) | ui::Style::Grow(_))));
        let title = node.label.clone().or_else(|| hw::chart_summary(&node.element));
        let d = if node.tooltip.is_none() && matches!(node.element, Element::Sparkline(_)) { d.when_some(title.clone(), |d, x| d.tooltip(kit::tooltip(x))) } else { d };
        match &node.element {
            Element::Sparkline(s) => {
                let color = render::color(s.color.unwrap_or(ui::Token::Text2), t);
                let (lo, hi) = s.values.iter().filter(|v| v.is_finite()).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
                let values = s.values.clone();
                let filled = s.filled;
                let paint = canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        if values.len() < 2 || !lo.is_finite() {
                            return;
                        }
                        let span = if hi > lo { hi - lo } else { 1. };
                        let (wd, ht) = (f32::from(b.size.width), f32::from(b.size.height) - 2.);
                        let pts: Vec<_> = values.iter().enumerate().filter(|(_, v)| v.is_finite()).map(|(k, v)| point(b.origin.x + px(wd * k as f32 / (values.len() - 1) as f32), b.origin.y + px(1. + ht * (1. - ((v - lo) / span) as f32)))).collect();
                        if filled {
                            if let Some(a) = area(&pts, b.bottom()) {
                                window.paint_path(a, color.opacity(0.16));
                            }
                        }
                        if let Some(l) = line(&pts, 1.5) {
                            window.paint_path(l, color);
                        }
                    },
                )
                .size_full();
                d.flex_none().when(!w, |d| d.w(px(80.))).when(!h, |d| d.h(px(20.))).child(paint).into_any_element()
            }
            Element::BarChart(b) => {
                let n = b.labels.len();
                let at = |s: &ui::ChartSeries, k: usize| s.values.get(k).copied().filter(|v| v.is_finite()).unwrap_or(0.);
                let tops: Vec<f64> = if b.stacked { (0..n).map(|k| b.series.iter().map(|s| at(s, k).max(0.)).sum()).collect() } else { b.series.iter().flat_map(|s| (0..n).map(move |k| at(s, k))).collect() };
                let (lo, hi, step) = hw::axis(tops.into_iter(), None, None);
                let span = (hi - lo).max(f64::EPSILON);
                let unit = b.unit.clone();
                let share = move |v: f64| (((v - lo) / span) as f32).clamp(0., 1.);
                let cols = (0..n).map(|k| {
                    let label = b.labels[k].clone();
                    let bars: Vec<AnyElement> = if b.stacked {
                        let segs = b.series.iter().enumerate().map(|(i, s)| {
                            let v = at(s, k).max(0.);
                            let tip = format!("{label} · {}: {}", s.name, hw::number(v, unit.as_deref()));
                            div().id(SharedString::from(format!("{id}-{k}-{i}"))).w_full().h(relative(((v / span) as f32).clamp(0., 1.))).bg(render::color(hw::series_token(s, i), t)).tooltip(kit::tooltip(tip))
                        });
                        vec![div().w(relative(0.62)).max_w(px(36.)).h_full().flex().flex_col_reverse().rounded_t(px(2.)).overflow_hidden().children(segs).into_any_element()]
                    } else {
                        b.series
                            .iter()
                            .enumerate()
                            .map(|(i, s)| {
                                let v = at(s, k);
                                let tip = format!("{label} · {}: {}", s.name, hw::number(v, unit.as_deref()));
                                div().id(SharedString::from(format!("{id}-{k}-{i}"))).flex_1().max_w(px(28.)).h(relative(share(v))).rounded_t(px(2.)).bg(render::color(hw::series_token(s, i), t)).tooltip(kit::tooltip(tip)).into_any_element()
                            })
                            .collect()
                    };
                    div().flex_1().h_full().flex().items_end().justify_center().gap(px(2.)).px(px(3.)).children(bars)
                });
                let plot = div().relative().flex_1().h_full().border_b_1().border_color(t.line_strong).child(grid(lo, hi, step, t.line.opacity(0.6))).child(div().absolute().inset_0().flex().children(cols));
                let body = div().flex_1().min_h(px(60.)).flex().child(y_axis(lo, hi, step, b.unit.as_deref(), t)).child(plot);
                d.when(!h, |d| d.h(px(160.))).when(!w, |d| d.w_full()).gap(px(6.)).child(body).child(x_axis(&b.labels, 12, t)).children(legend(&b.series, t)).into_any_element()
            }
            Element::LineChart(l) => {
                let n = l.labels.len();
                let (lo, hi, step) = hw::axis(l.series.iter().flat_map(|s| s.values.iter().copied()), l.min, l.max);
                let span = (hi - lo).max(f64::EPSILON);
                let lines: Vec<(Vec<f64>, Hsla)> = l.series.iter().enumerate().map(|(i, s)| (s.values.clone(), render::color(hw::series_token(s, i), t))).collect();
                let paint = canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        if n == 0 {
                            return;
                        }
                        let (wd, ht) = (f32::from(b.size.width), f32::from(b.size.height));
                        for (values, color) in &lines {
                            let pts: Vec<_> = values.iter().take(n).enumerate().filter(|(_, v)| v.is_finite()).map(|(k, v)| point(b.origin.x + px(wd * (k as f32 + 0.5) / n as f32), b.origin.y + px(ht * (1. - (((v - lo) / span) as f32).clamp(0., 1.))))).collect();
                            if let Some(p) = line(&pts, 1.75) {
                                window.paint_path(p, *color);
                            }
                            for pt in &pts {
                                let r = px(2.25);
                                window.paint_quad(gpui::fill(gpui::Bounds::new(point(pt.x - r, pt.y - r), gpui::size(r * 2., r * 2.)), *color).corner_radii(r));
                            }
                        }
                    },
                )
                .absolute()
                .inset_0();
                let unit = l.unit.clone();
                let hover = t.text.opacity(0.06);
                let cols = (0..n).map(|k| {
                    let mut tip = l.labels[k].clone();
                    for s in &l.series {
                        if let Some(v) = s.values.get(k).filter(|v| v.is_finite()) {
                            tip.push_str(&format!("\n{}: {}", s.name, hw::number(*v, unit.as_deref())));
                        }
                    }
                    div().id(SharedString::from(format!("{id}-{k}"))).flex_1().h_full().hover(move |s| s.bg(hover)).tooltip(kit::tooltip(tip))
                });
                let plot = div().relative().flex_1().h_full().border_b_1().border_color(t.line_strong).child(grid(lo, hi, step, t.line.opacity(0.6))).child(paint).child(div().absolute().inset_0().flex().children(cols));
                let body = div().flex_1().min_h(px(60.)).flex().child(y_axis(lo, hi, step, l.unit.as_deref(), t)).child(plot);
                d.when(!h, |d| d.h(px(180.))).when(!w, |d| d.w_full()).gap(px(6.)).child(body).child(x_axis(&l.labels, 8, t)).children(legend(&l.series, t)).into_any_element()
            }
            Element::StatusTimeline(tl) => {
                let can = handler(node, Listen::Value).is_some();
                let segs: Vec<AnyElement> = tl
                    .segments
                    .iter()
                    .enumerate()
                    .map(|(k, s)| {
                        let color = match s.tone {
                            None => t.text_4,
                            Some(_) => fb::tone_colors(super::widgets::tone(s.tone), t).0,
                        };
                        let el = div()
                            .id(SharedString::from(format!("{id}-{k}")))
                            .debug_selector({
                                let x = format!("tile-segment-{path}/{}", s.key);
                                move || x.clone()
                            })
                            .flex_grow(if s.weight.is_finite() && s.weight > 0. { s.weight } else { 1. })
                            .flex_basis(px(0.))
                            .min_w(px(3.))
                            .h_full()
                            .rounded(px(2.))
                            .bg(color)
                            .tooltip(kit::tooltip(s.label.clone()));
                        if can {
                            let (p, key) = (path.to_string(), s.key.clone());
                            el.cursor_pointer()
                                .hover(|d| d.opacity(0.8))
                                .on_click(cx.listener(move |v, _: &gpui::ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    v.tell(&p, Outcome::Value(ControlValue::Choice(key.clone())), cx);
                                }))
                                .into_any_element()
                        } else {
                            el.into_any_element()
                        }
                    })
                    .collect();
                d.flex_row().gap(px(2.)).when(!h, |d| d.h(px(16.))).when(!w, |d| d.w_full()).children(segs).into_any_element()
            }
            _ => d.into_any_element(),
        }
    }
}
