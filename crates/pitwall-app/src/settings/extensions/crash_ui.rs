//! An installed extension's card when the tile guard paused its tiles or
//! turned it off (`crate::tiles::guard`): what happened, in a box under
//! the card's top row, with Resume and Turn off (paused), and Go back to
//! the version before when the last update kept it: a crash that came
//! with an update can't be rolled back by the update itself (the process
//! was killed), so the way back is offered here.

use gpui::{div, prelude::*, px, AnyElement, Context, SharedString};
use pitwall_core::extensions::Manifest;

use super::super::view::SettingsView;
use super::super::widgets::Ui;
use crate::kit::BtnKind;
use crate::tiles::guard::{self, Note};

impl SettingsView {
    /// The note, when there is one.
    pub(super) fn crash_note(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<AnyElement> {
        let note = guard::note(&m.id, cx)?;
        let name = m.name.clone();
        let (text, version) = match &note {
            Note::Paused { version } => (format!("{} {} Resume them, or turn it off.", guard::paused_words(&name).0, guard::paused_words(&name).1), version),
            Note::TurnedOff { version } => (format!("Turned off: {} Turn it on again when you want to try it again.", guard::off_words(&name).0), version),
        };
        let text = match version {
            Some(v) => format!("{text} (It was version {v}.)"),
            None => text,
        };
        let mut buttons = div().flex().flex_wrap().items_center().gap(px(8.));
        if matches!(note, Note::Paused { .. }) {
            let id = m.id.clone();
            let sel = format!("extension-crash-resume-{}", m.id);
            buttons = buttons.child(
                crate::kit::text_button(SharedString::from(sel.clone()), "Resume", BtnKind::Small, false, &ui.t)
                    .debug_selector(move || sel)
                    .tooltip(crate::kit::tooltip("Start its tiles again"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let id = id.clone();
                        cx.defer(move |cx| guard::resume(&id, cx));
                    })),
            );
            let id = m.id.clone();
            let sel = format!("extension-crash-off-{}", m.id);
            buttons = buttons.child(
                crate::kit::text_button(SharedString::from(sel.clone()), format!("Turn {name} off"), BtnKind::GhostSm, false, &ui.t)
                    .debug_selector(move || sel)
                    .on_click(cx.listener(move |_, _, _, cx| guard::turn_off(&id, cx))),
            );
        }
        buttons = buttons.children(self.go_back_button_as(format!("extension-crash-back-{}", m.id), BtnKind::Small, ui, m, cx));
        let sel = format!("extension-crash-{}", m.id);
        Some(
            div()
                .id(SharedString::from(sel.clone()))
                .debug_selector(move || sel)
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(10.))
                .rounded(px(8.))
                .border_1()
                .border_color(ui.t.amber.opacity(0.5))
                .bg(ui.t.surface_2)
                .child(div().text_size(px(12.)).line_height(gpui::relative(1.45)).text_color(ui.t.text).child(text))
                .child(buttons)
                .into_any_element(),
        )
    }
}
