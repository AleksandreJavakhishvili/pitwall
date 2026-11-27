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
use pitwall_proto::tile_tools::{TileRan, TileReply, TileRun, TileShow, TileShown, TileTarget, TileTool};
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
    let skill = dir.and_then(|d| pitwall_core::extensions::agent_skill::read(d, &m.id)).and_then(Result::ok).map(|s| s.description);
    let base = TileTool { extension: m.id.clone(), extension_name: m.name.clone(), summary: m.description.clone().unwrap_or_default(), skill, kind: "show".into(), ..Default::default() };
    out.push(TileTool { tiles: agents.iter().map(|t| target(t)).collect(), ..base.clone() });
    for t in &agents {
        for d in &t.tools {
            let expose = if d.expose.is_empty() { Vec::new() } else { d.expose.iter().map(|e| e.word().to_string()).collect() };
            out.push(TileTool { tool: Some(d.name.clone()), kind: d.kind.word().into(), description: d.description.clone(), input: Some(d.schema()), tiles: vec![target(t)], destructive: d.destructive, idempotent: d.idempotent, expose, title: d.title.clone(), ..base.clone() });
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

/// The MCP names of [`tools`] that launch-time allow lists add: the
/// show-only ones offered to agents. `read` and `act` tools are never
/// allowed there: the agent's own tool prompt may ask, and Pitwall asks
/// the user in any case (agent-tools.md §11.5).
pub fn tool_names(cx: &App) -> Vec<String> {
    tools(cx).iter().filter(|t| t.show_only() && t.exposed("agents")).map(TileTool::name).collect()
}

/// The skills for agents of the extensions (agent-tools.md §11.11) as
/// agents get them: each extension that is on, whose `agents/SKILL.md`
/// reads and whose tools agents may use now (Tile tools on, a tile's
/// switch on), for the projects the user gave it to.
pub fn skills(cx: &App) -> Vec<pitwall_core::rules::ext_skills::ExtSkill> {
    if !enabled(cx) {
        return Vec::new();
    }
    let ui = crate::ui_state::get(cx);
    let mut out = Vec::new();
    for m in crate::extensions::enabled(&crate::extensions::found(cx), &ui) {
        let projects = ui.extension(&m.id).skill_projects;
        if projects.is_empty() || !m.contributes.tiles.iter().any(|t| t.agents && tile_on(&m.id, &t.id, cx)) {
            continue;
        }
        let Some(Ok(skill)) = crate::extensions::folder_of(&m.id, cx).and_then(|d| pitwall_core::extensions::agent_skill::read(&d, &m.id)) else { continue };
        out.push(pitwall_core::rules::ext_skills::ExtSkill { id: m.id.clone(), name: m.name.clone(), description: skill.description.clone(), text: skill.for_agents(&m.name), projects });
    }
    out
}

/// Keep the skills agents get in step with [`skills`] (on every change of
/// the settings and the extensions found): a change makes the agents'
/// rules stale, and their next apply adds or removes it.
pub fn sync_skills(cx: &App) {
    let Some(e) = crate::extensions::engine(cx) else { return };
    pitwall_core::rules::ext_skills::sync(&pitwall_core::rules::Dirs::of(e.paths()), &skills(cx));
}

/// Whether agents of `project` get extension `ext`'s skill.
pub fn skill_in(ext: &str, project: &str, cx: &App) -> bool {
    crate::ui_state::get(cx).extension(ext).skill_projects.iter().any(|p| p == project)
}

/// Give agents of `project` extension `ext`'s skill, or take it away.
pub fn set_skill_in(ext: &str, project: &str, on: bool, cx: &mut App) {
    let (e, p) = (ext.to_string(), project.to_string());
    crate::ui_state::update(cx, |ui| {
        *ui = std::mem::take(ui).with_extension(&e, |x| {
            x.skill_projects.retain(|q| *q != p);
            if on {
                x.skill_projects.push(p.clone());
            }
        })
    });
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
    let large = req.large.as_ref().map(|t| t.len() as u64);
    let call = ShowCall { tool: req.tool.clone(), tile: def.id.clone(), item: req.item.clone(), input: req.input.clone(), caller, append: req.append, large };
    let tiles = super::tiles(cx);
    let begun = tiles.update(cx, |t, _| t.calls(&m.id).begin(&call, Instant::now()));
    // The file's text waits for the component's `input-part` reads.
    if let (Ok((id, _)), Some(text)) = (&begun, &req.large) {
        if let Some(l) = tiles.update(cx, |t, cx| t.large_inputs(cx)) {
            l.put(&m.id, *id, text.clone());
        }
    }
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
    // Opened for the calling agent's project (when its space has none).
    if let Some(folder) = req.project.as_deref().filter(|p| !p.is_empty()) {
        tiles.update(cx, |t, cx| t.hint_project(&ext, &tile, folder, cx));
    }
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
    super::tiles(cx).update(cx, |t, cx| {
        t.forget_call(ext, tile, call);
        if let Some(l) = t.large_inputs(cx) {
            l.forget(ext, call);
        }
    });
    let name = pitwall_proto::tile_tools::mcp_name(ext, req.tool.as_deref());
    record(ext, &name, req, outcome.clone().err(), cx);
    if outcome.is_ok() && req.input.is_some() {
        let tiles = super::tiles(cx);
        let agent = (req.agent_id.clone(), req.agent_name.clone());
        // A `present` call can be taken back (Undo beside the mark).
        let presents = req.tool.as_ref().is_some_and(|tool| {
            crate::extensions::manifest_of(ext, cx).and_then(|m| m.contributes.tile(tile).and_then(|d| d.tools.iter().find(|d| d.name == *tool).map(|d| d.kind == pitwall_core::extensions::tools::ToolKind::Present))).unwrap_or(false)
        });
        if presents {
            tiles.update(cx, |t, cx| t.set_presented(ext, tile, agent, call, cx));
        } else {
            tiles.update(cx, |t, cx| t.set_origin(ext, tile, agent, cx));
        }
    }
}

/// What the user's approval of a `read` or `act` call looks like, and
/// whether it is needed.
pub struct ToAsk {
    pub summary: String,
    pub details: Vec<String>,
    /// "Always for <agent>" may be offered.
    pub always: Option<String>,
}

/// A `read` or `act` call checked on the main thread: allowed as it is
/// (`None`), or the approval to ask for.
pub fn check_run(req: &TileRun, cx: &App) -> Result<Option<ToAsk>, ErrorBody> {
    if !enabled(cx) {
        return Err(ErrorBody::new(code::OFF, OFF.replace("Nothing was shown", "Nothing was done")));
    }
    let m = crate::extensions::manifest_of(&req.extension, cx).filter(|m| crate::ui_state::get(cx).extension_on(&m.id)).ok_or_else(|| ErrorBody::new(code::OFF, format!("The extension “{}” is off or not installed in Pitwall. Nothing was done.", req.extension)))?;
    let def = m.contributes.tile(&req.tile).filter(|t| t.agents).ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("{} has no tile “{}” agents may use", m.name, req.tile)))?;
    if !tile_on(&m.id, &def.id, cx) {
        return Err(ErrorBody::new(code::OFF, format!("The user turned off agents' use of {} (Settings → Extensions → {} → “Agents can use {}”). Nothing was done.", def.title, m.name, def.title)));
    }
    let tool = def.tools.iter().find(|d| d.name == req.tool && d.kind.word() == req.kind).ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("{} has no tool “{}”", def.title, req.tool)))?;
    // The user's own read runs; their act asks (a script can't act
    // behind their back).
    if req.by_user && req.kind == "read" {
        return Ok(None);
    }
    if !req.by_user && granted(&m.id, &req.agent_id, &req.tool, cx) {
        return Ok(None);
    }
    let verb = if req.kind == "read" { format!("read what {} has", def.title) } else { tool.title.clone().map(|t| t.to_lowercase()).unwrap_or_else(|| format!("run {}", tool.name)) };
    let summary = format!("{verb} ({}: {})", m.name, tool.name);
    let mut details = vec![tool.description.clone()];
    if let Some(p) = req.project.as_deref() {
        details.push(format!("Project: {p}"));
    }
    details.extend(input_lines(&req.input));
    // "Always" only for an agent, never for tiles that can make agents
    // act (decision 5).
    let g = super::grants_of(&m, cx);
    let risky = g.has(pitwall_core::extensions::permissions::AGENTS_QUEUE) || g.has(pitwall_core::extensions::permissions::AGENTS_START);
    let always = (!req.by_user && !(risky && req.kind == "act")).then(|| format!("Always for {}", req.agent_name));
    Ok(Some(ToAsk { summary, details, always }))
}

/// The input as the dialog shows it: one line per property, long values
/// folded.
fn input_lines(input: &str) -> Vec<String> {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(input) else { return Vec::new() };
    map.iter()
        .map(|(k, v)| {
            let text = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let short: String = text.chars().take(160).collect();
            let more = if short.len() < text.len() { format!("… ({} characters)", text.chars().count()) } else { String::new() };
            format!("{k}: {short}{more}")
        })
        .collect()
}

/// Whether the user allowed agent `agent` tool `tool` of extension `ext`
/// always.
pub fn granted(ext: &str, agent: &str, tool: &str, cx: &App) -> bool {
    !agent.is_empty() && crate::ui_state::get(cx).extension(ext).tool_grants.iter().any(|g| g.agent == agent && g.tool == tool)
}

/// "Always for <agent>": kept until revoked.
pub fn grant(ext: &str, agent: &str, name: &str, tool: &str, cx: &mut App) {
    let g = crate::main_screen::workspace::ToolGrant { agent: agent.into(), name: name.into(), tool: tool.into(), at: pitwall_core::extensions::permissions::Decision::now(true).at };
    let e = ext.to_string();
    crate::ui_state::update(cx, |ui| {
        *ui = std::mem::take(ui).with_extension(&e, |p| {
            p.tool_grants.retain(|x| !(x.agent == g.agent && x.tool == g.tool));
            p.tool_grants.push(g.clone());
        })
    });
}

/// Revoke a grant ([`grant`]); `tool` `None`: all of the agent's.
pub fn revoke(ext: &str, agent: &str, tool: Option<&str>, cx: &mut App) {
    let (e, a, t) = (ext.to_string(), agent.to_string(), tool.map(str::to_string));
    crate::ui_state::update(cx, |ui| *ui = std::mem::take(ui).with_extension(&e, |p| p.tool_grants.retain(|x| !(x.agent == a && t.as_ref().is_none_or(|t| x.tool == *t)))));
}

/// A `read` or `act` call handed to its tile; its answer still to come.
pub struct Running {
    pub call: u64,
    pub answer: mpsc::Receiver<Answer>,
}

/// On the main thread, once allowed: call `req` of its tile.
pub fn start_run(req: &TileRun, cx: &mut App) -> Result<Running, ErrorBody> {
    use pitwall_tile_host::agent::{ToolCall, ToolKind};
    let kind = if req.kind == "read" { ToolKind::Read } else { ToolKind::Act };
    let tiles = super::tiles(cx);
    let agent = (!req.by_user).then_some(req.agent_id.as_str());
    let id = tiles.update(cx, |t, _| t.calls(&req.extension).begin_tool(agent, &req.tile, &req.input, Instant::now())).map_err(|r| ErrorBody::new(code::OTHER, r.to_string()))?;
    let caller = (!req.by_user).then(|| Caller { agent_id: req.agent_id.clone(), agent_name: req.agent_name.clone(), project: req.project.clone() });
    let call = ToolCall { id, tool: req.tool.clone(), kind, caller, input: req.input.clone(), view: None };
    let (tx, rx) = mpsc::channel();
    let ext = req.extension.clone();
    tiles.update(cx, |t, cx| t.tool_call(&ext, call, tx, cx)).map_err(|why| ErrorBody::new(code::OTHER, format!("{} can't take it now: {why}", req.extension)))?;
    Ok(Running { call: id, answer: rx })
}

/// Off the UI thread: wait for the tile's answer to a `read` or `act`
/// call (at most `within`). `None`: it ran out of time.
pub fn wait_run(r: &Running, within: std::time::Duration) -> Option<TileRan> {
    let answer = r.answer.recv_timeout(within).ok()?;
    Some(match answer {
        Answer::Handled => TileRan { ok: true, text: "Done.".into(), ..Default::default() },
        Answer::Reply(ToolOutput::Ok(ok)) => TileRan { ok: true, text: ok.text, json: ok.json, ..Default::default() },
        Answer::Reply(ToolOutput::Error(e)) => TileRan { ok: false, text: error_text(&e), ..Default::default() },
        Answer::NotDelivered(why) => TileRan { ok: false, text: format!("The tile didn't get it: {why}. Nothing was done."), ..Default::default() },
    })
}

/// After a `read` or `act` call, on the main thread: let go (the tile
/// hears `tool-cancelled` when it ran out of time), in the activity log.
pub fn ran(req: &TileRun, call: Option<u64>, ran: &TileRan, cx: &mut App) {
    if let (Some(call), true) = (call, ran.timed_out) {
        let ext = req.extension.clone();
        super::tiles(cx).update(cx, |t, _| t.cancel_tool(&ext, call));
    }
    use pitwall_core::extensions::activity::{Entry, Outcome, Via};
    let outcome = if ran.ok {
        Outcome::Ok
    } else {
        Outcome::Refused { code: if ran.refused { "refused".into() } else { "error".into() }, message: ran.text.clone() }
    };
    // `act`: the whole input; `read`: its first 200 characters.
    let input: String = if req.kind == "act" { req.input.clone() } else { req.input.chars().take(200).collect() };
    let who = if req.by_user { "You".to_string() } else { req.agent_name.clone() };
    let name = pitwall_proto::tile_tools::mcp_name(&req.extension, Some(&req.tool));
    let log = crate::extensions::calls::activity(&req.extension, cx);
    log.lock().unwrap_or_else(|e| e.into_inner()).push(Entry::now(Via::Agent, &name, outcome).with_detail(Some(format!("{who} ({}): {input}", req.kind))));
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
