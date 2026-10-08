//! The main window (phase 0): a sidebar of agents grouped by project, live
//! from the engine, and a placeholder where `pitwall-term-view` will draw the
//! selected agent's terminal. When the app refused to host an engine
//! (`home::Refusal`), the window says why instead.

mod sidebar;

use gpui::{
    div, prelude::*, px, Context, Entity, FocusHandle, IntoElement, Render, SharedString,
    Subscription, Task, Window,
};

use crate::agents::{status_word, AgentStore};
use crate::home::Refusal;
use crate::theme::{self, Theme, RADIUS_LG};
use crate::window_state;

/// What the window shows.
#[derive(Clone)]
pub enum Content {
    Live {
        store: Entity<AgentStore>,
        data_dir: SharedString,
    },
    Refused(Refusal),
}

pub struct MainView {
    content: Content,
    selected: Option<String>,
    /// Settings, About and onboarding (settings::Layer).
    layer: Entity<crate::settings::Layer>,
    focus: FocusHandle,
    /// Where the window's bounds are saved (`None`: refused, nothing saved).
    root: Option<std::path::PathBuf>,
    save_bounds: Option<Task<()>>,
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
                crate::settings::appearance::os_changed(cx);
                crate::settings::appearance::apply_window(window, cx);
            }),
            crate::settings::attach_window(window, cx),
            cx.observe_window_bounds(window, |this, window, cx| this.bounds_changed(window, cx)),
        ];
        if let Content::Live { store, .. } = &content {
            subs.push(cx.observe(store, |_, _, cx| cx.notify()));
        }
        let store = match &content {
            Content::Live { store, .. } => Some(store.clone()),
            Content::Refused(_) => None,
        };
        let layer = cx.new(|cx| crate::settings::Layer::new(store, cx));
        subs.push(cx.subscribe(&layer, |this, _, e: &crate::settings::LayerEvent, cx| {
            let crate::settings::LayerEvent::HandOver { agents, .. } = e;
            if let Some(first) = agents.first() {
                this.select(first.id.clone(), cx);
            }
        }));
        let focus = cx.focus_handle();
        window.focus(&focus);
        MainView {
            content,
            selected: None,
            layer,
            focus,
            root,
            save_bounds: None,
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

    pub(super) fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected = Some(id);
        cx.notify();
    }

    fn placeholder(&self, t: &Theme, cx: &Context<Self>) -> impl IntoElement {
        let agent = match (&self.content, &self.selected) {
            (Content::Live { store, .. }, Some(id)) => store.read(cx).agent(id).cloned(),
            _ => None,
        };
        let (title, lines): (SharedString, Vec<SharedString>) = match agent {
            Some(a) => (
                a.name.clone().into(),
                vec![
                    format!("{} · {}", a.kind_name, status_word(a.status)).into(),
                    a.cwd_display.clone().into(),
                    format!("{}×{}", a.cols, a.rows).into(),
                ],
            ),
            None => (
                "No agent selected".into(),
                vec!["Pick one in the sidebar.".into()],
            ),
        };
        div()
            .id("terminal-placeholder")
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.term_bg)
            .child(
                div()
                    .h(px(36.))
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(t.line)
                    .text_sm()
                    .text_color(t.text_2)
                    .child(title.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        div()
                            .px_6()
                            .py_5()
                            .rounded(RADIUS_LG)
                            .border_1()
                            .border_dashed()
                            .border_color(t.line_strong)
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1()
                            .child(div().text_color(t.text).child(title))
                            .children(
                                lines
                                    .into_iter()
                                    .map(|l| div().text_xs().text_color(t.text_3).child(l)),
                            )
                            .child(
                                div()
                                    .mt_3()
                                    .text_xs()
                                    .text_color(t.text_4)
                                    .child("The terminal (pitwall-term-view) is drawn here."),
                            ),
                    ),
            )
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        let root = div()
            .id("main")
            .track_focus(&self.focus)
            .key_context("MainView")
            .relative()
            .size_full()
            .flex()
            .bg(crate::settings::appearance::backdrop(cx))
            .text_color(t.text);
        let root = match &self.content {
            Content::Refused(r) => root.child(Self::refused(r, &t)),
            Content::Live { store, data_dir } => {
                let chrome = crate::settings::appearance::panel(&t, cx);
                let sidebar = sidebar::render(
                    store.read(cx),
                    self.selected.as_deref(),
                    data_dir,
                    &chrome,
                    cx,
                );
                let sidebar = crate::settings::chrome_panel("sidebar", sidebar, cx);
                root.child(sidebar).child(self.placeholder(&t, cx))
            }
        };
        root.child(self.layer.clone())
    }
}
