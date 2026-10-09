//! The bottom status strip (React `StatusStrip.tsx`): fixed height, calm /
//! amber "needs you" / mint "finished", so it never resizes the terminals.
//! And the toast stack (`Toasts.tsx`, `useToasts.ts`): max 4, one per agent,
//! blocked 9 s, error 8 s, others 5 s; click jumps to the agent.

use crate::kit::Ellipsis as _;
use std::time::Duration;

use crate::kit::HoverText;
use gpui::{div, prelude::*, px, Context, FontWeight, IntoElement, SharedString, Task};

use super::model::{strip, Tone};
use crate::kit::{kbd, label_t};
use super::{Breakpoint, MainScreen};
use crate::theme::{Theme, RADIUS_LG};

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
    /// A click opens Settings (the Full Disk Access hint).
    pub opens_settings: bool,
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
            opens_settings: false,
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
        // `.strip-btn`: pops when pressed (`skin`: its surface).
        let btn = |id: &'static str, skin: crate::kit::motion::Skin| {
            let eid = gpui::ElementId::Name(id.into());
            crate::kit::motion::pressable(div().id(eid.clone()), &eid, false, skin)
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(STRIP_H - 8.))
                .px(px(9.))
                .rounded(px(4.))
                .font_weight(crate::kit::WEIGHT_650)
                .cursor_pointer()
        };
        // Background (left, right), border and text colour per tone; they
        // ease over 0.16 s when the tone changes (`.status-strip`).
        let colors = match (s.tone, s.lead.is_some()) {
            (Tone::Blocked, true) => vec![
                t.surface.blend(t.amber.opacity(0.2)),
                t.surface.blend(t.amber_soft),
                t.amber.opacity(0.5),
                t.text,
            ],
            (Tone::Done, true) => vec![
                t.surface.blend(t.finish.opacity(0.14)),
                t.surface.blend(t.finish_soft),
                t.finish.opacity(0.4),
                t.text,
            ],
            _ => vec![t.surface, t.surface, t.line, t.text_3],
        };
        let el = match (s.tone, s.lead) {
            (Tone::Blocked, Some(lead)) => base
                .child(
                    crate::kit::text_glow(div().text_color(t.amber).child("▲"), t.amber_glow, 10.)
                        .w(px(12.))
                        .justify_center(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .ellipsis()
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
                                    .ellipsis()
                                    .text_size(px(11.5))
                                    .text_color(t.text_2)
                                    .font_family(crate::kit::MONO_FONT)
                                    .child(crate::kit::one_line(format!(": {det}"))),
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
                    btn("strip-jump", crate::kit::motion::Skin::new(Some(t.amber), None, px(4.)))
                        .when(!crate::kit::motion::pressing(&"strip-jump".into()), |d| d.bg(t.amber))
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
                base.child(div().w(px(12.)).text_color(t.finish).child("⚑"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .ellipsis()
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
                        btn(
                            "strip-show",
                            crate::kit::motion::Skin::new(None, Some(t.finish.opacity(0.45)), px(4.)),
                        )
                            .text_color(t.finish)
                            .border_1()
                            .when(!crate::kit::motion::pressing(&"strip-show".into()), |d| {
                                d.border_color(t.finish.opacity(0.45))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_agent(&id, window, cx)
                            }))
                            .child("Show"),
                    )
            }
            _ => base
                .child(div().flex_1().min_w_0().ellipsis().flex().map(|d| {
                    if s.total == 0 {
                        d.child(crate::kit::one_line("No agents yet"))
                    } else {
                        d.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(t.text_2)
                                .font_family(crate::kit::MONO_FONT)
                                .child(format!("◐ {} working", s.working)),
                        )
                        .when(bp != Breakpoint::Xs, |d| d.child(" · Nothing needs you"))
                    }
                }))
                .when(!bp.compact(), |d| {
                    d.child(
                        div()
                            .id("strip-keys")
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .pr(px(8.))
                            .text_size(px(11.5))
                            .text_color(t.text_4)
                            .cursor_pointer()
                            .hover_text(t.text_3, |s| s)
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(super::ScreenEvent::OpenPalette)
                            }))
                            .child(kbd("⌘K", t))
                            .child("commands"),
                    )
                }),
        };
        crate::kit::motion::tween_colors(
            "strip-tone",
            colors,
            crate::kit::motion::STRIP,
            crate::kit::motion::Ease::EaseOut,
            move |c| {
                el.bg(gpui::linear_gradient(
                    90.,
                    gpui::linear_color_stop(c[0], 0.),
                    gpui::linear_color_stop(c[1], 0.6),
                ))
                .border_color(c[2])
                .text_color(c[3])
                .into_any_element()
            },
        )
    }

    pub(super) fn render_toasts(&mut self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // `.toasts`: 320 px, 14 px from the right, 12 px above the strip.
        div()
            .absolute()
            .right(px(14.))
            .bottom(px(STRIP_H + 12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .w(px(320.))
            .children(self.toasts.iter().map(|toast| {
                let (glyph, color) = match toast.tone {
                    ToastTone::Blocked => ("▲", t.amber),
                    ToastTone::Done => ("⚑", t.flag),
                    ToastTone::Error => ("✕", t.red),
                    ToastTone::Info => ("●", t.text_3),
                };
                let id = toast.id;
                let agent = toast.agent.clone();
                let opens_settings = toast.opens_settings;
                let blocked = toast.tone == ToastTone::Blocked;
                let mut shadow = t.drop_shadow();
                if blocked {
                    shadow.extend(t.amber_glow(22., -6.));
                }
                // `.toast`: padding 10 8 10 12 around a 1 px ring (the
                // border here, so 1 px less padding).
                let el = div()
                    .id(("toast", id as usize))
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .pt(px(9.))
                    .pb(px(9.))
                    .pl(px(11.))
                    .pr(px(7.))
                    .rounded(RADIUS_LG)
                    .bg(t.raised)
                    .border_1()
                    .border_color(if blocked {
                        t.amber.opacity(0.55)
                    } else {
                        t.line_strong
                    })
                    .shadow(shadow)
                    .child(
                        div()
                            .flex_none()
                            .w(px(14.))
                            .flex()
                            .justify_center()
                            .text_color(color)
                            .map(|d| {
                                if toast.tone == ToastTone::Info {
                                    d.mt(px(3.)).text_size(px(9.))
                                } else {
                                    d.mt(px(1.))
                                }
                            })
                            .child(glyph),
                    )
                    .child(
                        div()
                            .id(("toast-body", id as usize))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.toasts.retain(|t| t.id != id);
                                if let Some(a) = &agent {
                                    this.show_agent(a, window, cx);
                                }
                                if opens_settings {
                                    window.dispatch_action(Box::new(crate::menu::OpenSettings), cx);
                                }
                                cx.notify();
                            }))
                            .child(
                                label_t(toast.title.clone(), t.text, 13.5, 0.08)
                                    .font_weight(FontWeight::BOLD),
                            )
                            .when_some(toast.detail.clone(), |d, det| {
                                d.child(div().text_size(px(12.)).text_color(t.text_2).child(det))
                            }),
                    )
                    .child(
                        crate::kit::glyph_btn(("toast-x", id as usize), "✕", t)
                            .size(px(22.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toasts.retain(|t| t.id != id);
                                cx.notify();
                            })),
                    );
                crate::kit::motion::enter(("toast-in", id as usize), crate::kit::motion::Fx::TOAST, el)
            }))
    }
}

/// A macOS privacy refusal in an error's text (`isAccessError`:
/// /operation not permitted|\bEPERM\b|os error 1\b/i).
pub fn is_access_error(text: &str) -> bool {
    let low = text.to_lowercase();
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    // `needle` with a word boundary after it (and before it when it starts
    // with a letter).
    let bounded = |needle: &str, before: bool| {
        low.match_indices(needle).any(|(i, m)| {
            (!before || !word(low[..i].chars().next_back()))
                && !word(low[i + m.len()..].chars().next())
        })
    };
    low.contains("operation not permitted") || bounded("eperm", true) || bounded("os error 1", false)
}

#[cfg(test)]
mod access_tests {
    use super::is_access_error;

    #[test]
    fn privacy_refusals_are_recognised() {
        assert!(is_access_error("git: Operation not permitted"));
        assert!(is_access_error("read failed: EPERM"));
        assert!(is_access_error("Permission denied (os error 1)"));
        assert!(!is_access_error("No such file (os error 13)"));
        assert!(!is_access_error("EPERMISSIVE"));
        assert!(!is_access_error("network down"));
    }
}
