//! The bottom status strip (React `StatusStrip.tsx`): fixed height, calm /
//! amber "needs you" / mint "finished", so it never resizes the terminals.
//! And the toast stack (`Toasts.tsx`, `useToasts.ts`): max 4, one per agent,
//! blocked 9 s, error 8 s, others 5 s; click jumps to the agent.

use std::time::Duration;

use gpui::{div, prelude::*, px, Context, FontWeight, IntoElement, SharedString, Task};

use super::model::{strip, Tone};
use super::widgets::{kbd, label_t};
use super::{Breakpoint, MainScreen};
use crate::theme::{Theme, RADIUS, RADIUS_LG};

/// `--statusbar-h`.
pub const STRIP_H: f32 = 28.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastTone {
    Blocked,
    Done,
    Error,
    Info,
}

pub struct Toast {
    id: u64,
    pub tone: ToastTone,
    pub title: SharedString,
    pub detail: Option<SharedString>,
    pub agent: Option<String>,
    _timer: Option<Task<()>>,
}

impl Toast {
    fn new(tone: ToastTone, title: String, detail: Option<String>, agent: Option<String>) -> Toast {
        Toast {
            id: 0,
            tone,
            title: title.into(),
            detail: detail.map(Into::into),
            agent,
            _timer: None,
        }
    }
    pub fn blocked(title: String, agent: String) -> Toast {
        Toast::new(ToastTone::Blocked, title, None, Some(agent))
    }
    pub fn done(title: String, agent: String) -> Toast {
        Toast::new(ToastTone::Done, title, None, Some(agent))
    }
    pub fn error(title: String, detail: String) -> Toast {
        Toast::new(ToastTone::Error, title, Some(detail), None)
    }
    pub fn info(title: &str, detail: Option<String>) -> Toast {
        Toast::new(ToastTone::Info, title.to_string(), detail, None)
    }

    fn lifetime(&self) -> Duration {
        match self.tone {
            ToastTone::Blocked => Duration::from_secs(9),
            ToastTone::Error => Duration::from_secs(8),
            _ => Duration::from_secs(5),
        }
    }
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Add a toast: one per agent (the newer replaces), at most 4.
pub fn push(screen: &mut MainScreen, mut toast: Toast, cx: &mut Context<MainScreen>) {
    toast.id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Some(a) = &toast.agent {
        screen.toasts.retain(|t| t.agent.as_ref() != Some(a));
    }
    let id = toast.id;
    let life = toast.lifetime();
    toast._timer = Some(cx.spawn(async move |this, cx| {
        cx.background_executor().timer(life).await;
        let _ = this.update(cx, |s, cx| {
            s.toasts.retain(|t| t.id != id);
            cx.notify();
        });
    }));
    screen.toasts.push(toast);
    while screen.toasts.len() > 4 {
        screen.toasts.remove(0);
    }
    cx.notify();
}

impl MainScreen {
    pub(super) fn render_strip(
        &mut self,
        t: &Theme,
        bp: Breakpoint,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let s = strip(&self.ordered(cx));
        let base = div()
            .id("status-strip")
            .h(px(STRIP_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(12.))
            .pr(px(4.))
            .overflow_hidden()
            .border_t_1()
            .text_size(px(12.))
            .text_color(t.text_3);
        let btn = |id: &'static str| {
            div()
                .id(id)
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(STRIP_H - 8.))
                .px(px(9.))
                .rounded(px(4.))
                .font_weight(FontWeight::SEMIBOLD)
                .cursor_pointer()
        };
        match (s.tone, s.lead) {
            (Tone::Blocked, Some(lead)) => base
                .bg(t.amber_soft)
                .border_color(t.amber.opacity(0.5))
                .text_color(t.text)
                .child(div().w(px(12.)).text_color(t.amber).child("▲"))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .flex()
                        .items_center()
                        .child(
                            label_t(format!("{} needs you", lead.name), t.text, 13., 0.08)
                                .font_weight(FontWeight::BOLD),
                        )
                        .when_some(lead.status_detail.clone(), |d, det| {
                            d.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(t.text_2)
                                    .font_family(super::text_input::mono_font())
                                    .child(format!(": {det}")),
                            )
                        })
                        .when(s.more > 0, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_color(t.text_3)
                                    .child(format!(" · +{} more", s.more)),
                            )
                        }),
                )
                .child(
                    btn("strip-jump")
                        .bg(t.amber)
                        .text_color(gpui::rgb(0x1a1204))
                        .on_click(cx.listener(|this, _, window, cx| this.next_blocked(window, cx)))
                        .child("Jump")
                        .child(
                            kbd("⌘J", t)
                                .bg(gpui::transparent_black())
                                .border_color(gpui::rgba(0x1a12044d))
                                .text_color(gpui::rgba(0x1a1204b3)),
                        ),
                ),
            (Tone::Done, Some(lead)) => {
                let id = lead.id.clone();
                base.bg(t.finish_soft)
                    .border_color(t.finish.opacity(0.4))
                    .text_color(t.text)
                    .child(div().w(px(12.)).text_color(t.finish).child("⚑"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .flex()
                            .child(
                                label_t(format!("{} finished", lead.name), t.text, 13., 0.08)
                                    .font_weight(FontWeight::BOLD),
                            )
                            .when(s.more > 0, |d| {
                                d.child(
                                    div()
                                        .text_color(t.text_3)
                                        .child(format!(" · +{} more", s.more)),
                                )
                            })
                            .when(s.working > 0 && bp != Breakpoint::Xs, |d| {
                                d.child(
                                    div()
                                        .text_color(t.text_3)
                                        .child(format!(" · ◐ {} working", s.working)),
                                )
                            }),
                    )
                    .child(
                        btn("strip-show")
                            .text_color(t.finish)
                            .border_1()
                            .border_color(t.finish.opacity(0.45))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_agent(&id, window, cx)
                            }))
                            .child("Show"),
                    )
            }
            _ => base
                .bg(t.surface)
                .border_color(t.line)
                .child(div().flex_1().min_w_0().truncate().flex().map(|d| {
                    if s.total == 0 {
                        d.child("No agents yet")
                    } else {
                        d.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(t.text_2)
                                .font_family(super::text_input::mono_font())
                                .child(format!("◐ {} working", s.working)),
                        )
                        .when(bp != Breakpoint::Xs, |d| d.child(" · Nothing needs you"))
                    }
                }))
                .when(!bp.compact(), |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .pr(px(8.))
                            .text_size(px(11.5))
                            .text_color(t.text_4)
                            .child(kbd("⌘K", t))
                            .child("commands"),
                    )
                }),
        }
    }

    pub(super) fn render_toasts(&mut self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .right(px(12.))
            .bottom(px(STRIP_H + 10.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .w(px(340.))
            .children(self.toasts.iter().map(|toast| {
                let (glyph, color) = match toast.tone {
                    ToastTone::Blocked => ("▲", t.amber),
                    ToastTone::Done => ("⚑", t.finish),
                    ToastTone::Error => ("✕", t.red),
                    ToastTone::Info => ("●", t.text_3),
                };
                let id = toast.id;
                let agent = toast.agent.clone();
                div()
                    .id(("toast", id as usize))
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .p(px(10.))
                    .rounded(RADIUS_LG)
                    .bg(t.raised)
                    .border_1()
                    .border_color(if toast.tone == ToastTone::Blocked {
                        t.amber.opacity(0.5)
                    } else {
                        t.line_strong
                    })
                    .shadow_lg()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toasts.retain(|t| t.id != id);
                        if let Some(a) = &agent {
                            this.show_agent(a, window, cx);
                        }
                        cx.notify();
                    }))
                    .child(div().text_color(color).child(glyph))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                label_t(toast.title.clone(), t.text, 13.5, 0.08)
                                    .font_weight(FontWeight::BOLD),
                            )
                            .when_some(toast.detail.clone(), |d, det| {
                                d.child(div().text_size(px(12.)).text_color(t.text_3).child(det))
                            }),
                    )
                    .child(
                        div()
                            .id(("toast-x", id as usize))
                            .px(px(4.))
                            .rounded(RADIUS)
                            .text_color(t.text_4)
                            .hover(|s| s.text_color(t.text))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toasts.retain(|t| t.id != id);
                                cx.notify();
                            }))
                            .child("✕"),
                    )
            }))
    }
}
