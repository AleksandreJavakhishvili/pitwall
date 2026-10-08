//! Small in-house widgets for Settings and onboarding, styled after
//! `src/styles/base.css` and `overlays.css` (buttons, chips, segmented
//! control, toggle, checkbox, radio, select, a one-line text field). They
//! live here until the shared kit (`src/kit/`, phase 1) has them: promote or
//! replace at merge.

use std::rc::Rc;

use gpui::{
    anchored, canvas, deferred, div, point, prelude::*, px, AnimationExt, AnyElement, AnyView, App,
    ClickEvent, Context, Div, EventEmitter, FocusHandle, FontWeight, Hsla, KeyDownEvent,
    MouseButton, PathBuilder, SharedString, Stateful, Window,
};

use crate::theme::{Theme, RADIUS, RADIUS_SM};

use super::appearance::{appearance, GlassTokens};

/// A click handler that needs no view.
pub type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
/// Called with the picked index (segmented controls, dropdowns).
pub type OnPick = Rc<dyn Fn(usize, &mut Window, &mut App)>;
/// Called when a popover closes.
pub type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// What every widget needs: the theme to draw with and whether Glass is on
/// (pill buttons, glass segmented controls).
#[derive(Clone)]
pub struct Ui {
    pub t: Theme,
    pub glass: Option<GlassTokens>,
}

impl Ui {
    pub fn new(t: Theme, cx: &App) -> Ui {
        let ap = appearance(cx);
        Ui {
            t,
            glass: ap.glass.map(|_| ap.glass_tokens.clone()),
        }
    }

    fn btn_radius(&self) -> gpui::Pixels {
        if self.glass.is_some() {
            px(999.)
        } else {
            RADIUS
        }
    }

    /// `.label`: small caps, condensed (Barlow Condensed once fonts land).
    pub fn label(&self, text: impl Into<SharedString>) -> Div {
        let text: SharedString = text.into();
        div()
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(self.t.text_3)
            .child(SharedString::from(text.to_uppercase()))
    }

    /// `.muted` (13 px, text-2).
    pub fn muted(&self, text: impl Into<SharedString>) -> Div {
        div().text_color(self.t.text_2).child(text.into())
    }

    /// `.hint` / `.muted-sm` (12 px, text-3).
    pub fn hint(&self, text: impl Into<SharedString>) -> Div {
        div()
            .text_size(px(12.))
            .text_color(self.t.text_3)
            .child(text.into())
    }

    pub fn mono(&self, text: impl Into<SharedString>) -> Div {
        div()
            .font_family("Menlo")
            .text_size(px(12.))
            .text_color(self.t.text_3)
            .child(text.into())
    }

    pub fn error(&self, text: impl Into<SharedString>) -> Div {
        div()
            .text_size(px(12.))
            .text_color(self.t.red)
            .child(text.into())
    }

    pub fn chip(&self, text: impl Into<SharedString>, tone: Tone) -> Div {
        let text: SharedString = text.into();
        let (fg, bg, border): (Hsla, Option<Hsla>, Hsla) = match tone {
            Tone::Ok => (
                self.t.green,
                Some(self.t.green_soft),
                gpui::transparent_black(),
            ),
            Tone::Warn => (
                self.t.amber,
                Some(self.t.amber_soft),
                gpui::transparent_black(),
            ),
            Tone::Subtle => (
                self.t.text_3,
                Some(self.t.surface_3),
                gpui::transparent_black(),
            ),
            Tone::Plain => (self.t.text_3, None, self.t.line_strong),
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .px(px(5.))
            .py(px(2.))
            .rounded(px(3.))
            .border_1()
            .border_color(border)
            .when_some(bg, |d, bg| d.bg(bg))
            .text_size(px(10.))
            .font_weight(FontWeight::SEMIBOLD)
            .line_height(px(11.))
            .text_color(fg)
            .whitespace_nowrap()
            .child(SharedString::from(text.to_uppercase()))
    }

    /// The smaller chips of onboarding lists (`.onb-chips .chip`).
    pub fn chip_sm(&self, text: impl Into<SharedString>, tone: Tone) -> Div {
        self.chip(text, tone)
            .px(px(4.))
            .pt(px(2.))
            .pb(px(1.))
            .text_size(px(9.5))
    }

    /// A button of one of the CSS kinds. Attach `.on_click` to it.
    pub fn button(
        &self,
        id: impl Into<gpui::ElementId>,
        text: impl Into<SharedString>,
        kind: Btn,
        disabled: bool,
    ) -> Stateful<Div> {
        let t = &self.t;
        let base = div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(7.))
            .whitespace_nowrap()
            .font_weight(FontWeight::SEMIBOLD)
            .when(!disabled, |d| d.cursor_pointer())
            .when(disabled, |d| d.opacity(0.5));
        let radius = self.btn_radius();
        match kind {
            Btn::Primary | Btn::PrimaryLg => {
                let lg = kind == Btn::PrimaryLg;
                let fg = self.glass.as_ref().map(|g| g.on_primary).unwrap_or(t.bg);
                let hover = t.text.opacity(0.88);
                base.h(px(if lg { 38. } else { 30. }))
                    .px(px(if lg { 16. } else { 12. }))
                    .text_size(px(if lg { 14. } else { 12.5 }))
                    .rounded(radius)
                    .bg(t.text)
                    .text_color(fg)
                    .when(!disabled, |d| d.hover(move |s| s.bg(hover)))
            }
            Btn::GhostSm => {
                let (fg, hover_bg) = (t.text, t.surface_3);
                base.h(px(26.))
                    .px(px(10.))
                    .text_size(px(12.5))
                    .rounded(radius)
                    .border_1()
                    .border_color(t.line_strong)
                    .text_color(t.text_2)
                    .when(!disabled, |d| {
                        d.hover(move |s| s.text_color(fg).bg(hover_bg))
                    })
            }
            Btn::Ghost => {
                let (fg, hover_bg) = (t.text, t.surface_3);
                base.h(px(30.))
                    .px(px(12.))
                    .text_size(px(12.5))
                    .rounded(radius)
                    .border_1()
                    .border_color(t.line_strong)
                    .text_color(t.text_2)
                    .when(!disabled, |d| {
                        d.hover(move |s| s.text_color(fg).bg(hover_bg))
                    })
            }
            Btn::Small => {
                let hover = t.text_4;
                base.h(px(26.))
                    .px(px(9.))
                    .text_size(px(12.))
                    .rounded(radius)
                    .bg(t.surface_3)
                    .border_1()
                    .border_color(t.line_strong)
                    .text_color(t.text)
                    .when(!disabled, |d| d.hover(move |s| s.border_color(hover)))
            }
            Btn::Link => {
                let fg = t.text;
                base.text_size(px(12.))
                    .text_color(t.text_3)
                    .underline()
                    .when(!disabled, |d| d.hover(move |s| s.text_color(fg)))
            }
        }
        .child(text.into())
    }

    /// `.icon-btn` with a text glyph (the icon set comes with the kit).
    pub fn icon_button(
        &self,
        id: impl Into<gpui::ElementId>,
        glyph: &'static str,
    ) -> Stateful<Div> {
        let (fg, bg) = (self.t.text, self.t.surface_3);
        div()
            .id(id)
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS)
            .text_size(px(13.))
            .text_color(self.t.text_3)
            .cursor_pointer()
            .hover(move |s| s.text_color(fg).bg(bg))
            .child(glyph)
    }

    /// `.seg`: a row of radio buttons. `on_pick` gets the chosen index.
    pub fn seg(&self, id: &'static str, items: Vec<SegItem>, on_pick: OnPick) -> Div {
        let t = &self.t;
        let (bg, on_bg) = match &self.glass {
            Some(g) => (g.surface_2, g.surface_3),
            None => (t.surface_2, t.surface_3),
        };
        div()
            .flex()
            .flex_none()
            .gap(px(1.))
            .p(px(2.))
            .rounded(RADIUS)
            .bg(bg)
            .when_some(self.glass.as_ref(), |d, g| {
                d.border_1().border_color(g.line)
            })
            .children(items.into_iter().enumerate().map(|(i, item)| {
                let pick = on_pick.clone();
                let hover = t.text;
                div()
                    .id((id, i))
                    .h(px(24.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(RADIUS_SM)
                    .text_size(px(12.))
                    .cursor_pointer()
                    .text_color(if item.on { t.text } else { t.text_3 })
                    .when(item.on, |d| d.bg(on_bg).font_weight(FontWeight::SEMIBOLD))
                    .hover(move |s| s.text_color(hover))
                    .when_some(item.tooltip, |d, tip| d.tooltip(tooltip(tip)))
                    .on_click(move |_, window, cx| pick(i, window, cx))
                    .child(item.label)
            }))
    }

    /// `.toggle`: the switch. Attach `.on_click`.
    pub fn toggle(&self, id: impl Into<gpui::ElementId>, on: bool) -> Stateful<Div> {
        let t = &self.t;
        div()
            .id(id)
            .flex_none()
            .w(px(26.))
            .h(px(15.))
            .rounded(px(10.))
            .border_1()
            .cursor_pointer()
            .border_color(if on { t.text } else { t.line_strong })
            .bg(if on { t.text } else { t.surface_3 })
            .relative()
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(px(if on { 13. } else { 2. }))
                    .size(px(9.))
                    .rounded_full()
                    .bg(if on { t.bg } else { t.text_3 }),
            )
    }

    /// A checkbox square (the row around it takes the click).
    pub fn checkbox(&self, checked: bool, disabled: bool) -> Div {
        let t = &self.t;
        div()
            .flex_none()
            .size(px(14.))
            .rounded(px(3.))
            .border_1()
            .flex()
            .items_center()
            .justify_center()
            .border_color(if checked { t.text } else { t.text_4 })
            .when(checked, |d| d.bg(t.text))
            .when(disabled, |d| d.opacity(0.45))
            .text_size(px(10.))
            .font_weight(FontWeight::BOLD)
            .text_color(t.bg)
            .when(checked, |d| d.child("✓"))
    }

    /// A radio dot (the row around it takes the click).
    pub fn radio(&self, checked: bool) -> Div {
        let t = &self.t;
        div()
            .flex_none()
            .size(px(14.))
            .rounded_full()
            .border_1()
            .flex()
            .items_center()
            .justify_center()
            .border_color(if checked { t.text } else { t.text_4 })
            .when(checked, |d| {
                d.child(div().size(px(6.)).rounded_full().bg(t.text))
            })
    }

    /// The checklist's spinning ring (`onb-spin`): a faint track with a
    /// bright quarter turning; still when motion is off.
    pub fn spinner(&self, id: &'static str, size: f32, motion: bool) -> AnyElement {
        let (track, head) = (self.t.line_strong, self.t.text);
        let ring = move |turn: f32| {
            div()
                .flex_none()
                .size(px(size))
                .rounded_full()
                .border(px(1.5))
                .border_color(track)
                .child(
                    canvas(
                        |_, _, _| {},
                        move |b, _, window, _| {
                            let r = f32::from(b.size.width) / 2. - 0.75;
                            let c = b.center();
                            let mut path = PathBuilder::stroke(px(1.5));
                            let start = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                            for i in 0..=12 {
                                let a = start + i as f32 / 12. * std::f32::consts::FRAC_PI_2;
                                let pt = point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
                                if i == 0 {
                                    path.move_to(pt);
                                } else {
                                    path.line_to(pt);
                                }
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, head);
                            }
                        },
                    )
                    .absolute()
                    .top(px(-1.5))
                    .left(px(-1.5))
                    .size(px(size)),
                )
        };
        if motion {
            ring(0.)
                .with_animation(
                    id,
                    gpui::Animation::new(std::time::Duration::from_millis(900)).repeat(),
                    move |_, d| ring(d),
                )
                .into_any_element()
        } else {
            ring(0.).into_any_element()
        }
    }

    /// `.confirm-box`.
    pub fn confirm_box(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(14.))
            .py(px(12.))
            .rounded(RADIUS)
            .bg(self.t.surface_2)
            .border_1()
            .border_color(self.t.line_strong)
    }

    /// Numbered steps in hint style (`.hint.settings-steps`).
    pub fn steps(&self, items: Vec<gpui::AnyElement>) -> Div {
        self.list(items, true)
            .text_size(px(12.))
            .text_color(self.t.text_3)
    }

    /// A bulleted (or numbered) list in 12.5 px text-2 (`.confirm-box ul`).
    pub fn list(&self, items: Vec<gpui::AnyElement>, numbered: bool) -> Div {
        let fg = self.t.text_2;
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(12.5))
            .text_color(fg)
            .pl(px(4.))
            .children(items.into_iter().enumerate().map(move |(i, it)| {
                let mark: SharedString = if numbered {
                    format!("{}.", i + 1).into()
                } else {
                    "•".into()
                };
                div()
                    .flex()
                    .gap(px(5.))
                    .child(div().flex_none().w(px(12.)).child(mark))
                    .child(div().flex_1().min_w_0().child(it))
            }))
    }

    /// A dropdown's trigger (`select.input`): the current value and a caret.
    pub fn select_trigger(
        &self,
        id: impl Into<gpui::ElementId>,
        text: impl Into<SharedString>,
    ) -> Stateful<Div> {
        let t = &self.t;
        div()
            .id(id)
            .h(px(26.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(RADIUS)
            .bg(t.bg)
            .border_1()
            .border_color(t.line_strong)
            .text_size(px(12.))
            .text_color(t.text)
            .cursor_pointer()
            .child(
                div()
                    .max_w(px(260.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(text.into()),
            )
            .child(div().text_size(px(9.)).text_color(t.text_3).child("▾"))
    }

    /// The open dropdown, floating under its trigger; `on_pick(index)`,
    /// `on_close` on a click outside.
    pub fn select_menu(
        &self,
        id: impl Into<gpui::ElementId>,
        items: Vec<(SharedString, bool)>,
        on_pick: OnPick,
        on_close: OnClose,
    ) -> impl IntoElement {
        let t = self.t.clone();
        let id = id.into();
        div().absolute().left_0().top(px(28.)).child(deferred(
            anchored().snap_to_window().child(
                div()
                    .id(id)
                    .occlude()
                    .min_w(px(180.))
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .rounded(RADIUS)
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.line_strong)
                    .shadow_lg()
                    .on_mouse_down_out(move |_, window, cx| on_close(window, cx))
                    .children(items.into_iter().enumerate().map(|(i, (label, on))| {
                        let pick = on_pick.clone();
                        let hover = t.surface_3;
                        div()
                            .id(i)
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(RADIUS_SM)
                            .text_size(px(12.))
                            .text_color(if on { t.text } else { t.text_2 })
                            .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .on_click(move |_, window, cx| pick(i, window, cx))
                            .child(label)
                    })),
            ),
        ))
    }
}

/// Chip and status tones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Subtle,
    Plain,
}

impl From<super::permissions::Tone> for Tone {
    fn from(t: super::permissions::Tone) -> Tone {
        match t {
            super::permissions::Tone::Ok => Tone::Ok,
            super::permissions::Tone::Warn => Tone::Warn,
            super::permissions::Tone::Subtle => Tone::Subtle,
        }
    }
}

/// The CSS button kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Primary,
    PrimaryLg,
    /// 30 px (`.ghost-btn`).
    Ghost,
    /// 26 px (`.setting-row .ghost-btn`).
    GhostSm,
    Small,
    Link,
}

pub struct SegItem {
    pub label: SharedString,
    pub on: bool,
    pub tooltip: Option<SharedString>,
}

/// A plain text tooltip.
pub struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = super::appearance::appearance(cx).theme();
        div()
            .max_w(px(320.))
            .px(px(8.))
            .py(px(5.))
            .rounded(RADIUS)
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .text_size(px(12.))
            .text_color(t.text_2)
            .shadow_md()
            .child(self.0.clone())
    }
}

pub fn tooltip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

/// A one-line text field (typed path fallbacks). No IME or selection: the
/// kit's text input (phase 1, `EntityInputHandler`) replaces it.
pub struct LineInput {
    pub text: String,
    placeholder: SharedString,
    focus: FocusHandle,
}

pub enum LineInputEvent {
    Submit,
    Cancel,
}

impl EventEmitter<LineInputEvent> for LineInput {}

impl LineInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> LineInput {
        LineInput {
            text: String::new(),
            placeholder: placeholder.into(),
            focus: cx.focus_handle(),
        }
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let cmd = k.modifiers.platform || k.modifiers.control;
        match k.key.as_str() {
            "enter" => cx.emit(LineInputEvent::Submit),
            "escape" => {
                cx.emit(LineInputEvent::Cancel);
                cx.stop_propagation();
            }
            "backspace" => {
                if cmd {
                    self.text.clear();
                } else {
                    self.text.pop();
                }
            }
            "v" if cmd => {
                if let Some(s) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    self.text.push_str(s.trim_end_matches(['\n', '\r']));
                }
            }
            _ => match &k.key_char {
                Some(ch) if !cmd => self.text.push_str(ch),
                _ => return,
            },
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for LineInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = appearance(cx).theme();
        let focused = self.focus.is_focused(window);
        let empty = self.text.is_empty();
        div()
            .id("line-input")
            .track_focus(&self.focus)
            .key_context("LineInput")
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| this.focus(window)),
            )
            .flex_1()
            .min_w(px(220.))
            .h(px(30.))
            .px(px(9.))
            .flex()
            .items_center()
            .rounded(RADIUS)
            .bg(t.bg)
            .border_1()
            .border_color(if focused { t.text_3 } else { t.line_strong })
            .font_family("Menlo")
            .text_size(px(12.))
            .cursor_text()
            .child(
                div()
                    .text_color(if empty { t.text_4 } else { t.text })
                    .child(if empty {
                        self.placeholder.clone()
                    } else {
                        SharedString::from(self.text.clone())
                    }),
            )
            .when(focused, |d| d.child(div().w(px(1.)).h(px(14.)).bg(t.text)))
    }
}
