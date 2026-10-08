//! A text field, single- or multi-line (soft-wrapped), with IME, selection,
//! word motion and the clipboard. Written from GPUI's own `examples/input.rs`
//! (Apache-2.0). It lives here until the shared kit (`src/kit/`, phase 1)
//! has one: promote it there.

use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem, Context,
    CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, GlobalElementId, Hsla, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, Style, TextRun,
    UTF16Selection, UnderlineStyle, Window, WrappedLine,
};

use super::text::{next_grapheme, next_word, prev_grapheme, prev_word};
use crate::theme::theme;

actions!(
    pw_text_field,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        Home,
        End,
        SelectHome,
        SelectEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        /// Enter: a new line in a multi-line field, `Enter` otherwise.
        Enter,
        ShiftEnter,
        /// ⌘↵ / Ctrl+↵ (either works everywhere, as in the React app).
        Submit,
    ]
);

const CONTEXT: &str = "PwTextField";

pub fn bindings() -> Vec<KeyBinding> {
    let c = Some(CONTEXT);
    let mac = cfg!(target_os = "macos");
    let m = |k: &str| {
        if mac {
            format!("cmd-{k}")
        } else {
            format!("ctrl-{k}")
        }
    };
    let w = |k: &str| {
        if mac {
            format!("alt-{k}")
        } else {
            format!("ctrl-{k}")
        }
    };
    let mut b = vec![
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new(&w("backspace"), DeleteWordLeft, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new(&w("left"), WordLeft, c),
        KeyBinding::new(&w("right"), WordRight, c),
        KeyBinding::new(&w("shift-left"), SelectWordLeft, c),
        KeyBinding::new(&w("shift-right"), SelectWordRight, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("shift-home", SelectHome, c),
        KeyBinding::new("shift-end", SelectEnd, c),
        KeyBinding::new(&m("a"), SelectAll, c),
        KeyBinding::new(&m("c"), Copy, c),
        KeyBinding::new(&m("x"), Cut, c),
        KeyBinding::new(&m("v"), Paste, c),
        KeyBinding::new("enter", Enter, c),
        KeyBinding::new("shift-enter", ShiftEnter, c),
        KeyBinding::new("cmd-enter", Submit, c),
        KeyBinding::new("ctrl-enter", Submit, c),
    ];
    if mac {
        b.extend([
            KeyBinding::new("cmd-left", Home, c),
            KeyBinding::new("cmd-right", End, c),
            KeyBinding::new("cmd-shift-left", SelectHome, c),
            KeyBinding::new("cmd-shift-right", SelectEnd, c),
            KeyBinding::new("ctrl-a", Home, c),
            KeyBinding::new("ctrl-e", End, c),
        ]);
    }
    b
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFieldEvent {
    Changed,
    /// Enter in a single-line field (`shift`: Shift+Enter).
    Enter {
        shift: bool,
    },
    /// ⌘↵ / Ctrl+↵.
    Submit,
}

impl EventEmitter<TextFieldEvent> for TextField {}

/// What the last paint laid out (for the mouse and the IME).
struct Laid {
    bounds: Bounds<Pixels>,
    /// One per logical line: its byte start in the content, its top (px,
    /// relative to the text origin) and its wrapped layout.
    lines: Vec<(usize, Pixels, WrappedLine)>,
    line_height: Pixels,
}

pub struct TextField {
    focus: FocusHandle,
    content: String,
    placeholder: SharedString,
    multiline: bool,
    /// Visible rows of a multi-line field.
    rows: usize,
    mono: bool,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    laid: Option<Laid>,
    selecting: bool,
    /// Scrolled (px): horizontally in a single-line field, vertically in a
    /// multi-line one.
    scroll: Pixels,
    /// Remembered x for up/down.
    goal_x: Option<Pixels>,
    disabled: bool,
}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextField {
    pub fn single(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self::new(placeholder.into(), false, 1, cx)
    }

    pub fn multi(
        placeholder: impl Into<SharedString>,
        rows: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new(placeholder.into(), true, rows.max(1), cx)
    }

    fn new(
        placeholder: SharedString,
        multiline: bool,
        rows: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        TextField {
            focus: cx.focus_handle(),
            content: String::new(),
            placeholder,
            multiline,
            rows,
            mono: false,
            selected: 0..0,
            reversed: false,
            marked: None,
            laid: None,
            selecting: false,
            scroll: px(0.),
            goal_x: None,
            disabled: false,
        }
    }

    /// Draw the text in the monospace font (prompts, as the React textarea).
    pub fn mono(mut self, on: bool) -> Self {
        self.mono = on;
        self
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let text: String = text.into();
        self.content = if self.multiline {
            text
        } else {
            text.replace(['\r', '\n'], " ")
        };
        let end = self.content.len();
        self.selected = end..end;
        self.reversed = false;
        self.marked = None;
        cx.emit(TextFieldEvent::Changed);
        cx.notify();
    }

    pub fn set_rows(&mut self, rows: usize, cx: &mut Context<Self>) {
        if rows.max(1) != self.rows {
            self.rows = rows.max(1);
            cx.notify();
        }
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        cx.notify();
    }

    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.selected = 0..self.content.len();
        self.reversed = false;
        cx.notify();
    }

    /// The selected text (empty when nothing is).
    pub fn selected_text(&self) -> &str {
        &self.content[self.selected.clone()]
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        self.selected = offset..offset;
        self.reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.notify();
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let text = if self.multiline {
            text.replace("\r\n", "\n")
        } else {
            text.replace(['\r', '\n'], " ")
        };
        self.content.replace_range(range.clone(), &text);
        let end = range.start + text.len();
        self.selected = end..end;
        self.reversed = false;
        self.marked = None;
        self.goal_x = None;
        cx.emit(TextFieldEvent::Changed);
        cx.notify();
    }

    /// Start and end of the logical line around `offset`.
    fn line_bounds(&self, offset: usize) -> (usize, usize) {
        let start = self.content[..offset].rfind('\n').map_or(0, |i| i + 1);
        let end = self.content[offset..]
            .find('\n')
            .map_or(self.content.len(), |i| offset + i);
        (start, end)
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.move_to(prev_grapheme(&self.content, self.cursor()), cx);
        } else {
            self.move_to(self.selected.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.move_to(next_grapheme(&self.content, self.cursor()), cx);
        } else {
            self.move_to(self.selected.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(prev_grapheme(&self.content, self.cursor()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_grapheme(&self.content, self.cursor()), cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(prev_word(&self.content, self.cursor()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(next_word(&self.content, self.cursor()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(prev_word(&self.content, self.cursor()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_word(&self.content, self.cursor()), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let (s, _) = self.line_bounds(self.cursor());
        self.move_to(s, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let (_, e) = self.line_bounds(self.cursor());
        self.move_to(e, cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        let (s, _) = self.line_bounds(self.cursor());
        self.select_to(s, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let (_, e) = self.line_bounds(self.cursor());
        self.select_to(e, cx);
    }

    /// The offset one visual row up or down from the cursor (`None`: the
    /// first/last row).
    fn vertical(&mut self, down: bool) -> Option<usize> {
        let laid = self.laid.as_ref()?;
        let p = self.position_of(self.cursor())?;
        let x = *self.goal_x.get_or_insert(p.x);
        let y = if down {
            p.y + laid.line_height * 1.5
        } else {
            p.y - laid.line_height * 0.5
        };
        if y < px(0.) {
            return Some(0);
        }
        Some(self.offset_at(point(x, y)))
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if !self.multiline {
            return cx.propagate();
        }
        let goal = self.goal_x;
        if let Some(o) = self.vertical(false) {
            let g = self.goal_x;
            self.move_to(o, cx);
            self.goal_x = g.or(goal);
        }
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if !self.multiline {
            return cx.propagate();
        }
        let goal = self.goal_x;
        if let Some(o) = self.vertical(true) {
            let g = self.goal_x;
            self.move_to(o, cx);
            self.goal_x = g.or(goal);
        }
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(o) = self.vertical(false) {
            let g = self.goal_x;
            self.select_to(o, cx);
            self.goal_x = g;
        }
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(o) = self.vertical(true) {
            let g = self.goal_x;
            self.select_to(o, cx);
            self.goal_x = g;
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text(cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let c = self.cursor();
            self.selected = prev_grapheme(&self.content, c)..c;
        }
        self.replace(self.selected.clone(), "", cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let c = self.cursor();
            self.selected = c..next_grapheme(&self.content, c);
        }
        self.replace(self.selected.clone(), "", cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let c = self.cursor();
            self.selected = prev_word(&self.content, c)..c;
        }
        self.replace(self.selected.clone(), "", cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text().to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text().to_string()));
            self.replace(self.selected.clone(), "", cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.replace(self.selected.clone(), &text, cx);
        }
    }

    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.replace(self.selected.clone(), "\n", cx);
        } else {
            cx.emit(TextFieldEvent::Enter { shift: false });
        }
    }

    fn shift_enter(&mut self, _: &ShiftEnter, _: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.replace(self.selected.clone(), "\n", cx);
        } else {
            cx.emit(TextFieldEvent::Enter { shift: true });
        }
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextFieldEvent::Submit);
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let at = self.offset_at_window(e.position);
        if e.click_count >= 2 {
            let (s, end) = if e.click_count == 2 {
                let r = super::text::word_at(&self.content, at);
                (r.start, r.end)
            } else {
                self.line_bounds(at)
            };
            self.selected = s..end;
            self.reversed = false;
            cx.notify();
            return;
        }
        self.selecting = true;
        if e.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.move_to(at, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn on_mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            let at = self.offset_at_window(e.position);
            self.select_to(at, cx);
        }
    }

    fn origin(&self) -> Option<Point<Pixels>> {
        let laid = self.laid.as_ref()?;
        Some(if self.multiline {
            point(laid.bounds.left(), laid.bounds.top() - self.scroll)
        } else {
            point(laid.bounds.left() - self.scroll, laid.bounds.top())
        })
    }

    fn offset_at_window(&self, p: Point<Pixels>) -> usize {
        match self.origin() {
            Some(o) => self.offset_at(p - o),
            None => self.content.len(),
        }
    }

    /// The offset at a point relative to the text origin.
    fn offset_at(&self, p: Point<Pixels>) -> usize {
        let Some(laid) = self.laid.as_ref() else {
            return self.content.len();
        };
        if self.content.is_empty() {
            return 0;
        }
        let ix = laid
            .lines
            .iter()
            .rposition(|(_, top, _)| *top <= p.y)
            .unwrap_or(0);
        let (start, top, line) = &laid.lines[ix];
        let local = point(p.x.max(px(0.)), (p.y - *top).max(px(0.)));
        let i = match line.closest_index_for_position(local, laid.line_height) {
            Ok(i) | Err(i) => i,
        };
        start + i.min(line.len())
    }

    /// Where an offset is drawn, relative to the text origin.
    fn position_of(&self, offset: usize) -> Option<Point<Pixels>> {
        let laid = self.laid.as_ref()?;
        let ix = laid
            .lines
            .iter()
            .rposition(|(start, _, _)| *start <= offset)
            .unwrap_or(0);
        let (start, top, line) = laid.lines.get(ix)?;
        let local = (offset - start).min(line.len());
        let p = line
            .position_for_index(local, laid.line_height)
            .unwrap_or(point(line.unwrapped_layout.width, px(0.)));
        Some(point(p.x, p.y + *top))
    }

    fn utf16_to_offset(&self, u: usize) -> usize {
        let mut utf16 = 0;
        for (i, ch) in self.content.char_indices() {
            if utf16 >= u {
                return i;
            }
            utf16 += ch.len_utf16();
        }
        self.content.len()
    }

    fn offset_to_utf16(&self, o: usize) -> usize {
        self.content[..o.min(self.content.len())]
            .chars()
            .map(char::len_utf16)
            .sum()
    }

    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.utf16_to_offset(r.start)..self.utf16_to_offset(r.end)
    }

    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end)
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.range_from_utf16(&range);
        actual.replace(self.range_to_utf16(&r));
        Some(self.content[r].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.replace(r, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        new_selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let r = range
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.content.replace_range(r.clone(), text);
        self.marked = (!text.is_empty()).then(|| r.start..r.start + text.len());
        self.selected = new_selected
            .as_ref()
            .map(|s| {
                let s = r.start + s.start..r.start + s.end;
                s.start.min(self.content.len())..s.end.min(self.content.len())
            })
            .unwrap_or_else(|| r.start + text.len()..r.start + text.len());
        cx.emit(TextFieldEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let r = self.range_from_utf16(&range);
        let o = self.origin()?;
        let lh = self.laid.as_ref()?.line_height;
        let a = self.position_of(r.start)?;
        let b = self.position_of(r.end)?;
        Some(Bounds::from_corners(
            o + a,
            o + point(b.x.max(a.x + px(1.)), b.y + lh),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let o = self.offset_at_window(p);
        Some(self.offset_to_utf16(o))
    }
}

/// The element that lays out, paints and hands the IME its input handler.
struct FieldElement {
    field: Entity<TextField>,
}

impl IntoElement for FieldElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

struct Prepared {
    lines: Vec<(usize, Pixels, WrappedLine)>,
    line_height: Pixels,
}

impl Element for FieldElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepared;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let f = self.field.read(cx);
        let rows = if f.multiline { f.rows } else { 1 };
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (window.line_height() * rows as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepared {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let t = theme(cx).clone();
        let f = self.field.read(cx);
        let empty = f.content.is_empty();
        let (text, color): (String, Hsla) = if empty {
            (f.placeholder.to_string(), t.text_4)
        } else {
            (f.content.clone(), style.color)
        };
        let multiline = f.multiline;
        let marked = f.marked.clone();
        let wrap = multiline.then_some(bounds.size.width);
        let mut lines = Vec::new();
        let mut start = 0;
        let mut top = px(0.);
        let shown: Vec<&str> = if multiline {
            text.split('\n').collect()
        } else {
            vec![text.as_str()]
        };
        for l in shown {
            let run = |len: usize| TextRun {
                len,
                font: style.font(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            // The IME's marked text is underlined.
            let runs = match marked.as_ref().filter(|_| !empty) {
                Some(m) if m.start < start + l.len() && m.end > start => {
                    let a = m.start.saturating_sub(start).min(l.len());
                    let b = (m.end - start).min(l.len());
                    let mut v = vec![run(a)];
                    v.push(TextRun {
                        underline: Some(UnderlineStyle {
                            color: Some(color),
                            thickness: px(1.),
                            wavy: false,
                        }),
                        ..run(b - a)
                    });
                    v.push(run(l.len() - b));
                    v.into_iter().filter(|r| r.len > 0).collect()
                }
                _ => vec![run(l.len())],
            };
            let shaped = window
                .text_system()
                .shape_text(
                    SharedString::from(l.to_string()),
                    font_size,
                    &runs,
                    wrap,
                    None,
                )
                .ok()
                .and_then(|mut v| v.pop())
                .unwrap_or_default();
            let h = line_height * (shaped.wrap_boundaries.len() + 1) as f32;
            lines.push((start, top, shaped));
            top += h;
            start += l.len() + 1;
        }
        Prepared { lines, line_height }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prep: &mut Prepared,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.field.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.field.clone()),
            cx,
        );
        let lines = std::mem::take(&mut prep.lines);
        let lh = prep.line_height;
        // Remember the layout first: positions below use it.
        self.field.update(cx, |f, _| {
            f.laid = Some(Laid {
                bounds,
                lines: lines.clone(),
                line_height: lh,
            });
        });
        let t = theme(cx).clone();
        let (cursor, sel, empty, multiline) = {
            let f = self.field.read(cx);
            (
                f.cursor(),
                f.selected.clone(),
                f.content.is_empty(),
                f.multiline,
            )
        };
        // Keep the cursor in view.
        let cpos = self.field.read(cx).position_of(cursor).unwrap_or_default();
        self.field.update(cx, |f, _| {
            if multiline {
                let h = bounds.size.height;
                if cpos.y < f.scroll {
                    f.scroll = cpos.y;
                } else if cpos.y + lh > f.scroll + h {
                    f.scroll = cpos.y + lh - h;
                }
            } else {
                let w = bounds.size.width - px(2.);
                if cpos.x < f.scroll {
                    f.scroll = cpos.x;
                } else if cpos.x > f.scroll + w {
                    f.scroll = cpos.x - w;
                }
            }
        });
        let f = self.field.read(cx);
        let origin = f.origin().unwrap_or(bounds.origin);
        let focused = focus.is_focused(window);
        // Selection, row by row.
        if !sel.is_empty() && !empty {
            let sel_color = t.focus.opacity(0.35);
            let mut rects = Vec::new();
            for (start, top, line) in &lines {
                let end = start + line.len();
                if sel.end < *start || sel.start > end {
                    continue;
                }
                let a = sel.start.max(*start) - start;
                let b = sel.end.min(end) - start;
                let rows = line.wrap_boundaries.len() + 1;
                for row in 0..rows {
                    let y = *top + lh * row as f32;
                    let pa = line.position_for_index(a, lh).unwrap_or_default();
                    let pb = line.position_for_index(b, lh).unwrap_or_default();
                    let row_y = lh * row as f32;
                    if pb.y < row_y || pa.y > row_y {
                        continue;
                    }
                    let x0 = if pa.y < row_y { px(0.) } else { pa.x };
                    let x1 = if pb.y > row_y {
                        bounds.size.width
                    } else {
                        pb.x.max(x0 + px(4.))
                    };
                    rects.push(Bounds::new(origin + point(x0, y), size(x1 - x0, lh)));
                }
            }
            window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                for r in rects {
                    window.paint_quad(fill(r, sel_color));
                }
            });
        }
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for (_, top, line) in &lines {
                let _ = line.paint(
                    origin + point(px(0.), *top),
                    lh,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            if focused {
                let p = if empty { point(px(0.), px(0.)) } else { cpos };
                window.paint_quad(fill(Bounds::new(origin + p, size(px(1.5), lh)), t.text));
            }
        });
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        div()
            .id("pw-text-field")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::shift_enter))
            .on_action(cx.listener(Self::submit))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .overflow_hidden()
            .when(self.mono, |d| d.font_family(super::style::mono_family()))
            .when(self.disabled, |d| d.opacity(0.6))
            .text_color(t.text)
            .child(FieldElement { field: cx.entity() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, TestAppContext};

    #[gpui::test]
    fn typing_editing_and_events(cx: &mut TestAppContext) {
        let f = cx.new(|cx| TextField::single("Find", cx));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let ev = events.clone();
        cx.update(|cx| {
            cx.subscribe(&f, move |_, e: &TextFieldEvent, _| {
                ev.borrow_mut().push(e.clone())
            })
            .detach()
        });
        f.update(cx, |f, cx| {
            f.set_text("გამარჯობა\nworld", cx);
            assert_eq!(f.text(), "გამარჯობა world", "single line: no line breaks");
            // Backspace removes one Georgian letter (three bytes).
            f.selected = 3..3;
            f.replace(prev_grapheme(&f.content, 3)..3, "", cx);
            assert_eq!(f.text(), "ამარჯობა world");
            f.select_all_text(cx);
            f.replace(f.selected.clone(), "x", cx);
            assert_eq!(f.text(), "x");
            cx.emit(TextFieldEvent::Enter { shift: true });
        });
        assert!(events.borrow().contains(&TextFieldEvent::Changed));
        assert!(events
            .borrow()
            .contains(&TextFieldEvent::Enter { shift: true }));
    }

    #[gpui::test]
    fn multi_line_keeps_line_breaks(cx: &mut TestAppContext) {
        let f = cx.new(|cx| TextField::multi("What should change here?", 3, cx));
        f.update(cx, |f, cx| {
            f.set_text("a\r\nb", cx);
            f.replace(f.selected.clone(), "\nc", cx);
            assert_eq!(f.text(), "a\r\nb\nc");
            assert_eq!(f.line_bounds(3), (3, 4));
            assert_eq!(f.line_bounds(5), (5, 6));
        });
    }
}
