//! Checks and sizes of the batch 2 components (ui.md §3.10): form
//! controls, feedback, layout, data display and charts, for
//! [`crate::frame::check`]; and what the app and the test harness both
//! draw from them the same way: a relative time's words, an avatar's
//! initials and colour, a chart's summary for screen readers.

use crate::limits::{MAX_CHART_LABELS, MAX_CHART_POINTS, MAX_CHOICES, MAX_COPY_BYTES, MAX_CRUMBS, MAX_INPUT_BYTES, MAX_KEY_BYTES, MAX_KEY_VALUES, MAX_SERIES, MAX_SEGMENTS, MAX_SHORT_TEXT, MAX_URL_BYTES};
use crate::ui::{ChartSeries, ControlValue, Element, Frame, Node, Skeleton, Token};

/// What is wrong with a component, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// More than this many choices, crumbs, lines, series, labels or
    /// segments (the limit is in the text).
    TooMany(&'static str, usize),
    /// A choice's, crumb's or segment's key is longer than
    /// [`MAX_KEY_BYTES`] or used twice.
    Key,
    /// A slider's or number field's range or step isn't usable.
    Range,
    /// A text over its limit (a badge's, a copyable's, a search field's,
    /// a link's address).
    Long(&'static str),
    /// A split without exactly two children.
    SplitPanes(usize),
    /// A popover as the root (it needs a parent to anchor to).
    LoosePopover,
    /// A second popover or confirm sheet in one frame.
    Second(&'static str),
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::TooMany(what, n) => write!(f, "has more than {n} {what}"),
            Problem::Key => write!(f, "has a key used twice or longer than {MAX_KEY_BYTES} bytes"),
            Problem::Range => write!(f, "has a range or step that isn't usable (min below max, a finite step of 0 or more)"),
            Problem::Long(what) => write!(f, "has a {what} that is too long"),
            Problem::SplitPanes(n) => write!(f, "is a split with {n} children (it takes two panes)"),
            Problem::LoosePopover => write!(f, "is a popover without a parent to anchor to"),
            Problem::Second(what) => write!(f, "is a second {what} in the frame (at most one)"),
        }
    }
}

fn keys<'a>(keys: impl Iterator<Item = &'a str>) -> Result<(), Problem> {
    let mut seen = std::collections::HashSet::new();
    for k in keys {
        if k.len() > MAX_KEY_BYTES || !seen.insert(k) {
            return Err(Problem::Key);
        }
    }
    Ok(())
}

fn at_most(n: usize, max: usize, what: &'static str) -> Result<(), Problem> {
    if n > max {
        Err(Problem::TooMany(what, max))
    } else {
        Ok(())
    }
}

fn series(s: &[ChartSeries], points: usize) -> Result<(), Problem> {
    at_most(s.len(), MAX_SERIES, "series")?;
    for x in s {
        at_most(x.values.len(), points, "values in a series")?;
    }
    Ok(())
}

/// Check node `n`'s component, if it is one of batch 2.
pub fn check(n: &Node) -> Result<(), Problem> {
    match &n.element {
        Element::RadioGroup(g) | Element::Segmented(g) => {
            at_most(g.items.len(), MAX_CHOICES, "choices")?;
            keys(g.items.iter().map(|i| i.key.as_str()))?;
        }
        Element::Slider(s) => {
            if !(s.min.is_finite() && s.max.is_finite() && s.min < s.max && s.step.is_finite() && s.step >= 0.) {
                return Err(Problem::Range);
            }
        }
        Element::NumberField(f) => {
            let ok = f.step.is_finite() && f.step > 0. && f.decimals <= 6 && f.min.is_none_or(f64::is_finite) && f.max.is_none_or(f64::is_finite) && !matches!((f.min, f.max), (Some(a), Some(b)) if a > b);
            if !ok {
                return Err(Problem::Range);
            }
        }
        Element::SearchField(s) if s.value.as_ref().is_some_and(|v| v.len() > MAX_INPUT_BYTES) => return Err(Problem::Long("value")),
        Element::Skeleton(Skeleton::Lines(l)) => at_most(*l as usize, 20, "lines")?,
        Element::Badge(b) if b.text.len() > MAX_SHORT_TEXT => return Err(Problem::Long("text")),
        Element::Breadcrumbs(c) => {
            at_most(c.len(), MAX_CRUMBS, "crumbs")?;
            keys(c.iter().map(|c| c.key.as_str()))?;
        }
        Element::KeyValues(kv) => at_most(kv.len(), MAX_KEY_VALUES, "lines")?,
        Element::Copyable(c) if c.text.len() > MAX_COPY_BYTES => return Err(Problem::Long("text")),
        Element::Link(l) if l.url.as_ref().is_some_and(|u| u.len() > MAX_URL_BYTES) => return Err(Problem::Long("address")),
        Element::Sparkline(s) => at_most(s.values.len(), MAX_CHART_POINTS, "values")?,
        Element::BarChart(b) => {
            at_most(b.labels.len(), MAX_CHART_LABELS, "labels")?;
            series(&b.series, MAX_CHART_LABELS)?;
        }
        Element::LineChart(l) => {
            at_most(l.labels.len(), MAX_CHART_POINTS, "labels")?;
            series(&l.series, MAX_CHART_POINTS)?;
        }
        Element::StatusTimeline(t) => {
            at_most(t.segments.len(), MAX_SEGMENTS, "segments")?;
            keys(t.segments.iter().map(|s| s.key.as_str()))?;
        }
        _ => {}
    }
    Ok(())
}

/// Check what spans nodes: a split's two panes, a popover's anchor, one
/// popover and one confirm sheet a frame. `Err((node index, problem))`.
pub fn check_tree(frame: &Frame) -> Result<(), (usize, Problem)> {
    let nodes = &frame.nodes;
    let mut kids = vec![0usize; nodes.len()];
    for n in nodes {
        if let Some(p) = n.parent.and_then(|p| kids.get_mut(p as usize)) {
            *p += 1;
        }
    }
    let (mut popover, mut sheet) = (false, false);
    for (i, n) in nodes.iter().enumerate() {
        match &n.element {
            Element::Split(_) if kids[i] != 2 => return Err((i, Problem::SplitPanes(kids[i]))),
            Element::Popover(_) if n.parent.is_none() => return Err((i, Problem::LoosePopover)),
            Element::Popover(_) if std::mem::replace(&mut popover, true) => return Err((i, Problem::Second("popover"))),
            Element::ConfirmSheet(_) if std::mem::replace(&mut sheet, true) => return Err((i, Problem::Second("confirm sheet"))),
            _ => {}
        }
    }
    Ok(())
}

/// About how many bytes node `n`'s component takes through the canonical
/// ABI (0 for other elements).
pub fn bytes(n: &Node) -> usize {
    let opt = |s: &Option<String>| s.as_ref().map_or(0, String::len);
    let series = |s: &[ChartSeries]| s.iter().map(|x| 16 + x.name.len() + x.values.len() * 8).sum::<usize>();
    match &n.element {
        Element::Checkbox(c) | Element::Switch(c) => 8 + c.label.len(),
        Element::RadioGroup(g) | Element::Segmented(g) => 16 + opt(&g.selected) + g.items.iter().map(|i| 8 + i.key.len() + i.label.len()).sum::<usize>(),
        Element::Slider(_) => 40,
        Element::NumberField(f) => 48 + opt(&f.placeholder),
        Element::SearchField(s) => 16 + opt(&s.value) + opt(&s.placeholder),
        Element::FormRow(r) => 8 + r.label.len() + opt(&r.help) + opt(&r.error),
        Element::EmptyState(e) => 16 + e.title.len() + opt(&e.text) + opt(&e.action),
        Element::Banner(b) => 16 + b.text.len() + opt(&b.title) + opt(&b.action),
        Element::Badge(b) => 8 + b.text.len(),
        Element::Chip(c) => 16 + c.label.len(),
        Element::Kbd(k) => k.len(),
        Element::Disclosure(d) => 8 + d.title.len() + opt(&d.detail),
        Element::Card(c) => 8 + opt(&c.title) + opt(&c.detail),
        Element::Breadcrumbs(c) => c.iter().map(|c| 8 + c.key.len() + c.label.len()).sum(),
        Element::ConfirmSheet(s) => 16 + s.title.len() + opt(&s.text) + s.confirm.len() + opt(&s.cancel),
        Element::KeyValues(kv) => kv.iter().map(|k| 8 + k.key.len() + k.value.len()).sum(),
        Element::Avatar(a) => 4 + a.name.len(),
        Element::Copyable(c) => 4 + c.text.len(),
        Element::Link(l) => 8 + l.label.len() + opt(&l.url),
        Element::Sparkline(s) => 8 + s.values.len() * 8,
        Element::BarChart(b) => 16 + b.labels.iter().map(|l| l.len() + 8).sum::<usize>() + series(&b.series) + opt(&b.unit),
        Element::LineChart(l) => 32 + l.labels.iter().map(|l| l.len() + 8).sum::<usize>() + series(&l.series) + opt(&l.unit),
        Element::StatusTimeline(t) => t.segments.iter().map(|s| 16 + s.key.len() + s.label.len()).sum(),
        _ => 0,
    }
}

/// A time `at` (ms since the epoch) as a tile's `relative-time` draws it
/// at `now`: "just now", "5 min ago", "3 h ago", "yesterday", "4 days
/// ago", then the date ("12 Mar 2026"). A time ahead of now: "in 5 min".
pub fn relative_time(at_ms: i64, now_ms: i64) -> String {
    let ahead = at_ms > now_ms + 30_000;
    let s = (now_ms - at_ms).unsigned_abs() / 1000;
    let (n, unit) = match s {
        0..=44 if !ahead => return "just now".into(),
        0..=59 => (s.max(1), "s"),
        60..=3599 => (s / 60, "min"),
        3600..=86_399 => (s / 3600, "h"),
        _ if !ahead && s < 2 * 86_400 => return "yesterday".into(),
        _ if s < 7 * 86_400 => {
            let d = s / 86_400;
            let days = if d == 1 { "1 day".to_string() } else { format!("{d} days") };
            return if ahead { format!("in {days}") } else { format!("{days} ago") };
        }
        _ => return date(at_ms),
    };
    if ahead {
        format!("in {n} {unit}")
    } else {
        format!("{n} {unit} ago")
    }
}

/// How long until [`relative_time`]'s words for `at` may change, in ms:
/// the app redraws then (at least every second, at most every hour).
pub fn relative_time_next(at_ms: i64, now_ms: i64) -> u64 {
    let age = (now_ms - at_ms).unsigned_abs();
    match age {
        0..=59_999 => 1000,
        60_000..=3_599_999 => 60_000 - age % 60_000,
        _ => (3_600_000 - age % 3_600_000).min(3_600_000),
    }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The civil date (UTC) of `ms` since the epoch: (year, month 1–12, day).
fn civil(ms: i64) -> (i64, u32, u32) {
    // Howard Hinnant's days-from-civil, inverted.
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// "12 Mar 2026".
pub fn date(ms: i64) -> String {
    let (y, m, d) = civil(ms);
    format!("{d} {} {y}", MONTHS[(m - 1) as usize])
}

/// "12 Mar 2026, 14:05 UTC": a relative time's tooltip.
pub fn date_time(ms: i64) -> String {
    let mins = ms.div_euclid(60_000).rem_euclid(24 * 60);
    format!("{}, {:02}:{:02} UTC", date(ms), mins / 60, mins % 60)
}

/// The initials an avatar shows for `name`: the first letters of its
/// first and last words ("Ada Lovelace": "AL"; "ci-bot": "C").
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| c.is_whitespace() || c == '.' || c == '_').filter(|w| !w.is_empty()).collect();
    let first = |w: &str| w.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    match words.as_slice() {
        [] => "?".into(),
        [one] => first(one),
        [a, .., b] => first(a) + &first(b),
    }
}

/// The colours an avatar is drawn in for `name` (background, text): one
/// of the theme's soft tokens and its strong one, picked by the name so a
/// person has the same colour everywhere.
pub fn avatar_tokens(name: &str) -> (Token, Token) {
    const SET: [(Token, Token); 5] = [(Token::DoneSoft, Token::Done), (Token::GoodSoft, Token::Good), (Token::WarnSoft, Token::Warn), (Token::BadSoft, Token::Bad), (Token::Surface3, Token::Text2)];
    // FNV-1a, stable across runs and builds.
    let h = name.trim().to_lowercase().bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    SET[(h % SET.len() as u32) as usize]
}

/// The colours a chart's series take when they name none, in order.
pub const SERIES_TOKENS: [Token; 6] = [Token::Focus, Token::Done, Token::Good, Token::Warn, Token::Bad, Token::Text3];

/// Series `i`'s colour: its own, or the next of [`SERIES_TOKENS`].
pub fn series_token(s: &ChartSeries, i: usize) -> Token {
    s.color.unwrap_or(SERIES_TOKENS[i % SERIES_TOKENS.len()])
}

/// A number as a chart's axis and tooltips write it: at most two
/// decimals, no trailing zeros, thousands as "k" and millions as "M"
/// past 10 000.
pub fn number(v: f64, unit: Option<&str>) -> String {
    let unit = unit.map(|u| if u == "%" { u.to_string() } else { format!(" {u}") }).unwrap_or_default();
    if !v.is_finite() {
        return "–".into();
    }
    let (v, suffix) = if v.abs() >= 10_000_000. {
        (v / 1_000_000., "M")
    } else if v.abs() >= 10_000. {
        (v / 1000., "k")
    } else {
        (v, "")
    };
    let mut s = format!("{v:.2}");
    while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
        s.pop();
    }
    if s == "-0" {
        s = "0".into();
    }
    format!("{s}{suffix}{unit}")
}

/// A chart's lowest and highest axis values: from `min` and `max` when
/// given, else from the data (0 included, so bars start at the axis),
/// rounded out to "nice" steps. (low, high, step)
pub fn axis(values: impl Iterator<Item = f64>, min: Option<f64>, max: Option<f64>) -> (f64, f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in values.filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if !lo.is_finite() {
        (lo, hi) = (0., 1.);
    }
    let lo = min.filter(|v| v.is_finite()).unwrap_or(lo.min(0.));
    let mut hi = max.filter(|v| v.is_finite()).unwrap_or(hi);
    if hi <= lo {
        hi = lo + 1.;
    }
    // About four steps of 1, 2 or 5 × 10ⁿ.
    let raw = (hi - lo) / 4.;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1., 2., 2.5, 5., 10.].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10. * mag);
    let lo = if min.is_some() { lo } else { (lo / step).floor() * step };
    let hi = if max.is_some() { hi } else { (hi / step).ceil() * step };
    (lo, hi, step)
}

/// What a screen reader reads for a chart without a label of its own:
/// its kind, its series and their latest or largest values.
pub fn chart_summary(e: &Element) -> Option<String> {
    let last = |v: &[f64]| v.iter().rev().find(|v| v.is_finite()).copied();
    match e {
        Element::Sparkline(s) => Some(match last(&s.values) {
            Some(v) => format!("Trend of {} values, latest {}", s.values.len(), number(v, None)),
            None => "Trend, no values".into(),
        }),
        Element::BarChart(b) => {
            let parts: Vec<String> = b.series.iter().map(|s| format!("{} up to {}", s.name, number(s.values.iter().copied().filter(|v| v.is_finite()).fold(f64::NAN, f64::max), b.unit.as_deref()))).collect();
            Some(format!("Bar chart of {} labels: {}", b.labels.len(), parts.join("; ")))
        }
        Element::LineChart(l) => {
            let parts: Vec<String> = l.series.iter().map(|s| format!("{} latest {}", s.name, last(&s.values).map_or("–".into(), |v| number(v, l.unit.as_deref())))).collect();
            Some(format!("Line chart over {} labels: {}", l.labels.len(), parts.join("; ")))
        }
        Element::StatusTimeline(t) => {
            let bad = t.segments.iter().filter(|s| s.tone == Some(crate::ui::Tone::Bad)).count();
            Some(format!("{} runs, {bad} failed{}", t.segments.len(), t.segments.last().map(|s| format!("; latest: {}", s.label)).unwrap_or_default()))
        }
        _ => None,
    }
}

/// A slider's value `v` kept in its range and snapped to its step.
pub fn snap(v: f64, min: f64, max: f64, step: f64) -> f64 {
    let v = if v.is_finite() { v } else { min };
    let v = if step > 0. { min + ((v - min) / step).round() * step } else { v };
    v.clamp(min, max)
}

/// A number field's value `v` kept within its bounds and rounded to its
/// decimals.
pub fn clamp_number(v: f64, min: Option<f64>, max: Option<f64>, decimals: u8) -> f64 {
    let p = 10f64.powi(i32::from(decimals.min(6)));
    let v = (v * p).round() / p;
    let v = min.map_or(v, |m| v.max(m));
    max.map_or(v, |m| v.min(m))
}

/// A number field's value as it shows it.
pub fn format_number(v: f64, decimals: u8) -> String {
    format!("{v:.*}", decimals.min(6) as usize)
}

/// What a key or a click on a focused component does, the same in the
/// app and the test harness.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// `value-changed` with this value.
    Value(ControlValue),
    /// `dismissed`.
    Dismiss,
    /// A `click` (a confirm sheet's Enter, a card's Space).
    Click,
}

/// Whether element `e` takes keyboard focus (a Tab stop) of its own, as
/// the app draws it (fields have theirs: number and search fields).
pub fn focusable(e: &Element, clickable: bool) -> bool {
    match e {
        Element::Checkbox(c) | Element::Switch(c) => !c.disabled,
        Element::RadioGroup(g) | Element::Segmented(g) => !g.disabled && g.items.iter().any(|i| !i.disabled),
        Element::Slider(s) => !s.disabled,
        Element::Disclosure(_) | Element::Copyable(_) | Element::Link(_) => true,
        Element::Chip(c) => !c.disabled && (clickable || c.removable),
        Element::Card(_) | Element::Breadcrumbs(_) | Element::StatusTimeline(_) => clickable,
        _ => false,
    }
}

/// Choice `selected` of `items` moved `by` (−1, +1; ±∞ for Home and End)
/// past disabled ones; `None` when it doesn't move.
fn roving(items: &[crate::ui::ChoiceItem], selected: Option<&str>, by: i64) -> Option<String> {
    let live: Vec<&crate::ui::ChoiceItem> = items.iter().filter(|i| !i.disabled).collect();
    if live.is_empty() {
        return None;
    }
    let at = selected.and_then(|k| live.iter().position(|i| i.key == k));
    let to = match (at, by) {
        (_, i64::MIN) => 0,
        (_, i64::MAX) => live.len() - 1,
        (None, _) => 0,
        (Some(i), by) => (i as i64 + by).rem_euclid(live.len() as i64) as usize,
    };
    (Some(to) != at).then(|| live[to].key.clone())
}

/// A click on element `e` (the whole of it: a checkbox and its label, a
/// disclosure's header): what changes.
pub fn on_click(e: &Element) -> Option<Outcome> {
    match e {
        Element::Checkbox(c) | Element::Switch(c) if !c.disabled => Some(Outcome::Value(ControlValue::On(c.mixed || !c.checked))),
        Element::Disclosure(d) => Some(Outcome::Value(ControlValue::On(!d.open))),
        _ => None,
    }
}

/// Key `key` (as GPUI names it: `space`, `enter`, `left`, `pageup`,
/// `home`, `backspace`, `escape`…) pressed while element `e` has focus.
pub fn on_key(e: &Element, key: &str) -> Option<Outcome> {
    let by = match key {
        "left" | "up" => -1,
        "right" | "down" => 1,
        "home" => i64::MIN,
        "end" => i64::MAX,
        _ => 0,
    };
    match e {
        Element::Checkbox(_) | Element::Switch(_) if key == "space" => on_click(e),
        Element::Disclosure(_) if key == "space" || key == "enter" => on_click(e),
        Element::RadioGroup(g) | Element::Segmented(g) if !g.disabled && by != 0 => roving(&g.items, g.selected.as_deref(), by).map(|k| Outcome::Value(ControlValue::Choice(k))),
        Element::RadioGroup(g) | Element::Segmented(g) if !g.disabled && key == "space" => {
            let none = g.selected.is_none();
            roving(&g.items, None, 0).filter(|_| none).map(|k| Outcome::Value(ControlValue::Choice(k)))
        }
        Element::Slider(s) if !s.disabled => {
            let small = if s.step > 0. { s.step } else { (s.max - s.min) / 100. };
            let big = ((s.max - s.min) / 10.).max(small);
            let v = match key {
                "left" | "down" => s.value - small,
                "right" | "up" => s.value + small,
                "pagedown" => s.value - big,
                "pageup" => s.value + big,
                "home" => s.min,
                "end" => s.max,
                _ => return None,
            };
            let v = snap(v, s.min, s.max, s.step);
            (v != s.value).then_some(Outcome::Value(ControlValue::Number(v)))
        }
        Element::NumberField(f) if !f.disabled && !f.readonly => {
            let d = match key {
                "up" => f.step,
                "down" => -f.step,
                _ => return None,
            };
            // An empty field steps to its lowest value (or 0).
            let v = match f.value {
                Some(v) => v + d,
                None => f.min.unwrap_or(0.),
            };
            let v = clamp_number(v, f.min, f.max, f.decimals);
            (Some(v) != f.value).then_some(Outcome::Value(ControlValue::Number(v)))
        }
        Element::Chip(c) if c.removable && !c.disabled && (key == "backspace" || key == "delete") => Some(Outcome::Dismiss),
        Element::ConfirmSheet(_) if key == "enter" => Some(Outcome::Click),
        Element::ConfirmSheet(_) | Element::Popover(_) if key == "escape" => Some(Outcome::Dismiss),
        Element::Card(_) | Element::Link(_) | Element::Copyable(_) | Element::Chip(_) if key == "enter" || key == "space" => Some(Outcome::Click),
        _ => None,
    }
}

/// A number field's − or + (`up`): the value after the step.
pub fn step_number(f: &crate::ui::NumberField, up: bool) -> Option<f64> {
    match on_key(&Element::NumberField(f.clone()), if up { "up" } else { "down" }) {
        Some(Outcome::Value(ControlValue::Number(v))) => Some(v),
        _ => None,
    }
}

/// Text typed into a number field, read when it is submitted: the value
/// to send (`Some(None)`: emptied), or `None` when it isn't a number (the
/// field shows its value again).
pub fn parse_number(text: &str, f: &crate::ui::NumberField) -> Option<Option<f64>> {
    let t = text.trim().replace(',', ".").replace([' ', '\u{a0}', '_'], "");
    if t.is_empty() {
        return Some(None);
    }
    let v: f64 = t.parse().ok().filter(|v: &f64| v.is_finite())?;
    Some(Some(clamp_number(v, f.min, f.max, f.decimals)))
}

/// A split's share kept so both panes keep their smallest sizes in
/// `total` logical pixels (and within 0.05–0.95).
pub fn split_share(share: f32, total: f32, min_first: f32, min_second: f32) -> f32 {
    let share = if share.is_finite() { share.clamp(0.05, 0.95) } else { 0.5 };
    if total <= 0. {
        return share;
    }
    let lo = (min_first.max(0.) / total).min(0.95);
    let hi = (1. - min_second.max(0.) / total).max(0.05);
    if lo > hi {
        return share;
    }
    share.clamp(lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{ChoiceGroup, ChoiceItem, Crumb, NumberField, Slider, Split};

    fn node(parent: Option<u32>, key: &str, element: Element) -> Node {
        Node { parent, key: key.into(), element, style: vec![], on: vec![], focusable: false, tooltip: None, label: None }
    }

    #[test]
    fn components_keep_to_their_limits() {
        let item = |k: &str| ChoiceItem { key: k.into(), label: k.into(), disabled: false };
        let group = |items| node(None, "r", Element::RadioGroup(ChoiceGroup { items, selected: None, horizontal: false, disabled: false }));
        assert_eq!(check(&group(vec![item("a"), item("b")])), Ok(()));
        assert_eq!(check(&group(vec![item("a"), item("a")])), Err(Problem::Key));
        assert!(matches!(check(&group((0..65).map(|i| item(&i.to_string())).collect())), Err(Problem::TooMany(..))));
        let slider = |min, max, step| node(None, "s", Element::Slider(Slider { value: 0., min, max, step, show_value: false, disabled: false }));
        assert_eq!(check(&slider(0., 10., 1.)), Ok(()));
        assert_eq!(check(&slider(10., 0., 1.)), Err(Problem::Range));
        assert_eq!(check(&slider(0., f64::NAN, 1.)), Err(Problem::Range));
        let num = NumberField { value: None, min: Some(5.), max: Some(1.), step: 1., decimals: 0, placeholder: None, readonly: false, disabled: false };
        assert_eq!(check(&node(None, "n", Element::NumberField(num))), Err(Problem::Range));
        let crumbs = node(None, "b", Element::Breadcrumbs(vec![Crumb { key: "x".into(), label: "X".into() }; 2]));
        assert_eq!(check(&crumbs), Err(Problem::Key));
    }

    #[test]
    fn splits_take_two_panes_and_popovers_an_anchor() {
        let f = |nodes| Frame { nodes, focus: None, keymap: vec![] };
        let split = || Element::Split(Split { orientation: crate::ui::Orientation::Horizontal, initial: 0.5, min_first: 100., min_second: 100. });
        let pop = || Element::Popover(crate::ui::Popover { placement: crate::ui::Placement::Below, width: None, at: None });
        assert_eq!(check_tree(&f(vec![node(None, "r", split()), node(Some(0), "a", Element::Box), node(Some(0), "b", Element::Box)])), Ok(()));
        assert_eq!(check_tree(&f(vec![node(None, "r", split()), node(Some(0), "a", Element::Box)])), Err((0, Problem::SplitPanes(1))));
        assert_eq!(check_tree(&f(vec![node(None, "r", pop())])), Err((0, Problem::LoosePopover)));
        assert_eq!(check_tree(&f(vec![node(None, "r", Element::Box), node(Some(0), "a", pop()), node(Some(0), "b", pop())])), Err((2, Problem::Second("popover"))));
    }

    #[test]
    fn relative_times_read_as_pitwall_writes_them() {
        let now = 1_773_324_300_000; // 12 Mar 2026, 14:05 UTC
        assert_eq!(relative_time(now - 10_000, now), "just now");
        assert_eq!(relative_time(now - 50_000, now), "50 s ago");
        assert_eq!(relative_time(now - 5 * 60_000, now), "5 min ago");
        assert_eq!(relative_time(now - 3 * 3_600_000, now), "3 h ago");
        assert_eq!(relative_time(now - 30 * 3_600_000, now), "yesterday");
        assert_eq!(relative_time(now - 4 * 86_400_000, now), "4 days ago");
        assert_eq!(relative_time(now - 40 * 86_400_000, now), "31 Jan 2026");
        assert_eq!(relative_time(now + 5 * 60_000, now), "in 5 min");
        assert_eq!(date_time(now), "12 Mar 2026, 14:05 UTC");
        assert_eq!(relative_time_next(now - 90_000, now), 30_000);
    }

    #[test]
    fn avatars_charts_and_numbers() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("ci-bot"), "C");
        assert_eq!(initials("  "), "?");
        assert_eq!(avatar_tokens("Ada Lovelace"), avatar_tokens(" ada lovelace "));
        assert_eq!(number(1234.5, Some("ms")), "1234.5 ms");
        assert_eq!(number(12_500., None), "12.5k");
        assert_eq!(number(42., Some("%")), "42%");
        assert_eq!(axis([3., 17.].into_iter(), None, None), (0., 20., 5.));
        assert_eq!(snap(4.4, 0., 10., 0.5), 4.5);
        assert_eq!(clamp_number(12.345, Some(0.), Some(10.), 1), 10.);
        assert_eq!(format_number(2.5, 2), "2.50");
    }

    #[test]
    fn keys_move_controls_as_in_pitwall() {
        let item = |k: &str, off| ChoiceItem { key: k.into(), label: k.into(), disabled: off };
        let g = Element::Segmented(ChoiceGroup { items: vec![item("a", false), item("b", true), item("c", false)], selected: Some("a".into()), horizontal: true, disabled: false });
        assert_eq!(on_key(&g, "right"), Some(Outcome::Value(ControlValue::Choice("c".into()))), "past the disabled one");
        assert_eq!(on_key(&g, "left"), Some(Outcome::Value(ControlValue::Choice("c".into()))), "wrapping");
        assert_eq!(on_key(&g, "home"), None, "already first");
        let s = Element::Slider(Slider { value: 5., min: 0., max: 10., step: 1., show_value: false, disabled: false });
        assert_eq!(on_key(&s, "right"), Some(Outcome::Value(ControlValue::Number(6.))));
        assert_eq!(on_key(&s, "end"), Some(Outcome::Value(ControlValue::Number(10.))));
        let n = NumberField { value: Some(9.), min: Some(0.), max: Some(10.), step: 2., decimals: 0, placeholder: None, readonly: false, disabled: false };
        assert_eq!(step_number(&n, true), Some(10.));
        assert_eq!(parse_number("3,4", &NumberField { decimals: 1, ..n.clone() }), Some(Some(3.4)));
        assert_eq!(parse_number("abc", &n), None);
        assert_eq!(parse_number(" ", &n), Some(None));
        assert_eq!(split_share(0.1, 1000., 200., 200.), 0.2);
    }
}
