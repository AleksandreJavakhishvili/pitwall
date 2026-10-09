//! The File menu where gpui draws no native menu bar (Windows; Tauri:
//! `menu::build_file_menu`): Settings… Ctrl+Shift+,, Close Window, Quit
//! Pitwall, Quit and Stop Agents. A small in-window dropdown, drawn
//! in-house; the title bar places [`AppMenu`] at its left edge. Linux has
//! no menu (`HostInfo::menu` = None): the palette has these commands.

use crate::kit::HoverText;
use gpui::{
    anchored, deferred, div, prelude::*, px, Action, Context, Corner, IntoElement, Render,
    SharedString, Window,
};

use crate::menu::{CloseWindow, OpenSettings, Quit, QuitAndStopAgents};
use crate::theme::{self, RADIUS, RADIUS_SM};

/// One row of the menu.
pub enum Entry {
    Item {
        label: &'static str,
        /// As this OS writes the shortcut ("Ctrl+Shift+,"), if any.
        keys: Option<&'static str>,
        action: Box<dyn Action>,
    },
    Separator,
}

/// The File menu's rows, in Tauri's order.
pub fn file_menu() -> Vec<Entry> {
    let item = |label, keys, action: Box<dyn Action>| Entry::Item {
        label,
        keys,
        action,
    };
    vec![
        item("Settings…", Some("Ctrl+Shift+,"), Box::new(OpenSettings)),
        Entry::Separator,
        item("Close Window", None, Box::new(CloseWindow)),
        Entry::Separator,
        item("Quit Pitwall", None, Box::new(Quit)),
        item("Quit and Stop Agents", None, Box::new(QuitAndStopAgents)),
    ]
}

/// Whether this desktop shows the in-window menu (`HostInfo::menu` = File).
pub fn shown() -> bool {
    cfg!(windows)
}

/// The "File" button and its dropdown.
#[derive(Default)]
pub struct AppMenu {
    open: bool,
}

impl AppMenu {
    pub fn new() -> AppMenu {
        AppMenu::default()
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        cx.notify();
    }

    fn run(&mut self, action: Box<dyn Action>, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        cx.notify();
        window.dispatch_action(action, cx);
    }
}

impl Render for AppMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let button = div()
            .id("app-menu-file")
            .h(px(22.))
            .px_2()
            .flex()
            .items_center()
            .rounded(RADIUS)
            .text_xs()
            .text_color(if self.open { t.text } else { t.text_2 })
            .when(self.open, |d| d.bg(t.surface_3))
            .hover_text(t.text, |s| s.bg(t.surface_3))
            .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
            .child("File");
        let mut root = div().relative().child(button);
        if self.open {
            let mut list = div()
                .id("app-menu-list")
                .occlude()
                .min_w(px(220.))
                .p_1()
                .rounded(RADIUS)
                .bg(t.raised)
                .border_1()
                .border_color(t.line_strong)
                .shadow_lg()
                .flex()
                .flex_col()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.open = false;
                    cx.notify();
                }));
            for (i, entry) in file_menu().into_iter().enumerate() {
                list = match entry {
                    Entry::Separator => list.child(div().my_1().h(px(1.)).bg(t.line)),
                    Entry::Item {
                        label,
                        keys,
                        action,
                    } => list.child(
                        div()
                            .id(("app-menu-item", i))
                            .h(px(26.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_4()
                            .rounded(RADIUS_SM)
                            .text_xs()
                            .text_color(t.text)
                            .hover_probed(|s| s.bg(t.surface_3))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run(action.boxed_clone(), window, cx)
                            }))
                            .child(div().flex_1().child(SharedString::from(label)))
                            .children(keys.map(|k| div().text_color(t.text_3).child(k))),
                    ),
                };
            }
            root = root.child(deferred(
                anchored()
                    .anchor(Corner::TopLeft)
                    .offset(gpui::point(px(0.), px(24.)))
                    .child(list),
            ));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_menu_matches_tauris() {
        let labels: Vec<_> = file_menu()
            .iter()
            .map(|e| match e {
                Entry::Item { label, .. } => *label,
                Entry::Separator => "—",
            })
            .collect();
        assert_eq!(
            labels,
            [
                "Settings…",
                "—",
                "Close Window",
                "—",
                "Quit Pitwall",
                "Quit and Stop Agents"
            ]
        );
        let Entry::Item { keys, action, .. } = &file_menu()[0] else {
            unreachable!()
        };
        assert_eq!(*keys, Some("Ctrl+Shift+,"));
        assert!(action.partial_eq(&OpenSettings));
    }
}
