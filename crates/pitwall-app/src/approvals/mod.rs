//! Pitwall's approval dialog (Tauri: `src/components/ApprovalDialog.tsx`,
//! `src-tauri/src/server.rs`, `commands/cli.rs`).
//!
//! A request from the `pitwall` CLI that changes something outside Pitwall
//! (an agw session, say) waits in [`Approvals`] (120 s, `host.rs`). The
//! pending list reaches [`AgentStore::approvals`] through the bridge; this
//! dialog shows the first one in Pitwall's windows: "<requester> wants to
//! <summary>.", the details, who asked, the "remember" checkbox when
//! allowed, a countdown to the automatic denial and how many more wait.
//! Allow / Deny answer it; ✕, Esc and a click outside deny. Only this
//! process answers (in-process, never over the socket), so an agent can't
//! approve its own request. A new request after none bounces the Dock /
//! flashes the taskbar.

pub mod text;

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    div, prelude::*, px, AnyView, App, Context, Entity, FocusHandle, FontWeight, Global,
    IntoElement, Render, SharedString, Subscription, Task, Window,
};

use pitwall_daemon::Approvals;
use pitwall_proto::{ApprovalAnswer, ApprovalView};

use crate::agents::AgentStore;
use crate::kit;
use crate::menu::Dismiss;
use crate::theme::{self, Mode, Theme};

/// Who takes the user's answer: the in-process [`Approvals`] (tests use a
/// recorder).
pub type Answerer = Arc<dyn Fn(&ApprovalAnswer) -> Result<(), String>>;

/// The dialog's state, shared by every window.
pub struct ApprovalDialog {
    store: Entity<AgentStore>,
    answer: Answerer,
    /// The request the checkbox and error belong to.
    current: Option<String>,
    remember: bool,
    error: Option<String>,
    /// Unix ms, for the countdown (advanced by `tick`).
    now: u64,
    focus: FocusHandle,
    tick: Option<Task<()>>,
    _store: Subscription,
}

struct Global_(Entity<ApprovalDialog>);
impl Global for Global_ {}

/// Set up the dialog for `store`'s approvals, answered by `approvals`.
pub fn init(store: &Entity<AgentStore>, approvals: Arc<Approvals>, cx: &mut App) {
    let answer: Answerer = Arc::new(move |a: &ApprovalAnswer| approvals.answer(a));
    let dialog = cx.new(|cx| ApprovalDialog::new(store.clone(), answer, cx));
    cx.set_global(Global_(dialog));
}

/// The dialog, for a window to draw over everything, while a request
/// waits. Takes the keyboard in the active window so Esc denies.
pub fn overlay(window: &mut Window, cx: &mut App) -> Option<AnyView> {
    let dialog = cx.try_global::<Global_>()?.0.clone();
    dialog.read(cx).pending(cx)?;
    let focus = dialog.read(cx).focus.clone();
    if window.is_window_active() && !focus.contains_focused(window, cx) {
        window.focus(&focus);
    }
    Some(dialog.into())
}

impl ApprovalDialog {
    pub fn new(store: Entity<AgentStore>, answer: Answerer, cx: &mut Context<Self>) -> Self {
        let mut was_empty = store.read(cx).approvals.is_empty();
        let sub = cx.observe(&store, move |this, store, cx| {
            let empty = store.read(cx).approvals.is_empty();
            // A new request after none: make sure the user notices.
            if was_empty && !empty {
                crate::platform::request_attention(cx);
            }
            was_empty = empty;
            this.sync(cx);
        });
        let mut this = ApprovalDialog {
            store,
            answer,
            current: None,
            remember: false,
            error: None,
            now: text::now_ms(),
            focus: cx.focus_handle(),
            tick: None,
            _store: sub,
        };
        this.sync(cx);
        this
    }

    /// The request shown and how many wait (itself included).
    fn pending(&self, cx: &App) -> Option<(ApprovalView, usize)> {
        let list = &self.store.read(cx).approvals;
        list.first().map(|a| (a.clone(), list.len()))
    }

    /// Follow the list: a new first request starts unticked with no error;
    /// the countdown ticks only while something waits.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let first = self.pending(cx).map(|(a, _)| a.id);
        if first != self.current {
            self.current = first.clone();
            self.remember = false;
            self.error = None;
        }
        match (&first, &self.tick) {
            (Some(_), None) => {
                self.now = text::now_ms();
                self.tick = Some(cx.spawn(async move |this, cx| loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    let alive = this.update(cx, |this, cx| {
                        this.now = text::now_ms();
                        cx.notify();
                    });
                    if alive.is_err() {
                        break;
                    }
                }));
            }
            (None, Some(_)) => self.tick = None,
            _ => {}
        }
        cx.notify();
    }

    /// Answer the request shown (Deny also for ✕, Esc and a click outside).
    pub fn answer(&mut self, allow: bool, cx: &mut Context<Self>) {
        let Some((a, _)) = self.pending(cx) else {
            return;
        };
        let reply = ApprovalAnswer {
            id: a.id,
            allow,
            remember: allow && self.remember && a.rememberable,
        };
        if let Err(e) = (self.answer)(&reply) {
            self.error = Some(e);
        }
        cx.notify();
    }

    pub fn set_remember(&mut self, on: bool, cx: &mut Context<Self>) {
        self.remember = on;
        cx.notify();
    }

    fn head(t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .pl(px(18.))
            .pr(px(12.))
            .pt(px(14.))
            .pb(px(6.))
            .flex()
            .items_center()
            .justify_between()
            .bg(t.amber_soft)
            .border_b_1()
            .border_color(t.amber.opacity(0.35))
            .child(kit::label_t("Approval needed", t.amber, 15., 0.08))
            .child(
                kit::icon_btn("approval-close", "x", "Close (esc)", false, t)
                    .on_click(cx.listener(|this, _, _, cx| this.answer(false, cx))),
            )
    }

    fn checkbox(&self, a: &ApprovalView, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let on = self.remember;
        div()
            .id("approval-remember")
            .flex()
            .items_start()
            .gap(px(10.))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.set_remember(!on, cx)))
            .child(kit::check(on, t).mt(px(3.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .child(text::remember_label(a))
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.text_3)
                            .child("Until Pitwall quits."),
                    ),
            )
    }
}

impl Render for ApprovalDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let Some((a, waiting)) = self.pending(cx) else {
            return div().id("approval-none").into_any_element();
        };
        let backdrop = t.scrim();
        let ask = gpui::StyledText::new(format!("{} wants to {}.", a.requester.name, a.summary))
            .with_highlights([(
                0..a.requester.name.len(),
                gpui::HighlightStyle {
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                },
            )]);
        let body = div()
            .px(px(18.))
            .pt(px(8.))
            .pb(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().text_size(px(14.)).line_height(px(21.)).child(ask))
            .when(!a.details.is_empty(), |d| {
                d.child(
                    div()
                        .pl(px(6.))
                        .flex()
                        .flex_col()
                        .text_size(px(12.5))
                        .line_height(px(20.))
                        .text_color(t.text_2)
                        .children(a.details.iter().map(|line| {
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(div().flex_none().child("•"))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(SharedString::from(line.clone())),
                                )
                        })),
                )
            })
            .child(div().text_xs().text_color(t.text_3).child(format!(
                "Asked by {}. Pitwall decides who asked; the caller can't approve this itself.",
                text::requester_line(&a)
            )))
            .when(a.rememberable, |d| d.child(self.checkbox(&a, &t, cx)))
            .when_some(self.error.clone(), |d, e| {
                d.child(div().text_xs().text_color(t.red).child(e))
            })
            .child(
                div()
                    .mt(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(t.text_3)
                            .child(text::footer(&a, waiting, self.now)),
                    )
                    .child(
                        kit::text_button("approval-deny", "Deny", kit::BtnKind::Ghost, false, &t)
                            .on_click(cx.listener(|this, _, _, cx| this.answer(false, cx))),
                    )
                    .child(
                        button("approval-allow", "Allow", &t, true)
                            .on_click(cx.listener(|this, _, _, cx| this.answer(true, cx))),
                    ),
            );
        let panel_motion = theme::motion_on(cx);
        let backdrop_el = div()
            .id("approval-backdrop")
            .key_context("ApprovalDialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.answer(false, cx)))
            .absolute()
            .inset_0()
            .p(px(24.))
            .bg(backdrop)
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| this.answer(false, cx)));
        let panel = div()
                    .id("approval")
                    .w(px(500.))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .bg(t.raised)
                    .text_color(t.text)
                    .border_1()
                    .border_color(t.amber.opacity(0.55))
                    .shadow(vec![
                        gpui::BoxShadow {
                            color: gpui::black().opacity(if t.mode == Mode::Dark {
                                0.55
                            } else {
                                0.18
                            }),
                            offset: gpui::point(px(0.), px(18.)),
                            blur_radius: px(50.),
                            spread_radius: px(0.),
                        },
                        gpui::BoxShadow {
                            color: t.amber.opacity(0.3),
                            offset: gpui::point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(1.),
                        },
                        gpui::BoxShadow {
                            color: t
                                .amber
                                .opacity(if t.mode == Mode::Dark { 0.3 } else { 0.25 }),
                            offset: gpui::point(px(0.), px(0.)),
                            blur_radius: px(14.),
                            spread_radius: px(-6.),
                        },
                    ])
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(Self::head(&t, cx))
                    .child(body);
        let panel = if panel_motion {
            kit::motion::enter("approval-in", kit::motion::Fx::MODAL, panel).into_any_element()
        } else {
            panel.into_any_element()
        };
        kit::scrim_in("approval-fade", backdrop_el.child(panel))
    }
}

/// The amber `.approval-allow` button (a primary button in amber).
fn button(
    id: &'static str,
    label: &'static str,
    t: &Theme,
    _primary: bool,
) -> gpui::Stateful<gpui::Div> {
    kit::accent_button(id, t.amber, gpui::rgb(0x1a1200).into(), t).child(label)
}

#[cfg(test)]
mod tests {
    use super::text::tests::view;
    use super::*;
    use crate::bridge::{AppEvent, Bridge};
    use gpui::{AppContext, TestAppContext, VisualTestContext};
    use pitwall_daemon::approvals::{Ask, Decision};
    use pitwall_proto::{Requester, RequesterKind, Risk};
    use std::sync::Mutex;

    fn recorder() -> (Answerer, Arc<Mutex<Vec<ApprovalAnswer>>>) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        (
            Arc::new(move |a: &ApprovalAnswer| {
                sink.lock().unwrap().push(a.clone());
                Ok(())
            }),
            got,
        )
    }

    #[gpui::test]
    fn answers_the_first_request_and_resets_for_the_next(cx: &mut TestAppContext) {
        let store = cx.new(|_| AgentStore::new(vec![]));
        let (answer, got) = recorder();
        let dialog = cx.new(|cx| ApprovalDialog::new(store.clone(), answer, cx));
        store.update(cx, |s, cx| {
            s.approvals = vec![
                view("a", RequesterKind::Agent),
                view("b", RequesterKind::Outside),
            ];
            cx.notify();
        });
        cx.run_until_parked();
        dialog.update(cx, |d, cx| {
            d.set_remember(true, cx);
            d.answer(true, cx);
        });
        assert_eq!(
            got.lock().unwrap()[0],
            ApprovalAnswer {
                id: "a".into(),
                allow: true,
                remember: true
            }
        );
        // The next request starts unticked.
        store.update(cx, |s, cx| {
            s.approvals.remove(0);
            cx.notify();
        });
        cx.run_until_parked();
        dialog.update(cx, |d, cx| {
            assert!(!d.remember);
            d.answer(false, cx);
        });
        assert_eq!(
            got.lock().unwrap()[1],
            ApprovalAnswer {
                id: "b".into(),
                allow: false,
                remember: false
            }
        );
    }

    #[gpui::test]
    fn remember_needs_a_rememberable_request(cx: &mut TestAppContext) {
        let store = cx.new(|_| AgentStore::new(vec![]));
        let (answer, got) = recorder();
        let dialog = cx.new(|cx| ApprovalDialog::new(store.clone(), answer, cx));
        let mut high = view("h", RequesterKind::Agent);
        high.rememberable = false;
        store.update(cx, |s, cx| {
            s.approvals = vec![high];
            cx.notify();
        });
        cx.run_until_parked();
        dialog.update(cx, |d, cx| {
            d.set_remember(true, cx);
            d.answer(true, cx);
        });
        assert!(!got.lock().unwrap()[0].remember);
    }

    #[gpui::test]
    fn a_failed_answer_shows_its_error(cx: &mut TestAppContext) {
        let store = cx.new(|_| AgentStore::new(vec![]));
        let failing: Answerer = Arc::new(|_| Err("no pending approval a".into()));
        let dialog = cx.new(|cx| ApprovalDialog::new(store.clone(), failing, cx));
        store.update(cx, |s, cx| {
            s.approvals = vec![view("a", RequesterKind::Agent)];
            cx.notify();
        });
        cx.run_until_parked();
        dialog.update(cx, |d, cx| d.answer(true, cx));
        dialog.read_with(cx, |d, _| {
            assert_eq!(d.error.as_deref(), Some("no pending approval a"))
        });
    }

    /// The real flow: a CLI request waits in `Approvals`, reaches the store
    /// through the bridge, shows in a window, and Allow lets it through.
    #[gpui::test]
    fn a_waiting_request_is_allowed_from_the_window(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Theme::dark());
            cx.bind_keys(crate::menu::bindings());
        });
        let store = cx.new(|_| AgentStore::new(vec![]));
        let (bridge, rx) = Bridge::new();
        cx.update(|cx| AgentStore::listen(&store, rx, cx));
        let approvals = Approvals::new(Duration::from_secs(30));
        approvals.on_change(move |l| bridge.send(AppEvent::Approvals(l)));
        cx.update(|cx| init(&store, approvals.clone(), cx));

        let asking = approvals.clone();
        let waiter = std::thread::spawn(move || {
            asking.ask(Ask {
                action: "session.add".into(),
                summary: "start the session \"work\" on vm-1 (agw)".into(),
                details: vec!["Machine: vm-1".into()],
                requester: Requester {
                    kind: RequesterKind::Outside,
                    agent_id: None,
                    name: "A terminal outside Pitwall".into(),
                    pid: None,
                    process: None,
                },
                caller_key: "test".into(),
                risk: Risk::Low,
            })
        });
        while approvals.pending().is_empty() {
            std::thread::yield_now();
        }
        cx.run_until_parked();

        let window = cx.add_window(|_, _| Host);
        let mut vcx = VisualTestContext::from_window(*window, cx);
        vcx.run_until_parked();
        assert_eq!(store.read_with(&vcx, |s, _| s.approvals.len()), 1);
        let dialog = vcx.update(|_, cx| cx.global::<Global_>().0.clone());
        dialog.update(&mut vcx, |d, cx| d.answer(true, cx));
        assert_eq!(waiter.join().unwrap(), Decision::Allowed);
        vcx.run_until_parked();
        assert!(store.read_with(&vcx, |s, _| s.approvals.is_empty()));
    }

    /// A window that draws the overlay, as `MainView` does.
    struct Host;
    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().children(overlay(window, cx))
        }
    }
}
