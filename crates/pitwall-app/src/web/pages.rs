//! The web pages, by tile id, across the app: one [`WebPage`] per entry of
//! `ui.json`'s `webTiles`, whoever draws it (a Pitwall window's layout, or
//! a window of its own), so a page keeps its view and state as it moves.
//! An entry gone from `webTiles` (its tile closed, a Preview whose agent
//! went) drops its page, which frees its view; a window holding it closes.

use std::collections::HashMap;

use gpui::{App, AppContext, Entity, Global};
use pitwall_proto::AgentView;

use super::page::{Host, WebPage};

#[derive(Default)]
pub(crate) struct Pages {
    pages: HashMap<String, Entity<WebPage>>,
    /// Watching `ui.json` (pages gone) and the agents (Previews follow).
    watching: bool,
    /// A Preview couldn't be made and the user was told why (once).
    told: bool,
}

impl Global for Pages {}

fn pages(cx: &mut App) -> &mut Pages {
    if !cx.has_global::<Pages>() {
        cx.set_global(Pages::default());
    }
    cx.global_mut::<Pages>()
}

/// Page `id`, if made.
pub fn get(id: &str, cx: &App) -> Option<Entity<WebPage>> {
    cx.try_global::<Pages>()?.pages.get(id).cloned()
}

/// Every page made.
pub fn all(cx: &App) -> Vec<(String, Entity<WebPage>)> {
    cx.try_global::<Pages>().map(|p| p.pages.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default()
}

/// Page `id`, made from `ui.json` when it isn't yet (a restart, a tile
/// just placed; a Preview's agent from `agents`). `None`: no such entry.
pub fn ensure(id: &str, agents: &[AgentView], cx: &mut App) -> Option<Entity<WebPage>> {
    if let Some(p) = get(id, cx) {
        return Some(p);
    }
    let kept = crate::ui_state::get(cx).web_tiles.get(id).cloned()?;
    let agent = kept.agent.as_ref().and_then(|a| agents.iter().find(|x| x.id == *a).cloned());
    let page = cx.new(|cx| WebPage::new(id, &kept, agent.as_ref(), cx));
    pages(cx).pages.insert(id.to_string(), page.clone());
    watch(cx);
    Some(page)
}

/// Page `id` drawn by `host` (in `window`): it moves there.
pub fn hold(id: &str, host: Host, agents: &[AgentView], window: &mut gpui::Window, cx: &mut App) -> Option<Entity<WebPage>> {
    let page = ensure(id, agents, cx)?;
    page.update(cx, |p, cx| p.set_host(host, window, cx));
    Some(page)
}

/// A Preview couldn't be made; whether the user was told so already (and
/// now they have been).
pub fn told_once(cx: &mut App) -> bool {
    std::mem::replace(&mut pages(cx).told, true)
}

/// The app's agents (a window holding a page outside a layout).
pub fn agents(cx: &App) -> Vec<AgentView> {
    match cx.try_global::<crate::AppState>().map(|s| &s.content) {
        Some(crate::ui::Content::Live { store }) => store.read(cx).agents.clone(),
        _ => Vec::new(),
    }
}

/// Once: pages follow `ui.json` (gone with their entry).
fn watch(cx: &mut App) {
    if pages(cx).watching {
        return;
    }
    pages(cx).watching = true;
    let store = crate::ui_state::store(cx);
    cx.observe(&store, |_, cx| prune(cx)).detach();
}

/// The agents changed (the Pitwall windows hear it): Previews follow,
/// wherever they are (a second call for the same change does nothing).
pub fn follow_all(agents: &[AgentView], cx: &mut App) {
    for (_, p) in all(cx) {
        p.update(cx, |p, cx| p.follow_agents(agents, cx));
    }
}

/// Pages whose entry left `ui.json` go (their views with them), and the
/// windows holding them close.
pub fn prune(cx: &mut App) {
    let kept = crate::ui_state::get(cx).web_tiles.clone();
    let gone: Vec<String> = all(cx).into_iter().map(|(id, _)| id).filter(|id| !kept.contains_key(id)).collect();
    for id in gone {
        drop_page(&id, cx);
    }
}

/// Page `id` dropped: the keys back to its holder if it had them, its
/// window (if it is in one) closed.
fn drop_page(id: &str, cx: &mut App) {
    let Some(page) = pages(cx).pages.remove(id) else { return };
    let host = page.read(cx).host().cloned();
    if let Some(h) = host {
        let focus = page.read(cx).focus_handle().clone();
        let _ = h.window.update(cx, |_, window, cx| {
            if focus.contains_focused(window, cx) {
                window.focus(&h.focus, cx);
            }
        });
    }
    if let Some(w) = crate::windows::web_window(id, cx) {
        cx.defer(move |cx| {
            let _ = w.update(cx, |w, window, _| {
                w.closing();
                window.remove_window();
            });
        });
    }
    drop(page);
}

/// Page `id` closed (its tile's ×, its window's close): out of the layout
/// and `ui.json`, its view freed.
pub fn close(id: &str, cx: &mut App) {
    let id = id.to_string();
    crate::ui_state::update(cx, |s| *s = s.clone().close_web(&id));
    drop_page(&id, cx);
}

/// Page `id`, made by its holder before its `ui.json` entry (a Preview
/// whose view must be made first).
pub fn insert(id: &str, page: Entity<WebPage>, cx: &mut App) {
    pages(cx).pages.insert(id.to_string(), page);
    watch(cx);
}
