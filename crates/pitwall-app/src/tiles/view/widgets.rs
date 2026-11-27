//! The form controls, feedback, layout and data display components of a
//! tile's view (docs/spec/tile-sdk/ui.md §3.10), drawn with Pitwall's own
//! kit so a tile's controls are the app's ("kit parity"); the charts are
//! in [`super::charts`].
//!
//! - Controls are controlled: a click, a key or a drag sends
//!   `value-changed` and the next frame shows the new value. What a key
//!   or a click does comes from `pitwall_tile_host::widgets`, the rules
//!   the harness uses too. A slider shows the value under the pointer
//!   while it is dragged (sent at most every 50 ms, and at the end).
//! - A number field is Pitwall's text field: what is typed is read on
//!   Enter or when it loses focus (not a number: its value comes back);
//!   Up and Down step it. A search field is a text field like `input`,
//!   with a magnifier and a clear button.
//! - A split's bar is dragged; the share the user left is kept for the
//!   view (in `ui.json`) and wins over the tile's `initial`.
//! - A popover floats at its place in its parent, closed on a click
//!   outside it or Esc; a confirm sheet covers the tile, Enter confirms
//!   and Esc or a click outside it cancels.
//! - A link opens its page in Pitwall's web view only when the extension
//!   has `show`; otherwise the tile hears the click. A copy button writes
//!   the clipboard only when it is clicked.
//! - A relative time is redrawn when its words change: the tile needs no
//!   timer.

use std::time::{Duration, Instant};

use gpui::{anchored, deferred, div, prelude::*, px, relative, AnyElement, App, ClipboardItem, Context, Anchor as Corner, Entity, Focusable as _, MouseButton, SharedString, Subscription, Window};
use pitwall_tile_host::ui::{self, ControlValue, Element, Event, Listen, Node};
use pitwall_tile_host::widgets::{self as hw, Outcome};

use super::{bounds_probe, focus_ring, handler, NodeState, TileView};
use crate::kit::{self, feedback as fb, InputEvent, TextInput};
use crate::theme::{Theme, RADIUS, RADIUS_LG};
use crate::tiles::render::{self, Inherit};

/// A slider's value reaches the tile at most this often while dragged.
const SLIDE_EVERY: Duration = Duration::from_millis(50);
/// How long a copy button shows its tick.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// What a component node keeps across frames.
#[derive(Default)]
pub(super) struct WidgetState {
    number: Option<NumberInput>,
    /// A slider's value while it is dragged.
    live: Option<f64>,
    last_sent: Option<Instant>,
    /// A split's share the user dragged to.
    share: Option<f32>,
    dragging: bool,
    /// A copy button was clicked then.
    copied: Option<Instant>,
}

struct NumberInput {
    input: Entity<TextInput>,
    /// The value the frame named last, applied once.
    named: Option<Option<f64>>,
    _subs: Vec<Subscription>,
}

/// What a slider's knob or a split's bar drags (only for `on_drag_move`).
struct SlideDrag(String);
struct SplitDrag(String);

struct NoGhost;

impl Render for NoGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Whether element `e` is drawn by this module (or [`super::charts`]).
pub(super) fn is_widget(e: &Element) -> bool {
    matches!(
        e,
        Element::Checkbox(_)
            | Element::Switch(_)
            | Element::RadioGroup(_)
            | Element::Segmented(_)
            | Element::Slider(_)
            | Element::NumberField(_)
            | Element::SearchField(_)
            | Element::FormRow(_)
            | Element::Progress(_)
            | Element::Spinner
            | Element::Skeleton(_)
            | Element::EmptyState(_)
            | Element::Banner(_)
            | Element::Badge(_)
            | Element::Chip(_)
            | Element::Kbd(_)
            | Element::Split(_)
            | Element::Disclosure(_)
            | Element::Card(_)
            | Element::Divider(_)
            | Element::Breadcrumbs(_)
            | Element::Toolbar
            | Element::Popover(_)
            | Element::ConfirmSheet(_)
            | Element::KeyValues(_)
            | Element::Avatar(_)
            | Element::RelativeTime(_)
            | Element::Copyable(_)
            | Element::Link(_)
            | Element::Sparkline(_)
            | Element::BarChart(_)
            | Element::LineChart(_)
            | Element::StatusTimeline(_)
    )
}

/// A search field as the text field Pitwall keeps for it.
pub(super) fn search_input(s: &ui::SearchField) -> ui::Input {
    ui::Input { value: s.value.clone(), revision: s.revision, selection: None, placeholder: s.placeholder.clone(), multiline: false, rows: None, disabled: s.disabled }
}

/// The kit's tone for a tile's.
pub(super) fn tone(t: Option<ui::Tone>) -> fb::Tone {
    match t {
        None => fb::Tone::Plain,
        Some(ui::Tone::Info) => fb::Tone::Info,
        Some(ui::Tone::Good) => fb::Tone::Good,
        Some(ui::Tone::Warn) => fb::Tone::Warn,
        Some(ui::Tone::Bad) => fb::Tone::Bad,
    }
}

/// A key as Pitwall writes it: `cmd-shift-k` is ⌘⇧K.
pub(super) fn key_label(keys: &str) -> String {
    keys.split(' ')
        .map(|chord| {
            chord
                .split('-')
                .map(|part| match part {
                    "cmd" | "platform" | "secondary" => "⌘".to_string(),
                    "ctrl" | "control" => "⌃".to_string(),
                    "alt" | "option" => "⌥".to_string(),
                    "shift" => "⇧".to_string(),
                    "enter" => "↩".to_string(),
                    "escape" => "Esc".to_string(),
                    "tab" => "⇥".to_string(),
                    "backspace" => "⌫".to_string(),
                    "up" => "↑".to_string(),
                    "down" => "↓".to_string(),
                    "left" => "←".to_string(),
                    "right" => "→".to_string(),
                    "space" => "Space".to_string(),
                    p if p.chars().count() == 1 => p.to_uppercase(),
                    p => {
                        let mut c = p.chars();
                        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tell node `path`'s tile that its choice is now `key` (see
/// [`TileView::choose`]).
pub(super) fn tell_value(v: &TileView, path: &str, key: &str, cx: &App) -> bool {
    v.tell(path, Outcome::Value(ControlValue::Choice(key.to_string())), cx)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

impl TileView {
    /// Node `node` at `path` was applied: its component's state made or
    /// updated.
    pub(super) fn sync_widget(&mut self, st: &mut NodeState, node: &Node, path: &str, cx: &mut Context<Self>) {
        if !is_widget(&node.element) {
            st.widget = None;
            return;
        }
        let w = st.widget.get_or_insert_with(WidgetState::default);
        match &node.element {
            Element::NumberField(f) => {
                let n = w.number.get_or_insert_with(|| {
                    let input = cx.new(|cx| TextInput::new(cx, "", ""));
                    let p = path.to_string();
                    let subs = vec![cx.subscribe(&input, move |this: &mut TileView, _, e: &InputEvent, cx| {
                        if matches!(e, InputEvent::Submit | InputEvent::SubmitShift | InputEvent::Blur) {
                            this.number_typed(&p, cx);
                        }
                    })];
                    NumberInput { input, named: None, _subs: subs }
                });
                let (placeholder, off) = (f.placeholder.clone().unwrap_or_default(), f.disabled || f.readonly);
                n.input.update(cx, |i, cx| {
                    i.set_placeholder(placeholder);
                    i.set_disabled(off, cx);
                });
                if n.named != Some(f.value) {
                    let text = f.value.map(|v| hw::format_number(v, f.decimals)).unwrap_or_default();
                    n.input.update(cx, |i, cx| i.set_text(text, cx));
                    n.named = Some(f.value);
                }
            }
            Element::Split(_) if w.share.is_none() => {
                let key = format!("{}/{path}", self.tile);
                w.share = crate::ui_state::get(cx).extension(&self.ext).splits.get(&key).copied();
            }
            _ => {}
        }
    }

    /// The focus handle of component `e` at `path` for Tab: a number
    /// field's text field, or its own when it takes focus now.
    pub(super) fn widget_focus(&self, st: &NodeState, node: &Node, cx: &App) -> Option<gpui::FocusHandle> {
        match &node.element {
            Element::NumberField(f) if !f.disabled => st.widget.as_ref()?.number.as_ref().map(|n| n.input.read(cx).focus_handle(cx)),
            Element::SearchField(s) if !s.disabled => st.field.as_ref().map(|f| f.input.read(cx).focus_handle(cx)),
            e if hw::focusable(e, handler(node, Listen::Click).is_some()) || node.focusable => st.focus.clone(),
            _ => None,
        }
    }

    /// Whether a number field's text field has the keyboard (the node's).
    pub(super) fn number_focused(&self, window: &Window, cx: &App) -> Option<String> {
        self.nodes.iter().find_map(|(p, st)| st.widget.as_ref()?.number.as_ref().filter(|n| n.input.read(cx).is_focused(window)).map(|_| p.clone()))
    }

    /// Send what `o` says to the tile, with node `path`'s handlers.
    pub(super) fn tell(&self, path: &str, o: Outcome, cx: &App) -> bool {
        let Some(n) = self.node(path) else { return false };
        let ev = match o {
            Outcome::Value(value) => handler(n, Listen::Value).map(|h| Event::ValueChanged(ui::ValueChange { handler: h, value })),
            Outcome::Dismiss => handler(n, Listen::Dismiss).map(Event::Dismissed),
            Outcome::Click => handler(n, Listen::Click).map(|h| Event::Click(ui::Click { handler: h, button: ui::MouseButton::Left, modifiers: ui::Modifiers::empty(), count: 1, at: ui::Point { x: 0., y: 0. } })),
        };
        ev.is_some_and(|e| self.send(e, cx))
    }

    /// Key `key` (no modifiers) for the components: an open confirm sheet
    /// takes Enter and Esc, an open popover Esc, then the focused
    /// component its keys. Whether it was used.
    pub(super) fn widget_key(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(frame) = self.frame.clone() else { return false };
        let modal = frame.nodes.iter().position(|n| matches!(n.element, Element::ConfirmSheet(_))).or_else(|| frame.nodes.iter().position(|n| matches!(n.element, Element::Popover(_))).filter(|_| key == "escape"));
        if let Some(i) = modal {
            if let Some(o) = hw::on_key(&frame.nodes[i].element, key) {
                let p = self.paths[i].clone();
                self.tell(&p, o, cx);
                return true;
            }
            if matches!(frame.nodes[i].element, Element::ConfirmSheet(_)) {
                return false;
            }
        }
        let Some(path) = self.number_focused(window, cx).or_else(|| self.focused_node(window, cx)) else { return false };
        let Some(n) = self.node(&path) else { return false };
        if !is_widget(&n.element) {
            return false;
        }
        match hw::on_key(&n.element, key) {
            Some(Outcome::Click) if handler(n, Listen::Click).is_none() => {
                // A copy button or a link without a listener of the tile's.
                let e = n.element.clone();
                self.activate(&path, &e, cx);
                true
            }
            Some(o) => {
                if matches!(n.element, Element::Copyable(_) | Element::Link(_)) {
                    let e = n.element.clone();
                    self.activate(&path, &e, cx);
                    return true;
                }
                self.tell(&path, o, cx);
                true
            }
            None => false,
        }
    }

    /// A copy button or a link used (clicked, Enter): it copies or opens,
    /// and the tile hears the click when it listens and the link didn't
    /// open.
    fn activate(&mut self, path: &str, e: &Element, cx: &mut Context<Self>) {
        match e {
            Element::Copyable(c) => {
                cx.write_to_clipboard(ClipboardItem::new_string(c.text.clone()));
                if let Some(w) = self.nodes.get_mut(path).and_then(|s| s.widget.as_mut()) {
                    w.copied = Some(Instant::now());
                }
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(COPIED_FOR).await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                })
                .detach();
                self.tell(path, Outcome::Click, cx);
                cx.notify();
            }
            Element::Link(l) => {
                let opened = l.url.as_deref().is_some_and(|u| self.open_page(u, cx));
                if !opened {
                    self.tell(path, Outcome::Click, cx);
                }
            }
            _ => {}
        }
    }

    /// Open web page `url` in Pitwall's web view for the tile: only with
    /// `show` granted and an address the web view takes.
    fn open_page(&self, url: &str, cx: &mut App) -> bool {
        let Some(m) = crate::extensions::manifest_of(&self.ext, cx) else { return false };
        if !crate::tiles::grants_of(&m, cx).has(pitwall_core::extensions::permissions::SHOW) || pitwall_core::extensions::view::web_view_url(url).is_err() {
            return false;
        }
        let api = crate::extensions::calls::api(cx);
        let Some(server) = api.server.clone() else { return false };
        let Some(caller) = server.extension_caller(&self.ext) else { return false };
        let params = serde_json::json!({"agentId": null, "url": url});
        cx.background_executor()
            .spawn(async move {
                let _ = server.call_as(&caller, pitwall_proto::method::WEB_OPEN, params, pitwall_core::extensions::activity::Via::View);
            })
            .detach();
        true
    }

    /// What was typed into number field `path`, read: sent when it is a
    /// new number, else the field shows its value again.
    fn number_typed(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(Element::NumberField(f)) = self.node(path).map(|n| n.element.clone()) else { return };
        let Some(n) = self.nodes.get_mut(path).and_then(|s| s.widget.as_mut()).and_then(|w| w.number.as_mut()) else { return };
        let text = n.input.read(cx).text().to_string();
        match hw::parse_number(&text, &f) {
            Some(v) if v != f.value => {
                let value = v.map_or(ControlValue::Empty, ControlValue::Number);
                // Shown as the field keeps it (bounds, decimals).
                let shown = v.map(|v| hw::format_number(v, f.decimals)).unwrap_or_default();
                n.input.update(cx, |i, cx| i.set_text(shown, cx));
                self.tell(path, Outcome::Value(value), cx);
            }
            _ => {
                let back = f.value.map(|v| hw::format_number(v, f.decimals)).unwrap_or_default();
                if text != back {
                    n.input.update(cx, |i, cx| i.set_text(back, cx));
                }
            }
        }
    }

    /// A slider at `path` dragged or clicked at window x `x`; `end`: let go.
    fn slide(&mut self, path: &str, x: f32, end: bool, cx: &mut Context<Self>) {
        let Some(Element::Slider(s)) = self.node(path).map(|n| n.element.clone()) else { return };
        let Some(st) = self.nodes.get_mut(path) else { return };
        let Some(b) = st.bounds.get() else { return };
        let w = st.widget.get_or_insert_with(WidgetState::default);
        let share = ((x - f32::from(b.left())) / f32::from(b.size.width).max(1.)).clamp(0., 1.) as f64;
        let v = hw::snap(s.min + share * (s.max - s.min), s.min, s.max, s.step);
        let due = end || w.last_sent.is_none_or(|t| t.elapsed() >= SLIDE_EVERY);
        w.live = (!end).then_some(v);
        let changed = w.live != Some(s.value) || end;
        if due && changed && v != s.value {
            w.last_sent = Some(Instant::now());
            self.tell(path, Outcome::Value(ControlValue::Number(v)), cx);
        }
        cx.notify();
    }

    /// A split's bar dragged to window position `at` in `bounds`; `end`:
    /// let go (the share is told and kept).
    fn split_to(&mut self, path: &str, at: f32, bounds: gpui::Bounds<gpui::Pixels>, end: bool, cx: &mut Context<Self>) {
        let Some(Element::Split(s)) = self.node(path).map(|n| n.element.clone()) else { return };
        let Some(w) = self.nodes.get_mut(path).and_then(|st| st.widget.as_mut()) else { return };
        if end {
            if !w.dragging {
                return;
            }
            w.dragging = false;
            let share = w.share.unwrap_or(s.initial);
            let (ext, key) = (self.ext.clone(), format!("{}/{path}", self.tile));
            crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(&ext, |e| {
                e.splits.insert(key, share);
            }));
            self.tell(path, Outcome::Value(ControlValue::Number(f64::from(share))), cx);
            return;
        }
        let horizontal = s.orientation == ui::Orientation::Horizontal;
        let (start, total) = if horizontal { (f32::from(bounds.left()), f32::from(bounds.size.width)) } else { (f32::from(bounds.top()), f32::from(bounds.size.height)) };
        w.dragging = true;
        w.share = Some(hw::split_share((at - start) / total.max(1.), total, s.min_first, s.min_second));
        cx.notify();
    }

    /// The box a component is drawn in: its styles, key, tooltip, focus
    /// (with the ring) and where it was drawn.
    fn shell(&self, node: &Node, path: &str, st: Option<&NodeState>, window: &Window, t: &Theme) -> gpui::Stateful<gpui::Div> {
        let id = SharedString::from(format!("tile-{}-{}-{path}", self.ext, self.tile));
        let sel = format!("tile-node-{path}");
        let (d, _) = render::apply(div(), &node.style, t);
        let mut d = d.id(id).debug_selector(move || sel.clone()).relative().child(bounds_probe(st)).when_some(node.tooltip.clone(), |d, tip| d.tooltip(kit::tooltip(tip)));
        if let Some(h) = st.and_then(|s| s.focus.as_ref()) {
            let focused = h.is_focused(window);
            d = d.track_focus(h).when(focused, |d| d.shadow(vec![focus_ring(t)]).rounded(RADIUS));
        }
        d
    }

    /// `d`, telling the tile `o` when clicked (and taking focus).
    fn on_tap(&self, d: gpui::Stateful<gpui::Div>, path: &str, st: Option<&NodeState>, o: Outcome, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let p = path.to_string();
        let focus = st.and_then(|s| s.focus.clone());
        d.cursor_pointer().on_click(cx.listener(move |v, _: &gpui::ClickEvent, window, cx| {
            cx.stop_propagation();
            if let Some(f) = &focus {
                window.focus(f, cx);
            }
            v.tell(&p, o.clone(), cx);
        }))
    }

    /// Node `i`'s children, built.
    fn kids_of(&self, i: usize, inherit: Inherit, window: &mut Window, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.kids[i].clone().into_iter().map(|k| self.build(k, inherit, window, t, cx)).collect()
    }

    /// Component node `i` at `path` as an element.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_widget(&self, i: usize, node: &Node, path: &str, inherit: Inherit, window: &mut Window, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let st = self.nodes.get(path);
        let w = st.and_then(|s| s.widget.as_ref());
        let id = SharedString::from(format!("tile-{}-{}-{path}", self.ext, self.tile));
        let label = |text: &str, color| div().text_size(px(13.)).text_color(color).child(text.to_string());
        match &node.element {
            Element::Checkbox(c) | Element::Switch(c) => {
                let is_box = matches!(node.element, Element::Checkbox(_));
                let mark = if is_box {
                    if c.mixed {
                        div().flex_none().size(px(14.)).rounded(px(3.)).bg(t.text).flex().items_center().justify_center().child(div().w(px(8.)).h(px(2.)).rounded(px(1.)).bg(t.on_text())).into_any_element()
                    } else {
                        kit::checkbox_box(c.checked, false, 14., t).into_any_element()
                    }
                } else {
                    kit::toggle(SharedString::from(format!("{id}-switch")), c.checked, t).into_any_element()
                };
                let d = self.shell(node, path, st, window, t).flex_row().items_center().gap(px(8.)).child(mark).child(label(&c.label, t.text)).when(c.disabled, |d| d.opacity(0.5));
                match hw::on_click(&node.element) {
                    Some(o) if handler(node, Listen::Value).is_some() => self.on_tap(d, path, st, o, cx).into_any_element(),
                    _ => d.into_any_element(),
                }
            }
            Element::RadioGroup(g) => {
                let can = !g.disabled && handler(node, Listen::Value).is_some();
                let rows: Vec<AnyElement> = g
                    .items
                    .iter()
                    .enumerate()
                    .map(|(ix, item)| {
                        let on = g.selected.as_deref() == Some(item.key.as_str());
                        let row = div().id(SharedString::from(format!("{id}-{ix}"))).debug_selector({
                            let s = format!("tile-choice-{path}/{}", item.key);
                            move || s.clone()
                        });
                        let row = row.flex().items_center().gap(px(8.)).child(kit::radio(on, t)).child(label(&item.label, t.text)).when(item.disabled, |d| d.opacity(0.5));
                        if can && !item.disabled && !on {
                            let (p, key) = (path.to_string(), item.key.clone());
                            let focus = st.and_then(|s| s.focus.clone());
                            row.cursor_pointer()
                                .on_click(cx.listener(move |v, _: &gpui::ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    if let Some(f) = &focus {
                                        window.focus(f, cx);
                                    }
                                    v.tell(&p, Outcome::Value(ControlValue::Choice(key.clone())), cx);
                                }))
                                .into_any_element()
                        } else {
                            row.into_any_element()
                        }
                    })
                    .collect();
                let d = self.shell(node, path, st, window, t).gap(px(6.)).when(g.horizontal, |d| d.flex_row().gap(px(16.))).children(rows).when(g.disabled, |d| d.opacity(0.5));
                d.into_any_element()
            }
            Element::Segmented(g) => {
                // Tabs' segmented control, as a form value.
                let can = !g.disabled && handler(node, Listen::Value).is_some();
                let keys: Vec<(String, bool)> = g.items.iter().map(|i| (i.key.clone(), i.disabled)).collect();
                let items = g.items.iter().map(|i| kit::SegItem::new(i.label.clone(), g.selected.as_deref() == Some(i.key.as_str()))).collect();
                let me = cx.entity().downgrade();
                let p = path.to_string();
                let pick: kit::OnPick = std::rc::Rc::new(move |ix, _, cx| {
                    if let Some((key, false)) = keys.get(ix).cloned().filter(|_| can) {
                        let _ = me.update(cx, |v, cx| v.tell(&p, Outcome::Value(ControlValue::Choice(key)), cx));
                    }
                });
                self.shell(node, path, st, window, t).flex_row().child(kit::seg(id, items, t, pick)).when(g.disabled, |d| d.opacity(0.5)).into_any_element()
            }
            Element::Slider(s) => {
                let value = w.and_then(|w| w.live).unwrap_or(s.value);
                let share = if s.max > s.min { ((value - s.min) / (s.max - s.min)) as f32 } else { 0. };
                let focused = st.and_then(|s| s.focus.as_ref()).is_some_and(|h| h.is_focused(window));
                let track_id = SharedString::from(format!("{id}-track"));
                let can = !s.disabled && handler(node, Listen::Value).is_some();
                let mut track = div().id(track_id).flex_1().min_w(px(60.)).h(px(16.)).flex().items_center().relative().child(bounds_probe(st)).child(fb::slider_track(share, focused, s.disabled, t));
                if can {
                    let (p1, p2, p3, p4, p5) = (path.to_string(), path.to_string(), path.to_string(), path.to_string(), path.to_string());
                    let focus = st.and_then(|s| s.focus.clone());
                    track = track
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, cx.listener(move |v, e: &gpui::MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if let Some(f) = &focus {
                                window.focus(f, cx);
                            }
                            v.slide(&p1, f32::from(e.position.x), false, cx);
                        }))
                        .on_drag(SlideDrag(path.to_string()), |_, _, _, cx| cx.new(|_| NoGhost))
                        .on_drag_move::<SlideDrag>(cx.listener(move |v, e: &gpui::DragMoveEvent<SlideDrag>, _, cx| {
                            if e.drag(cx).0 == p2 {
                                v.slide(&p2, f32::from(e.event.position.x), false, cx);
                            }
                        }))
                        .on_mouse_up(MouseButton::Left, cx.listener(move |v, e: &gpui::MouseUpEvent, _, cx| v.slide_end(&p3, f32::from(e.position.x), cx)))
                        .on_mouse_up_out(MouseButton::Left, cx.listener(move |v, e: &gpui::MouseUpEvent, _, cx| v.slide_end(&p4, f32::from(e.position.x), cx)))
                        .on_drop(cx.listener(move |v, _: &SlideDrag, window, cx| v.slide_end(&p5, f32::from(window.mouse_position().x), cx)));
                }
                let shown = s.show_value.then(|| div().flex_none().min_w(px(32.)).text_size(px(12.)).text_color(t.text_2).child(hw::number(value, None)));
                // The node's focus ring goes round the track's row.
                self.shell(node, path, st, window, t).flex_row().items_center().gap(px(10.)).child(track).children(shown).into_any_element()
            }
            Element::NumberField(f) => {
                let Some(n) = w.and_then(|w| w.number.as_ref()) else { return div().into_any_element() };
                let can = !f.disabled && !f.readonly && handler(node, Listen::Value).is_some();
                let step = |up: bool, name: &str| {
                    let p = path.to_string();
                    let f = f.clone();
                    let b = kit::icon_btn(SharedString::from(format!("{id}-{name}")), if up { "plus" } else { "minus" }, if up { "More" } else { "Less" }, true, t).debug_selector({
                        let s = format!("tile-step-{path}-{name}");
                        move || s.clone()
                    });
                    let b = b.when(hw::step_number(&f, up).is_none(), |d| d.opacity(0.4));
                    b.on_click(cx.listener(move |v, _: &gpui::ClickEvent, _, cx| {
                        cx.stop_propagation();
                        if let Some(x) = hw::step_number(&f, up) {
                            v.tell(&p, Outcome::Value(ControlValue::Number(x)), cx);
                        }
                    }))
                };
                let steps = can.then(|| div().flex().flex_none().gap(px(2.)).child(step(false, "down")).child(step(true, "up")));
                let d = self.shell(node, path, st, window, t);
                let d = d.flex_row().items_center().gap(px(4.)).child(div().flex_1().min_w(px(48.)).child(n.input.clone())).children(steps);
                d.when(f.disabled, |d| d.opacity(0.5)).into_any_element()
            }
            Element::SearchField(s) => {
                let Some(field) = st.and_then(|s| s.field.as_ref()) else { return div().into_any_element() };
                let has_text = !field.input.read(cx).text().is_empty();
                let p = path.to_string();
                let clear = (has_text && !s.disabled).then(|| {
                    let input = field.input.clone();
                    kit::icon_btn(SharedString::from(format!("{id}-clear")), "x", "Clear", true, t)
                        .debug_selector({
                            let s = format!("tile-clear-{path}");
                            move || s.clone()
                        })
                        .on_click(cx.listener(move |v, _: &gpui::ClickEvent, window, cx| {
                            cx.stop_propagation();
                            input.update(cx, |i, cx| i.set_text("", cx));
                            v.search_cleared(&p, cx);
                            let h = input.read(cx).focus_handle(cx);
                            window.focus(&h, cx);
                        }))
                });
                let d = self.shell(node, path, st, window, t);
                d.flex_row()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .px(px(8.))
                    .rounded(RADIUS)
                    .border_1()
                    .border_color(t.line_strong)
                    .bg(t.bg)
                    .child(kit::icon("search", 13., t.text_3))
                    .child(div().flex_1().min_w_0().child(field.input.clone()))
                    .children(clear)
                    .when(s.disabled, |d| d.opacity(0.5))
                    .into_any_element()
            }
            Element::FormRow(r) => {
                let kids = self.kids_of(i, inherit, window, t, cx);
                let head = div().flex().items_center().gap(px(3.)).text_size(px(12.5)).font_weight(kit::WEIGHT_550).text_color(t.text_2).child(r.label.clone()).when(r.required, |d| d.child(div().text_color(t.red).child("*")));
                let foot = div().flex().flex_col().gap(px(2.)).when_some(r.error.clone(), |d, e| d.child(kit::error_text(e, t))).when_some(r.help.clone(), |d, h| d.child(kit::hint(h, t)));
                let d = self.shell(node, path, st, window, t);
                if r.inline {
                    d.flex_row().items_start().gap(px(12.)).child(head.w(px(120.)).flex_none().pt(px(6.))).child(div().flex_1().flex().flex_col().gap(px(4.)).children(kids).child(foot)).into_any_element()
                } else {
                    d.gap(px(5.)).child(head).children(kids).child(foot).into_any_element()
                }
            }
            Element::Progress(p) => {
                let color = match p.tone {
                    None => t.text_2,
                    Some(_) => fb::tone_colors(tone(p.tone), t).0,
                };
                let bar = fb::progress(SharedString::from(format!("{id}-bar")), p.value, color, kit::motion::on(), t);
                let d = self.shell(node, path, st, window, t).min_w(px(60.)).gap(px(4.));
                d.when_some(node.label.clone(), |d, l| d.child(div().flex().justify_between().text_size(px(12.)).text_color(t.text_3).child(l).when_some(p.value, |d, v| d.child(format!("{:.0}%", v.clamp(0., 1.) * 100.))))).child(bar).into_any_element()
            }
            Element::Spinner => {
                let size = inherit.size.max(12.);
                self.shell(node, path, st, window, t).flex_none().child(kit::spinner(SharedString::from(format!("{id}-spin")), size, kit::motion::on(), t)).into_any_element()
            }
            Element::Skeleton(s) => {
                let d = self.shell(node, path, st, window, t);
                match s {
                    ui::Skeleton::Lines(n) => {
                        let n = (*n).max(1);
                        d.gap(px(8.)).w_full().children((0..n).map(|k| fb::skeleton_bar(10., t).w(relative(if k + 1 == n && n > 1 { 0.6 } else if k % 2 == 1 { 0.92 } else { 1. })))).into_any_element()
                    }
                    ui::Skeleton::Block => d.child(fb::skeleton_bar(0., t).size_full().min_h(px(40.))).into_any_element(),
                    ui::Skeleton::Circle => d.flex_none().child(fb::skeleton_bar(0., t).size_full().min_w(px(16.)).min_h(px(16.)).rounded_full()).into_any_element(),
                }
            }
            Element::EmptyState(e) => {
                let icon = e.icon.as_ref().map(|ui::Icon::Named(n)| n.as_str()).filter(|n| kit::has_icon(n)).unwrap_or("inbox");
                let mut calm = crate::main_screen::calm(icon, &e.title, e.text.clone().unwrap_or_default(), t);
                if let (Some(action), Some(_)) = (&e.action, handler(node, Listen::Click)) {
                    let p = path.to_string();
                    calm = calm.child(kit::text_button(SharedString::from(format!("{id}-action")), action.clone(), kit::BtnKind::Primary, false, t).mt(px(4.)).on_click(cx.listener(move |v, _: &gpui::ClickEvent, _, cx| {
                        cx.stop_propagation();
                        v.tell(&p, Outcome::Click, cx);
                    })));
                }
                self.shell(node, path, st, window, t).child(calm).into_any_element()
            }
            Element::Banner(b) => {
                let k = tone(Some(b.tone));
                let body = div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).when_some(b.title.clone(), |d, title| d.child(div().font_weight(kit::WEIGHT_550).child(title))).child(div().text_color(t.text_2).child(kit::para(b.text.clone())));
                let mut bx = fb::banner(k, t).child(body);
                if let (Some(action), Some(_)) = (&b.action, handler(node, Listen::Click)) {
                    let p = path.to_string();
                    bx = bx.child(kit::text_button(SharedString::from(format!("{id}-action")), action.clone(), kit::BtnKind::Small, false, t).on_click(cx.listener(move |v, _: &gpui::ClickEvent, _, cx| {
                        cx.stop_propagation();
                        v.tell(&p, Outcome::Click, cx);
                    })));
                }
                if b.dismissible && handler(node, Listen::Dismiss).is_some() {
                    let p = path.to_string();
                    bx = bx.child(
                        kit::icon_btn(SharedString::from(format!("{id}-close")), "x", "Close", true, t)
                            .debug_selector({
                                let s = format!("tile-close-{path}");
                                move || s.clone()
                            })
                            .on_click(cx.listener(move |v, _: &gpui::ClickEvent, _, cx| {
                                cx.stop_propagation();
                                v.tell(&p, Outcome::Dismiss, cx);
                            })),
                    );
                }
                self.shell(node, path, st, window, t).child(bx).into_any_element()
            }
            Element::Badge(b) => self.shell(node, path, st, window, t).flex_none().flex_row().child(fb::badge(b.text.clone(), tone(b.tone), t)).into_any_element(),
            Element::Chip(c) => {
                let (fg, bg) = match c.tone {
                    Some(_) => fb::tone_colors(tone(c.tone), t),
                    None => (t.text_2, t.surface_2),
                };
                let (fg, bg, edge) = if c.selected { (t.text, t.surface_3, t.text_3) } else { (fg, bg, t.line) };
                let clickable = !c.disabled && handler(node, Listen::Click).is_some();
                let mut d = self
                    .shell(node, path, st, window, t)
                    .flex_none()
                    .flex_row()
                    .items_center()
                    .gap(px(5.))
                    .h(px(24.))
                    .pl(px(9.))
                    .pr(px(if c.removable { 3. } else { 9. }))
                    .rounded_full()
                    .border_1()
                    .border_color(edge)
                    .bg(bg)
                    .text_size(px(12.))
                    .text_color(fg)
                    .when_some(c.icon.as_ref().map(|ui::Icon::Named(n)| n.clone()).filter(|n| kit::has_icon(n)), |d, n| d.child(kit::icon(&n, 12., fg)))
                    .child(c.label.clone())
                    .when(c.disabled, |d| d.opacity(0.5));
                if c.removable && !c.disabled && handler(node, Listen::Dismiss).is_some() {
                    let p = path.to_string();
                    d = d.child(
                        div()
                            .id(SharedString::from(format!("{id}-remove")))
                            .debug_selector({
                                let s = format!("tile-remove-{path}");
                                move || s.clone()
                            })
                            .size(px(18.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(t.surface_3))
                            .tooltip(kit::tooltip(format!("Remove {}", c.label)))
                            .child(kit::icon("x", 10., fg))
                            .on_click(cx.listener(move |v, _: &gpui::ClickEvent, _, cx| {
                                cx.stop_propagation();
                                v.tell(&p, Outcome::Dismiss, cx);
                            })),
                    );
                }
                if clickable {
                    d = self.on_tap(d, path, st, Outcome::Click, cx).hover(|s| s.border_color(t.text_3));
                }
                d.into_any_element()
            }
            Element::Kbd(k) => self.shell(node, path, st, window, t).flex_none().flex_row().child(kit::kbd(&key_label(k), t)).into_any_element(),
            Element::Split(s) => {
                let horizontal = s.orientation == ui::Orientation::Horizontal;
                let share = w.and_then(|w| w.share).unwrap_or(s.initial).clamp(0.05, 0.95);
                let dragging = w.is_some_and(|w| w.dragging);
                let mut kids = self.kids_of(i, inherit, window, t, cx).into_iter();
                let (first, second) = (kids.next().unwrap_or_else(|| div().into_any_element()), kids.next().unwrap_or_else(|| div().into_any_element()));
                let pane = |el: AnyElement| div().flex().flex_col().min_w_0().min_h_0().overflow_hidden().child(el);
                let first = if horizontal { pane(first).w(relative(share)).h_full() } else { pane(first).h(relative(share)).w_full() };
                let second = pane(second).flex_1().when(horizontal, |d| d.h_full()).when(!horizontal, |d| d.w_full());
                let bar = kit::divider(SharedString::from(format!("{id}-bar")), horizontal, dragging, t).border_color(t.line).when(horizontal, |d| d.border_l_1()).when(!horizontal, |d| d.border_t_1()).on_drag(SplitDrag(path.to_string()), |_, _, _, cx| cx.new(|_| NoGhost)).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
                let (p1, p2, p3) = (path.to_string(), path.to_string(), path.to_string());
                let d = self.shell(node, path, st, window, t).when(horizontal, |d| d.flex_row()).when(!horizontal, |d| d.flex_col());
                d.child(first)
                    .child(bar)
                    .child(second)
                    .on_drag_move::<SplitDrag>(cx.listener(move |v, e: &gpui::DragMoveEvent<SplitDrag>, _, cx| {
                        if e.drag(cx).0 == p1 {
                            let at = if horizontal { f32::from(e.event.position.x) } else { f32::from(e.event.position.y) };
                            v.split_to(&p1, at, e.bounds, false, cx);
                        }
                    }))
                    .on_drop(cx.listener(move |v, _: &SplitDrag, _, cx| v.split_to(&p2, 0., gpui::Bounds::default(), true, cx)))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(move |v, _: &gpui::MouseUpEvent, _, cx| v.split_to(&p3, 0., gpui::Bounds::default(), true, cx)))
                    .into_any_element()
            }
            Element::Disclosure(dc) => {
                let kids = if dc.open { self.kids_of(i, inherit, window, t, cx) } else { vec![] };
                let head = div()
                    .id(SharedString::from(format!("{id}-head")))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .child(kit::chevron_at(dc.open, 12., t.text_3))
                    .child(div().text_size(px(13.)).font_weight(kit::WEIGHT_550).text_color(t.text).child(dc.title.clone()))
                    .when_some(dc.detail.clone(), |d, x| d.child(div().text_size(px(12.)).text_color(t.text_3).child(x)));
                let head = if handler(node, Listen::Value).is_some() {
                    let p = path.to_string();
                    let focus = st.and_then(|s| s.focus.clone());
                    let o = hw::on_click(&node.element);
                    head.cursor_pointer().on_click(cx.listener(move |v, _: &gpui::ClickEvent, window, cx| {
                        cx.stop_propagation();
                        if let Some(f) = &focus {
                            window.focus(f, cx);
                        }
                        if let Some(o) = o.clone() {
                            v.tell(&p, o, cx);
                        }
                    }))
                } else {
                    head
                };
                self.shell(node, path, st, window, t).child(head).child(div().pl(px(18.)).flex().flex_col().gap(px(6.)).children(kids)).into_any_element()
            }
            Element::Card(c) => {
                let kids = self.kids_of(i, inherit, window, t, cx);
                let head = (c.title.is_some() || c.detail.is_some()).then(|| {
                    div()
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .gap(px(8.))
                        .when_some(c.title.clone(), |d, x| d.child(div().text_size(px(13.)).font_weight(kit::WEIGHT_550).text_color(t.text).child(x)))
                        .when_some(c.detail.clone(), |d, x| d.child(div().text_size(px(12.)).text_color(t.text_3).child(x)))
                });
                let d = self.shell(node, path, st, window, t).gap(px(8.)).p(px(12.)).rounded(RADIUS_LG).border_1().border_color(t.line).bg(t.surface).children(head).children(kids);
                if handler(node, Listen::Click).is_some() {
                    let hover = t.surface_2;
                    self.on_tap(d, path, st, Outcome::Click, cx).hover(move |s| s.bg(hover)).into_any_element()
                } else {
                    d.into_any_element()
                }
            }
            Element::Divider(o) => {
                let d = self.shell(node, path, st, window, t).flex_none().bg(t.line);
                if *o == ui::Orientation::Vertical {
                    d.w(px(1.)).self_stretch().min_h(px(12.)).into_any_element()
                } else {
                    d.h(px(1.)).w_full().into_any_element()
                }
            }
            Element::Breadcrumbs(crumbs) => {
                let can = handler(node, Listen::Value).is_some();
                let n = crumbs.len();
                let mut row = self.shell(node, path, st, window, t).flex_row().items_center().gap(px(4.)).text_size(px(12.5));
                for (k, c) in crumbs.iter().enumerate() {
                    if k > 0 {
                        row = row.child(div().text_color(t.text_4).child("/"));
                    }
                    let last = k + 1 == n;
                    let el = div().id(SharedString::from(format!("{id}-{k}"))).debug_selector({
                        let s = format!("tile-crumb-{path}/{}", c.key);
                        move || s.clone()
                    });
                    let el = el.px(px(3.)).rounded(px(3.)).text_color(if last { t.text } else { t.text_3 }).when(last, |d| d.font_weight(kit::WEIGHT_550)).child(c.label.clone());
                    row = if can && !last {
                        let (p, key, hover) = (path.to_string(), c.key.clone(), t.text);
                        row.child(el.cursor_pointer().hover(move |s| s.text_color(hover)).on_click(cx.listener(move |v, _: &gpui::ClickEvent, _, cx| {
                            cx.stop_propagation();
                            v.tell(&p, Outcome::Value(ControlValue::Choice(key.clone())), cx);
                        })))
                    } else {
                        row.child(el)
                    };
                }
                row.into_any_element()
            }
            Element::Toolbar => {
                let kids = self.kids_of(i, inherit, window, t, cx);
                self.shell(node, path, st, window, t).flex_row().items_center().gap(px(8.)).h(px(40.)).px(px(8.)).border_b_1().border_color(t.line).bg(t.surface).children(kids).into_any_element()
            }
            Element::Popover(p) => {
                let kids = self.kids_of(i, inherit, window, t, cx);
                let ft = t.float();
                let path2 = path.to_string();
                let sel = format!("tile-node-{path}");
                let panel = kit::menu_panel(&ft)
                    .id(id)
                    .debug_selector(move || sel.clone())
                    .occlude()
                    .when_some(p.width, |d, w| d.w(px(w.clamp(80., 640.))))
                    .when(p.width.is_none(), |d| d.min_w(px(160.)))
                    .children(kids)
                    .on_mouse_down_out(cx.listener(move |v, _, _, cx| {
                        v.tell(&path2, Outcome::Dismiss, cx);
                    }));
                let corner = match p.placement {
                    ui::Placement::Above => Corner::BottomLeft,
                    ui::Placement::Start => Corner::TopRight,
                    _ => Corner::TopLeft,
                };
                let offset = match p.placement {
                    ui::Placement::Above => gpui::point(px(0.), px(-4.)),
                    _ => gpui::point(px(0.), px(4.)),
                };
                match p.at.filter(|a| a.x.is_finite() && a.y.is_finite()) {
                    // At a point of its parent (a box of a canvas): on
                    // the side `placement` says.
                    Some(at) => {
                        let offset = match p.placement {
                            ui::Placement::Above => gpui::point(px(0.), px(-4.)),
                            ui::Placement::Start => gpui::point(px(-4.), px(0.)),
                            ui::Placement::End => gpui::point(px(4.), px(0.)),
                            ui::Placement::Below => gpui::point(px(0.), px(4.)),
                        };
                        div()
                            .absolute()
                            .left(px(at.x))
                            .top(px(at.y))
                            .size_0()
                            .child(deferred(anchored().anchor(corner).offset(offset).snap_to_window_with_margin(px(8.)).child(panel)).with_priority(1))
                            .into_any_element()
                    }
                    // At its place in its parent: under what comes before it.
                    None => div().relative().size_0().child(deferred(anchored().anchor(corner).offset(offset).snap_to_window_with_margin(px(8.)).child(panel)).with_priority(1)).into_any_element(),
                }
            }
            // Drawn over the whole tile ([`TileView::sheet_overlay`]).
            Element::ConfirmSheet(_) => div().absolute().size_0().into_any_element(),
            Element::KeyValues(kv) => {
                let rows: Vec<AnyElement> = kv
                    .iter()
                    .enumerate()
                    .map(|(k, e)| {
                        let value = div().flex_1().min_w_0().text_color(t.text).when(e.mono, |d| d.font_family(kit::MONO_FONT).text_size(px(12.))).child(kit::para(e.value.clone()));
                        let copy = e.copyable.then(|| {
                            let text = e.value.clone();
                            kit::icon_btn(SharedString::from(format!("{id}-copy-{k}")), "copy", "Copy", true, t).on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            })
                        });
                        div().flex().items_start().gap(px(12.)).min_h(px(22.)).child(div().w(px(110.)).flex_none().text_color(t.text_3).child(e.key.clone())).child(value).children(copy).into_any_element()
                    })
                    .collect();
                self.shell(node, path, st, window, t).gap(px(4.)).text_size(px(12.5)).children(rows).into_any_element()
            }
            Element::Avatar(a) => {
                let size = match a.size {
                    ui::AvatarSize::Small => 20.,
                    ui::AvatarSize::Medium => 28.,
                    ui::AvatarSize::Large => 40.,
                };
                let (soft, strong) = hw::avatar_tokens(&a.name);
                let el = fb::avatar(hw::initials(&a.name), size, render::color(soft, t), render::color(strong, t));
                let d = self.shell(node, path, st, window, t).flex_none();
                let d = if node.tooltip.is_none() { d.tooltip(kit::tooltip(a.name.clone())) } else { d };
                d.child(el).into_any_element()
            }
            Element::RelativeTime(r) => {
                let now = now_ms();
                self.tick_after(Duration::from_millis(hw::relative_time_next(r.at, now)), cx);
                let d = self.shell(node, path, st, window, t);
                let d = if node.tooltip.is_none() { d.tooltip(kit::tooltip(hw::date_time(r.at))) } else { d };
                d.flex_row().text_color(inherit.color).child(hw::relative_time(r.at, now)).into_any_element()
            }
            Element::Copyable(c) => {
                let copied = w.and_then(|w| w.copied).is_some_and(|at| at.elapsed() < COPIED_FOR);
                let p = path.to_string();
                let e = node.element.clone();
                let button = kit::icon_btn(SharedString::from(format!("{id}-copy")), if copied { "check" } else { "copy" }, if copied { "Copied" } else { "Copy" }, true, t)
                    .debug_selector({
                        let s = format!("tile-copy-{path}");
                        move || s.clone()
                    })
                    .on_click(cx.listener(move |v, _: &gpui::ClickEvent, _, cx| {
                        cx.stop_propagation();
                        v.activate(&p, &e, cx);
                    }));
                let text = div().min_w_0().text_color(t.text).when(c.mono, |d| d.font_family(kit::MONO_FONT).text_size(px(12.))).child(kit::one_line(c.text.clone()));
                self.shell(node, path, st, window, t).flex_row().items_center().gap(px(4.)).child(text).child(button).into_any_element()
            }
            Element::Link(l) => {
                let p = path.to_string();
                let e = node.element.clone();
                let (fg, hover) = (t.focus, t.text);
                let d = self.shell(node, path, st, window, t).flex_row().items_center().gap(px(3.)).cursor_pointer().text_color(fg).hover(move |s| s.text_color(hover)).child(l.label.clone());
                let d = d.when(l.url.is_some(), |d| d.child(kit::icon("external", 11., fg)));
                let d = if node.tooltip.is_none() { d.when_some(l.url.clone(), |d, u| d.tooltip(kit::tooltip(u))) } else { d };
                let focus = st.and_then(|s| s.focus.clone());
                d.on_click(cx.listener(move |v, _: &gpui::ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(f) = &focus {
                        window.focus(f, cx);
                    }
                    v.activate(&p, &e, cx);
                }))
                .into_any_element()
            }
            Element::Sparkline(_) | Element::BarChart(_) | Element::LineChart(_) | Element::StatusTimeline(_) => {
                let d = self.shell(node, path, st, window, t);
                self.build_chart(d, node, path, t, cx)
            }
            _ => div().into_any_element(),
        }
    }

    /// What key `key` does to component `path` while it has focus (tests
    /// and bench commands: no key is pressed). Whether it did anything.
    pub fn component_key(&mut self, path: &str, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(e) = self.node(path).map(|n| n.element.clone()) else { return false };
        match hw::on_key(&e, key) {
            Some(Outcome::Click) if matches!(e, Element::Copyable(_) | Element::Link(_)) => {
                self.activate(path, &e, cx);
                true
            }
            Some(o) => self.tell(path, o, cx),
            None => false,
        }
    }

    /// A click on component `path`: a checkbox, switch or disclosure
    /// flips, a copy button copies, a link opens (tests and bench
    /// commands: nothing is clicked on the screen).
    pub fn component_click(&mut self, path: &str, cx: &mut Context<Self>) -> bool {
        let Some(e) = self.node(path).map(|n| n.element.clone()) else { return false };
        match (&e, hw::on_click(&e)) {
            (Element::Copyable(_) | Element::Link(_), _) => {
                self.activate(path, &e, cx);
                true
            }
            (_, Some(o)) => self.tell(path, o, cx),
            _ => self.tell(path, Outcome::Click, cx),
        }
    }

    /// The slider at `path` let go at window x `x`.
    fn slide_end(&mut self, path: &str, x: f32, cx: &mut Context<Self>) {
        if self.nodes.get(path).and_then(|s| s.widget.as_ref()).is_some_and(|w| w.live.is_some()) {
            self.slide(path, x, true, cx);
        }
    }

    /// The search field at `path` emptied with its clear button: the tile
    /// hears it now.
    fn search_cleared(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(f) = self.nodes.get_mut(path).and_then(|s| s.field.as_mut()) {
            f.rev += 1;
        }
        self.tell_change(path, cx);
    }

    /// Draw again after `d` (a relative time's words change then).
    /// A badge, an avatar or a relative time in a list's row (`None` for
    /// other elements): drawn as in a frame, without focus or listeners.
    pub(super) fn row_widget(&self, node: &Node, inherit: Inherit, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (d, _) = render::apply(div(), &node.style, t);
        let d = d.flex_none().flex().items_center();
        Some(match &node.element {
            Element::Badge(b) => d.id(SharedString::from(format!("badge-{}", b.text))).when_some(node.tooltip.clone(), |d, tip| d.tooltip(kit::tooltip(tip))).child(fb::badge(b.text.clone(), tone(b.tone), t)).into_any_element(),
            Element::Avatar(a) => {
                let size = match a.size {
                    ui::AvatarSize::Small => 20.,
                    ui::AvatarSize::Medium => 28.,
                    ui::AvatarSize::Large => 40.,
                };
                let (soft, strong) = hw::avatar_tokens(&a.name);
                let tip = node.tooltip.clone().unwrap_or_else(|| a.name.clone());
                d.id(SharedString::from(format!("avatar-{}", a.name))).tooltip(kit::tooltip(tip)).child(fb::avatar(hw::initials(&a.name), size, render::color(soft, t), render::color(strong, t))).into_any_element()
            }
            Element::RelativeTime(r) => {
                let now = now_ms();
                self.tick_after(Duration::from_millis(hw::relative_time_next(r.at, now)), cx);
                let tip = node.tooltip.clone().unwrap_or_else(|| hw::date_time(r.at));
                d.id(SharedString::from(format!("time-{}", r.at))).tooltip(kit::tooltip(tip)).text_color(inherit.color).child(hw::relative_time(r.at, now)).into_any_element()
            }
            _ => return None,
        })
    }

    fn tick_after(&self, d: Duration, cx: &mut Context<Self>) {
        let at = Instant::now() + d.max(Duration::from_millis(500));
        if self.tick.get().is_some_and(|t| t <= at) {
            return;
        }
        self.tick.set(Some(at));
        let tick = self.tick.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(d.max(Duration::from_millis(500))).await;
            if tick.get() == Some(at) {
                tick.set(None);
            }
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// The open confirm sheet, over the whole tile.
    pub(super) fn sheet_overlay(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let frame = self.frame.as_ref()?;
        let i = frame.nodes.iter().position(|n| matches!(n.element, Element::ConfirmSheet(_)))?;
        let Element::ConfirmSheet(s) = &frame.nodes[i].element else { return None };
        let path = self.paths[i].clone();
        let id = format!("tile-{}-{}-{path}", self.ext, self.tile);
        let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());
        let cancel = kit::text_button(SharedString::from(format!("{id}-cancel")), s.cancel.clone().unwrap_or_else(|| "Cancel".into()), kit::BtnKind::Ghost, false, t)
            .debug_selector({
                let s = format!("tile-sheet-cancel-{path}");
                move || s.clone()
            })
            .on_click(cx.listener(move |v, _: &gpui::ClickEvent, _, cx| {
                cx.stop_propagation();
                v.tell(&p1, Outcome::Dismiss, cx);
            }));
        let confirm = kit::text_button(SharedString::from(format!("{id}-confirm")), s.confirm.clone(), if s.danger { kit::BtnKind::Danger } else { kit::BtnKind::Primary }, false, t)
            .debug_selector({
                let s = format!("tile-sheet-confirm-{path}");
                move || s.clone()
            })
            .on_click(cx.listener(move |v, _: &gpui::ClickEvent, _, cx| {
                cx.stop_propagation();
                v.tell(&p2, Outcome::Click, cx);
            }));
        let card = div()
            .id(SharedString::from(format!("{id}-card")))
            .debug_selector({
                let s = format!("tile-node-{path}");
                move || s.clone()
            })
            .occlude()
            .w(px(340.))
            .max_w(relative(0.92))
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(16.))
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(t.line_strong)
            .bg(t.raised)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(14.)).font_weight(kit::WEIGHT_650).text_color(t.text).child(s.title.clone()))
            .when_some(s.text.clone(), |d, x| d.child(div().text_size(px(13.)).text_color(t.text_2).child(kit::para(x))))
            .child(div().mt(px(8.)).flex().justify_end().gap(px(8.)).child(cancel).child(confirm));
        Some(
            div()
                .id(SharedString::from(format!("{id}-scrim")))
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::black().opacity(0.32))
                .on_mouse_down(MouseButton::Left, cx.listener(move |v, _, _, cx| {
                    cx.stop_propagation();
                    v.tell(&p3, Outcome::Dismiss, cx);
                }))
                .child(card)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_written_as_pitwall_writes_them() {
        assert_eq!(key_label("cmd-k"), "⌘K");
        assert_eq!(key_label("cmd-shift-p"), "⌘⇧P");
        assert_eq!(key_label("/"), "/");
        assert_eq!(key_label("escape"), "Esc");
        assert_eq!(key_label("g g"), "G G");
    }
}
