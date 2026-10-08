//! The single-line text input (built in-house; GPUI's own
//! `examples/input.rs`, Apache-2.0, is the reference): IME through
//! `EntityInputHandler` (marked text underlined, the candidate window at the
//! caret), selection with mouse and shift-arrows, word moves and word
//! delete, clipboard, select all. Enter and Esc are reported as events so
//! the owner decides (submit, cancel).
//!
//! Two looks: boxed (`.input`: 34 px, hairline, the focus border), and
//! [`TextInput::bare`] for fields drawn inside another box (search bars,
//! quick open); a bare input lets Enter, Esc and the arrows through to its
//! owner's key bindings too.

use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem, Context,
    CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine, SharedString, Style,
    TextRun, UTF16Selection, UnderlineStyle, Window,
};

use crate::theme;

use super::fonts::MONO_FONT;

actions!(
    pw_text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Enter,
        Escape,
        WordLeft,
        WordRight,
        WordBackspace,
    ]
);

const CONTEXT: &str = "PwTextInput";

/// Key bindings of every input (registered once by `kit::init`).
pub fn bindings() -> Vec<KeyBinding> {
    let c = Some(CONTEXT);
    let mut b = vec![
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("enter", Enter, c),
        KeyBinding::new("escape", Escape, c),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "alt-backspace"
            } else {
                "ctrl-backspace"
            },
            WordBackspace,
            c,
        ),
    ];
    if cfg!(target_os = "macos") {
        b.extend([
            KeyBinding::new("cmd-a", SelectAll, c),
            KeyBinding::new("cmd-v", Paste, c),
            KeyBinding::new("cmd-c", Copy, c),
            KeyBinding::new("cmd-x", Cut, c),
            KeyBinding::new("cmd-left", Home, c),
            KeyBinding::new("cmd-right", End, c),
            KeyBinding::new("alt-left", WordLeft, c),
            KeyBinding::new("alt-right", WordRight, c),
        ]);
    } else {
        b.extend([
            KeyBinding::new("ctrl-a", SelectAll, c),
            KeyBinding::new("ctrl-v", Paste, c),
            KeyBinding::new("ctrl-c", Copy, c),
            KeyBinding::new("ctrl-x", Cut, c),
            KeyBinding::new("ctrl-left", WordLeft, c),
            KeyBinding::new("ctrl-right", WordRight, c),
        ]);
    }
    b
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Changed,
    Submit,
    Cancel,
    Blur,
}

pub struct TextInput {
    focus: FocusHandle,
    content: String,
    placeholder: SharedString,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    selecting: bool,
    mono: bool,
    bare: bool,
    max_len: Option<usize>,
    was_focused: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl TextInput {
    pub fn new(cx: &mut Context<Self>, text: &str, placeholder: &str) -> TextInput {
        TextInput {
            focus: cx.focus_handle().tab_stop(true),
            content: text.to_string(),
            placeholder: placeholder.to_string().into(),
            selected: text.len()..text.len(),
            reversed: false,
            marked: None,
            last_layout: None,
            last_bounds: None,
            selecting: false,
            mono: false,
            bare: false,
            max_len: None,
            was_focused: false,
        }
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// No box of its own (the owner draws one); Enter, Esc and the arrows
    /// also reach the owner's bindings.
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }

    pub fn set_placeholder(&mut self, text: impl Into<SharedString>) {
        self.placeholder = text.into();
    }

    /// Replace the text and report it ([`InputEvent::Changed`]).
    pub fn replace_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.set_text(text, cx);
        cx.emit(InputEvent::Changed);
    }

    /// Focus it with everything selected (typing replaces the old text).
    pub fn focus_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text(cx);
        window.focus(&self.focus);
    }

    pub fn max_len(mut self, n: usize) -> Self {
        self.max_len = Some(n);
        self
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    /// Replace the text (the cursor goes to the end); no Changed event.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = text.to_string();
        self.selected = text.len()..text.len();
        self.marked = None;
        cx.notify();
    }

    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.selected = 0..self.content.len();
        self.reversed = false;
        cx.notify();
    }

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    fn move_to(&mut self, at: usize, cx: &mut Context<Self>) {
        self.selected = at..at;
        cx.notify();
    }

    fn select_to(&mut self, at: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selected.start = at;
        } else {
            self.selected.end = at;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.notify();
    }

    fn prev(&self, at: usize) -> usize {
        self.content[..at]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn next(&self, at: usize) -> usize {
        self.content[at..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| at + i)
            .unwrap_or(self.content.len())
    }

    fn word_left(&self, at: usize) -> usize {
        let s = &self.content[..at];
        let t = s.trim_end_matches(|c: char| !c.is_alphanumeric());
        t.rfind(|c: char| !c.is_alphanumeric())
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    fn word_right(&self, at: usize) -> usize {
        let s = &self.content[at..];
        let skip = s.len() - s.trim_start_matches(|c: char| !c.is_alphanumeric()).len();
        let rest = &s[skip..];
        at + skip
            + rest
                .find(|c: char| !c.is_alphanumeric())
                .unwrap_or(rest.len())
    }

    fn edit(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let mut text = text.replace(['\n', '\r'], " ");
        if let Some(max) = self.max_len {
            let room = max.saturating_sub(
                self.content.chars().count() - self.content[range.clone()].chars().count(),
            );
            text = text.chars().take(room).collect();
        }
        self.content.replace_range(range.clone(), &text);
        let at = range.start + text.len();
        self.selected = at..at;
        self.marked = None;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn on_left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let at = if self.selected.is_empty() {
            self.prev(self.cursor())
        } else {
            self.selected.start
        };
        self.move_to(at, cx);
    }

    fn on_right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let at = if self.selected.is_empty() {
            self.next(self.selected.end)
        } else {
            self.selected.end
        };
        self.move_to(at, cx);
    }

    fn on_word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.word_left(self.cursor()), cx);
    }

    fn on_word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.word_right(self.cursor()), cx);
    }

    fn on_select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.prev(self.cursor()), cx);
    }

    fn on_select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next(self.cursor()), cx);
    }

    fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text(cx);
    }

    fn on_home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn on_end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn on_backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let at = self.cursor();
            self.selected = self.prev(at)..at;
        }
        self.edit(self.selected.clone(), "", cx);
    }

    fn on_word_backspace(&mut self, _: &WordBackspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let at = self.cursor();
            self.selected = self.word_left(at)..at;
        }
        self.edit(self.selected.clone(), "", cx);
    }

    fn on_delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let at = self.cursor();
            self.selected = at..self.next(at);
        }
        self.edit(self.selected.clone(), "", cx);
    }

    fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.edit(self.selected.clone(), &text, cx);
        }
    }

    fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected.clone()].to_string(),
            ));
        }
    }

    fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected.clone()].to_string(),
            ));
            self.edit(self.selected.clone(), "", cx);
        }
    }

    fn on_enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit);
        if self.bare {
            cx.propagate();
        }
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel);
        if self.bare {
            cx.propagate();
        }
    }

    fn index_for(&self, pos: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return self.content.len();
        };
        if self.content.is_empty() || pos.y < bounds.top() {
            return 0;
        }
        if pos.y > bounds.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(pos.x - bounds.left())
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.selecting = true;
        let at = self.index_for(e.position);
        if e.click_count >= 2 {
            self.selected = 0..self.content.len();
            cx.notify();
        } else if e.modifiers.shift {
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
            self.select_to(self.index_for(e.position), cx);
        }
    }

    fn offset_to_utf16(&self, at: usize) -> usize {
        self.content[..at.min(self.content.len())]
            .encode_utf16()
            .count()
    }

    fn utf16_to_offset(&self, at: usize) -> usize {
        let mut n = 0;
        for (i, ch) in self.content.char_indices() {
            if n >= at {
                return i;
            }
            n += ch.len_utf16();
        }
        self.content.len()
    }

    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end)
    }

    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.utf16_to_offset(r.start)..self.utf16_to_offset(r.end)
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
            .map(|s| s.start + r.start..s.end + r.start)
            .unwrap_or_else(|| r.start + text.len()..r.start + text.len());
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let r = self.range_from_utf16(&range);
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(r.start), bounds.top()),
            point(bounds.left() + line.x_for_index(r.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.last_bounds?.localize(&p)?;
        let line = self.last_layout.as_ref()?;
        let i = line.index_for_x(p.x - local.x)?;
        Some(self.offset_to_utf16(i))
    }
}

struct TextLine {
    input: Entity<TextInput>,
}

struct Prepaint {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextLine {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextLine {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

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
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
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
    ) -> Prepaint {
        let t = theme::theme(cx).clone();
        let input = self.input.read(cx);
        let style = window.text_style();
        let (text, color): (SharedString, _) = if input.content.is_empty() {
            (input.placeholder.clone(), t.text_4)
        } else {
            (input.content.clone().into(), style.color)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match input.marked.as_ref().filter(|_| !input.content.is_empty()) {
            Some(m) => [
                TextRun {
                    len: m.start,
                    ..run.clone()
                },
                TextRun {
                    len: m.end - m.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: text.len() - m.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(text, font_size, &runs, None);
        let (selection, cursor) = if input.content.is_empty() || input.selected.is_empty() {
            let x = if input.content.is_empty() {
                px(0.)
            } else {
                line.x_for_index(input.cursor())
            };
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + x, bounds.top()),
                        size(px(1.5), bounds.size.height),
                    ),
                    t.text,
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(input.selected.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(input.selected.end),
                            bounds.bottom(),
                        ),
                    ),
                    t.focus.opacity(0.5),
                )),
                None,
            )
        };
        Prepaint {
            line: Some(line),
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        pre: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(s) = pre.selection.take() {
            window.paint_quad(s);
        }
        let Some(line) = pre.line.take() else { return };
        let _ = line.paint(bounds.origin, window.line_height(), window, cx);
        if focus.is_focused(window) {
            if let Some(c) = pre.cursor.take() {
                window.paint_quad(c);
            }
        }
        self.input.update(cx, |input, _| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
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
            .on_action(cx.listener(Self::on_word_backspace))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_left))
            .on_action(cx.listener(Self::on_right))
            .on_action(cx.listener(Self::on_word_left))
            .on_action(cx.listener(Self::on_word_right))
            .on_action(cx.listener(Self::on_select_left))
            .on_action(cx.listener(Self::on_select_right))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_home))
            .on_action(cx.listener(Self::on_end))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_enter))
            .on_action(cx.listener(Self::on_escape))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .when(self.mono, |d| d.font_family(MONO_FONT));
        if self.bare {
            return d
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .overflow_hidden()
                .child(TextLine { input: cx.entity() });
        }
        d.w_full()
            .h(px(34.))
            .px(px(9.))
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(theme::RADIUS)
            .bg(t.bg)
            .border_1()
            .border_color(if focused { t.text_3 } else { t.line_strong })
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(TextLine { input: cx.entity() })
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
            assert_eq!(i.prev(3), 1, "é is two bytes");
            assert_eq!(i.next(1), 3);
            let end = i.content.len();
            i.edit(end..end, " world", cx);
            assert_eq!(i.text(), "héllo wo", "max length");
            assert_eq!(i.word_left(i.content.len()), 7);
            assert_eq!(i.word_right(0), 6);
            assert_eq!(i.offset_to_utf16(3), 2);
            assert_eq!(i.utf16_to_offset(2), 3);
            i.set_text("x", cx);
        });
        assert_eq!(*events.borrow(), vec![InputEvent::Changed]);
    }
}
