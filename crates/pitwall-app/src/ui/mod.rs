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
    /// Its Review route (opened on a file or worktree from elsewhere).
    pub review: Option<Entity<crate::review::ReviewRoute>>,
    /// The command palette (⌘K).
    palette: Entity<crate::palette::PaletteHost>,
    /// The working dots' rings over the screen (`kit::motion::PulseLayer`).
    pulses: Entity<crate::kit::motion::PulseLayer>,
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
            let e = cx.new(|cx| crate::explorer::Explorer::new(store.clone(), cx));
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
            |this, _, e: &crate::settings::LayerEvent, window, cx| match e {
                crate::settings::LayerEvent::Opened => this.one_modal(Owner::Layer, window, cx),
                crate::settings::LayerEvent::HandOver { agents, failures } => {
                    let Some(screen) = this.screen.clone() else { return };
                    let ids: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
                    let failures = failures.clone();
                    screen.update(cx, |s, cx| {
                        // React toasts each failure ("Couldn't start <name>").
                        for f in failures {
                            let (title, detail) = match f.split_once(": ") {
                                Some((t, d)) => (t.to_string(), d.to_string()),
                                None => (f.clone(), String::new()),
                            };
                            s.toast_error(title, detail, cx);
                        }
                        s.hand_over(&ids, window, cx);
                    });
                }
            },
        ));
        let focus = cx.focus_handle();
        window.focus(&focus);
        let screen = match &content {
            Content::Live { store, .. } => {
                let store = store.clone();
                Some(cx.new(|cx| crate::main_screen::MainScreen::new(store, window, cx)))
            }
            Content::Refused(_) => None,
        };
        let review = screen.as_ref().map(|s| crate::review::mount(s, window, cx));
        if let Some(s) = &screen {
            crate::wall::mount(s, cx);
        }
        if let (Some(screen), Some(e), Some(r)) = (&screen, &explorer, &review) {
            subs.extend(explorer_seams(screen, e, r, window, cx));
        }
        let palette =
            cx.new(|cx| crate::palette::PaletteHost::new(screen.clone(), explorer.as_ref(), window, cx));
        subs.extend(crate::windows::attach(screen.as_ref(), window, cx));
        // One modal at a time (React keeps a single `modal` state): whichever
        // opens closes the others.
        subs.push(cx.subscribe_in(
            &palette,
            window,
            |this, _, _: &crate::palette::PaletteHostEvent, window, cx| {
                this.one_modal(Owner::Palette, window, cx)
            },
        ));
        if let Some(s) = &screen {
            subs.push(cx.subscribe_in(s, window, |this, _, e: &crate::main_screen::ScreenEvent, window, cx| {
                if matches!(e, crate::main_screen::ScreenEvent::ModalOpened) {
                    this.one_modal(Owner::Screen, window, cx);
                }
            }));
        }
        if let Some(e) = &explorer {
            subs.push(cx.subscribe_in(e, window, |this, _, e: &crate::explorer::ExplorerEvent, window, cx| {
                if matches!(e, crate::explorer::ExplorerEvent::QuickOpened) {
                    this.one_modal(Owner::QuickOpen, window, cx);
                }
            }));
        }
        // Onboarding is the main window's (a moved-out space's window skips it).
        let main = crate::windows::label_of(window, cx) == crate::windows::MAIN;
        if !main {
            layer.update(cx, |l, cx| l.close(cx));
        }
        MainView {
            palette,
            pulses: cx.new(|_| crate::kit::motion::PulseLayer),
            content,
            explorer,
            layer,
            focus,
            // Secondary windows' bounds live in windows.json only.
            root: root.filter(|_| main),
            save_bounds: None,
            screen,
            review,
            _subscriptions: subs,
        }
    }

    /// `owner`'s modal opened: close every other one.
    fn one_modal(&mut self, owner: Owner, window: &mut Window, cx: &mut Context<Self>) {
        if owner != Owner::Palette {
            self.palette.update(cx, |p, cx| p.close(window, cx));
        }
        if owner != Owner::Layer {
            self.layer.update(cx, |l, cx| l.close_dialog(cx));
        }
        if owner != Owner::Screen {
            if let Some(s) = &self.screen {
                s.update(cx, |s, cx| s.close_dialogs(cx));
            }
        }
        if owner != Owner::QuickOpen {
            if let Some(e) = &self.explorer {
                e.update(cx, |e, cx| e.close_quick(cx));
            }
        }
    }

    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.layer.update(cx, |l, cx| l.open_settings(cx));
    }

    /// A bench command (`crate::bench`, `src/lib/bench.ts`).
    pub fn bench(&mut self, cmd: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut words = cmd.split_whitespace();
        let (Some(what), on) = (words.next(), words.next() != Some("off")) else {
            return;
        };
        match what {
            "wall" | "review" | "visit-all" | "visit-all-4" => {
                if let Some(s) = &self.screen {
                    s.update(cx, |s, cx| s.bench(what, on, window, cx));
                }
            }
            "palette" => self.palette.update(cx, |p, cx| {
                if on {
                    p.open(window, cx)
                } else {
                    p.close(window, cx)
                }
            }),
            "settings" => self.layer.update(cx, |l, cx| {
                if on {
                    l.open_settings(cx)
                } else {
                    l.close(cx)
                }
            }),
            // The window off screen and back (its GPU memory is released).
            "hidden" => {
                if on {
                    crate::platform::hide(window)
                } else {
                    window.activate_window()
                }
            }
            _ => {}
        }
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
        window_state::pend(&root, saved);
        self.save_bounds = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            window_state::flush();
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

/// Who owns a modal in the window (see [`MainView::one_modal`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// The command palette.
    Palette,
    /// Settings, About, "Scan again".
    Layer,
    /// The main screen's dialogs (New agent, Remove, Bring in, …).
    Screen,
    /// The explorer's quick open.
    QuickOpen,
}

/// An empty view (a Files tab for an agent whose files can't be read).
struct Nothing;

impl Render for Nothing {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The explorer's viewer as the main screen's Explorer route.
struct ExplorerRoute(Entity<crate::explorer::Explorer>);

impl Render for ExplorerRoute {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .size_full()
            .flex()
            .children(self.0.read(cx).main_view())
    }
}

/// Plug the explorer into the main screen: its Files tab in the right
/// panel, its viewer as the Explorer route, the focused agent, and the
/// route following the viewer (and back).
fn explorer_seams(
    screen: &Entity<crate::main_screen::MainScreen>,
    explorer: &Entity<crate::explorer::Explorer>,
    review: &Entity<crate::review::ReviewRoute>,
    window: &mut Window,
    cx: &mut Context<MainView>,
) -> Vec<Subscription> {
    use crate::explorer::ExplorerEvent;
    use crate::main_screen::{Route, ScreenEvent};
    let e = explorer.downgrade();
    crate::main_screen::register_files_panel(cx, move |agent, window, cx| {
        let panel = e.upgrade().and_then(|e| {
            e.update(cx, |e, cx| {
                e.set_agent(Some(agent.id.clone()), cx);
                e.panel(window, cx)
            })
        });
        panel.unwrap_or_else(|| cx.new(|_| Nothing).into())
    });
    let e = explorer.clone();
    crate::main_screen::register_route(cx, Route::Explorer, move |_, _, cx| {
        let e = e.clone();
        cx.new(|cx| {
            cx.observe(&e, |_, _, cx| cx.notify()).detach();
            ExplorerRoute(e)
        })
        .into()
    });
    let (s1, s2) = (screen.clone(), screen.clone());
    let e1 = explorer.clone();
    let review = review.clone();
    vec![
        // The focused pane's agent is the explorer's.
        cx.observe(screen, move |_, screen, cx| {
            let id = screen.read(cx).focused_agent_id();
            e1.update(cx, |e, cx| e.set_agent(id, cx));
        }),
        cx.subscribe_in(explorer, window, move |_, e, ev: &ExplorerEvent, window, cx| {
            let open = e.read(cx).viewer_open();
            match ev {
                ExplorerEvent::ViewerToggled => s1.update(cx, |s, cx| {
                    if open {
                        s.set_route(Route::Explorer, cx);
                    } else if s.route() == Route::Explorer {
                        s.set_route(Route::Space, cx);
                    }
                }),
                // Review on that file's diff.
                ExplorerEvent::ShowDiff { agent_id, path } => {
                    let focus = crate::review::Focus::Agent {
                        id: agent_id.clone(),
                        path: Some(path.clone()),
                    };
                    review.update(cx, |r, cx| r.open_at(Some(focus), window, cx));
                    s1.update(cx, |s, cx| s.set_route(Route::Review, cx))
                }
                // `MainView::one_modal` answers it.
                ExplorerEvent::QuickOpened => {}
            }
        }),
        cx.subscribe(&s2, {
            let e = explorer.clone();
            move |_, _, ev: &ScreenEvent, cx| {
                if let ScreenEvent::RouteChanged(r) = ev {
                    if *r != Route::Explorer && e.read(cx).viewer_open() {
                        e.update(cx, |e, cx| e.close_viewer(cx));
                    }
                }
            }
        }),
    ]
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::frame_stats::render_started();
        let t = theme::theme(cx).clone();
        let root = div()
            .id("main")
            .track_focus(&self.focus)
            .key_context("MainView")
            .relative()
            .size_full()
            .flex()
            .bg(crate::theme::backdrop(cx))
            // `body`: Inter 13 px, line height 1.45, for every overlay too.
            .font_family(crate::kit::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(crate::kit::BODY_LINE_HEIGHT))
            .text_color(t.text);
        let root = match &self.explorer {
            Some(e) => crate::explorer::on_actions(root, e),
            None => root,
        };
        let root = crate::palette::on_actions(root, &self.palette);
        let root = match &self.content {
            Content::Refused(r) => root.child(Self::refused(r, &t)),
            // The screen draws itself every frame, but its big pieces
            // (sidebar, top bar, right panel, Wall / Review) and its
            // terminals are cached views: a frame for one terminal or for
            // the working dots' rings (the pulse layer) replays the rest
            // (main_screen/part.rs).
            // Without parts (Liquid Glass regions) the screen itself is
            // the cached view, as before.
            Content::Live { .. } => root
                .children(self.screen.clone().map(|s| {
                    if crate::main_screen::parts_cached(cx) {
                        s.into_any_element()
                    } else {
                        gpui::AnyView::from(s)
                            .cached(gpui::StyleRefinement::default().size_full())
                            .into_any_element()
                    }
                }))
                .child(self.pulses.clone())
                .children(self.explorer.clone()),
        };
        let root = root
            .child(self.layer.clone())
            .child(self.palette.clone())
            .children(crate::approvals::overlay(window, cx))
            .children(crate::frame_stats::probe());
        crate::platform::decorations::frame(root, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    #[gpui::test]
    fn one_modal_at_a_time(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::kit::init(cx);
            crate::settings::init(cx, None, None);
            crate::palette::init(cx);
        });
        let refused = Content::Refused(Refusal::InUse {
            root: "/nowhere".into(),
        });
        let (view, vcx) =
            cx.add_window_view(move |window, cx| MainView::new(refused, None, window, cx));
        let vcx: &mut VisualTestContext = vcx;
        view.update_in(vcx, |v, _, cx| v.open_settings(cx));
        vcx.run_until_parked();
        assert!(view.read_with(vcx, |v, cx| v.layer.read(cx).is_open()));
        // ⌘K over Settings: the palette replaces it.
        view.update_in(vcx, |v, window, cx| {
            v.palette.update(cx, |p, cx| p.open(window, cx))
        });
        vcx.run_until_parked();
        view.read_with(vcx, |v, cx| {
            assert!(v.palette.read(cx).is_open());
            assert!(!v.layer.read(cx).is_open(), "Settings closed");
        });
        // And Settings from the palette replaces the palette.
        view.update_in(vcx, |v, _, cx| v.open_settings(cx));
        vcx.run_until_parked();
        view.read_with(vcx, |v, cx| {
            assert!(v.layer.read(cx).is_open());
            assert!(!v.palette.read(cx).is_open(), "the palette closed");
        });
    }
}
