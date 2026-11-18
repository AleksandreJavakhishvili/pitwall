//! Review's list of changes on the left, a [`crate::kit::side`] column as
//! the file viewer's tree: hidden and shown with the bar's sidebar button
//! or ⌘⇧E (the diff then has the whole width), its width dragged at its
//! edge (a double click: the default), both kept in `ui.json`
//! (`reviewList`). A Review too narrow for the list and a diff starts with
//! the list hidden.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, Context, Pixels, Size, Window};

use crate::kit::side::{Limits, SideState};

use super::ReviewView;

/// The list's widths.
pub const LIMITS: Limits = Limits { min: 200., max: 560., keep: 420., default: 300. };

/// The list's id (its edge's drags say it).
pub const LIST: &str = "rv-list";

/// What Review's list remembers on this screen.
#[derive(Default)]
pub(super) struct ListSide {
    /// Toggled here (`None`: as kept).
    pub(super) open: Option<bool>,
    /// The width while the edge is dragged.
    pub(super) drag_w: Option<f32>,
    /// Review's size, as last laid out.
    pub(super) measured: Rc<Cell<Option<Size<Pixels>>>>,
}

fn kept(cx: &App) -> SideState {
    cx.try_global::<crate::ui_state::UiHandle>()
        .map(|h| h.0.read(cx).state.review_list)
        .unwrap_or_default()
}

impl ReviewView {
    fn review_w(&self) -> Option<f32> {
        self.side.measured.get().map(|s| f32::from(s.width))
    }

    /// The list is shown.
    pub fn list_shown(&self, cx: &App) -> bool {
        LIMITS.shown(self.side.open, kept(cx), self.review_w())
    }

    /// The list's width now.
    pub fn list_width(&self, cx: &App) -> f32 {
        match self.side.drag_w {
            Some(w) => LIMITS.clamp(w, self.review_w()),
            None => LIMITS.width(kept(cx), self.review_w()),
        }
    }

    pub(super) fn list_dragging(&self) -> bool {
        self.side.drag_w.is_some()
    }

    /// Hide or show the list (and keep that). The keyboard, when it was in
    /// the list, goes to Review.
    pub fn toggle_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let on = !self.list_shown(cx);
        self.set_list(on, window, cx);
    }

    pub fn set_list(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !on && self.list_focus.contains_focused(window, cx) {
            window.focus(&self.focus, cx);
        }
        self.side.open = Some(on);
        crate::ui_state::update(cx, |s| s.review_list.open = Some(on));
        cx.notify();
    }

    pub(super) fn drag_list(&mut self, w: f32, cx: &mut Context<Self>) {
        self.side.drag_w = Some(LIMITS.clamp(w, self.review_w()));
        cx.notify();
    }

    pub(super) fn keep_list_width(&mut self, cx: &mut Context<Self>) {
        if let Some(w) = self.side.drag_w.take() {
            let w = LIMITS.clamp(w, self.review_w());
            crate::ui_state::update(cx, |s| s.review_list.width = Some(w));
            cx.notify();
        }
    }

    pub(super) fn reset_list_width(&mut self, cx: &mut Context<Self>) {
        self.side.drag_w = None;
        crate::ui_state::update(cx, |s| s.review_list.width = None);
        cx.notify();
    }
}
