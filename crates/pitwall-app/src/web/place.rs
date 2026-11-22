//! Where a web page goes (docs/spec/web.md): a spot of a Pitwall window's
//! layout (next to a terminal, along an edge, in a terminal's place), or a
//! window of its own; moved by the bar's "Move…" / "Dock…" menu, its grip
//! (from a window: [`crate::explorer::drag`], the same as files), "Open in
//! a window of its own", and agents (`super::show`). The page itself is the
//! same throughout ([`super::pages`]); only `ui.json` changes here. An
//! extension's inbox moves with the same places and spots
//! ([`holder_of`], [`spot_near`], [`land`], [`replaceable_near`]; its own
//! part is `crate::main_screen::inbox_window`).

use std::collections::HashMap;

use gpui::{App, Global, Pixels, Point, Window, WindowHandle};

use crate::explorer::dock::FilesPlace;
use crate::main_screen::tree::Content;
use crate::main_screen::workspace::{whereabouts, FilesSpot, Fit, UiState, Whereabouts};

/// The terminal (agent) each page was placed against, by tile id: what its
/// place is said from while it still touches it.
#[derive(Default)]
struct Anchors(HashMap<String, String>);

impl Global for Anchors {}

fn anchors(cx: &mut App) -> &mut HashMap<String, String> {
    if !cx.has_global::<Anchors>() {
        cx.set_global(Anchors::default());
    }
    &mut cx.global_mut::<Anchors>().0
}

/// The agent page `id` belongs to: a Preview's, else the agent that opened
/// it with its tool.
pub fn anchor_agent(id: &str, cx: &App) -> Option<String> {
    let ui = crate::ui_state::get(cx);
    let t = ui.web_tiles.get(id)?;
    t.agent.clone().or_else(|| t.by.clone())
}

/// The window, space and tile showing page `id` in a layout.
pub fn holder(id: &str, cx: &App) -> Option<(String, String, String)> {
    holder_of(&Content::Web(id.to_string()), cx)
}

/// The window, space and tile showing `what` (a page, an inbox) in a
/// layout.
pub fn holder_of(what: &Content, cx: &App) -> Option<(String, String, String)> {
    let ui = crate::ui_state::get(cx);
    let (space, pane) = ui.tile_of(what)?;
    Some((ui.window_of_space(&space).to_string(), space, pane))
}

/// The space showing agent `agent`'s terminal in the frontmost Pitwall
/// window that shows it: (window, space, its tile).
fn showing(agent: &str, cx: &mut App) -> Option<(String, String, String)> {
    let (label, _) = crate::windows::showing(agent, cx)?;
    let ui = crate::ui_state::get(cx);
    ui.spaces
        .iter()
        .filter(|sp| ui.window_of_space(&sp.id) == label)
        .find_map(|sp| sp.layout.find_by_agent(agent).map(|p| (label.clone(), sp.id.clone(), p.id)))
}

/// The frontmost Pitwall window and the space it shows.
fn front_space(cx: &mut App) -> Option<(String, String)> {
    let (label, h, _) = crate::windows::frames_front_to_back(cx).into_iter().next()?;
    let space = h.update(cx, |view, _, cx| view.screen().and_then(|s| s.read(cx).current_space().map(|c| c.id))).ok().flatten()?;
    Some((label, space))
}

/// Where page `id` goes for `place`, next to (or in place of) agent
/// `of`'s terminal (`None`: the page's agent's): the window, its space and
/// the spot.
pub fn spot_for(id: &str, place: FilesPlace, of: Option<&str>, cx: &mut App) -> Result<(String, String, FilesSpot), String> {
    let anchor = of.map(String::from).or_else(|| anchor_agent(id, cx));
    spot_near(&Content::Web(id.to_string()), place, anchor, cx)
}

/// Where `what` (a page, an inbox) goes for `place`, next to (or in place
/// of) agent `anchor`'s terminal: the window, its space and the spot.
pub fn spot_near(what: &Content, place: FilesPlace, anchor: Option<String>, cx: &mut App) -> Result<(String, String, FilesSpot), String> {
    match place {
        FilesPlace::Window => Err("a window is not a spot in the layout".into()),
        FilesPlace::Next(_) | FilesPlace::Replace => {
            let anchor = anchor.ok_or("no terminal to place it next to")?;
            let (label, space, pane) = showing(&anchor, cx).ok_or_else(|| "no open Pitwall window shows that terminal".to_string())?;
            let spot = match place {
                FilesPlace::Next(side) => FilesSpot::Tile { pane, side },
                _ => FilesSpot::Replace(pane),
            };
            Ok((label, space, spot))
        }
        FilesPlace::Edge(_) | FilesPlace::Own => {
            // Where it is now; else the space showing its agent; else the
            // frontmost window's.
            let (label, space) = match holder_of(what, cx) {
                Some((l, s, _)) => (l, s),
                None => match anchor.as_deref().and_then(|a| showing(a, cx)) {
                    Some((l, s, _)) => (l, s),
                    None => front_space(cx).ok_or("no Pitwall window is open")?,
                },
            };
            let spot = match place {
                FilesPlace::Edge(side) => FilesSpot::Edge(side),
                _ => FilesSpot::Auto,
            };
            Ok((label, space, spot))
        }
    }
}

/// Put page `id` at `spot` of space `space` (in window `label`): a tile
/// moved there, or the page's window's page moved in (the window closes).
/// That space is shown in its window; nothing comes forward and the
/// keyboard stays where it is.
pub fn dock(id: &str, label: &str, space: &str, spot: &FilesSpot, cx: &mut App) -> Result<(), String> {
    // The terminal it is placed against, if any (its place is said from it).
    let anchor = match spot {
        FilesSpot::Tile { pane, .. } => crate::ui_state::get(cx)
            .space(space)
            .and_then(|sp| sp.layout.leaves().into_iter().find(|p| p.id == *pane && p.content().is_none()))
            .and_then(|p| p.agent_id),
        _ => None,
    };
    match anchor {
        Some(a) => anchors(cx).insert(id.to_string(), a),
        None => anchors(cx).remove(id),
    };
    let i = id.to_string();
    land(&Content::Web(id.to_string()), label, space, cx, |ui, fit| ui.web_to_layout(space, &i, spot, fit))?;
    super::window::close_for_move(id, cx);
    Ok(())
}

/// Put `what` (a page, an inbox) in space `space` of Pitwall window
/// `label` with `put` (the layout change, given that space's fit there),
/// and show that space in its window. Nothing comes forward and the
/// keyboard stays where it is.
pub fn land(what: &Content, label: &str, space: &str, cx: &mut App, put: impl FnOnce(UiState, Option<Fit>) -> (UiState, Option<String>)) -> Result<(), String> {
    let handle = crate::windows::main_window(label, cx).ok_or("that Pitwall window is gone")?;
    let fit = handle.update(cx, |view, _, cx| view.screen().and_then(|s| s.read(cx).fit_for(space))).ok().flatten();
    let mut placed = None;
    crate::ui_state::update(cx, |ui| {
        let (next, tile) = put(ui.clone(), fit);
        placed = tile;
        *ui = next;
    });
    if placed.is_none() && crate::ui_state::get(cx).tile_of(what).is_none() {
        return Err("that spot isn't in the layout".into());
    }
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
    Ok(())
}

/// Page `id` out of its tile into a window of its own (at `at` on screen,
/// else where the last one was): the same page. The window takes the
/// keyboard only with `focus`.
pub fn pop_out(id: &str, at: Option<Point<Pixels>>, focus: bool, cx: &mut App) -> Result<WindowHandle<super::window::WebWindow>, String> {
    if !crate::ui_state::get(cx).web_tiles.contains_key(id) {
        return Err("no such web page".into());
    }
    anchors(cx).remove(id);
    crate::ui_state::update(cx, |ui| *ui = ui.clone().web_to_window(id));
    super::window::open(id, at, focus, cx)
}

/// Move page `id` (a tile or a window) to `place`, near agent `of`'s
/// terminal. Returns where, in words.
pub fn place(id: &str, place: FilesPlace, of: Option<&str>, cx: &mut App) -> Result<String, String> {
    if place == FilesPlace::Window {
        if crate::windows::web_window(id, cx).is_none() {
            let front = crate::platform::app_in_front(cx);
            pop_out(id, None, front, cx)?;
        }
        return Ok("a window of its own".into());
    }
    let (label, space, spot) = spot_for(id, place, of, cx)?;
    let gone = crate::explorer::dock::replaced_by(&space, &spot, cx);
    dock(id, &label, &space, &spot, cx)?;
    Ok(crate::explorer::dock::with_replaced(described(id, cx), gone.as_deref(), cx))
}

/// The agents whose terminals page `id` could take the place of: those in
/// the space its tile is in, else the space showing its agent; its agent
/// first. `(id, name)`.
pub fn replaceable(id: &str, cx: &mut App) -> Vec<(String, String)> {
    let anchor = anchor_agent(id, cx);
    replaceable_near(&Content::Web(id.to_string()), anchor, cx)
}

/// The agents whose terminals `what` (a page, an inbox) could take the
/// place of: those in the space its tile is in, else the space showing
/// agent `anchor`, else the frontmost window's; `anchor` first.
pub fn replaceable_near(what: &Content, anchor: Option<String>, cx: &mut App) -> Vec<(String, String)> {
    let space = match holder_of(what, cx) {
        Some((_, s, _)) => Some(s),
        None => anchor.as_deref().and_then(|a| showing(a, cx)).map(|(_, s, _)| s).or_else(|| front_space(cx).map(|(_, s)| s)),
    };
    let ui = crate::ui_state::get(cx);
    let Some(sp) = space.and_then(|s| ui.space(&s).cloned()) else { return Vec::new() };
    let mut agents = sp.layout.agents();
    agents.sort_by_key(|a| Some(a) != anchor.as_ref());
    agents
        .into_iter()
        .map(|a| {
            let name = crate::explorer::dock::agent_name(&a, cx);
            (a, name)
        })
        .collect()
}

/// Where page `id` is, in words ("a web tile in space Main (window main),
/// left of api's terminal"; "a window of its own").
pub fn described(id: &str, cx: &mut App) -> String {
    if crate::windows::web_window(id, cx).is_some() {
        return "a window of its own".into();
    }
    let ui = crate::ui_state::get(cx);
    let Some((space, pane)) = ui.web_tile(id) else { return "nowhere".into() };
    let Some(sp) = ui.space(&space) else { return "nowhere".into() };
    let anchor = cx.try_global::<Anchors>().and_then(|a| a.0.get(id).cloned());
    let at = whereabouts(&sp.layout, &pane, anchor.as_deref()).words(&|a: &str| crate::explorer::dock::agent_name(a, cx));
    format!("a web tile in space {} (window {}), {at}", sp.name, ui.window_of_space(&space))
}

/// Where page `id`'s tile is (see [`whereabouts`]).
pub fn whereabouts_of(id: &str, cx: &App) -> Option<Whereabouts> {
    let ui = crate::ui_state::get(cx);
    let (space, pane) = ui.web_tile(id)?;
    let anchor = cx.try_global::<Anchors>().and_then(|a| a.0.get(id).cloned());
    Some(whereabouts(&ui.space(&space)?.layout, &pane, anchor.as_deref()))
}

/// The grip of a page in a window of its own was pressed: it is dragged
/// (the window with it) and lands in the layout of the Pitwall window it
/// is let go over ([`crate::explorer::drag`], as files).
pub fn drag_start(id: &str, window: &mut Window, cx: &mut App) {
    let from = crate::explorer::drag::From::Window(window.window_handle());
    crate::explorer::drag::start_what(Content::Web(id.to_string()), from, window, cx);
}
