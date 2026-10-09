//! The Wall in the main window: the main screen's Wall route
//! ([`crate::main_screen::Route::Wall`], ⌘E and the top bar's Wall). Its
//! folded sections are `ui.wallCollapsed`, its font the terminals'
//! (`ui.fontSize`), its look Flat or Glass; a tile (or Enter) shows that
//! agent in the main screen, Back / Esc leaves. While another route is
//! shown it watches no screens. (Whether the Wall is on is `ui.wall`,
//! which the main screen keeps.)

use std::collections::BTreeSet;

use gpui::{
    div, prelude::*, AppContext, Context, Entity, Focusable, IntoElement, Render, Subscription,
    WeakEntity, Window,
};

use super::{Look, WallEvent, WallView};
use crate::main_screen::{MainScreen, Route, ScreenEvent};

/// The Wall route's view.
pub struct WallRoute {
    wall: Entity<WallView>,
    screen: WeakEntity<MainScreen>,
    _subs: Vec<Subscription>,
}

impl WallRoute {
    fn new(
        store: Entity<crate::agents::AgentStore>,
        screen: &Entity<MainScreen>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> WallRoute {
        let wall = cx.new(|cx| WallView::new(store, window, cx));
        let ui = crate::ui_state::store(cx);
        let subs = vec![
            cx.subscribe_in(&wall, window, Self::wall_event),
            cx.subscribe_in(screen, window, |this, _, e: &ScreenEvent, window, cx| {
                if let ScreenEvent::RouteChanged(r) = e {
                    if *r == Route::Wall {
                        window.focus(&this.wall.focus_handle(cx));
                    } else {
                        this.wall.update(cx, |w, _| w.stop_watching());
                        // Hidden, it can't keep the keyboard: let it go so
                        // the main screen takes it back when it draws.
                        if this.wall.focus_handle(cx).contains_focused(window, cx) {
                            window.blur();
                        }
                    }
                }
            }),
            cx.observe(&ui, |this, _, cx| this.follow_settings(cx)),
            cx.observe_global::<crate::theme::Theme>(|this, cx| this.follow_settings(cx)),
        ];
        let mut route = WallRoute {
            wall,
            screen: screen.downgrade(),
            _subs: subs,
        };
        route.follow_settings(cx);
        route
    }

    pub fn wall(&self) -> &Entity<WallView> {
        &self.wall
    }

    /// `ui.wallCollapsed`, `ui.fontSize` and the look, into the Wall.
    fn follow_settings(&mut self, cx: &mut Context<Self>) {
        let ui = crate::ui_state::get(cx);
        let collapsed: BTreeSet<String> = ui.wall_collapsed.iter().cloned().collect();
        let look = if crate::theme::theme(cx).is_glass() {
            Look::Glass
        } else {
            Look::Flat
        };
        let font = crate::theme::clamp_font(ui.font_size.round() as i32) as f32;
        self.wall.update(cx, |w, cx| {
            if w.collapsed() != &collapsed {
                w.set_collapsed(collapsed, cx);
            }
            if w.look() != look {
                w.set_look(look, cx);
            }
            if w.font_size() != font {
                w.set_font_size(font, cx);
            }
        });
    }

    fn wall_event(
        &mut self,
        _: &Entity<WallView>,
        e: &WallEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match e {
            WallEvent::Open(id) => {
                let id = id.clone();
                if let Some(s) = self.screen.upgrade() {
                    s.update(cx, |s, cx| s.show_agent(&id, window, cx));
                }
            }
            WallEvent::Exit => {
                if let Some(s) = self.screen.upgrade() {
                    s.update(cx, |s, cx| s.set_route(Route::Space, cx));
                }
            }
            WallEvent::Collapsed(keys) => {
                let keys: Vec<String> = keys.iter().cloned().collect();
                crate::ui_state::update(cx, |s| s.wall_collapsed = keys);
            }
        }
    }
}

impl Render for WallRoute {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .size_full()
            .flex()
            .child(self.wall.clone())
    }
}

/// Put the Wall in `screen`'s window as its Wall route (built the first
/// time the Wall is shown).
pub fn mount(screen: &Entity<MainScreen>, cx: &mut gpui::App) {
    let screen = screen.downgrade();
    crate::main_screen::register_route(cx, Route::Wall, move |ctx, window, cx| {
        // Built while the screen draws: it is not read here.
        let screen = ctx.screen.upgrade().or(screen.upgrade());
        let screen = screen.expect("the main screen builds its routes");
        cx.new(|cx| WallRoute::new(ctx.store, &screen, window, cx))
            .into()
    });
}
