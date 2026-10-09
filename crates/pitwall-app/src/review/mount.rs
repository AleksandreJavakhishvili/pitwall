//! Review in the main window: the main screen's Review route
//! ([`crate::main_screen::Route::Review`]). It opens when the route does
//! (⌘R, the top bar's Review, the Changes panel's "Open in Review") on the
//! focused agent, or on one file ([`ReviewRoute::open_at`]: the explorer's
//! "Show diff"), scoped to the active space (`reviewScope`); it closes when
//! the route does. Esc / Back leaves it, "Open terminal" on a worktree
//! starts a shell there through the main screen.

use gpui::{
    div, prelude::*, App, AppContext, Context, Entity, IntoElement, Render, Subscription,
    WeakEntity, Window,
};

use super::model::Space;
use super::{Focus, ReviewEvent, ReviewSlot};
use crate::agents::AgentStore;
use crate::main_screen::workspace::{self, SpaceKind};
use crate::main_screen::{MainScreen, Route, ScreenEvent};

/// Review's scope for a main-screen space: All, or a project or custom
/// space with its members and the agents shown in its panes.
pub fn review_space(sp: Option<&workspace::Space>) -> Space {
    let Some(sp) = sp else { return Space::All };
    let mut members = sp.members.clone();
    for id in sp.layout.leaves().into_iter().filter_map(|p| p.agent_id) {
        if !members.contains(&id) {
            members.push(id);
        }
    }
    match sp.kind {
        SpaceKind::All => Space::All,
        SpaceKind::Project => Space::Project {
            name: sp.name.clone(),
            project: sp.project.clone().unwrap_or_default(),
            members,
        },
        SpaceKind::Custom => Space::Custom {
            name: sp.name.clone(),
            members,
        },
    }
}

/// The Review route's view: Review while the route is shown.
pub struct ReviewRoute {
    store: Entity<AgentStore>,
    screen: WeakEntity<MainScreen>,
    slot: ReviewSlot,
    /// The scope it was last given: (space, focused agent).
    context: Option<(Space, Option<String>)>,
    _view_sub: Option<Subscription>,
    _subs: Vec<Subscription>,
}

impl ReviewRoute {
    fn new(
        screen: &Entity<MainScreen>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ReviewRoute {
        let store = screen.read(cx).store().clone();
        let subs = vec![
            cx.subscribe_in(screen, window, |this, _, e: &ScreenEvent, window, cx| match e {
                ScreenEvent::RouteChanged(r) => {
                    if *r != Route::Review {
                        this.close(cx);
                    } else if !this.is_open() {
                        // (Already open when an entry point opened it on a file.)
                        this.open_at(None, window, cx);
                    }
                }
                // A sidebar worktree row: Review on that worktree.
                ScreenEvent::ReviewWorktree { project_id, path } => {
                    this.open_worktree(project_id.clone(), path.clone(), window, cx);
                }
                _ => {}
            }),
            // The space or the focused agent changed while Review is open.
            cx.observe(screen, |this, _, cx| this.follow(cx)),
        ];
        let mut route = ReviewRoute {
            store,
            screen: screen.downgrade(),
            slot: ReviewSlot::default(),
            context: None,
            _view_sub: None,
            _subs: subs,
        };
        // The window opened on Review.
        if screen.read(cx).route() == Route::Review {
            route.open_at(None, window, cx);
        }
        route
    }

    /// Whether Review is open.
    pub fn is_open(&self) -> bool {
        self.slot.view().is_some()
    }

    /// The open Review (tests, the host).
    pub fn view(&self) -> Option<&Entity<super::ReviewView>> {
        self.slot.view()
    }

    /// The window's scope now: its active space and focused agent.
    fn scope(&self, cx: &App) -> (Space, Option<String>) {
        match self.screen.upgrade() {
            Some(s) => {
                let s = s.read(cx);
                (
                    review_space(s.current_space().as_ref()),
                    s.focused_agent_id(),
                )
            }
            None => (Space::All, None),
        }
    }

    /// Open Review (or show `focus` in the open one): on `focus`, else on
    /// the focused agent when it can be reviewed.
    pub fn open_at(&mut self, focus: Option<Focus>, window: &mut Window, cx: &mut Context<Self>) {
        let (space, focused) = self.scope(cx);
        let focus = focus.or_else(|| {
            let id = focused.clone()?;
            let reviewable = self
                .store
                .read(cx)
                .agent(&id)
                .is_some_and(|a| a.caps.review);
            reviewable.then_some(Focus::Agent { id, path: None })
        });
        let was_open = self.is_open();
        self.context = Some((space.clone(), focused.clone()));
        let store = self.store.clone();
        self.slot.open(
            &store,
            space,
            focused,
            focus,
            |r: &mut ReviewRoute| &mut r.slot,
            window,
            cx,
        );
        if !was_open {
            if let Some(v) = self.slot.view().cloned() {
                self._view_sub = Some(cx.subscribe_in(&v, window, Self::review_event));
            }
        }
        cx.notify();
    }

    /// Review on one worktree of a project (the sidebar's worktree rows;
    /// the main screen switches to the Review route itself).
    pub fn open_worktree(
        &mut self,
        project_id: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_at(Some(Focus::Worktree { project_id, path }), window, cx);
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.is_open() {
            self.slot.close();
            self._view_sub = None;
            self.context = None;
            cx.notify();
        }
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        if !self.is_open() {
            return;
        }
        let now = self.scope(cx);
        if self.context.as_ref() != Some(&now) {
            self.slot.set_context(now.0.clone(), now.1.clone(), cx);
            self.context = Some(now);
        }
    }

    fn review_event(
        &mut self,
        _: &Entity<super::ReviewView>,
        e: &ReviewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(screen) = self.screen.upgrade() else {
            return;
        };
        match e {
            ReviewEvent::Exit => {
                self._view_sub = None;
                self.context = None;
                screen.update(cx, |s, cx| s.set_route(Route::Space, cx));
            }
            ReviewEvent::OpenTerminal(path) => {
                let path = path.clone();
                screen.update(cx, |s, cx| s.open_terminal(path, window, cx));
            }
        }
    }
}

impl Render for ReviewRoute {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .size_full()
            .flex()
            .children(self.slot.view().cloned())
    }
}

/// Put Review in `screen`'s window: its route view, opened and closed with
/// the route. The returned entity opens it on a file
/// ([`ReviewRoute::open_at`]).
pub fn mount<V: 'static>(
    screen: &Entity<MainScreen>,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Entity<ReviewRoute> {
    let route = cx.new(|cx| ReviewRoute::new(screen, window, cx));
    let r = route.clone();
    crate::main_screen::register_route(cx, Route::Review, move |_, _, _| r.clone().into());
    route
}
