//! The app's one text input, single- or multi-line (soft-wrapped), built
//! in-house (GPUI's own `examples/input.rs`, Apache-2.0, is the
//! reference): IME through `EntityInputHandler` (marked text underlined, the
//! candidate window at the caret), selection with the mouse (double click a
//! word, triple click a line) and shift-arrows, grapheme and word moves
//! ([`super::edit`]), word delete, clipboard, select all. Enter, ⌘↵ and Esc
//! are reported as events so the owner decides (submit, cancel); in a
//! multi-line input Enter is a new line and ⌘↵ / Ctrl+↵ submits.
//!
//! Looks: boxed (`.input`: 34 px, hairline, the focus border; a multi-line
//! one is the `textarea`), [`TextInput::plain`] for a field drawn inside
//! the owner's own box (the code view's Find), and [`TextInput::bare`]
//! (search bars, quick open), which also lets Enter, Esc and the arrows
//! through to its owner's key bindings.

use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem, Context,
    CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, GlobalElementId, Hsla, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, Style, TextRun,
    UTF16Selection, UnderlineStyle, Window, WrappedLine,
};

use crate::theme;

use super::edit::{next_grapheme, next_word, prev_grapheme, prev_word, word_at};
use super::fonts::MONO_FONT;

actions!(
    pw_text_input,
    [
        Backspace,
        Delete,
        WordBackspace,
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
        /// The browser's undo history of a field: typing runs undo together.
        Undo,
        Redo,
        /// Enter: a new line in a multi-line input, [`InputEvent::Submit`]
        /// otherwise.
        Enter,
        ShiftEnter,
        /// ⌘↵ / Ctrl+↵ (either works everywhere, as in the React app).
        Submit,
        Escape,
    ]
);

const CONTEXT: &str = "PwTextInput";

/// Key bindings of every input (registered once by `kit::init`).
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
        KeyBinding::new(&w("backspace"), WordBackspace, c),
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
        KeyBinding::new(&m("z"), Undo, c),
        KeyBinding::new(&m("shift-z"), Redo, c),
        KeyBinding::new("enter", Enter, c),
        KeyBinding::new("shift-enter", ShiftEnter, c),
        KeyBinding::new("cmd-enter", Submit, c),
        KeyBinding::new("ctrl-enter", Submit, c),
        KeyBinding::new("escape", Escape, c),
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
pub enum InputEvent {
    Changed,
    /// Enter in a one-line input; ⌘↵ / Ctrl+↵ in any.
    Submit,
    /// Shift+Enter in a one-line input (Find: the previous match).
    SubmitShift,
    /// Esc.
    Cancel,
    /// The keyboard left it.
    Blur,
}

impl EventEmitter<InputEvent> for TextInput {}

/// The events of Review's former `TextField` (`crate::code_view::input`),
/// still sent for owners written against it: Enter on one line (with
/// Shift or not) and ⌘↵ apart. New code listens to [`InputEvent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFieldEvent {
    Changed,
    /// Enter in a one-line input (`shift`: Shift+Enter).
    Enter { shift: bool },
    /// ⌘↵ / Ctrl+↵.
    Submit,
}

impl EventEmitter<TextFieldEvent> for TextInput {}

/// What the last paint laid out (for the mouse and the IME).
struct Laid {
    bounds: Bounds<Pixels>,
    /// One per logical line: its byte start in the content, its top (px,
    /// relative to the text origin) and its wrapped layout.
    lines: Vec<(usize, Pixels, WrappedLine)>,
    line_height: Pixels,
}

/// How the input is framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// `.input` / `textarea`.
    Boxed,
    /// No box (the owner draws one); keys stay here.
    Plain,
    /// No box; Enter, Esc and the arrows also reach the owner's bindings.
    Bare,
}

pub struct TextInput {
    focus: FocusHandle,
    content: String,
    placeholder: SharedString,
    multiline: bool,
    /// Visible rows of a multi-line input.
    rows: usize,
    mono: bool,
    frame: Frame,
    max_len: Option<usize>,
    disabled: bool,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    laid: Option<Laid>,
    selecting: bool,
    /// Scrolled (px): horizontally on one line, vertically on several.
    scroll: Pixels,
    /// Remembered x for up/down.
    goal_x: Option<Pixels>,
    was_focused: bool,
    /// Earlier states (text, selection), newest last; and undone ones.
    undo: Vec<(String, Range<usize>)>,
    redo: Vec<(String, Range<usize>)>,
    /// Where the last typed character ended and when (a typing run is one
    /// undo step).
    typing: Option<(usize, std::time::Instant)>,
}

impl TextInput {
    /// A one-line input holding `text`.
    pub fn new(cx: &mut Context<Self>, text: &str, placeholder: &str) -> TextInput {
        TextInput {
            focus: cx.focus_handle().tab_stop(true),
            content: text.to_string(),
            placeholder: placeholder.to_string().into(),
            multiline: false,
            rows: 1,
            mono: false,
            frame: Frame::Boxed,
            max_len: None,
            disabled: false,
            selected: text.len()..text.len(),
            reversed: false,
            marked: None,
            laid: None,
            selecting: false,
            scroll: px(0.),
            goal_x: None,
            was_focused: false,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: None,
        }
    }

    /// An empty one-line input with no box (Review's former
    /// `TextField::single`).
    pub fn single(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> TextInput {
        let mut t = TextInput::new(cx, "", "").plain();
        t.placeholder = placeholder.into();
        t
    }

    /// An empty multi-line input with no box, `rows` visible (Review's
    /// former `TextField::multi`).
    pub fn multi(
        placeholder: impl Into<SharedString>,
        rows: usize,
        cx: &mut Context<Self>,
    ) -> TextInput {
        TextInput::single(placeholder, cx).multiline(rows)
    }

    /// Several soft-wrapped lines, `rows` of them visible: Enter is a new
    /// line, ⌘↵ / Ctrl+↵ submits.
    pub fn multiline(mut self, rows: usize) -> Self {
        self.multiline = true;
        self.rows = rows.max(1);
        self
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// No box of its own (the owner draws one).
    pub fn plain(mut self) -> Self {
        self.frame = Frame::Plain;
        self
    }

    /// No box of its own; Enter, Esc and the arrows also reach the owner's
    /// bindings.
    pub fn bare(mut self) -> Self {
        self.frame = Frame::Bare;
        self
    }

    pub fn max_len(mut self, n: usize) -> Self {
        self.max_len = Some(n);
        self
    }

    pub fn set_placeholder(&mut self, text: impl Into<SharedString>) {
        self.placeholder = text.into();
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    /// Replace the text (the cursor goes to the end); no Changed event.
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
        self.undo.clear();
        self.redo.clear();
        self.typing = None;
        cx.notify();
    }

    /// Replace the text and report it ([`InputEvent::Changed`]).
    pub fn replace_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.set_text(text, cx);
        cx.emit(InputEvent::Changed);
        cx.emit(TextFieldEvent::Changed);
    }

    /// Visible rows of a multi-line input.
    pub fn set_rows(&mut self, rows: usize, cx: &mut Context<Self>) {
        if rows.max(1) != self.rows {
            self.rows = rows.max(1);
            cx.notify();
        }
    }

    /// Read-only and dimmed (while its form is busy).
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

    /// Focus it with everything selected (typing replaces the old text).
    pub fn focus_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text(cx);
        window.focus(&self.focus);
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    fn passes_keys(&self) -> bool {
        self.frame == Frame::Bare
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

    fn edit(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let mut text = if self.multiline {
            text.replace("\r\n", "\n")
        } else {
            text.replace(['\r', '\n'], " ")
        };
        if let Some(max) = self.max_len {
            let room = max.saturating_sub(
                self.content.chars().count() - self.content[range.clone()].chars().count(),
            );
            text = text.chars().take(room).collect();
        }
        let typed = range.is_empty() && text.chars().count() == 1;
        let run = typed
            && self.typing.is_some_and(|(at, when)| {
                at == range.start && when.elapsed() < std::time::Duration::from_secs(1)
            });
        if !run {
            self.undo.push((self.content.clone(), self.selected.clone()));
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.content.replace_range(range.clone(), &text);
        let end = range.start + text.len();
        self.typing = typed.then(|| (end, std::time::Instant::now()));
        self.selected = end..end;
        self.reversed = false;
        self.marked = None;
        self.goal_x = None;
        cx.emit(InputEvent::Changed);
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

    fn on_left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.move_to(prev_grapheme(&self.content, self.cursor()), cx);
        } else {
            self.move_to(self.selected.start, cx);
        }
    }

    fn on_right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.move_to(next_grapheme(&self.content, self.cursor()), cx);
        } else {
            self.move_to(self.selected.end, cx);
        }
    }

    fn on_select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(prev_grapheme(&self.content, self.cursor()), cx);
    }

    fn on_select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_grapheme(&self.content, self.cursor()), cx);
    }

    fn on_word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(prev_word(&self.content, self.cursor()), cx);
    }

    fn on_word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(next_word(&self.content, self.cursor()), cx);
    }

    fn on_select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(prev_word(&self.content, self.cursor()), cx);
    }

    fn on_select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(next_word(&self.content, self.cursor()), cx);
    }

    fn on_home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let (s, _) = self.line_bounds(self.cursor());
        self.move_to(s, cx);
    }

    fn on_end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let (_, e) = self.line_bounds(self.cursor());
        self.move_to(e, cx);
    }

    fn on_select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        let (s, _) = self.line_bounds(self.cursor());
        self.select_to(s, cx);
    }

    fn on_select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let (_, e) = self.line_bounds(self.cursor());
        self.select_to(e, cx);
    }

    /// The offset one visual row up or down from the cursor.
    fn vertical(&mut self, down: bool) -> Option<usize> {
        let laid = self.laid.as_ref()?;
        let lh = laid.line_height;
        let p = self.position_of(self.cursor())?;
        let x = *self.goal_x.get_or_insert(p.x);
        let y = if down { p.y + lh * 1.5 } else { p.y - lh * 0.5 };
        if y < px(0.) {
            return Some(0);
        }
        Some(self.offset_at(point(x, y)))
    }

    fn on_up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
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

    fn on_down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
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

    fn on_select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(o) = self.vertical(false) {
            let g = self.goal_x;
            self.select_to(o, cx);
            self.goal_x = g;
        }
    }

    fn on_select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(o) = self.vertical(true) {
            let g = self.goal_x;
            self.select_to(o, cx);
            self.goal_x = g;
        }
    }

    fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text(cx);
    }

    fn on_backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let c = self.cursor();
            self.selected = prev_grapheme(&self.content, c)..c;
        }
        self.edit(self.selected.clone(), "", cx);
    }

    fn on_delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let c = self.cursor();
            self.selected = c..next_grapheme(&self.content, c);
        }
        self.edit(self.selected.clone(), "", cx);
    }

    fn on_word_backspace(&mut self, _: &WordBackspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let c = self.cursor();
            self.selected = prev_word(&self.content, c)..c;
        }
        self.edit(self.selected.clone(), "", cx);
    }

    fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text().to_string()));
        }
    }

    fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text().to_string()));
            self.edit(self.selected.clone(), "", cx);
        }
    }

    fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.edit(self.selected.clone(), &text, cx);
        }
    }

    fn restore(&mut self, undo: bool, cx: &mut Context<Self>) {
        let (from, to) = if undo {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        let Some((text, sel)) = from.pop() else { return };
        to.push((std::mem::replace(&mut self.content, text), self.selected.clone()));
        self.selected = sel.start.min(self.content.len())..sel.end.min(self.content.len());
        self.reversed = false;
        self.marked = None;
        self.typing = None;
        cx.emit(InputEvent::Changed);
        cx.emit(TextFieldEvent::Changed);
        cx.notify();
    }

    fn on_undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.disabled {
            self.restore(true, cx);
        }
    }

    fn on_redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.disabled {
            self.restore(false, cx);
        }
    }

    fn on_enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.edit(self.selected.clone(), "\n", cx);
            return;
        }
        cx.emit(InputEvent::Submit);
        cx.emit(TextFieldEvent::Enter { shift: false });
        if self.passes_keys() {
            cx.propagate();
        }
    }

    fn on_shift_enter(&mut self, _: &ShiftEnter, _: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.edit(self.selected.clone(), "\n", cx);
        } else {
            cx.emit(InputEvent::SubmitShift);
            cx.emit(TextFieldEvent::Enter { shift: true });
        }
    }

    fn on_submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit);
        cx.emit(TextFieldEvent::Submit);
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel);
        if self.passes_keys() {
            cx.propagate();
        }
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let at = self.offset_at_window(e.position);
        if e.click_count >= 2 {
            let (s, end) = if e.click_count == 2 {
                let r = word_at(&self.content, at);
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
        if self.content.is_empty() || laid.lines.is_empty() {
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

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for TextInput {
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
        self.edit(r, text, cx);
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
            .map(|s| self.range_from_utf16(s))
            .map(|s| {
                let s = r.start + s.start..r.start + s.end;
                s.start.min(self.content.len())..s.end.min(self.content.len())
            })
            .unwrap_or_else(|| r.start + text.len()..r.start + text.len());
        cx.emit(InputEvent::Changed);
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
struct Field {
    input: Entity<TextInput>,
}

impl IntoElement for Field {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

struct Prepared {
    lines: Vec<(usize, Pixels, WrappedLine)>,
    line_height: Pixels,
}

impl Element for Field {
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
        let f = self.input.read(cx);
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
        let t = theme::theme(cx).clone();
        let f = self.input.read(cx);
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
                    let v = vec![
                        run(a),
                        TextRun {
                            underline: Some(UnderlineStyle {
                                color: Some(color),
                                thickness: px(1.),
                                wavy: false,
                            }),
                            ..run(b - a)
                        },
                        run(l.len() - b),
                    ];
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
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let lines = std::mem::take(&mut prep.lines);
        let lh = prep.line_height;
        // Remember the layout first: positions below use it.
        self.input.update(cx, |f, _| {
            f.laid = Some(Laid {
                bounds,
                lines: lines.clone(),
                line_height: lh,
            });
        });
        let t = theme::theme(cx).clone();
        let (cursor, sel, empty, multiline) = {
            let f = self.input.read(cx);
            (
                f.cursor(),
                f.selected.clone(),
                f.content.is_empty(),
                f.multiline,
            )
        };
        // Keep the cursor in view.
        let cpos = self.input.read(cx).position_of(cursor).unwrap_or_default();
        self.input.update(cx, |f, _| {
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
        let f = self.input.read(cx);
        let origin = f.origin().unwrap_or(bounds.origin);
        let focused = focus.is_focused(window);
        // The selection, row by row.
        if !sel.is_empty() && !empty {
            let sel_color = t.focus.opacity(0.5);
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
                    let pb = line
                        .position_for_index(b, lh)
                        .unwrap_or(point(line.unwrapped_layout.width, px(0.)));
                    let row_y = lh * row as f32;
                    if pb.y < row_y || pa.y > row_y {
                        continue;
                    }
                    let x0 = if pa.y < row_y { px(0.) } else { pa.x };
                    let x1 = if pb.y > row_y {
                        bounds.size.width
                    } else if multiline && b == line.len() && sel.end > end {
                        // The line break is selected too.
                        pb.x + px(4.)
                    } else {
                        pb.x
                    };
                    if x1 > x0 {
                        rects.push(Bounds::new(origin + point(x0, y), size(x1 - x0, lh)));
                    }
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
            if focused && (sel.is_empty() || empty) {
                let p = if empty { point(px(0.), px(0.)) } else { cpos };
                window.paint_quad(fill(Bounds::new(origin + p, size(px(1.5), lh)), t.text));
            }
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let focused = self.focus.is_focused(window);
        if self.was_focused && !focused {
            cx.emit(InputEvent::Blur);
        }
        self.was_focused = focused;
        let d = div()
            .id("text-input")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::on_backspace))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_word_backspace))
            .on_action(cx.listener(Self::on_left))
            .on_action(cx.listener(Self::on_right))
            .on_action(cx.listener(Self::on_up))
            .on_action(cx.listener(Self::on_down))
            .on_action(cx.listener(Self::on_select_left))
            .on_action(cx.listener(Self::on_select_right))
            .on_action(cx.listener(Self::on_select_up))
            .on_action(cx.listener(Self::on_select_down))
            .on_action(cx.listener(Self::on_word_left))
            .on_action(cx.listener(Self::on_word_right))
            .on_action(cx.listener(Self::on_select_word_left))
            .on_action(cx.listener(Self::on_select_word_right))
            .on_action(cx.listener(Self::on_home))
            .on_action(cx.listener(Self::on_end))
            .on_action(cx.listener(Self::on_select_home))
            .on_action(cx.listener(Self::on_select_end))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_redo))
            // The macOS Edit menu's items.
            .on_action(cx.listener(|v, _: &crate::menu::Undo, w, cx| v.on_undo(&Undo, w, cx)))
            .on_action(cx.listener(|v, _: &crate::menu::Redo, w, cx| v.on_redo(&Redo, w, cx)))
            .on_action(cx.listener(|v, _: &crate::menu::Cut, w, cx| v.on_cut(&Cut, w, cx)))
            .on_action(cx.listener(|v, _: &crate::menu::Copy, w, cx| v.on_copy(&Copy, w, cx)))
            .on_action(cx.listener(|v, _: &crate::menu::Paste, w, cx| v.on_paste(&Paste, w, cx)))
            .on_action(cx.listener(|v, _: &crate::menu::SelectAll, w, cx| {
                v.on_select_all(&SelectAll, w, cx)
            }))
            .on_action(cx.listener(Self::on_enter))
            .on_action(cx.listener(Self::on_shift_enter))
            .on_action(cx.listener(Self::on_submit))
            .on_action(cx.listener(Self::on_escape))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .when(self.mono, |d| d.font_family(MONO_FONT))
            .when(self.disabled, |d| d.opacity(0.6));
        let field = Field { input: cx.entity() };
        match self.frame {
            Frame::Bare => d
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .overflow_hidden()
                .child(field),
            Frame::Plain => d.w_full().overflow_hidden().text_color(t.text).child(field),
            Frame::Boxed => d
                .w_full()
                .flex()
                .items_center()
                .overflow_hidden()
                .rounded(theme::RADIUS)
                .bg(t.bg)
                .border_1()
                .border_color(if focused { t.text_3 } else { t.line_strong })
                .px(px(9.))
                .text_color(t.text)
                .map(|d| {
                    if self.multiline {
                        // `textarea`: 19 px rows, 12.5 px mono or 13 px.
                        d.py(px(7.))
                            .text_size(px(if self.mono { 12.5 } else { 13. }))
                            .line_height(px(19.))
                    } else {
                        // `.input.mono`: 0.94em of 13 px.
                        d.h(px(34.))
                            .text_size(px(if self.mono { 13. * 0.94 } else { 13. }))
                            .line_height(px(18.))
                    }
                })
                .child(field),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, TestAppContext};

    #[gpui::test]
    fn edits_and_events(cx: &mut TestAppContext) {
        let input = cx.new(|cx| TextInput::new(cx, "héllo", "type").max_len(8));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = events.clone();
        cx.update(|cx| {
            cx.subscribe(&input, move |_, e: &InputEvent, _| {
                seen.borrow_mut().push(e.clone())
            })
            .detach()
        });
        input.update(cx, |i, cx| {
            assert_eq!(prev_grapheme(&i.content, 3), 1, "é is two bytes");
            assert_eq!(next_grapheme(&i.content, 1), 3);
            let end = i.content.len();
            i.edit(end..end, " world", cx);
            assert_eq!(i.text(), "héllo wo", "max length");
            assert_eq!(prev_word(&i.content, i.content.len()), 7);
            assert_eq!(next_word(&i.content, 0), 6);
            assert_eq!(i.offset_to_utf16(3), 2);
            assert_eq!(i.utf16_to_offset(2), 3);
            i.set_text("x", cx);
        });
        assert_eq!(*events.borrow(), vec![InputEvent::Changed]);
    }

    #[gpui::test]
    fn typing_undoes_as_one_step(cx: &mut TestAppContext) {
        let f = cx.new(|cx| TextInput::new(cx, "", ""));
        f.update(cx, |f, cx| {
            for (i, ch) in "abc".chars().enumerate() {
                f.edit(i..i, &ch.to_string(), cx);
            }
            f.edit(3..3, " pasted", cx);
            assert_eq!(f.text(), "abc pasted");
            f.restore(true, cx);
            assert_eq!(f.text(), "abc");
            f.restore(true, cx);
            assert_eq!(f.text(), "", "the typing run is one step");
            f.restore(false, cx);
            f.restore(false, cx);
            assert_eq!(f.text(), "abc pasted");
        });
    }

    #[gpui::test]
    fn one_line_has_no_line_breaks(cx: &mut TestAppContext) {
        let f = cx.new(|cx| TextInput::new(cx, "", "Find").plain());
        f.update(cx, |f, cx| {
            f.replace_text("გამარჯობა\nworld", cx);
            assert_eq!(f.text(), "გამარჯობა world");
            // Backspace removes one Georgian letter (three bytes).
            f.selected = 3..3;
            f.edit(prev_grapheme(&f.content, 3)..3, "", cx);
            assert_eq!(f.text(), "ამარჯობა world");
            f.select_all_text(cx);
            f.edit(f.selected.clone(), "x", cx);
            assert_eq!(f.text(), "x");
        });
    }

    #[gpui::test]
    fn multi_line_keeps_line_breaks(cx: &mut TestAppContext) {
        let f = cx.new(|cx| TextInput::new(cx, "", "What should change here?").multiline(3));
        f.update(cx, |f, cx| {
            f.set_text("a\r\nb", cx);
            f.edit(f.selected.clone(), "\nc", cx);
            assert_eq!(f.text(), "a\r\nb\nc");
            assert_eq!(f.line_bounds(3), (3, 4));
            assert_eq!(f.line_bounds(5), (5, 6));
            f.set_disabled(true, cx);
            f.edit(0..0, "no", cx);
            assert_eq!(f.text(), "a\r\nb\nc", "a disabled input is read-only");
        });
    }
}
