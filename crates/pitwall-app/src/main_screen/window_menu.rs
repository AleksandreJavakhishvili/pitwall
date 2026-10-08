//! A space tab's right-click menu: move the space to a new window, back to
//! main or to another open window (`crate::windows`).

use gpui::{Context, MouseDownEvent, Window};

use super::menu::MenuItem;
use super::MainScreen;
use crate::windows::{self, file::display_name, Target};

impl MainScreen {
    pub(super) fn space_window_menu(
        &mut self,
        space: &str,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = space.to_string();
        let mut items = vec![MenuItem::new("window", "Move to new window", {
            let id = id.clone();
            move |this, _, cx| this.move_space(id.clone(), cx)
        })];
        for label in windows::labels(cx).into_iter().filter(|l| *l != self.label) {
            let (id, from) = (id.clone(), self.label.clone());
            let text = format!("Move to {}", display_name(&label).to_lowercase());
            items.push(MenuItem::new("window", &text, move |_, _, cx| {
                let (id, from, to) = (id.clone(), from.clone(), label.clone());
                cx.defer(move |cx| windows::move_space(&id, &from, Target::Window(to), cx));
            }));
        }
        self.open_menu(e.position, items, window, cx);
    }
}
