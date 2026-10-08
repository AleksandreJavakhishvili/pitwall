//! The main window: the main screen (`crate::main_screen`) with the
//! explorer, Settings / About / onboarding (`settings::Layer`) and the CLI
//! approval dialog over it. When the app refused to host an engine
//! (`home::Refusal`), the window says why instead.

use gpui::{
    div, prelude::*, px, Context, Entity, FocusHandle, IntoElement, Render, Subscription, Task,
    Window,
};

use crate::agents::AgentStore;
use crate::home::Refusal;
use crate::theme::{self, Theme, RADIUS_LG};
use crate::window_state;

/// What the window shows.
#[derive(Clone)]
pub enum Content {
    Live { store: Entity<AgentStore> },
    Refused(Refusal),
}

pub struct MainView {
    content: Content,
    /// The read-only code explorer (Files panel, viewer, ⌘P, ⇧⌘F).
    explorer: Option<Entity<crate::explorer::Explorer>>,
    /// Settings, About and onboarding (settings::Layer).
    layer: Entity<crate::settings::Layer>,
    focus: FocusHandle,
    /// Where the window's bounds are saved (`None`: refused, nothing saved).
    root: Option<std::path::PathBuf>,
    save_bounds: Option<Task<()>>,
    /// The main screen (`crate::main_screen`) when the engine is hosted.
    pub screen: Option<Entity<crate::main_screen::MainScreen>>,
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    pub fn new(
        content: Content,
        root: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subs = vec![
            // Light or dark follows the system unless Settings → Appearance
            // forces one.
            cx.observe_window_appearance(window, |_, window, cx| {
                crate::theme::os_changed(cx);
                crate::theme::apply_window(window, cx);
            }),
            crate::settings::attach_window(window, cx),
            cx.observe_window_bounds(window, |this, window, cx| this.bounds_changed(window, cx)),
        ];
        let mut explorer = None;
        if let Content::Live { store, .. } = &content {
            subs.push(cx.observe(store, |_, _, cx| cx.notify()));
            let e = cx.new(|cx| crate::explorer::Explorer::new(store.clone(), root.clone(), cx));
            subs.push(cx.observe(&e, |_, _, cx| cx.notify()));
            explorer = Some(e);
        }
        let store = match &content {
            Content::Live { store, .. } => Some(store.clone()),
            Content::Refused(_) => None,
        };
        let layer = cx.new(|cx| crate::settings::Layer::new(store, cx));
        subs.push(cx.subscribe_in(
            &layer,
            window,
            |this, _, e: &crate::settings::LayerEvent, window, cx| {
                let crate::settings::LayerEvent::HandOver { agents, .. } = e;
                if let (Some(first), Some(screen)) = (agents.first(), this.screen.clone()) {
                    let id = first.id.clone();
                    screen.update(cx, |s, cx| s.show_agent(&id, window, cx));
                }
            },
        ));
        let focus = cx.focus_handle();
        window.focus(&focus);
        let screen = match &content {
            Content::Live { store, .. } => {
                let (store, root) = (store.clone(), root.clone());
                Some(cx.new(|cx| crate::main_screen::MainScreen::new(store, root, window, cx)))
            }
            Content::Refused(_) => None,
        };
        MainView {
            content,
            explorer,
            layer,
            focus,
            root,
            save_bounds: None,
            screen,
            _subscriptions: subs,
        }
    }

    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.layer.update(cx, |l, cx| l.open_settings(cx));
    }

    pub fn open_about(&mut self, cx: &mut Context<Self>) {
        self.layer.update(cx, |l, cx| l.open_about(cx));
    }

    /// Saved half a second after the last move or resize.
    fn bounds_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let mut saved = window_state::Saved::from_bounds(window.window_bounds());
        // gpui 0.2.2 on macOS reports the frame (title bar included) but opens
        // windows with that size as their content: save the content size, or
        // the window grows by a title bar on every launch. Other OSes: to be
        // checked with the per-OS window work (phase 1).
        if cfg!(target_os = "macos") && saved.state == window_state::State::Windowed {
            let content = window.viewport_size();
            (saved.width, saved.height) = (content.width.into(), content.height.into());
        }
        self.save_bounds = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            if let Err(e) = window_state::save(&root, saved) {
                eprintln!("pitwall: could not save the window's bounds: {e}");
            }
        }));
    }

    fn refused(r: &Refusal, t: &Theme) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.bg)
            .child(
                div()
                    .w(px(560.))
                    .p_6()
                    .rounded(RADIUS_LG)
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.line_strong)
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().text_color(t.amber).child("▲"))
                            .child(div().text_color(t.text).child(r.title())),
                    )
                    .children(
                        r.explanation()
                            .into_iter()
                            .map(|l| div().text_sm().text_color(t.text_2).child(l)),
                    ),
            )
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let root = div()
            .id("main")
            .track_focus(&self.focus)
            .key_context("MainView")
            .relative()
            .size_full()
            .flex()
            .bg(crate::theme::backdrop(cx))
            .text_color(t.text);
        let root = match &self.explorer {
            Some(e) => crate::explorer::on_actions(root, e),
            None => root,
        };
        let root = match &self.content {
            Content::Refused(r) => root.child(Self::refused(r, &t)),
            Content::Live { .. } => root
                .children(self.screen.clone())
                .children(self.explorer.clone()),
        };
        root.child(self.layer.clone())
            .children(crate::approvals::overlay(window, cx))
    }
}
