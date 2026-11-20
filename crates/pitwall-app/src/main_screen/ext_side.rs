//! An extension's section of the sidebar (docs/spec/extensions.md, draft):
//! below the projects, a heading shaped like a project's (chevron, the
//! extension's icon and name) that folds what is under it, kept folded in
//! `ui.json` `collapsed` as a project is. What an extension shows under its
//! heading isn't decided yet: a quiet line says so. Right-click turns it
//! off (Settings → General → Extensions turns it on again).

use gpui::{div, prelude::*, px, AnyElement, Context, MouseButton, MouseDownEvent, SharedString};

use pitwall_core::extensions::Manifest;

use super::menu::MenuItem;
use super::MainScreen;
use crate::kit::{chevron, icon, label_fit, tooltip, HoverText as _};
use crate::theme::Theme;

/// The line under an open extension heading, until extensions show
/// something of their own.
pub(super) const PLACEHOLDER: &str = "Nothing to show yet";

impl MainScreen {
    /// Extension `x`'s heading.
    pub(super) fn extension_head(&self, x: &Manifest, collapsed: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let key = crate::extensions::group_key(&x.id);
        let tip: SharedString = match &x.description {
            Some(d) => format!("{} (extension): {d}", x.name).into(),
            None => format!("{} (extension)", x.name).into(),
        };
        let (k, id) = (key.clone(), x.id.clone());
        div()
            .id(SharedString::from(format!("group-{key}")))
            .group("ext-head")
            .flex()
            .items_center()
            .pl(px(8.))
            .pr(px(6.))
            .pt(px(6.))
            .pb(px(2.))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    let id = id.clone();
                    let items = vec![MenuItem::new("", "Turn off", move |this, _, cx| this.update_ui(|s| s.set_extension(&id, false), cx))];
                    this.open_menu(e.position, items, window, cx);
                }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("toggle-{key}")))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(4.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .tooltip(tooltip(tip))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let k = k.clone();
                        this.update_ui(|s| s.toggle_collapsed(&k), cx);
                    }))
                    .child(chevron(SharedString::from(format!("chev-{key}")), !collapsed, 12., t.text_4))
                    .child(icon(crate::extensions::icon_of(x), 13., t.text_3))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .child(label_fit(x.name.clone(), t.text_3, 12., self.head_label_chars()).group_hover_text("ext-head", t.text_2, |s| s)),
                    ),
            )
            .into_any_element()
    }
}
