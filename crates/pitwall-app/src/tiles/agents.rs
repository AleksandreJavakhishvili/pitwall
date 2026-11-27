//! Agents open tiles and show content in them (docs/spec/tile-sdk/
//! agent-tools.md §11.2–11.10): the tile tools this Pitwall offers its
//! agents ([`tools`]: `<extension-id>__open` for every extension with
//! tiles, and the show tools its tiles declare), and a call shown
//! ([`show`]): its tile placed next to the calling agent's terminal like
//! `show_web`'s page (`where`, `of`, the same view reused wherever the
//! user moved it), its content delivered as `view-opened` or
//! `view-content`, the component's answer waited for off the UI thread
//! ([`finish`]), and the call in the extension's activity log.
//!
//! What can turn them off: Settings → Agents → "Tile tools"
//! (`agents.tileTools`), a tile's own "Agents can use <name>" on its card
//! ([`tile_on`]), `"agents": false` in its manifest, and the extension
//! being off. Each change shows in the tools list at once, and the
//! agents' MCP servers tell them (`notifications/tools/list_changed`).

use std::path::Path;
use std::sync::mpsc;
use std::time::Instant;

use gpui::App;
use pitwall_core::extensions::Manifest;
use pitwall_proto::tile_tools::{TileReply, TileShow, TileShown, TileTarget, TileTool};
use pitwall_proto::{code, ErrorBody, ViewerPlace};
use pitwall_tile_host::agent::{error_text, Answer, Caller, ShowCall, ToolOutput};
use serde_json::Value;

use crate::explorer::dock::FilesPlace;
use crate::explorer::show::{goal, Goal};
use crate::main_screen::tree::Surface;

/// The setting that lets agents use tiles' tools.
pub const SETTING: &str = "agents.tileTools";

/// What a call says while `agents.tileTools` is off.
pub const OFF: &str = "Tile tools are turned off in Pitwall (Settings → Agents → “Agents can use tiles”, agents.tileTools). Nothing was shown.";

/// The setting's value (on unless turned off).
pub fn enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(SETTING)
        .and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Whether agents may use tile `tile` of extension `ext`: its manifest
/// lets them (`"agents"`) and the user hasn't turned its switch off.
pub fn tile_on(ext: &str, tile: &str, cx: &App) -> bool {
    !crate::ui_state::get(cx).extension(ext).agents_off.iter().any(|t| t == tile)
}

/// "Agents can use <name>" turned on or off for tile `tile` of `ext`.
pub fn set_tile_on(ext: &str, tile: &str, on: bool, cx: &mut App) {
    let (e, t) = (ext.to_string(), tile.to_string());
    crate::ui_state::update(cx, |ui| {
        *ui = std::mem::take(ui).with_extension(&e, |p| {
            p.agents_off.retain(|x| *x != t);
            if !on {
                p.agents_off.push(t.clone());
            }
        })
    });
}

/// The schema in file `rel` of extension folder `dir`, if it reads.
fn schema_file(dir: Option<&Path>, rel: &str) -> Option<Value> {
    let text = std::fs::read_to_string(dir?.join(rel)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The tile tools extension `m` offers (in folder `dir`), whatever the
/// switches say.
pub fn tools_of(m: &Manifest, dir: Option<&Path>) -> Vec<TileTool> {
    let mut out = Vec::new();
    let agents: Vec<_> = m.contributes.tiles.iter().filter(|t| t.agents).collect();
    if agents.is_empty() {
        return out;
    }
    let target = |t: &pitwall_core::extensions::TileDef| TileTarget { id: t.id.clone(), title: t.title.clone(), input: t.input.as_deref().and_then(|i| schema_file(dir, i)) };
    let base = TileTool { extension: m.id.clone(), extension_name: m.name.clone(), summary: m.description.clone().unwrap_or_default(), kind: "show".into(), ..Default::default() };
    out.push(TileTool { tiles: agents.iter().map(|t| target(t)).collect(), ..base.clone() });
    for t in &agents {
        for d in &t.tools {
            out.push(TileTool { tool: Some(d.name.clone()), kind: d.kind.word().into(), description: d.description.clone(), input: Some(d.schema()), tiles: vec![target(t)], ..base.clone() });
        }
    }
    out.retain(|t| t.name().len() <= pitwall_proto::tile_tools::MAX_NAME);
    out
}

/// The tile tools agents may use now: tiles of extensions that are on,
/// whose switch is on; none while `agents.tileTools` is off.
pub fn tools(cx: &App) -> Vec<TileTool> {
    if !enabled(cx) {
        return Vec::new();
    }
    let ui = crate::ui_state::get(cx);
    let mut out = Vec::new();
    for m in crate::extensions::enabled(&crate::extensions::found(cx), &ui) {
        if m.contributes.tiles.is_empty() {
            continue;
        }
        let dir = crate::extensions::folder_of(&m.id, cx);
        let mut list = tools_of(&m, dir.as_deref());
        for t in &mut list {
            t.tiles.retain(|x| tile_on(&m.id, &x.id, cx));
        }
        list.retain(|t| !t.tiles.is_empty());
        out.extend(list);
    }
    out
}

/// The MCP names of [`tools`] (what launch-time allow lists add).
pub fn tool_names(cx: &App) -> Vec<String> {
    tools(cx).iter().map(TileTool::name).collect()
}

/// A call placed and delivered, its answer still to come.
pub struct Placed {
    pub shown: TileShown,
    pub extension: String,
    pub tile: String,
    pub call: u64,
    pub answer: mpsc::Receiver<Answer>,
}

/// On the main thread: show `req` (the input checked already). Its tile
/// placed and its content handed over (the answer comes later:
/// [`finish`]), or a refusal answered at once.
pub fn show(req: &TileShow, cx: &mut App) -> Result<Result<Placed, TileShown>, ErrorBody> {
    if !enabled(cx) {
        return Err(ErrorBody::new(code::OFF, OFF));
    }
    let m = crate::extensions::manifest_of(&req.extension, cx).filter(|m| crate::ui_state::get(cx).extension_on(&m.id)).ok_or_else(|| ErrorBody::new(code::OFF, format!("The extension “{}” is off or not installed in Pitwall. Nothing was shown.", req.extension)))?;
    let def = m.contributes.tile(&req.tile).filter(|t| t.agents).ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("{} has no tile “{}” agents may open", m.name, req.tile)))?;
    if !tile_on(&m.id, &def.id, cx) {
        return Err(ErrorBody::new(code::OFF, format!("The user turned off agents' use of {} (Settings → Extensions → {} → “Agents can use {}”). Nothing was shown.", def.title, m.name, def.title)));
    }
    if let Some(tool) = &req.tool {
        if !def.tools.iter().any(|d| d.name == *tool) {
            return Err(ErrorBody::new(code::NOT_FOUND, format!("{} has no tool “{tool}”", def.title)));
        }
    }
    let caller = Caller { agent_id: req.agent_id.clone(), agent_name: req.agent_name.clone(), project: req.project.clone() };
    let call = ShowCall { tool: req.tool.clone(), tile: def.id.clone(), item: req.item.clone(), input: req.input.clone(), caller };
    let tiles = super::tiles(cx);
    let begun = tiles.update(cx, |t, _| t.calls(&m.id).begin(&call, Instant::now()));
    let tool_name = pitwall_proto::tile_tools::mcp_name(&m.id, req.tool.as_deref());
    let (id, content) = match begun {
        Ok(v) => v,
        Err(refused) => {
            record(&m.id, &tool_name, req, Some(refused.to_string()), cx);
            let reply = TileReply { ok: false, text: refused.to_string(), ..Default::default() };
            return Ok(Err(TileShown { title: def.title.clone(), reply: Some(reply), ..Default::default() }));
        }
    };
    let key = Surface::View(m.id.clone(), def.id.clone()).key();
    let what = Surface::View(m.id.clone(), def.id.clone()).content();
    let in_layout = crate::ui_state::get(cx).tile_of(&what).is_some();
    let in_window = crate::windows::inbox_window(&key, cx).is_some();
    let reused = (in_layout || in_window) && tiles.read(cx).views(&m.id).contains(&def.id);
    // The content first: the view opens with it (or gets it, if open).
    let (tx, rx) = mpsc::channel();
    let (ext, tile) = (m.id.clone(), def.id.clone());
    if let Err(why) = tiles.update(cx, |t, cx| t.deliver(&ext, &tile, id, content, tx, cx)) {
        record(&m.id, &tool_name, req, Some(why.clone()), cx);
        return Err(ErrorBody::new(code::OTHER, format!("{} can't show it now: {why}", def.title)));
    }
    let agent = req.agent_id.as_str();
    let mut note = None;
    let mut gone = None;
    let placed = (|| -> Result<(), String> {
        let anchor = req.of.clone().unwrap_or_else(|| agent.to_string());
        let title = def.title.clone();
        let missing = |anchor: &str, cx: &mut App| format!("{}'s terminal isn't shown in any open Pitwall window, so {title} opened in a window of its own.", crate::explorer::dock::agent_name(anchor, cx));
        let to_window = match goal(req.place, in_layout, in_window) {
            Goal::Tile | Goal::InWindow => false,
            Goal::Window => true,
            Goal::Next(side) => match crate::web::place::spot_near(&what, FilesPlace::Next(side), Some(anchor.clone()), cx) {
                Ok((label, space, spot)) => {
                    let ui = crate::ui_state::get(cx);
                    let there = ui.tile_of(&what).and_then(|(s, p)| ui.space(&s).map(|sp| crate::main_screen::workspace::whereabouts(&sp.layout, &p, Some(&anchor)))) == Some(crate::main_screen::workspace::Whereabouts::Next(side, anchor.clone()));
                    if !there {
                        crate::main_screen::inbox_window::dock(&key, &label, &space, &spot, cx)?;
                    }
                    false
                }
                Err(_) => {
                    note = Some(missing(&anchor, cx));
                    true
                }
            },
            Goal::Replace => match crate::web::place::spot_near(&what, FilesPlace::Replace, Some(anchor.clone()), cx) {
                Ok((label, space, spot)) => {
                    gone = crate::explorer::dock::replaced_by(&space, &spot, cx);
                    crate::main_screen::inbox_window::dock(&key, &label, &space, &spot, cx)?;
                    false
                }
                Err(_) if in_layout => false,
                Err(_) => {
                    note = Some(missing(&anchor, cx));
                    true
                }
            },
        };
        if to_window && crate::windows::inbox_window(&key, cx).is_none() {
            // Never in front of the user's work, nor with the keyboard.
            crate::main_screen::inbox_window::pop_out(&key, None, false, cx)?;
        }
        Ok(())
    })();
    if let Err(why) = placed {
        tiles.update(cx, |t, _| t.forget_call(&m.id, &def.id, id));
        record(&m.id, &tool_name, req, Some(why.clone()), cx);
        return Err(ErrorBody::new(code::OTHER, format!("{} couldn't be placed: {why}", def.title)));
    }
    // Its tile made now (the layout draws it soon anyway): the view opens
    // with the content.
    crate::main_screen::view_tile::view(&key, cx);
    let in_window = crate::windows::inbox_window(&key, cx).is_some();
    let (window, place) = if in_window {
        (crate::main_screen::inbox_window::title(&key, cx), ViewerPlace::Window)
    } else {
        let label = crate::web::place::holder_of(&what, cx).map(|(l, _, _)| l).unwrap_or_default();
        (label, req.place.filter(|p| p.is_tile()).unwrap_or_default())
    };
    let at = crate::explorer::dock::with_replaced(crate::main_screen::inbox_window::described(&key, cx), gone.as_deref(), cx);
    let shown = TileShown { window, in_background: !crate::platform::app_in_front(cx), place, at, note, reused, title: def.title.clone(), reply: None };
    Ok(Ok(Placed { shown, extension: m.id.clone(), tile: def.id.clone(), call: id, answer: rx }))
}

/// Off the UI thread: wait for the component's answer to `p` (at most
/// `within`) and put it in what the agent is told.
pub fn finish(p: Placed, within: std::time::Duration) -> (TileShown, Result<(), String>) {
    let mut shown = p.shown;
    let answer = p.answer.recv_timeout(within).unwrap_or_else(|_| Answer::NotDelivered(format!("it didn't answer within {} seconds", within.as_secs())));
    let outcome = match answer {
        Answer::Handled => Ok(()),
        Answer::Reply(ToolOutput::Ok(ok)) => {
            shown.reply = Some(TileReply { ok: true, text: ok.text, json: ok.json, item: ok.item });
            Ok(())
        }
        Answer::Reply(ToolOutput::Error(e)) => {
            let text = error_text(&e);
            shown.reply = Some(TileReply { ok: false, text: text.clone(), ..Default::default() });
            Err(text)
        }
        Answer::NotDelivered(why) => {
            let text = format!("{} didn't get it: {why}. Nothing was shown.", shown.title);
            shown.reply = Some(TileReply { ok: false, text: text.clone(), ..Default::default() });
            Err(text)
        }
    };
    (shown, outcome)
}

/// After [`finish`], on the main thread: call `call` let go (a call that
/// timed out no longer waits), in the extension's activity log, and the
/// view's "from <agent>" mark when it showed something.
pub fn finished(ext: &str, tile: &str, call: u64, req: &TileShow, outcome: &Result<(), String>, cx: &mut App) {
    super::tiles(cx).update(cx, |t, _| t.forget_call(ext, tile, call));
    let name = pitwall_proto::tile_tools::mcp_name(ext, req.tool.as_deref());
    record(ext, &name, req, outcome.clone().err(), cx);
    if outcome.is_ok() && req.input.is_some() {
        let tiles = super::tiles(cx);
        let agent = (req.agent_id.clone(), req.agent_name.clone());
        tiles.update(cx, |t, cx| t.set_origin(ext, tile, agent, cx));
    }
}

/// A line in extension `ext`'s activity log for a call of tool `name`:
/// who, which item, the first 200 characters of the input, ok or why not.
fn record(ext: &str, name: &str, req: &TileShow, failed: Option<String>, cx: &mut App) {
    use pitwall_core::extensions::activity::{Entry, Outcome, Via};
    let outcome = match failed {
        None => Outcome::Ok,
        Some(message) => Outcome::Refused { code: "refused".into(), message },
    };
    let input: String = req.input.as_deref().unwrap_or("").chars().take(200).collect();
    let item = req.item.as_deref().map(|i| format!(", item “{i}”")).unwrap_or_default();
    let detail = format!("{}{item}{}", req.agent_name, if input.is_empty() { String::new() } else { format!(": {input}") });
    let log = crate::extensions::calls::activity(ext, cx);
    log.lock().unwrap_or_else(|e| e.into_inner()).push(Entry::now(Via::Agent, name, outcome).with_detail(Some(detail)));
}
