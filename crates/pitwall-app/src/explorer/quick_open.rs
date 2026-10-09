//! ⌘P: a palette-style fuzzy file picker over the agent's files
//! (`list_all_files`, ignore files respected), filtered here (a port of
//! `QuickOpen.tsx`). An empty query lists recently opened files first.

use crate::kit::Ellipsis as _;
use std::ops::Range;
use std::sync::Arc;

use gpui::{
    actions, div, prelude::*, px, uniform_list, App, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, HighlightStyle, Hsla, KeyBinding, MouseButton, ScrollStrategy,
    SharedString, StyledText, Subscription, UniformListScrollHandle, Window,
};

use pitwall_core::explorer::FileIndex;

use crate::kit::file_icons::file_icon;
use crate::kit::{InputEvent, TextInput};
use super::logic::{char_positions_to_byte_ranges, quick_open, split_path, QuickHit};
use super::source::{Res, Source};
use crate::kit::kbd;
use crate::theme::{self, RADIUS};

actions!(explorer_quick_open, [Prev, Next, Pick, Close]);

pub const CONTEXT: &str = "ExplorerQuickOpen";
const LIMIT: usize = 60;
const ROW_H: f32 = 34.;

pub fn bindings() -> Vec<KeyBinding> {
    let c = Some(CONTEXT);
    vec![
        KeyBinding::new("up", Prev, c),
        KeyBinding::new("down", Next, c),
        KeyBinding::new("enter", Pick, c),
        KeyBinding::new("escape", Close, c),
    ]
}

pub enum QuickOpenEvent {
    Picked(String),
    Dismissed,
}

pub struct QuickOpen {
    cwd_display: String,
    input: Entity<TextInput>,
    index: Option<Res<FileIndex>>,
    recent: Vec<String>,
    hits: Arc<Vec<QuickHit>>,
    active: usize,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<QuickOpenEvent> for QuickOpen {}

impl Focusable for QuickOpen {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl QuickOpen {
    pub fn new(
        source: Arc<dyn Source>,
        agent_id: String,
        agent_name: String,
        cwd_display: String,
        recent: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> QuickOpen {
        let placeholder = format!("Go to a file in {agent_name}");
        let input = cx.new(|cx| TextInput::new(cx, "", &placeholder).bare());
        let subs = vec![cx.subscribe(&input, |this, _, e: &InputEvent, cx| {
            if *e != InputEvent::Changed {
                return;
            }
            this.active = 0;
            this.filter(cx);
        })];
        input.update(cx, |i, cx| i.focus_all(window, cx));
        let load = cx
            .background_executor()
            .spawn(async move { source.list_all_files(&agent_id) });
        cx.spawn(async move |this, cx| {
            let index = load.await;
            let _ = this.update(cx, |this, cx| {
                this.index = Some(index);
                this.filter(cx);
            });
        })
        .detach();
        QuickOpen {
            cwd_display,
            input,
            index: None,
            recent,
            hits: Arc::default(),
            active: 0,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _subs: subs,
        }
    }

    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input
            .update(cx, |i, cx| i.replace_text(text, cx));
    }

    fn filter(&mut self, cx: &mut Context<Self>) {
        let Some(Ok(index)) = &self.index else { return };
        let q = self.input.read(cx).text().to_string();
        self.hits = Arc::new(quick_open(&index.files, &q, LIMIT, &self.recent));
        self.active = self.active.min(self.hits.len().saturating_sub(1));
        self.scroll.scroll_to_item(self.active, ScrollStrategy::Top);
        cx.notify();
    }

    fn prev(&mut self, _: &Prev, _: &mut Window, cx: &mut Context<Self>) {
        self.active = self.active.saturating_sub(1);
        self.scroll.scroll_to_item(self.active, ScrollStrategy::Top);
        cx.notify();
    }

    fn next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        self.active = (self.active + 1).min(self.hits.len().saturating_sub(1));
        self.scroll.scroll_to_item(self.active, ScrollStrategy::Top);
        cx.notify();
    }

    fn pick(&mut self, _: &Pick, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(h) = self.hits.get(self.active) {
            cx.emit(QuickOpenEvent::Picked(h.path.clone()));
        }
    }

    fn close(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(QuickOpenEvent::Dismissed);
    }
}

/// `text` with the matched characters bold in `hit` colour.
pub fn marked(text: &str, ranges: Vec<Range<usize>>, hit: Hsla) -> StyledText {
    StyledText::new(text.to_string()).with_highlights(ranges.into_iter().map(|r| {
        (
            r,
            HighlightStyle {
                color: Some(hit),
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
        )
    }))
}

impl Render for QuickOpen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let backdrop = t.scrim();
        let top = window.viewport_size().height * 0.12;
        let note = |text: String, color| {
            div()
                .px(px(14.))
                .py(px(12.))
                .text_size(px(12.5))
                .text_color(color)
                .child(text)
        };
        let status = match &self.index {
            None => Some(note("Reading files…".into(), t.text_3)),
            Some(Err(e)) => Some(note(
                format!(
                    "Couldn't list files: {}",
                    e.lines().next().unwrap_or_default()
                ),
                t.red,
            )),
            Some(Ok(_)) if self.hits.is_empty() => {
                Some(note("No matching files.".into(), t.text_3))
            }
            Some(Ok(_)) => None,
        };
        let truncated = matches!(&self.index, Some(Ok(i)) if i.truncated);
        let hits = self.hits.clone();
        let active = self.active;
        let list = uniform_list(
            "ex-quick",
            hits.len(),
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let t = theme::theme(cx).clone();
                range
                    .map(|ix| {
                        let h = &hits[ix];
                        let (dir, base) = split_path(&h.path);
                        let dir_chars = if dir.is_empty() {
                            0
                        } else {
                            dir.chars().count() + 1
                        };
                        let base_hits =
                            char_positions_to_byte_ranges(base, &h.positions, dir_chars);
                        let dir_hits = char_positions_to_byte_ranges(dir, &h.positions, 0);
                        let selected = ix == active;
                        div()
                            .id(ix)
                            .w_full()
                            .h(px(ROW_H))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .rounded(RADIUS)
                            .cursor_pointer()
                            .when(selected, |d| d.bg(t.surface_3))
                            .on_mouse_move(cx.listener(move |this: &mut QuickOpen, _, _, cx| {
                                if this.active != ix {
                                    this.active = ix;
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |this: &mut QuickOpen, _, _, cx| {
                                if let Some(h) = this.hits.get(ix) {
                                    cx.emit(QuickOpenEvent::Picked(h.path.clone()));
                                }
                            }))
                            .child(file_icon(&h.path, 16.))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(8.))
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_size(px(13.))
                                            .text_color(t.text_2)
                                            .child(marked(base, base_hits, t.text)),
                                    )
                                    .when(!dir.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .min_w_0()
                                                .ellipsis()
                                                .text_size(px(11.5))
                                                .text_color(t.text_3)
                                                .child(marked(dir, dir_hits, t.text_2)),
                                        )
                                    }),
                            )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .h(px((self.hits.len().min(11) as f32) * ROW_H));
        let foot = |text: SharedString| {
            div()
                .px(px(16.))
                .py(px(7.))
                .border_t_1()
                .border_color(t.line)
                .text_size(px(11.5))
                .text_color(t.text_3)
                .child(text)
        };
        let backdrop = div()
            .id("ex-quick-backdrop")
            .absolute()
            .inset_0()
            .bg(backdrop)
            .flex()
            .justify_center()
            .items_start()
            .pt(top)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(QuickOpenEvent::Dismissed)),
            )
;
        let panel = div()
                    .id("ex-quick")
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(Self::prev))
                    .on_action(cx.listener(Self::next))
                    .on_action(cx.listener(Self::pick))
                    .on_action(cx.listener(Self::close))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .w(px(600.))
                    .max_h(px(520.))
                    .flex()
                    .flex_col()
                    .rounded(px(12.))
                    .bg(t.raised)
                    .shadow(t.float_shadow())
                    .overflow_hidden()
                    .child(
                        div()
                            .px(px(18.))
                            .py(px(14.))
                            .border_b_1()
                            .border_color(t.line)
                            .text_size(px(15.))
                            .text_color(t.text)
                            .child(self.input.clone()),
                    )
                    .child(
                        div().py(px(6.)).pl(px(6.)).children(status).child(crate::kit::vscroll_list("ex-quick-bar", &self.scroll,
                            &t,
                            list.mr(px(6.)),
                        )),
                    )
                    .when(truncated, |d| {
                        d.child(foot("Only the first 100 000 files are listed.".into()))
                    })
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(7.))
                            .border_t_1()
                            .border_color(t.line)
                            .text_size(px(11.5))
                            .text_color(t.text_3)
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child("Read-only ·")
                            .child(kbd("↵", &t))
                            .child(
                                div()
                                    .min_w_0()
                                    .ellipsis()
                                    .child(crate::kit::one_line(format!("opens in the viewer · {}", self.cwd_display))),
                            ),
                    );
        let panel = if theme::motion_on(cx) {
            crate::kit::motion::enter("ex-quick-in", crate::kit::motion::Fx::PALETTE, panel)
                .into_any_element()
        } else {
            panel.into_any_element()
        };
        crate::kit::scrim_in("ex-quick-fade", backdrop.child(panel))
    }
}
