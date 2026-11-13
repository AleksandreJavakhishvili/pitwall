//! The main window: the main screen (`crate::main_screen`) with the
//! explorer, Settings / About / onboarding (`settings::Layer`) and the CLI
//! approval dialog over it. When the app refused to host an engine
//! (`home::Refusal`), the window says why instead.

use std::collections::HashMap;

use gpui::{
    div, prelude::*, px, Context, Entity, FocusHandle, Global, IntoElement, Render, Subscription,
    Task, WeakEntity, Window, WindowId,
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
                crate::settings::LayerEvent::Show(view) => {
                    let Some(screen) = this.screen.clone() else { return };
                    let view = (**view).clone();
                    screen.update(cx, |s, cx| s.created(view, window, cx));
                }
            },
        ));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
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
        // `demo <what>`: a dialog opened to look at (`MainScreen::demo_open`).
        if let Some(what) = cmd.strip_prefix("demo ") {
            if let Some(s) = &self.screen {
                s.update(cx, |s, cx| s.demo_open(what.trim(), window, cx));
            }
            return;
        }
        // `pop <what>`: drive the open glass pop-up (`kit::pop::debug`).
        if let Some(what) = cmd.strip_prefix("pop ") {
            crate::kit::pop::debug(what.trim(), cx);
            return;
        }
        let mut words = cmd.split_whitespace();
        let (Some(what), arg) = (words.next(), words.next()) else {
            return;
        };
        let on = arg != Some("off");
        // `settings <page>`: open on that page.
        if let Some(page) = arg.filter(|_| what == "settings").and_then(pitwall_proto::settings::Page::parse) {
            crate::settings::data::set_page(cx, page);
        }
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
            // ⌘P's quick open, to look at.
            "quick" => {
                if let Some(e) = &self.explorer {
                    e.update(cx, |e, cx| if on { e.go_to_file(window, cx) } else { e.close_quick(cx) });
                }
            }
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
        // gpui on macOS reports the frame (title bar included) but opens
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

/// Each window's explorer, for the app-wide Files tab hook.
#[derive(Default)]
struct WindowExplorers(HashMap<WindowId, WeakEntity<crate::explorer::Explorer>>);

impl Global for WindowExplorers {}

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
    // The Files tab hook is one for the app: it finds the explorer of the
    // window it draws in (capturing this one would hand every window the
    // last window's explorer).
    let windows = cx.default_global::<WindowExplorers>();
    windows.0.retain(|_, e| e.upgrade().is_some());
    windows
        .0
        .insert(window.window_handle().window_id(), explorer.downgrade());
    crate::main_screen::register_files_panel(cx, |agent, window, cx| {
        let explorer = cx
            .try_global::<WindowExplorers>()
            .and_then(|w| w.0.get(&window.window_handle().window_id()))
            .and_then(|e| e.upgrade());
        let panel = explorer.and_then(|e| {
            e.update(cx, |e, cx| {
                e.set_agent(Some(agent.id.clone()), cx);
                e.panel(window, cx)
            })
        });
        panel.unwrap_or_else(|| cx.new(|_| Nothing).into())
    });
    // The Explorer route is this window's viewer.
    let e = explorer.clone();
    let route = cx.new(|cx| {
        cx.observe(&e, |_, _, cx| cx.notify()).detach();
        ExplorerRoute(e)
    });
    screen.update(cx, |s, _| s.set_route_view(Route::Explorer, route.into()));
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
                    // Closed with the keyboard in it: let it go, as the
                    // Wall does, so the screen takes it back when it draws
                    // (a focus left on a viewer no longer drawn reaches
                    // nothing: ⌘P and the rest stop working).
                    if !open && e.read(cx).viewer_has_focus(window, cx) {
                        window.blur(cx);
                    }
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
        let approvals = crate::approvals::overlay(window, cx);
        // Settings, About, onboarding, the palette or an approval over the
        // screen.
        let over = self.layer.read(cx).is_open() || self.palette.read(cx).is_open() || approvals.is_some();
        let root = match &self.content {
            Content::Refused(r) => root.child(Self::refused(r, &t)),
            // The screen draws itself every frame, but its big pieces
            // (sidebar, top bar, right panel, Wall / Review) and its
            // terminals are cached views: a frame for one terminal or for
            // the working dots' rings (the pulse layer) replays the rest
            // (main_screen/part.rs).
            // With an overlay open the screen is a cached view as well: a
            // frame for the overlay alone (a Settings scroll step) replays
            // the whole screen, and a frame for something in the screen (a
            // terminal) renders the screen, whose parts and other
            // terminals still replay (`keep_inner_caches`). Closed, the
            // screen draws every frame again (it takes the keyboard back
            // when nothing has it).
            // Without parts (PITWALL_PERF_NO_PARTS) the screen itself is
            // the cached view. Under Liquid Glass each part places its own
            // glass piece (kit::glass); a replayed view keeps its pieces.
            Content::Live { .. } => root
                .children(self.screen.clone().map(|s| {
                    let parts = crate::main_screen::parts_cached(cx);
                    if parts && !over {
                        return s.into_any_element();
                    }
                    let cached = gpui::AnyView::from(s).cached(gpui::StyleRefinement::default().size_full());
                    if parts {
                        cached.keep_inner_caches().into_any_element()
                    } else {
                        cached.into_any_element()
                    }
                }))
                .child(self.pulses.clone())
                .children(self.explorer.clone()),
        };
        let root = root
            .child(self.layer.clone())
            .child(self.palette.clone())
            .children(approvals)
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

    /// Quick open's shortcut on this OS (`explorer::bindings`).
    const GO_TO_FILE: &str = if cfg!(target_os = "macos") { "cmd-p" } else { "ctrl-shift-p" };

    /// Two windows over one made-up agent whose files can be read, Flat or
    /// Liquid Glass (regions on: the screen is drawn without its parts, as
    /// one cached view). The second opens after the first, as
    /// `windows::restore` opens saved windows after main.
    fn two_windows(
        cx: &mut TestAppContext,
        glass: bool,
    ) -> (Entity<MainView>, Entity<MainView>, &mut VisualTestContext) {
        two_windows_with(cx, glass, false)
    }

    /// [`two_windows`], the agent in a git repository when `diff`.
    fn two_windows_with(
        cx: &mut TestAppContext,
        glass: bool,
        diff: bool,
    ) -> (Entity<MainView>, Entity<MainView>, &mut VisualTestContext) {
        use crate::explorer::source::fake::FakeSource;
        use crate::theme::{Appearance, GlassOffer, Inputs, Look, OsPrefs};
        cx.update(|cx| {
            crate::kit::init(cx);
            crate::settings::init(cx, None, None);
            crate::palette::init(cx);
            crate::main_screen::init(cx);
            crate::explorer::register(cx);
            crate::ui_state::init(None, cx);
            let look = if glass { Look::Glass } else { Look::Flat };
            let ap = Appearance::resolve(
                Inputs { look, ..Inputs::default() },
                gpui::WindowAppearance::Dark,
                GlassOffer::Liquid,
                OsPrefs::default(),
            );
            cx.set_global(ap.theme());
            cx.set_global(ap);
            assert_eq!(crate::theme::glass_regions(cx), glass);
            let src: std::sync::Arc<dyn crate::explorer::Source> = FakeSource::new(&[
                ("src/main.rs", "fn main() {\n    println!(\"hi\");\n}\n"),
                ("notes/todo.md", "one\ntwo\n"),
            ]);
            cx.set_global(crate::explorer::ExplorerSource(src));
        });
        let mut a = crate::agents::tests::agent("a1", "/work/alpha", "idle", 1, true);
        a.caps = pitwall_proto::AgentCaps {
            explorer: true,
            review: true,
            diff,
            ..Default::default()
        };
        let store = cx.new(|_| AgentStore::new(vec![a]));
        let s2 = store.clone();
        let (first, vcx) = cx.add_window_view(move |window, cx| {
            MainView::new(Content::Live { store }, None, window, cx)
        });
        vcx.run_until_parked();
        let second = vcx.update(|_, cx| {
            let h = cx
                .open_window(Default::default(), move |window, cx| {
                    cx.new(|cx| MainView::new(Content::Live { store: s2 }, None, window, cx))
                })
                .unwrap();
            h.entity(cx).unwrap()
        });
        vcx.run_until_parked();
        (first, second, vcx)
    }

    /// ⌘P, a file picked: the first window shows it, though a second
    /// window opened after it (each window's routes and Files tab are its
    /// own, not the last window's).
    fn quick_open_shows_the_file(glass: bool, cx: &mut TestAppContext) {
        let (first, second, vcx) = two_windows(cx, glass);
        let screen = first.read_with(vcx, |v, _| v.screen.clone().unwrap());
        screen.update_in(vcx, |s, window, cx| s.show_agent("a1", window, cx));
        vcx.run_until_parked();
        vcx.simulate_keystrokes(GO_TO_FILE);
        vcx.run_until_parked();
        vcx.simulate_input("main");
        vcx.run_until_parked();
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert_eq!(screen.read_with(vcx, |s, _| s.route()), crate::main_screen::Route::Explorer);
        assert!(vcx.debug_bounds("ex-viewer").is_some(), "the viewer is drawn");
        let code = vcx.debug_bounds("ex-code").expect("the file's text is drawn");
        assert!(code.size.width > gpui::px(0.) && code.size.height > gpui::px(0.));

        // The other per-window seams: Review and the Files tab.
        vcx.update(|window, cx| {
            let (review, explorer) = {
                let v = first.read(cx);
                (v.review.clone().unwrap(), v.explorer.clone().unwrap())
            };
            let shown = screen.read(cx).route_view_of(crate::main_screen::Route::Review);
            assert_eq!(shown.map(|v| v.entity_id()), Some(review.entity_id()));
            let agent = screen.read(cx).selected(cx).unwrap();
            let build = cx.global::<crate::main_screen::FilesPanel>().0.clone();
            let tab = build(&agent, window, cx);
            let own = explorer.update(cx, |e, cx| e.panel(window, cx)).unwrap();
            assert_eq!(tab.entity_id(), own.entity_id());
        });
        let other = second.read_with(vcx, |v, _| v.explorer.clone().unwrap());
        assert!(!other.read_with(vcx, |e, _| e.viewer_open()));
    }

    /// A file opened with ⌘P, Esc back to the agents: the keyboard is the
    /// screen's again, so ⌘P opens quick open once more.
    fn esc_gives_the_keyboard_back(glass: bool, cx: &mut TestAppContext) {
        let (first, _, vcx) = two_windows(cx, glass);
        let (screen, explorer) = first.read_with(vcx, |v, _| {
            (v.screen.clone().unwrap(), v.explorer.clone().unwrap())
        });
        screen.update_in(vcx, |s, window, cx| s.show_agent("a1", window, cx));
        vcx.run_until_parked();
        vcx.simulate_keystrokes(GO_TO_FILE);
        vcx.run_until_parked();
        vcx.simulate_input("main");
        vcx.run_until_parked();
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(explorer.read_with(vcx, |e, _| e.viewer_open()));
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(!explorer.read_with(vcx, |e, _| e.viewer_open()));
        assert_eq!(screen.read_with(vcx, |s, _| s.route()), crate::main_screen::Route::Space);
        vcx.simulate_keystrokes(GO_TO_FILE);
        vcx.run_until_parked();
        assert!(explorer.read_with(vcx, |e, _| e.quick_open_shown()), "⌘P opens quick open");
    }

    /// A diff open over the Changes list: a click on the backdrop, even
    /// over the same file, only closes it (the click doesn't reach the row
    /// under it and open the diff again, a flash of closing and opening).
    #[gpui::test]
    fn a_click_through_a_dialogs_backdrop_only_closes_it(cx: &mut TestAppContext) {
        let (first, _second, vcx) = two_windows_with(cx, true, true);
        vcx.simulate_resize(gpui::size(px(1500.), px(900.)));
        let screen = first.read_with(vcx, |v, _| v.screen.clone().unwrap());
        screen.update_in(vcx, |s, window, cx| s.show_agent("a1", window, cx));
        let file = |path: &str| pitwall_core::vcs::git::FileChange {
            path: path.into(),
            added: 2,
            removed: 1,
            untracked: false,
            binary: false,
            status: None,
            repo: None,
            branch: None,
            parts: None,
        };
        screen.update(vcx, |s, cx| s.set_changes_for_test("a1", vec![file("src/main.rs")], cx));
        vcx.run_until_parked();
        let click = |vcx: &mut VisualTestContext| {
            screen.update(vcx, |_, cx| cx.notify());
            vcx.run_until_parked();
            let at = vcx.debug_bounds("file-name-0").expect("the file is listed");
            vcx.simulate_click(at.center(), gpui::Modifiers::none());
            vcx.run_until_parked();
        };
        click(vcx);
        assert!(screen.read_with(vcx, |s, _| s.diff_dialog().is_some()), "its diff opened");
        click(vcx);
        assert!(screen.read_with(vcx, |s, _| s.diff_dialog().is_none()), "closed, not opened again");
    }

    /// Repositories inside the agent's folder: a section each, VS Code
    /// style; a click on its header folds it (its files go), another opens it.
    #[gpui::test]
    fn repository_sections_fold(cx: &mut TestAppContext) {
        let (first, _second, vcx) = two_windows_with(cx, true, true);
        vcx.simulate_resize(gpui::size(px(1500.), px(900.)));
        let screen = first.read_with(vcx, |v, _| v.screen.clone().unwrap());
        screen.update_in(vcx, |s, window, cx| s.show_agent("a1", window, cx));
        let file = |path: &str, repo: Option<&str>| pitwall_core::vcs::git::FileChange {
            path: path.into(),
            added: 1,
            repo: repo.map(String::from),
            ..Default::default()
        };
        let files = vec![file("main.rs", None), file("upstreams/lib/a.rs", Some("upstreams/lib"))];
        screen.update(vcx, |s, cx| s.set_changes_for_test("a1", files, cx));
        let draw = |vcx: &mut VisualTestContext| {
            screen.update(vcx, |_, cx| cx.notify());
            vcx.run_until_parked();
        };
        draw(vcx);
        assert!(vcx.debug_bounds("repo-section-0").is_some(), "the agent's own section");
        let drawn = |vcx: &mut VisualTestContext| screen.read_with(vcx, |s, _| s.drawn_files());
        assert_eq!(drawn(vcx), ["main.rs", "upstreams/lib/a.rs"]);
        let header = vcx.debug_bounds("repo-section-1").expect("the clone's section");
        vcx.simulate_click(header.center(), gpui::Modifiers::none());
        draw(vcx);
        assert_eq!(drawn(vcx), ["main.rs"], "folded; the other section stays open");
        let header = vcx.debug_bounds("repo-section-1").unwrap();
        vcx.simulate_click(header.center(), gpui::Modifiers::none());
        draw(vcx);
        assert_eq!(drawn(vcx), ["main.rs", "upstreams/lib/a.rs"], "open again");
    }

    /// A diff open over the Changes list: one click on another file shows
    /// that file's diff (not a click to close, a second to open).
    #[gpui::test]
    fn a_click_on_another_file_switches_the_diff(cx: &mut TestAppContext) {
        let (first, _second, vcx) = two_windows_with(cx, true, true);
        vcx.simulate_resize(gpui::size(px(1500.), px(900.)));
        let screen = first.read_with(vcx, |v, _| v.screen.clone().unwrap());
        screen.update_in(vcx, |s, window, cx| s.show_agent("a1", window, cx));
        let file = |path: &str| pitwall_core::vcs::git::FileChange {
            path: path.into(),
            added: 2,
            removed: 1,
            untracked: false,
            binary: false,
            status: None,
            repo: None,
            branch: None,
            parts: None,
        };
        let files = vec![file("src/main.rs"), file("src/lib.rs")];
        screen.update(vcx, |s, cx| s.set_changes_for_test("a1", files, cx));
        vcx.run_until_parked();
        let click = |vcx: &mut VisualTestContext, row: usize| {
            screen.update(vcx, |_, cx| cx.notify());
            vcx.run_until_parked();
            let name = ["file-name-0", "file-name-1"][row];
            let at = vcx.debug_bounds(name).expect("the file is listed");
            vcx.simulate_click(at.center(), gpui::Modifiers::none());
            vcx.run_until_parked();
        };
        let shown = |vcx: &mut VisualTestContext| {
            screen.read_with(vcx, |s, cx| {
                s.diff_dialog().map(|d| d.read(cx).shows().1.to_string())
            })
        };
        click(vcx, 0);
        assert_eq!(shown(vcx).as_deref(), Some("src/main.rs"));
        click(vcx, 1);
        assert_eq!(shown(vcx).as_deref(), Some("src/lib.rs"), "one click switched");
        click(vcx, 1);
        assert_eq!(shown(vcx), None, "the same file only closes");
    }

    /// Liquid Glass: the chrome's native glass pieces (top bar, sidebar,
    /// right panel) stay as they are while a change in the right panel's
    /// list opens and closes, and on the frames that only redraw what's
    /// over the screen (a working dot's ring replays the cached screen):
    /// none is hidden or made again, which flickered the whole window.
    /// Review hides the right panel's piece and the agents show the same
    /// one again.
    #[gpui::test]
    fn glass_pieces_hold_while_a_change_opens(cx: &mut TestAppContext) {
        use crate::main_screen::Route;
        use crate::platform::glass::regions;
        let (first, _second, vcx) = two_windows_with(cx, true, true);
        vcx.simulate_resize(gpui::size(px(1500.), px(900.)));
        let (screen, pulses) =
            first.read_with(vcx, |v, _| (v.screen.clone().unwrap(), v.pulses.clone()));
        screen.update_in(vcx, |s, window, cx| s.show_agent("a1", window, cx));
        let file = |path: &str| pitwall_core::vcs::git::FileChange {
            path: path.into(),
            added: 2,
            removed: 1,
            untracked: false,
            binary: false,
            status: None,
            repo: None,
            branch: None,
            parts: None,
        };
        screen.update(vcx, |s, cx| {
            s.set_changes_for_test("a1", vec![file("src/main.rs"), file("notes/todo.md")], cx)
        });
        vcx.run_until_parked();
        let wid = vcx.update(|window, _| window.window_handle().window_id());
        let all = vec!["right-panel", "sidebar", "topbar"];
        let (shown, made) = regions::of(wid);
        assert_eq!(shown, all, "the chrome's pieces are placed");
        let ring_frame = |vcx: &mut VisualTestContext| {
            pulses.update(vcx, |_, cx| cx.notify());
            vcx.run_until_parked();
        };
        let steady = |when: &str| {
            assert_eq!(regions::of(wid), (all.clone(), made), "{when}: no piece hidden or made");
        };
        ring_frame(vcx);
        steady("a ring frame");

        // Click through the changed files: each opens its diff.
        for row in ["file-name-0", "file-name-1"] {
            // (A replayed frame keeps no debug bounds: draw the screen.)
            screen.update(vcx, |_, cx| cx.notify());
            vcx.run_until_parked();
            steady("the screen drawn again");
            let name = vcx
                .debug_bounds(row)
                .unwrap_or_else(|| panic!("{row} is listed"));
            vcx.simulate_click(name.center(), gpui::Modifiers::none());
            vcx.run_until_parked();
            assert!(screen.read_with(vcx, |s, _| s.has_dialog()), "its diff opened");
            steady("the diff open");
            ring_frame(vcx);
            steady("a ring frame over the diff");
            screen.update(vcx, |s, cx| s.close_dialogs(cx));
            vcx.run_until_parked();
            ring_frame(vcx);
            steady("the diff closed");
        }

        // Review: the right panel's piece hides; back: the same one shows.
        screen.update(vcx, |s, cx| s.set_route(Route::Review, cx));
        vcx.run_until_parked();
        // (The screen drawn on Review whatever frames ran first.)
        screen.update(vcx, |_, cx| cx.notify());
        vcx.run_until_parked();
        ring_frame(vcx);
        let (shown, counts) = regions::of(wid);
        assert_eq!(shown, vec!["sidebar", "topbar"]);
        assert_eq!((counts.made, counts.hidden), (made.made, made.hidden + 1));
        screen.update(vcx, |s, cx| s.set_route(Route::Space, cx));
        vcx.run_until_parked();
        screen.update(vcx, |_, cx| cx.notify());
        vcx.run_until_parked();
        let (shown, counts) = regions::of(wid);
        assert_eq!(shown, all);
        assert_eq!(counts.made, made.made, "shown again, not made again");
        assert_eq!(counts.reshown, made.reshown + 1);
    }

    /// A view that stands in for a terminal in the pane body.
    struct FakeTerm;

    impl Render for FakeTerm {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }

    /// Settings open over the screen: a frame for Settings alone (a scroll
    /// step) replays the whole screen; output in a terminal behind it
    /// renders the screen again, whose parts still replay; a change in
    /// the screen shows behind Settings; closed, the screen draws every
    /// frame again. Under Liquid Glass the chrome's pieces hold throughout.
    fn settings_over_the_screen(glass: bool, cx: &mut TestAppContext) {
        use crate::platform::glass::regions;
        use std::cell::Cell;
        use std::rc::Rc;
        // The fake terminal in the first window only (one view, one window).
        let term = cx.new(|_| FakeTerm);
        let in_window: Rc<Cell<Option<WindowId>>> = Rc::new(Cell::new(None));
        let (t, w) = (term.clone(), in_window.clone());
        cx.update(|cx| {
            crate::main_screen::set_terminal_slot(cx, move |_, window, _| {
                if w.get() == Some(window.window_handle().window_id()) {
                    gpui::AnyView::from(t.clone())
                        .cached(gpui::StyleRefinement::default().flex_1())
                        .into_any_element()
                } else {
                    div().into_any_element()
                }
            })
        });
        let (first, _second, vcx) = two_windows(cx, glass);
        vcx.simulate_resize(gpui::size(px(1500.), px(900.)));
        let wid = vcx.update(|window, _| window.window_handle().window_id());
        in_window.set(Some(wid));
        let screen = first.read_with(vcx, |v, _| v.screen.clone().unwrap());
        screen.update_in(vcx, |s, window, cx| s.show_agent("a1", window, cx));
        vcx.run_until_parked();
        let renders = |vcx: &mut VisualTestContext| screen.read_with(vcx, |s, cx| s.renders(cx));
        let pieces = || regions::of(wid);

        // No overlay: a frame for anything draws the screen (not its parts).
        let before = renders(vcx);
        first.update(vcx, |v, cx| v.layer.update(cx, |_, cx| cx.notify()));
        vcx.run_until_parked();
        let (screen_n, side, top) = renders(vcx);
        assert!(screen_n > before.0, "{before:?}");
        assert_eq!((side, top), (before.1, before.2));

        first.update(vcx, |v, cx| v.open_settings(cx));
        vcx.run_until_parked();
        assert!(first.read_with(vcx, |v, cx| v.layer.read(cx).is_open()));
        let open = renders(vcx);
        let held = pieces();
        if glass {
            assert_eq!(held.0, vec!["right-panel", "sidebar", "topbar"]);
        }

        // Settings scrolls (each step notifies its view): the screen replays.
        for _ in 0..3 {
            first.update(vcx, |v, cx| v.layer.update(cx, |_, cx| cx.notify()));
            vcx.run_until_parked();
        }
        assert_eq!(renders(vcx), open, "a frame for Settings re-rendered the screen");
        assert_eq!(pieces(), held, "replayed: no glass piece hidden or made");

        // Output in a terminal behind Settings: the screen renders. Its
        // parts weren't drawn in the frame before (the screen was replayed
        // whole), so what they kept is stale and they render once; while
        // the terminal keeps printing, they replay.
        term.update(vcx, |_, cx| cx.notify());
        vcx.run_until_parked();
        let (screen_n, side, top) = renders(vcx);
        assert_eq!(screen_n, open.0 + 1, "the terminal's frame draws the screen");
        assert_eq!((side, top), (open.1 + 1, open.2 + 1), "a part replayed a frame it wasn't drawn in");
        term.update(vcx, |_, cx| cx.notify());
        vcx.run_until_parked();
        let (screen_n, side, top) = {
            let again = renders(vcx);
            assert_eq!(again, (screen_n + 1, side, top), "the next terminal frame replays the parts");
            again
        };
        assert_eq!(pieces().0, held.0);

        // A change in the screen behind Settings: it shows.
        let store = first.read_with(vcx, |v, _| match &v.content {
            Content::Live { store } => store.clone(),
            Content::Refused(_) => unreachable!(),
        });
        store.update(vcx, |s, cx| {
            s.agents[0].status = pitwall_proto::Status::Working;
            cx.notify();
        });
        vcx.run_until_parked();
        let changed = renders(vcx);
        assert!(changed.0 > screen_n && changed.1 > side && changed.2 > top, "{open:?} -> {changed:?}");
        assert_eq!(pieces().0, held.0);

        // Closed: the screen draws every frame again.
        first.update(vcx, |v, cx| v.layer.update(cx, |l, cx| l.close(cx)));
        vcx.run_until_parked();
        let closed = renders(vcx);
        assert!(closed.0 > changed.0);
        first.update(vcx, |v, cx| v.layer.update(cx, |_, cx| cx.notify()));
        vcx.run_until_parked();
        assert_eq!(renders(vcx).0, closed.0 + 1);
        assert_eq!(pieces().0, held.0);

        // The palette is an overlay too: typing in it replays the screen.
        first.update_in(vcx, |v, window, cx| v.palette.update(cx, |p, cx| p.open(window, cx)));
        vcx.run_until_parked();
        let palette = renders(vcx);
        for _ in 0..2 {
            first.update(vcx, |v, cx| v.palette.update(cx, |_, cx| cx.notify()));
            vcx.run_until_parked();
        }
        assert_eq!(renders(vcx), palette, "a frame for the palette re-rendered the screen");
        assert_eq!(pieces().0, held.0);
    }

    #[gpui::test]
    fn settings_over_the_screen_flat(cx: &mut TestAppContext) {
        settings_over_the_screen(false, cx);
    }

    #[gpui::test]
    fn settings_over_the_screen_glass(cx: &mut TestAppContext) {
        settings_over_the_screen(true, cx);
    }

    #[gpui::test]
    fn esc_gives_the_keyboard_back_flat(cx: &mut TestAppContext) {
        esc_gives_the_keyboard_back(false, cx);
    }

    #[gpui::test]
    fn esc_gives_the_keyboard_back_glass(cx: &mut TestAppContext) {
        esc_gives_the_keyboard_back(true, cx);
    }

    #[gpui::test]
    fn quick_open_shows_the_picked_file_flat(cx: &mut TestAppContext) {
        quick_open_shows_the_file(false, cx);
    }

    #[gpui::test]
    fn quick_open_shows_the_picked_file_glass(cx: &mut TestAppContext) {
        quick_open_shows_the_file(true, cx);
    }

    /// Under Liquid Glass with pop-ups: quick open's panel is one, with
    /// the keyboard; a pick opens the file in the window, Esc and ⌘K
    /// close it.
    #[gpui::test]
    fn under_liquid_glass_quick_open_is_a_pop_up_with_the_keyboard(cx: &mut TestAppContext) {
        let (first, _second, vcx) = two_windows(cx, true);
        vcx.update(|_, cx| cx.set_global(crate::kit::pop::TestHosted));
        let (screen, explorer) = first.read_with(vcx, |v, _| (v.screen.clone().unwrap(), v.explorer.clone().unwrap()));
        screen.update_in(vcx, |s, window, cx| s.show_agent("a1", window, cx));
        vcx.run_until_parked();
        let pops = |vcx: &mut VisualTestContext| {
            let parent = vcx.update(|window, _| window.window_handle().window_id());
            let all = vcx.windows();
            crate::kit::pop::open_pops()
                .into_iter()
                .filter(|(_, p, _)| p.window_id() == parent)
                .filter_map(|(id, _, at)| Some((*all.iter().find(|w| w.window_id() == id)?, at?)))
                .collect::<Vec<_>>()
        };
        let keys = |pop: gpui::AnyWindowHandle, keys: &'static str, vcx: &mut VisualTestContext| {
            vcx.update_window(pop, |_, window, cx| {
                for k in keys.split(' ') {
                    window.dispatch_keystroke(gpui::Keystroke::parse(k).unwrap(), cx);
                }
            })
            .unwrap();
            vcx.run_until_parked();
        };
        assert!(pops(vcx).is_empty());
        vcx.simulate_keystrokes(GO_TO_FILE);
        vcx.run_until_parked();
        let open = pops(vcx);
        assert_eq!(open.len(), 1, "the panel is a pop-up");
        let (pop, at) = open[0];
        // Centred, 12% down the window (with the pop-up's clear space).
        let view = vcx.update(|window, _| window.viewport_size());
        let pad = px(crate::kit::pop::PAD);
        assert_eq!(at.size.width, px(600.) + pad * 2.);
        assert_eq!(at.origin, gpui::point((view.width - px(600.)) / 2. - pad, view.height * 0.12 - pad));
        keys(pop, "m a i n", vcx);
        keys(pop, "enter", vcx);
        assert!(pops(vcx).is_empty());
        assert!(!explorer.read_with(vcx, |e, _| e.quick_open_shown()));
        assert_eq!(screen.read_with(vcx, |s, _| s.route()), crate::main_screen::Route::Explorer);
        assert!(vcx.debug_bounds("ex-code").is_some(), "the file's text is drawn in the window");
        // The keyboard is the window's again: ⌘P, then Esc in the pop-up.
        vcx.simulate_keystrokes(GO_TO_FILE);
        vcx.run_until_parked();
        let pop = pops(vcx)[0].0;
        keys(pop, "escape", vcx);
        assert!(pops(vcx).is_empty());
        assert!(!explorer.read_with(vcx, |e, _| e.quick_open_shown()));
        // And a press on the scrim (the window has the keyboard again).
        vcx.simulate_keystrokes(GO_TO_FILE);
        vcx.run_until_parked();
        let pop = pops(vcx)[0].0;
        vcx.update_window(pop, |_, window, _| window.activate_window()).unwrap();
        vcx.run_until_parked();
        vcx.update(|window, _| window.activate_window());
        vcx.run_until_parked();
        assert!(pops(vcx).is_empty());
        assert!(!explorer.read_with(vcx, |e, _| e.quick_open_shown()));
    }
}
