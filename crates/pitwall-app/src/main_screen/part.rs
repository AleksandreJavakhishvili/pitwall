//! Cached pieces of the main screen (docs/spec/perf.md "GPUI").
//!
//! In GPUI a view that repaints dirties every view above it; a dirty
//! *cached* view then renders and lays out its whole subtree again, nested
//! cached views included. The pane terminals sit inside the main screen,
//! so when the screen itself was the cached view, each terminal frame
//! rebuilt and laid out everything in it: the sidebar, the top bar, the
//! right panel, and every other terminal.
//!
//! Instead the screen is drawn every frame (its own tree is small: the
//! pane layout around the terminals), and its big, rarely changing pieces
//! are cached views of their own ([`Part`]: sidebar, top bar, right panel,
//! Wall / Review, space bar, chips, pane headers): a terminal frame replays
//! their last paint, and the terminals that didn't change replay theirs. A part is
//! drawn by the screen's own functions and renders again whenever the
//! screen is notified (any change of the screen's state: what a cached
//! screen re-rendered on before), on a window refresh (focus, resize,
//! theme), and for its own hovers, scrolls and animations.

use std::collections::HashMap;

use gpui::{
    div, prelude::*, AnyElement, App, Context, Entity, SharedString, StyleRefinement, Subscription,
    WeakEntity, Window,
};

use super::{MainScreen, Route, SidebarMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PartKind {
    /// The docked, full-width sidebar.
    Sidebar,
    Topbar,
    /// The docked right panel (the selected agent's details).
    Panel,
    /// The Wall, Review or Explorer in the centre.
    Route,
    /// The space's bar (name, presets, density).
    Bar,
    /// A pane's header ([`Part::key`]: the pane id).
    Header,
    /// The space's chips (agents folded out of the layout).
    Chips,
    /// The status strip at the bottom.
    Strip,
}

pub(super) struct Part {
    screen: WeakEntity<MainScreen>,
    kind: PartKind,
    /// Which one, for kinds there are several of (a pane id).
    key: SharedString,
    _observe: Subscription,
    /// How often it rendered (tests).
    #[cfg(test)]
    pub(super) renders: usize,
}

/// The screen's parts, made on its first render.
pub(super) struct Parts {
    pub sidebar: Entity<Part>,
    pub topbar: Entity<Part>,
    pub panel: Entity<Part>,
    pub route: Entity<Part>,
    pub bar: Entity<Part>,
    pub chips: Entity<Part>,
    pub strip: Entity<Part>,
    /// Pane headers by pane id.
    headers: HashMap<String, Entity<Part>>,
}

impl Parts {
    pub fn new(screen: &Entity<MainScreen>, cx: &mut App) -> Parts {
        Parts {
            sidebar: Part::new(screen, PartKind::Sidebar, cx),
            topbar: Part::new(screen, PartKind::Topbar, cx),
            panel: Part::new(screen, PartKind::Panel, cx),
            route: Part::new(screen, PartKind::Route, cx),
            bar: Part::new(screen, PartKind::Bar, cx),
            chips: Part::new(screen, PartKind::Chips, cx),
            strip: Part::new(screen, PartKind::Strip, cx),
            headers: HashMap::new(),
        }
    }

    /// The header part of pane `pane_id` (made on first use).
    pub fn header(
        &mut self,
        pane_id: &str,
        screen: &Entity<MainScreen>,
        cx: &mut App,
    ) -> Entity<Part> {
        if let Some(h) = self.headers.get(pane_id) {
            return h.clone();
        }
        let h = Part::keyed(screen, PartKind::Header, pane_id.to_string().into(), cx);
        self.headers.insert(pane_id.to_string(), h.clone());
        h
    }

    /// The panes that have a header part (tests).
    #[cfg(test)]
    pub fn header_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.headers.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Keep only the headers of `panes` (the panes shown now).
    pub fn keep_headers(&mut self, panes: &[String], window: &Window) {
        self.headers.retain(|id, h| {
            let keep = panes.contains(id);
            if !keep {
                crate::kit::motion::pulse_forget(window, h.entity_id());
            }
            keep
        });
    }

    /// Drop the working-dot rings of the parts not drawn this frame.
    pub fn forget_unused(&self, drawn: &[PartKind], window: &Window) {
        let all = [
            (&self.sidebar, PartKind::Sidebar),
            (&self.topbar, PartKind::Topbar),
            (&self.panel, PartKind::Panel),
            (&self.route, PartKind::Route),
            (&self.bar, PartKind::Bar),
            (&self.chips, PartKind::Chips),
            (&self.strip, PartKind::Strip),
        ];
        for (p, kind) in all {
            if !drawn.contains(&kind) {
                crate::kit::motion::pulse_forget(window, p.entity_id());
            }
        }
    }
}

impl Part {
    fn new(screen: &Entity<MainScreen>, kind: PartKind, cx: &mut App) -> Entity<Part> {
        Part::keyed(screen, kind, SharedString::default(), cx)
    }

    fn keyed(
        screen: &Entity<MainScreen>,
        kind: PartKind,
        key: SharedString,
        cx: &mut App,
    ) -> Entity<Part> {
        cx.new(|cx| Part {
            screen: screen.downgrade(),
            kind,
            key,
            // Whatever re-renders the screen re-renders its parts.
            _observe: cx.observe(screen, |_, _, cx| cx.notify()),
            #[cfg(test)]
            renders: 0,
        })
    }
}

/// Whether the parts are drawn as cached views. Liquid Glass regions place
/// their native glass piece while they paint (a replayed paint would lose
/// it), so with them the screen draws everything itself.
pub fn cached(cx: &App) -> bool {
    !crate::theme::glass_regions(cx) && std::env::var_os("PITWALL_PERF_NO_PARTS").is_none()
}

/// `part` as a cached child laid out with `style` (its outer box: the
/// part's root element fills it).
pub(super) fn slot(part: &Entity<Part>, style: StyleRefinement) -> AnyElement {
    gpui::AnyView::from(part.clone())
        .cached(style)
        .into_any_element()
}

impl Render for Part {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let kind = self.kind;
        let key = self.key.clone();
        let drawn = self.screen.update(cx, |s, cx| {
            let off = s.modal.is_some() || s.elsewhere.bring.is_some();
            let t = crate::theme::theme(cx).clone();
            let el = match kind {
                PartKind::Sidebar => {
                    let agents = s.agents(cx).to_vec();
                    let slab = crate::theme::panel(&t, cx);
                    s.render_sidebar(SidebarMode::Full, &agents, &slab, window, cx)
                }
                PartKind::Topbar => {
                    let agents = s.agents(cx).to_vec();
                    let bp = MainScreen::breakpoint(window);
                    s.render_topbar(&agents, bp, &t.chrome(), window, cx)
                        .into_any_element()
                }
                PartKind::Panel => match s.selected(cx) {
                    Some(a) => {
                        let slab = crate::theme::panel(&t, cx);
                        s.render_panel(&a, false, &slab, window, cx)
                            .into_any_element()
                    }
                    None => div().into_any_element(),
                },
                PartKind::Route if s.route != Route::Space => s.render_route_body(&t, window, cx),
                PartKind::Route => div().into_any_element(),
                PartKind::Bar => match s.active_space().cloned() {
                    Some(space) => {
                        let agents = s.agents(cx).to_vec();
                        let f = s.fitted(&space, &agents);
                        s.space_bar(&space, &f, &t, cx).into_any_element()
                    }
                    None => div().into_any_element(),
                },
                PartKind::Header => s.header_of(&key, &t, cx),
                PartKind::Chips => s.chips_now(&t, cx),
                PartKind::Strip => {
                    let bp = MainScreen::breakpoint(window);
                    s.render_strip(&t.chrome(), bp, cx).into_any_element()
                }
            };
            (el, off)
        });
        match drawn {
            // Its working dots hand their rings to the pulse layer, as the
            // screen's do (kit::motion::pulse_scope).
            Ok((el, off)) => {
                crate::kit::motion::pulse_scope(div().size_full().child(el), off).into_any_element()
            }
            Err(_) => div().into_any_element(),
        }
    }
}
