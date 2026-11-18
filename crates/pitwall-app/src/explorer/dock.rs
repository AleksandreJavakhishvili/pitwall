//! Files tiles: an agent's file viewer as a tile of a space's layout, a
//! peer of the terminal tiles (`main_screen::tree`: a pane with `files`).
//! Where the tile sits is the layout's (`ui.json`, with the rest of the
//! space); this module keeps the viewers themselves, one per agent across
//! the app, so a viewer moves between a tile, another tile and a window of
//! its own as the same viewer: its tabs, scroll and unsaved edits go with
//! it, and nothing asks to save since nothing closes.
//!
//! What a tile has open is kept in `ui.json` (`dockedFiles`) and opened
//! again when the tile is drawn after a restart.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    anchored, deferred, div, prelude::*, px, AnyElement, AnyView, App, Entity, Global, Pixels, Point, Subscription, Task,
    Window, WindowHandle,
};

use super::source::Source;
use super::tree::TreeModel;
use super::viewer::{Host, Opened, Viewer, ViewerEvent};
use crate::agents::AgentStore;
use crate::main_screen::tree::Side;
use crate::main_screen::workspace::{whereabouts, DockedFiles, FilesSpot, Whereabouts, FILES_SHARE};
use crate::theme::Theme;
use crate::ui::MainView;

/// Where files go, as people (and agents) say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesPlace {
    /// On that side of a terminal's tile (the files' agent's, unless
    /// another is named).
    Next(Side),
    /// Along that edge of the main area: a column or row of its own.
    Edge(Side),
    /// A tile of its own where there is most room.
    Own,
    /// A window of its own.
    Window,
}

impl FilesPlace {
    /// What the menu says.
    pub fn label(self, agent: &str) -> String {
        match self {
            FilesPlace::Next(Side::Right) => format!("Right of {agent}'s terminal"),
            FilesPlace::Next(Side::Left) => format!("Left of {agent}'s terminal"),
            FilesPlace::Next(Side::Top) => format!("Above {agent}'s terminal"),
            FilesPlace::Next(Side::Bottom) => format!("Below {agent}'s terminal"),
            FilesPlace::Edge(Side::Right) => "Column on the right".into(),
            FilesPlace::Edge(Side::Left) => "Column on the left".into(),
            FilesPlace::Edge(Side::Top) => "Row along the top".into(),
            FilesPlace::Edge(Side::Bottom) => "Row along the bottom".into(),
            FilesPlace::Own => "Tile of its own".into(),
            FilesPlace::Window => "Window of its own".into(),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            FilesPlace::Next(Side::Left) | FilesPlace::Edge(Side::Left) => "page-left",
            FilesPlace::Next(Side::Top | Side::Bottom) | FilesPlace::Edge(Side::Top | Side::Bottom) => "page-below",
            FilesPlace::Own => "grid",
            FilesPlace::Window => "window",
            _ => "page-right",
        }
    }
}

/// What a viewer needs to be made: the app's file source and agents.
#[derive(Clone)]
struct Env {
    source: Arc<dyn Source>,
    store: Entity<AgentStore>,
}

impl Global for Env {}

/// The source and agents viewers are made with (the first explorer's).
pub fn remember_env(source: Option<Arc<dyn Source>>, store: &Entity<AgentStore>, cx: &mut App) {
    if let (Some(source), false) = (source, cx.has_global::<Env>()) {
        cx.set_global(Env { source, store: store.clone() });
    }
}

struct Tile {
    key: String,
    viewer: Entity<Viewer>,
    /// What was kept in `ui.json` last.
    kept: Opened,
    _subs: Vec<Subscription>,
}

/// The files tiles' viewers, by agent.
#[derive(Default)]
struct Tiles {
    tiles: HashMap<String, Tile>,
    /// The terminal (agent) each files tile was placed against, by owner:
    /// what its place is said from while it still touches it.
    anchors: HashMap<String, String>,
    /// Agents seen: their names, and their folders when on this computer
    /// (what is left of one once it is removed).
    known: HashMap<String, (String, Option<String>)>,
    /// Files kept open after their agent was removed, for unsaved edits.
    orphans: HashMap<String, Orphan>,
    poll: Option<Task<()>>,
}

/// Files of a removed agent, kept for their unsaved edits.
struct Orphan {
    /// The agent's name.
    name: String,
    /// The folder they are read from now (for people).
    folder: String,
    /// Closes them once nothing is unsaved.
    _sub: Subscription,
}

impl Global for Tiles {}

fn tiles(cx: &mut App) -> &mut Tiles {
    if !cx.has_global::<Tiles>() {
        cx.set_global(Tiles::default());
    }
    cx.global_mut::<Tiles>()
}

/// Agent `owner`'s files tile's viewer, if one is made.
pub fn viewer(owner: &str, cx: &App) -> Option<Entity<Viewer>> {
    cx.try_global::<Tiles>()?.tiles.get(owner).map(|t| t.viewer.clone())
}

/// What agent `owner`'s files tile reads.
pub fn key(owner: &str, cx: &App) -> Option<String> {
    cx.try_global::<Tiles>()?.tiles.get(owner).map(|t| t.key.clone())
}

/// The window, space and tile showing agent `owner`'s files.
pub fn holder(owner: &str, cx: &App) -> Option<(String, String, String)> {
    let ui = crate::ui_state::get(cx);
    let (space, pane) = ui.files_tile(owner)?;
    Some((ui.window_of_space(&space).to_string(), space, pane))
}

/// The view a files tile draws: the viewer (made again from what was kept
/// after a restart). `None`: it can't be read from here (its agent gone).
pub fn view_for(owner: &str, window: &mut Window, cx: &mut App) -> Option<AnyView> {
    if let Some(v) = viewer(owner, cx) {
        return Some(v.into());
    }
    let kept = crate::ui_state::get(cx).docked_files.get(owner).cloned();
    let key = kept.as_ref().map_or_else(|| owner.to_string(), |k| k.key.clone());
    let v = make(owner, &key, window, cx)?;
    if let Some(k) = kept {
        let opened = Opened { tabs: k.tabs, active: k.active, pane_files: true };
        v.update(cx, |v, cx| v.restore(&opened, cx));
    }
    adopt(owner, &key, v.clone(), cx);
    Some(v.into())
}

/// A new viewer of `key` for agent `owner`'s files tile.
pub fn make(owner: &str, key: &str, window: &mut Window, cx: &mut App) -> Option<Entity<Viewer>> {
    let env = cx.try_global::<Env>()?.clone();
    if super::source::root_of_key(key).is_none() && env.store.read(cx).agent(key).is_none() {
        return None;
    }
    let ignored = crate::ui_state::get(cx).explorer_show_ignored;
    let tree = cx.new(|_| TreeModel::new(env.source.clone(), key.to_string(), ignored));
    tree.update(cx, |t, cx| t.refresh(false, cx));
    let (store, k) = (env.store.clone(), key.to_string());
    let owner_name = env.store.read(cx).agent(owner).map(|a| a.name.clone());
    let v = cx.new(|cx| {
        let mut v = Viewer::new(env.source.clone(), store, k, tree, window, cx);
        v.set_host(Host::Tile, owner_name, cx);
        v
    });
    Some(v)
}

/// Keep `viewer` (made here, or moved in from a window) as agent `owner`'s
/// files tile's.
pub fn adopt(owner: &str, key: &str, viewer: Entity<Viewer>, cx: &mut App) {
    let name = cx.try_global::<Env>().and_then(|e| e.store.read(cx).agent(owner).map(|a| a.name.clone()));
    viewer.update(cx, |v, cx| {
        v.set_host(Host::Tile, name, cx);
        v.set_can_dock(true, cx);
    });
    let o = owner.to_string();
    let events = cx.subscribe(&viewer, move |v, e: &ViewerEvent, cx| on_event(&o, &v, e, cx));
    let o = owner.to_string();
    let changed = cx.observe(&viewer, move |v, cx| keep_open(&o, &v, cx));
    let old = tiles(cx).tiles.insert(
        owner.to_string(),
        Tile { key: key.to_string(), viewer: viewer.clone(), kept: Opened::default(), _subs: vec![events, changed] },
    );
    if let Some(old) = old.filter(|o| o.viewer != viewer) {
        old.viewer.update(cx, |v, cx| v.hidden(cx));
    }
    keep_open(owner, &viewer, cx);
    start_poll(cx);
}

/// Agent `owner`'s files tile's viewer, let go of (moving to a window).
pub fn release(owner: &str, cx: &mut App) -> Option<(String, Entity<Viewer>)> {
    let t = tiles(cx).tiles.remove(owner)?;
    Some((t.key, t.viewer))
}

/// What the tile has open, kept in `ui.json` when it changed.
fn keep_open(owner: &str, viewer: &Entity<Viewer>, cx: &mut App) {
    let opened = viewer.read(cx).opened();
    let Some(t) = tiles(cx).tiles.get_mut(owner) else { return };
    if t.kept == opened {
        return;
    }
    t.kept = opened.clone();
    let open = DockedFiles { key: t.key.clone(), tabs: opened.tabs, active: opened.active };
    let o = owner.to_string();
    // (Not while the viewer is being updated: after it.)
    cx.defer(move |cx| {
        if crate::ui_state::get(cx).docked_files.get(&o) != Some(&open) {
            crate::ui_state::update(cx, |ui| *ui = ui.clone().keep_files(&o, open));
        }
    });
}

/// Every 10 s while a window shows: the tiles' trees read their open
/// folders again, quietly.
fn start_poll(cx: &mut App) {
    if tiles(cx).poll.is_some() {
        return;
    }
    let task = cx.spawn(async move |cx| loop {
        cx.background_executor().timer(super::TREE_POLL).await;
        crate::platform::visible::until_any_visible(cx).await;
        cx.update(|cx| {
            let trees: Vec<Entity<TreeModel>> = cx
                .try_global::<Tiles>()
                .map(|t| t.tiles.values().map(|t| t.viewer.read(cx).tree()).collect())
                .unwrap_or_default();
            for t in trees {
                t.update(cx, |t, cx| t.refresh(true, cx));
            }
        });
    });
    tiles(cx).poll = Some(task);
}

fn on_event(owner: &str, v: &Entity<Viewer>, e: &ViewerEvent, cx: &mut App) {
    let owner = owner.to_string();
    match e {
        ViewerEvent::Close => cx.defer(move |cx| close(&owner, cx)),
        ViewerEvent::Undock => cx.defer(move |cx| {
            if let Err(e) = pop_out(&owner, None, cx) {
                eprintln!("pitwall: could not open the file viewer window: {e}");
            }
        }),
        ViewerEvent::Place(p) => {
            let p = *p;
            cx.defer(move |cx| {
                if let Err(e) = place(&owner, p, None, cx) {
                    eprintln!("pitwall: could not move the files: {e}");
                }
            })
        }
        ViewerEvent::Grip => {
            if let Some(h) = holder_window(&owner, cx) {
                let _ = h.update(cx, |_, window, cx| super::drag::start(&owner, super::drag::From::Tile, window, cx));
            }
        }
        ViewerEvent::QuickOpen | ViewerEvent::ShowDiff(_) | ViewerEvent::Opened(_) => {
            let Some(h) = holder_window(&owner, cx) else { return };
            let v = v.clone();
            let key = key(&owner, cx).unwrap_or_default();
            let _ = h.update(cx, move |view, window, cx| {
                let Some(ex) = view.explorer() else { return };
                ex.update(cx, |ex, cx| match e {
                    ViewerEvent::QuickOpen => ex.quick_open_into(&v, &key, window, cx),
                    ViewerEvent::ShowDiff(path) => ex.show_diff_of(&key, path, cx),
                    ViewerEvent::Opened(path) => ex.opened_in(&key, path),
                    _ => {}
                });
            });
        }
        ViewerEvent::SetIgnored(on) => {
            let on = *on;
            crate::ui_state::update(cx, |s| s.explorer_show_ignored = on);
            v.read(cx).tree().update(cx, |t, cx| t.set_ignored(on, cx));
        }
        ViewerEvent::Exit => {
            // Esc: the keyboard back to the terminal.
            if let Some(h) = holder_window(&owner, cx) {
                let v = v.clone();
                let _ = h.update(cx, move |_, window, cx| {
                    if gpui::Focusable::focus_handle(v.read(cx), cx).contains_focused(window, cx) {
                        window.blur(cx);
                    }
                });
            }
        }
        ViewerEvent::Pin => {}
    }
}

/// The agents in Pitwall now are `alive`. The files of a removed one close:
/// its tile leaves the layout (which closes the gap, as for a terminal),
/// its window closes, `dockedFiles` forgets it. Unless they hold unsaved
/// edits: then they stay, reading the agent's folder on this computer (a
/// `path:` key), with a note and Save / Discard ([`orphan_bar`]), and
/// close once saved or discarded. An agent on another machine leaves
/// nothing to save there: its files just close. A restart or resume keeps
/// the agent's id, so nothing happens then. Returns the agents whose files
/// stay so (their tiles are kept in the layout).
pub fn agents_gone(alive: &[String], cx: &mut App) -> Vec<String> {
    // Who is here: kept for when one goes.
    if let Some(env) = cx.try_global::<Env>().cloned() {
        let seen: Vec<(String, (String, Option<String>))> = env
            .store
            .read(cx)
            .agents
            .iter()
            .map(|a| {
                let here = a.machine.provider == pitwall_core::provider::ProviderId::LOCAL && !crate::agents::mirrored(a);
                (a.id.clone(), (a.name.clone(), here.then(|| a.cwd.clone())))
            })
            .collect();
        tiles(cx).known.extend(seen);
    }
    let ui = crate::ui_state::get(cx);
    let mut owners: Vec<String> = ui.spaces.iter().flat_map(|s| s.layout.files_owners()).collect();
    owners.extend(ui.docked_files.keys().cloned());
    owners.extend(tiles(cx).tiles.keys().cloned());
    owners.extend(crate::windows::viewer_window_owners(cx));
    owners.sort();
    owners.dedup();
    for o in owners.iter().filter(|o| !alive.contains(o)) {
        if tiles(cx).orphans.contains_key(o) {
            continue;
        }
        if !keep_for_edits(o, alive, cx) {
            close_everywhere(o, cx);
        }
    }
    tiles(cx).orphans.keys().cloned().collect()
}

/// Removed agent `owner`'s files (tile or window) stay when they hold
/// unsaved edits that can still be saved here. Whether they do.
fn keep_for_edits(owner: &str, alive: &[String], cx: &mut App) -> bool {
    let window = crate::windows::viewer_window(owner, cx);
    let (key, v) = match tiles(cx).tiles.get(owner) {
        Some(t) => (t.key.clone(), t.viewer.clone()),
        None => match window.and_then(|h| h.read(cx).ok().map(|w| (w.key().to_string(), w.viewer().clone()))) {
            Some(kv) => kv,
            None => return false,
        },
    };
    if v.read(cx).unsaved(cx).is_empty() {
        return false;
    }
    let known = tiles(cx).known.clone();
    let name = known.get(owner).map_or_else(|| owner.to_string(), |k| k.0.clone());
    // What they read now: a folder, an agent still here, else the removed
    // agent's folder on this computer (none: nothing to save to here).
    let next = if super::source::root_of_key(&key).is_some() || alive.contains(&key) {
        key.clone()
    } else {
        match known.get(&key).and_then(|k| k.1.clone()) {
            Some(folder) => super::source::path_key(&folder),
            None => return false,
        }
    };
    if next != key {
        v.update(cx, |v, cx| v.reroot(next.clone(), cx));
        match tiles(cx).tiles.get_mut(owner) {
            Some(t) => {
                t.key = next.clone();
                // (Kept in `ui.json` again, with the new key.)
                t.kept = Opened::default();
            }
            None => {
                if let Some(h) = window {
                    let k = next.clone();
                    let _ = h.update(cx, |w, _, cx| w.rerooted(&k, cx));
                }
            }
        }
    }
    let folder = super::show::root_display(&next, cx);
    let o = owner.to_string();
    let sub = cx.observe(&v, move |v, cx| {
        if v.read(cx).unsaved(cx).is_empty() {
            let o = o.clone();
            cx.defer(move |cx| {
                if tiles(cx).orphans.remove(&o).is_some() {
                    close_everywhere(&o, cx);
                }
            });
        }
    });
    tiles(cx).orphans.insert(owner.to_string(), Orphan { name, folder, _sub: sub });
    keep_open(owner, &v, cx);
    v.update(cx, |_, cx| cx.notify());
    true
}

/// Agent `owner`'s files closed wherever they are, and forgotten.
fn close_everywhere(owner: &str, cx: &mut App) {
    if let Some((_, v)) = release(owner, cx) {
        v.update(cx, |v, cx| v.hidden(cx));
    }
    tiles(cx).anchors.remove(owner);
    crate::ui_state::update(cx, |ui| *ui = ui.clone().undock_files(owner).forget_files(owner));
    if let Some(h) = crate::windows::viewer_window(owner, cx) {
        // (Not inside the update that found it gone.)
        cx.defer(move |cx| {
            let _ = h.update(cx, |_, window, _| window.remove_window());
        });
    }
}

/// Removed agent `owner`'s files are kept for their unsaved edits.
pub fn orphaned(owner: &str, cx: &App) -> bool {
    cx.try_global::<Tiles>().is_some_and(|t| t.orphans.contains_key(owner))
}

/// The note over files kept after their agent was removed, with Save
/// (every unsaved file; they close once written) and Discard (they close,
/// the edits dropped). `None`: not such files.
pub fn orphan_bar(owner: &str, viewer: &Entity<Viewer>, t: &Theme, cx: &App) -> Option<AnyElement> {
    let o = cx.try_global::<Tiles>()?.orphans.get(owner)?;
    let text = format!(
        "{} was removed from Pitwall. Its files stay open for your unsaved edits, read from {}: save or discard them, and they close.",
        o.name, o.folder
    );
    let (save, discard) = (viewer.clone(), owner.to_string());
    // (The look of the viewer's changed-on-disk bar.)
    Some(
        Viewer::notice("files-orphan", true, text.into(), t)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(6.))
                    .child(crate::kit::small_btn("files-orphan-save", "Save", t).on_click(move |_, _, cx| {
                        save.update(cx, |v, cx| v.save_all(cx)).detach();
                    }))
                    .child(crate::kit::small_btn("files-orphan-discard", "Discard", t).on_click(move |_, _, cx| discard_orphan(&discard, cx))),
            )
            .into_any_element(),
    )
}

/// Discard: removed agent `owner`'s kept files close, their edits dropped.
pub fn discard_orphan(owner: &str, cx: &mut App) {
    if tiles(cx).orphans.remove(owner).is_some() {
        close_everywhere(owner, cx);
    }
}

/// Save: removed agent `owner`'s kept files written (they close once
/// nothing is unsaved).
pub fn save_orphan(owner: &str, cx: &mut App) {
    let v = match tiles(cx).tiles.get(owner) {
        Some(t) => Some(t.viewer.clone()),
        None => crate::windows::viewer_window(owner, cx).and_then(|h| h.read(cx).ok().map(|w| w.viewer().clone())),
    };
    if let Some(v) = v.filter(|_| orphaned(owner, cx)) {
        v.update(cx, |v, cx| v.save_all(cx)).detach();
    }
}

/// The Pitwall window showing agent `owner`'s files tile.
fn holder_window(owner: &str, cx: &mut App) -> Option<WindowHandle<MainView>> {
    let (label, _, _) = holder(owner, cx)?;
    crate::windows::main_window(&label, cx)
}

/// Close agent `owner`'s files tile (its unsaved edits were asked about).
pub fn close(owner: &str, cx: &mut App) {
    if let Some((_, v)) = release(owner, cx) {
        v.update(cx, |v, cx| v.hidden(cx));
    }
    crate::ui_state::update(cx, |ui| *ui = ui.clone().undock_files(owner).forget_files(owner));
}

/// Where agent `owner`'s files go for `place`, near agent `of`'s terminal
/// (`None`: the owner's): the window, its space and the spot.
pub fn spot_for(owner: &str, place: FilesPlace, of: Option<&str>, cx: &mut App) -> Result<(String, String, FilesSpot), String> {
    let anchor = of.unwrap_or(owner);
    let ui = crate::ui_state::get(cx);
    // A space shown in an open window, holding the anchor's terminal.
    let in_window = |label: &str, agent: &str| {
        ui.spaces
            .iter()
            .filter(|sp| ui.window_of_space(&sp.id) == label)
            .find_map(|sp| sp.layout.find_by_agent(agent).map(|p| (sp.id.clone(), p.id)))
    };
    match place {
        FilesPlace::Window => Err("a window is not a spot in the layout".into()),
        FilesPlace::Next(side) => {
            let (label, _) = crate::windows::showing(anchor, cx).ok_or_else(|| "no open Pitwall window shows that terminal".to_string())?;
            let (space, pane) = in_window(&label, anchor).ok_or_else(|| "no open Pitwall window shows that terminal".to_string())?;
            Ok((label, space, FilesSpot::Tile { pane, side }))
        }
        FilesPlace::Edge(_) | FilesPlace::Own => {
            // Where the files are now; else the space showing the agent.
            let (label, space) = match holder(owner, cx) {
                Some((l, s, _)) => (l, s),
                None => {
                    let (label, _) = crate::windows::showing(anchor, cx).ok_or_else(|| "no open Pitwall window shows that terminal".to_string())?;
                    let (space, _) = in_window(&label, anchor).ok_or_else(|| "no open Pitwall window shows that terminal".to_string())?;
                    (label, space)
                }
            };
            let spot = match place {
                FilesPlace::Edge(side) => FilesSpot::Edge(side),
                _ => FilesSpot::Auto,
            };
            Ok((label, space, spot))
        }
    }
}

/// Put agent `owner`'s files at `spot` of space `space` (in window
/// `label`): its tile moved, its window's viewer moved in (the window
/// closes), or a new viewer reading `new_key` (`None`: the owner's
/// folder). Returns the viewer. The keyboard stays where it is.
pub fn dock(owner: &str, label: &str, space: &str, spot: &FilesSpot, new_key: Option<&str>, cx: &mut App) -> Result<Entity<Viewer>, String> {
    let handle = crate::windows::main_window(label, cx).ok_or("that Pitwall window is gone")?;
    // The viewer: the tile's, the window's, or a new one.
    let viewer = match viewer(owner, cx) {
        Some(v) if new_key.is_none_or(|k| key(owner, cx).as_deref() == Some(k)) => v,
        _ => {
            let from_window = crate::windows::viewer_window(owner, cx).and_then(|w| {
                let k = w.read(cx).ok()?.key().to_string();
                new_key.is_none_or(|n| n == k).then_some((w, k))
            });
            match from_window {
                Some((w, k)) => {
                    let v = w.update(cx, |w, _, cx| w.take_viewer(cx)).map_err(|_| "the files window is gone")?;
                    // (Closed after this update: it isn't drawn again.)
                    cx.defer(move |cx| {
                        let _ = w.update(cx, |_, window, _| window.remove_window());
                    });
                    adopt(owner, &k, v.clone(), cx);
                    v
                }
                None => {
                    let k = new_key.unwrap_or(owner).to_string();
                    let v = handle
                        .update(cx, |_, window, cx| make(owner, &k, window, cx))
                        .map_err(|_| "that Pitwall window is gone")?
                        .ok_or("its files can't be read from here")?;
                    adopt(owner, &k, v.clone(), cx);
                    v
                }
            }
        }
    };
    let share = FILES_SHARE;
    let fit = handle
        .update(cx, |view, _, cx| view.screen().and_then(|s| s.read(cx).fit_for(space)))
        .ok()
        .flatten();
    // The terminal it is placed against, if any (its place is said from it).
    let anchor = match spot {
        FilesSpot::Tile { pane, .. } => crate::ui_state::get(cx)
            .space(space)
            .and_then(|sp| sp.layout.leaves().into_iter().find(|p| p.id == *pane && p.files.is_none()))
            .and_then(|p| p.agent_id),
        _ => None,
    };
    match anchor {
        Some(a) => tiles(cx).anchors.insert(owner.to_string(), a),
        None => tiles(cx).anchors.remove(owner),
    };
    let mut placed = None;
    crate::ui_state::update(cx, |ui| {
        let (next, tile) = ui.clone().dock_files(space, owner, spot, share, fit);
        placed = tile;
        *ui = next;
    });
    if placed.is_none() && crate::ui_state::get(cx).files_tile(owner).is_none() {
        return Err("that spot isn't in the layout".into());
    }
    // That space shown in its window (nothing comes forward).
    let space = space.to_string();
    let _ = handle.update(cx, |view, _, cx| {
        if let Some(s) = view.screen() {
            s.update(cx, |s, cx| {
                if s.current_space().map(|c| c.id) != Some(space.clone()) {
                    s.show_space(&space, cx);
                }
                cx.notify();
            });
        }
    });
    Ok(viewer)
}

/// Agent `owner`'s files tile into a window of its own (at `at` on screen,
/// else where the last one was): the same viewer.
pub fn pop_out(owner: &str, at: Option<Point<Pixels>>, cx: &mut App) -> Result<WindowHandle<super::window::ViewerWindow>, String> {
    let (key, v) = release(owner, cx).ok_or("no files tile")?;
    crate::ui_state::update(cx, |ui| *ui = ui.clone().undock_files(owner).forget_files(owner));
    let front = crate::platform::app_in_front(cx);
    let h = super::window::open_with(owner, &key, Some(v.clone()), at, front, cx)?;
    if front {
        let _ = h.update(cx, |_, window, cx| v.update(cx, |v, cx| v.show(None, window, cx)));
    }
    Ok(h)
}

/// Move agent `owner`'s files (tile or window) to `place`, near agent
/// `of`'s terminal. Returns where, in words.
pub fn place(owner: &str, place: FilesPlace, of: Option<&str>, cx: &mut App) -> Result<String, String> {
    if place == FilesPlace::Window {
        if viewer(owner, cx).is_some() {
            pop_out(owner, None, cx)?;
        }
        return Ok("a window of its own".into());
    }
    let (label, space, spot) = spot_for(owner, place, of, cx)?;
    dock(owner, &label, &space, &spot, None, cx)?;
    Ok(described(owner, cx))
}

/// Where agent `owner`'s files tile is, in words ("a files tile in space
/// Main (window main), above api's terminal").
pub fn described(owner: &str, cx: &App) -> String {
    let ui = crate::ui_state::get(cx);
    let Some((space, pane)) = ui.files_tile(owner) else { return "nowhere".into() };
    let Some(sp) = ui.space(&space) else { return "nowhere".into() };
    let at = whereabouts(&sp.layout, &pane, anchor(owner, cx).as_deref()).words(&|id: &str| agent_name(id, cx));
    format!("a files tile in space {} (window {}), {at}", sp.name, ui.window_of_space(&space))
}

/// Where agent `owner`'s files tile is (see [`whereabouts`]).
pub fn whereabouts_of(owner: &str, cx: &App) -> Option<Whereabouts> {
    let ui = crate::ui_state::get(cx);
    let (space, pane) = ui.files_tile(owner)?;
    Some(whereabouts(&ui.space(&space)?.layout, &pane, anchor(owner, cx).as_deref()))
}

/// The terminal agent `owner`'s files tile was placed against.
fn anchor(owner: &str, cx: &App) -> Option<String> {
    cx.try_global::<Tiles>()?.anchors.get(owner).cloned()
}

/// Agent `id`'s name (its id when unknown).
pub fn agent_name(id: &str, cx: &App) -> String {
    cx.try_global::<Env>()
        .and_then(|e| e.store.read(cx).agent(id).map(|a| a.name.clone()))
        .unwrap_or_else(|| id.to_string())
}

/// The "Dock…" (in a window) / "Move…" (a tile) menu at `at`: `pick` hears
/// the place (`None`: dismissed).
pub fn menu(
    at: Point<Pixels>,
    tile: bool,
    agent: Option<String>,
    t: &Theme,
    pick: impl Fn(Option<FilesPlace>, &mut App) + Clone + 'static,
) -> AnyElement {
    let agent = agent.unwrap_or_else(|| "the agent".into());
    let mut places = vec![
        FilesPlace::Next(Side::Right),
        FilesPlace::Next(Side::Left),
        FilesPlace::Next(Side::Top),
        FilesPlace::Next(Side::Bottom),
        FilesPlace::Edge(Side::Right),
        FilesPlace::Edge(Side::Bottom),
        FilesPlace::Own,
    ];
    if tile {
        places.push(FilesPlace::Window);
    }
    let dismiss = pick.clone();
    let rows = places.into_iter().enumerate().map(move |(i, p)| {
        let pick = pick.clone();
        crate::kit::menu::menu_item(("files-place", i), p.icon(), p.label(&agent), None, false, t)
            .debug_selector(move || format!("files-place-{i}"))
            .on_click(move |_, _, cx| pick(Some(p), cx))
    });
    deferred(
        anchored().position(at).snap_to_window().child(
            crate::kit::menu::menu_panel(t)
                .id("files-place-menu")
                .debug_selector(|| "files-place-menu".into())
                .mt(px(6.))
                .child(
                    div()
                        .px(px(8.))
                        .py(px(4.))
                        .text_size(px(11.))
                        .text_color(t.text_3)
                        .child(if tile { "Move the files" } else { "Place the files in the layout" }),
                )
                .children(rows)
                .on_mouse_down_out(move |_, _, cx| dismiss(None, cx)),
        ),
    )
    .with_priority(2)
    .into_any_element()
}
