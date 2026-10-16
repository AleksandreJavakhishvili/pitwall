//! The Ask panel (⌘⇧A, collaboration/ui.md §11, caller side): pick a
//! teammate's shared agent, then one thread per agent: my messages, the
//! replies, and the turn's status ("2nd in line", "waiting for alice",
//! "changes waiting for alice's OK"), plus "New conversation". A reply
//! that came with changes shows them summed up; applying them comes with
//! "Can change code" (not in this version).

use gpui::{div, prelude::*, px, AppContext, Context, Div, Entity, IntoElement, Subscription, WeakEntity, Window};
use serde_json::json;

use pitwall_proto::api::collab::capability_chip;
use pitwall_proto::collab::{AskFrom, AskStatus, AskView, DirectoryEntry};
use pitwall_proto::method;

use crate::kit::dialog::form_error;
use crate::kit::{chip, label_t, text_button, BtnKind, TextInput, MONO_FONT};
use crate::main_screen::MainScreen;
use crate::theme::{self, Theme, RADIUS};

/// A turn's status in the thread's words.
pub fn status_line(a: &AskView) -> String {
    if let Some(m) = &a.status_message {
        return m.clone();
    }
    match a.status {
        AskStatus::Queued => "queued".into(),
        AskStatus::Working => "working…".into(),
        AskStatus::WaitingForOwner => format!("waiting for {}", a.share.split('/').next().unwrap_or("the owner")),
        AskStatus::Done => String::new(),
        AskStatus::Failed => "no answer".into(),
        AskStatus::Canceled => "canceled".into(),
        AskStatus::Refused => "refused".into(),
    }
}

/// `alice/api-expert` for a directory entry.
pub fn address(e: &DirectoryEntry) -> String {
    format!("{}/{}", super::short(&e.owner), e.slug)
}

pub struct AskPanel {
    _screen: WeakEntity<MainScreen>,
    composer: Entity<TextInput>,
    /// The next message starts a new conversation.
    pub fresh: bool,
    /// Sent and not answered yet (the call waits for the turn).
    pub pending: Vec<(String, String)>,
    pub error: Option<String>,
    _subs: Vec<Subscription>,
}

impl AskPanel {
    pub fn new(screen: WeakEntity<MainScreen>, _window: &mut Window, cx: &mut Context<Self>) -> AskPanel {
        let model = super::model_or_new(cx);
        let composer = cx.new(|cx| TextInput::new(cx, "", "Ask… (⌘↵ to send)").multiline(3));
        let subs = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&composer, |this, _, e: &crate::kit::InputEvent, cx| {
                if matches!(e, crate::kit::InputEvent::Submit) {
                    this.send(cx);
                }
            }),
        ];
        AskPanel { _screen: screen, composer, fresh: false, pending: vec![], error: None, _subs: subs }
    }

    fn target(&self, cx: &gpui::App) -> Option<String> {
        super::model(cx).and_then(|m| m.read(cx).ask_target.clone())
    }

    fn pick(&mut self, share: String, cx: &mut Context<Self>) {
        if let Some(m) = super::model(cx) {
            m.update(cx, |s, cx| {
                s.ask_target = Some(share);
                cx.notify();
            });
        }
        self.fresh = false;
        self.error = None;
    }

    pub fn send(&mut self, cx: &mut Context<Self>) {
        let Some(share) = self.target(cx) else { return };
        let text = self.composer.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.composer.update(cx, |i, cx| i.set_text("", cx));
        self.pending.push((share.clone(), text.clone()));
        let new = std::mem::take(&mut self.fresh);
        self.error = None;
        cx.notify();
        let this = cx.entity().downgrade();
        super::call::<AskView>(method::ASK_SEND, json!({"share": share, "message": text, "new": new, "timeout": 600}), cx, move |r, cx| {
            let _ = this.update(cx, |p, cx| {
                p.pending.retain(|(s, t)| !(*s == share && *t == text));
                if let Err(e) = r {
                    p.error = Some(e.message);
                }
                cx.notify();
            });
        });
    }
}

fn bubble(mine: bool, t: &Theme) -> Div {
    div()
        .max_w(px(560.))
        .px(px(10.))
        .py(px(7.))
        .rounded(RADIUS)
        .text_size(px(13.))
        .when(mine, |d| d.bg(t.surface_3).text_color(t.text))
        .when(!mine, |d| d.bg(t.surface).border_1().border_color(t.line).text_color(t.text))
}

impl AskPanel {
    fn thread(&self, share: &str, t: &Theme, cx: &mut Context<Self>) -> Div {
        let snap = super::snapshot(cx);
        let turns: Vec<AskView> = snap.asks.iter().filter(|a| a.share == share).cloned().collect();
        let mut out = div().flex().flex_col().gap(px(10.));
        let mut conversation = String::new();
        for a in &turns {
            if !conversation.is_empty() && a.conversation != conversation {
                out = out.child(div().text_size(px(11.)).text_color(t.text_4).child("— new conversation —"));
            }
            conversation = a.conversation.clone();
            let mine = a.from == AskFrom::Me;
            out = out.child(div().flex().when(mine, |d| d.justify_end()).child(bubble(mine, t).child(a.text.clone())));
            let status = status_line(a);
            if let Some(reply) = &a.reply {
                let source = a.reply_source.map(|s| serde_json::to_value(s).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default());
                out = out.child(
                    div().flex().flex_col().gap(px(2.)).child(bubble(false, t).child(reply.clone())).when_some(source.filter(|s| s == "terminal"), |d, _| {
                        d.child(div().text_size(px(11.)).text_color(t.text_4).child("read from the terminal; may be rough"))
                    }),
                );
            }
            if let Some(ch) = &a.changes {
                let state = serde_json::to_value(ch.state).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
                out = out.child(
                    div()
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(RADIUS)
                        .border_1()
                        .border_color(t.line_strong)
                        .font_family(MONO_FONT)
                        .text_size(px(12.))
                        .child(format!("changes: {} files, +{} −{} ({state})", ch.files, ch.added, ch.removed)),
                );
            }
            if !status.is_empty() {
                let color = match a.status {
                    AskStatus::Refused | AskStatus::Failed => t.amber,
                    _ => t.text_3,
                };
                out = out.child(div().text_size(px(11.5)).text_color(color).child(status));
            }
        }
        for (s, text) in &self.pending {
            if s == share {
                out = out.child(div().flex().justify_end().child(bubble(true, t).opacity(0.7).child(text.clone()))).child(div().text_size(px(11.5)).text_color(t.text_3).child("sent · waiting for the reply…"));
            }
        }
        if turns.is_empty() && self.pending.iter().all(|(s, _)| s != share) {
            out = out.child(div().text_size(px(12.5)).text_color(t.text_3).child("No messages yet. One conversation per shared agent: it remembers what you asked before."));
        }
        out
    }
}

impl gpui::Render for AskPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let snap = super::snapshot(cx);
        let entries = snap.directory();
        let target = self.target(cx).or_else(|| entries.first().map(address));
        let mut picker = div().flex().flex_col().gap(px(2.)).child(div().px(px(8.)).pb(px(6.)).child(label_t("SHARED AGENTS", t.text_3, 11., 0.08)));
        if entries.is_empty() {
            picker = picker.child(div().px(px(8.)).text_size(px(12.)).text_color(t.text_3).child("Nothing shared with you yet (People → Teams)."));
        }
        for (i, e) in entries.iter().enumerate() {
            let addr = address(e);
            let on = target.as_deref() == Some(addr.as_str());
            let a2 = addr.clone();
            let hover = t.surface_2;
            picker = picker.child(
                div()
                    .id(("ask-pick", i))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(RADIUS)
                    .cursor_pointer()
                    .when(on, |d| d.bg(t.surface_3))
                    .when(!on, |d| d.hover(move |s| s.bg(hover)))
                    .when(!e.online, |d| d.opacity(0.55))
                    .on_click(cx.listener(move |p, _, _, cx| p.pick(a2.clone(), cx)))
                    .child(div().flex_none().size(px(7.)).rounded_full().bg(if e.online { t.green } else { t.text_4 }))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_size(px(12.5)).child(addr))
                    .child(chip(capability_chip(e.capability), &t)),
            );
        }
        let off = snap.settings.as_ref().is_some_and(|s| !s.settings.use_teams);
        let right = match target.clone() {
            None => div().flex_1().p(px(20.)).text_color(t.text_3).child("Pick a shared agent on the left."),
            Some(share) => {
                let entry = entries.iter().find(|e| address(e) == share).cloned();
                let head = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .h(px(44.))
                    .border_b_1()
                    .border_color(t.line)
                    .child(div().text_color(t.text).child(share.clone()))
                    .children(entry.as_ref().map(|e| chip(capability_chip(e.capability), &t)))
                    .children(entry.as_ref().and_then(|e| e.left).map(|n| div().text_size(px(12.)).text_color(t.text_3).child(format!("{n} left today"))))
                    .child(div().flex_1())
                    .child(
                        text_button("new-conversation", if self.fresh { "Next message: new conversation ✓" } else { "New conversation" }, BtnKind::GhostSm, false, &t)
                            .on_click(cx.listener(|p, _, _, cx| {
                                p.fresh = !p.fresh;
                                cx.notify();
                            })),
                    );
                let thread = self.thread(&share, &t, cx);
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(head)
                    .child(div().id("ask-thread").flex_1().min_h_0().overflow_y_scroll().p(px(16.)).child(thread))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .p(px(12.))
                            .border_t_1()
                            .border_color(t.line)
                            .when(off, |d| d.child(div().text_size(px(12.)).text_color(t.amber).child("Collaboration is off: Settings → Collaboration → Use teams.")))
                            .when_some(self.error.clone(), |d, e| d.child(form_error(e, &t)))
                            .child(self.composer.clone())
                            .child(
                                div().flex().items_center().child(div().flex_1().text_size(px(11.5)).text_color(t.text_4).child("Sent as you, over the network; the owner sees every message.")).child(
                                    text_button("ask-send", "Send", BtnKind::Primary, false, &t).on_click(cx.listener(|p, _, _, cx| p.send(cx))),
                                ),
                            ),
                    )
            }
        };
        div()
            .id("ask")
            .key_context("PwAsk")
            .size_full()
            .flex()
            .child(div().id("ask-picker").w(px(240.)).flex_none().h_full().overflow_y_scroll().p(px(10.)).border_r_1().border_color(t.line).child(picker))
            .child(right)
    }
}
