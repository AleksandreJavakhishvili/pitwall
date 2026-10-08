//! Settings' and onboarding's own layout pieces, drawn with the kit: the
//! confirm box, bulleted and numbered lists. [`Ui`] carries the theme the
//! dialog draws with (`Theme::float`).

use gpui::{div, prelude::*, px, Div, SharedString};

use crate::theme::{Theme, RADIUS};

/// What the settings views draw with.
#[derive(Clone)]
pub struct Ui {
    pub t: Theme,
}

impl Ui {
    pub fn new(t: Theme) -> Ui {
        Ui { t }
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
}
