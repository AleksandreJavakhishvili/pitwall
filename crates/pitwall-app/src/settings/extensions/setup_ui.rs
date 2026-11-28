//! The Setup view on an extension's page (docs/spec/tile-sdk/ui.md
//! §3.12): the view its manifest names (`contributes.setup.view`), drawn
//! by its own component, between the page's header and Pitwall's parts
//! (permissions, Activity, Log), which stay Pitwall's so a Setup view
//! can't hide or restyle them.
//!
//! It is an ordinary view of the extension's component ([`TileView`]),
//! held while its page shows and let go when the page closes or another
//! extension's opens: its height is its content's, up to
//! [`SETUP_MAX_HEIGHT`], then it scrolls inside; theme and focus as in a
//! tile. In a frame with the extension's icon and name, as a tile's
//! header. Before its first frame, or while the extension is off, a
//! skeleton; when its component stopped, "Setup stopped" with Restart.
//! A card in the Installed list (not its page) says where its setup is.

use gpui::{div, prelude::*, px, Context, Div, Entity, SharedString};
use pitwall_core::extensions::Manifest;

use super::super::view::SettingsView;
use super::super::widgets::Ui;
use crate::kit::BtnKind;
use crate::tiles::{State, TileView};

/// The most a Setup view grows to on its page; taller, it scrolls inside.
pub const SETUP_MAX_HEIGHT: f32 = 560.;

/// The Setup view held for the page shown: (extension, its view).
pub type Held = Option<(String, Entity<TileView>)>;

impl SettingsView {
    /// The Setup view of extension `id` (view `view`), made when the page
    /// shows it; another extension's is let go.
    fn setup_view_of(&mut self, id: &str, view: &str, cx: &mut Context<Self>) -> Entity<TileView> {
        if let Some((held, v)) = &self.exts.setup_view {
            if held == id && v.read(cx).tile() == view {
                return v.clone();
            }
        }
        let (e, v) = (id.to_string(), view.to_string());
        let made = cx.new(|cx| TileView::new(&e, &v, cx));
        self.exts.setup_view = Some((id.to_string(), made.clone()));
        made
    }

    /// Let go of the Setup view held, unless it is extension `keep`'s
    /// (its page closed, or another's opened).
    pub(super) fn let_go_setup(&mut self, keep: Option<&str>) {
        if self.exts.setup_view.as_ref().is_some_and(|(id, _)| Some(id.as_str()) != keep) {
            self.exts.setup_view = None;
        }
    }

    /// The Setup view held now, by extension (tests).
    pub fn setup_view_shown(&self) -> Option<(String, Entity<TileView>)> {
        self.exts.setup_view.clone()
    }

    /// Extension `m`'s Setup view on its page, in its frame; `None` when
    /// its manifest names none.
    pub(super) fn setup_frame(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<Div> {
        let view = m.contributes.setup.as_ref()?.view.clone();
        let on = crate::ui_state::get(cx).extension_on(&m.id);
        let body = if on {
            let v = self.setup_view_of(&m.id, &view, cx);
            v.update(cx, |v, cx| v.set_shown(true, cx));
            let running = crate::tiles::try_tiles(cx).and_then(|t| t.read(cx).get(&m.id).map(|r| (r.state.clone(), r.frames.contains_key(&view))));
            match running {
                Some((State::Stopped(why), _)) => self.setup_stopped(ui, m, &crate::tiles::stop_words(&why), cx),
                Some((State::Running, true)) => div()
                    .id(SharedString::from(format!("extension-setup-scroll-{}", m.id)))
                    .debug_selector(|| "extension-setup-view".into())
                    .max_h(px(SETUP_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .child(div().flex().flex_col().child(v)),
                _ => skeleton(ui, None),
            }
        } else {
            self.let_go_setup(None);
            skeleton(ui, Some(format!("Turn it on to set it up. While it is off, nothing of {} runs.", m.name)))
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(7.))
            .h(px(32.))
            .px(px(10.))
            .border_b_1()
            .border_color(ui.t.line)
            .bg(ui.t.surface_2)
            .child(crate::kit::icon(crate::extensions::icon_of(m), 13., ui.t.text_2))
            .child(div().text_size(px(12.5)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(m.name.clone()))
            .child(div().text_size(px(12.)).text_color(ui.t.text_3).child("Setup"));
        let sel = format!("extension-setup-{}", m.id);
        Some(div().debug_selector(move || sel).flex().flex_col().rounded(px(8.)).border_1().border_color(ui.t.line).overflow_hidden().bg(ui.t.bg).child(head).child(body))
    }

    /// Its component stopped: why, and Restart.
    fn setup_stopped(&mut self, ui: &Ui, m: &Manifest, why: &str, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let id = m.id.clone();
        let restart = super::page::btn(format!("extension-setup-restart-{}", m.id), "Restart", BtnKind::Small, false, ui).on_click(cx.listener(move |_, _, _, cx| {
            let id = id.clone();
            crate::tiles::tiles(cx).update(cx, |t, cx| t.restart(&id, cx));
        }));
        div()
            .id(SharedString::from(format!("extension-setup-stopped-{}", m.id)))
            .debug_selector(|| "extension-setup-stopped".into())
            .flex()
            .items_start()
            .gap(px(10.))
            .p(px(14.))
            .child(div().mt(px(2.)).child(crate::kit::icon("alert", 14., ui.t.amber)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(13.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child("Setup stopped"))
                    .child(crate::kit::hint(format!("{} Restart it, or see its Log below.", why.trim_end()), &ui.t)),
            )
            .child(restart)
    }

    /// On a card in the Installed list: its setup is on its page.
    pub(super) fn setup_on_page(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let id = m.id.clone();
        let open = super::page::btn(format!("extension-setup-open-{}", m.id), "Open its page", BtnKind::Small, false, ui).on_click(cx.listener(move |v, _, _, cx| v.show_extension_detail(Some(&id), cx)));
        div().flex().items_center().gap(px(10.)).child(div().flex_1().child(crate::kit::hint(format!("{} is set up on its page, in its own Setup view.", m.name), &ui.t))).child(open)
    }
}

/// Placeholder lines while the Setup view isn't drawn yet, with a line
/// under them when it says why.
fn skeleton(ui: &Ui, caption: Option<String>) -> gpui::Stateful<Div> {
    let bar = |w: f32| crate::kit::feedback::skeleton_bar(10., &ui.t).w(gpui::relative(w));
    div()
        .id("extension-setup-skeleton")
        .debug_selector(|| "extension-setup-skeleton".into())
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(14.))
        .child(bar(0.55))
        .child(bar(0.8))
        .child(bar(0.4))
        .children(caption.map(|c| div().pt(px(4.)).child(crate::kit::hint(c, &ui.t))))
}
