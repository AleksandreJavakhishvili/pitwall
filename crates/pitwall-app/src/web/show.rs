//! An agent shows the user a web page (`web.open`: `pitwall open --web`,
//! the `pitwall` MCP server's `show_web`). It opens in a web tile of the
//! Pitwall window showing the agent, right (the default), left, above or
//! below its terminal (or the terminal `of` names), in that terminal's
//! place, or in a window of its own; a tile for an agent no open window
//! shows falls back to a window, and the answer says so.
//!
//! One page per agent: the page an agent opened is remembered as its
//! (`ui.json` `webTiles[id].by`), and its next call goes there, wherever
//! the user moved it (navigated when a url is given), unless `where` names
//! a place it isn't in. It is a page like one the user opened (no Preview
//! rules, it stays when the agent goes), in the browser profile of the
//! agent's project ([`super::profiles`]; Default when it names none). The
//! agent can't pick one. A page shown before that is in another profile
//! than the project's now moves into it at the agent's next call (loaded
//! again there), and the answer says so.
//!
//! Nothing comes forward and the keyboard stays where it is, whether or not
//! Pitwall is in front: the page is shown, not handed the keys (an empty
//! page's address field waits for a click). The agent never gets anything
//! back from the page.

use gpui::App;

use pitwall_proto::{code, ErrorBody, ViewerPlace, WebShow, WebShown};

use super::place;
use crate::explorer::dock::FilesPlace;
use crate::explorer::show::{goal, Goal};
use crate::main_screen::workspace::{new_web_id, WebTile, Whereabouts};

/// The setting that lets agents show web pages.
pub const SETTING: &str = "agents.showWeb";

/// The setting's value (on unless turned off).
pub fn enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(SETTING)
        .and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// On the main thread: show `req` and say where.
pub fn show(req: &WebShow, cx: &mut App) -> Result<WebShown, ErrorBody> {
    if !super::page::enabled(cx) || !enabled(cx) {
        return Err(ErrorBody::new(code::OFF, "Web pages from agents are turned off in Pitwall. Nothing was opened."));
    }
    // As the address field would take it ("example.com": https://…).
    let url = req.url.as_deref().map(pitwall_proto::web::address).transpose().map_err(|e| ErrorBody::new(code::BAD_PARAMS, e))?;
    let agents = super::pages::agents(cx);
    let agent = req.agent_id.as_str();
    let name_of = |id: &str| agents.iter().find(|a| a.id == id).map_or(id.to_string(), |a| a.name.clone());
    let name = name_of(agent);
    let in_front = crate::platform::app_in_front(cx);
    let err = |e: String| ErrorBody::new(code::OTHER, e);

    // The browser profile of the agent's project (never the agent's pick).
    let project = agents.iter().find(|a| a.id == agent).map(|a| a.project.clone()).unwrap_or_default();
    let profile = if project.is_empty() { None } else { super::profiles::for_project(&project, cx) };

    // Its page: the one it opened before, else a new one (not placed yet).
    let ui = crate::ui_state::get(cx);
    let (id, reused) = match ui.web_by(agent) {
        Some(id) => (id, true),
        None => {
            let id = new_web_id();
            let tile = WebTile { url: url.clone().unwrap_or_default(), by: Some(agent.to_string()), profile: profile.clone(), ..Default::default() };
            crate::ui_state::update(cx, |s| {
                s.web_tiles.insert(id.clone(), tile);
            });
            (id, false)
        }
    };
    let result = place_it(req, &id, reused, &name, &name_of, cx);
    let (mut note, gone) = match result {
        Ok(v) => v,
        Err(e) => {
            if !reused {
                crate::ui_state::update(cx, |s| {
                    s.web_tiles.remove(&id);
                });
                super::pages::prune(cx);
            }
            return Err(err(e));
        }
    };
    // A page shown before, in another profile than its project's now (the
    // project's changed, or the user switched the page): into the
    // project's, loaded again there.
    if reused {
        let had = crate::ui_state::get(cx).web_tiles.get(&id).and_then(|t| t.profile.clone());
        if had != profile {
            let (from, to) = (super::profiles::name(had.as_deref(), cx), super::profiles::name(profile.as_deref(), cx));
            match super::pages::get(&id, cx) {
                Some(p) => p.update(cx, |p, cx| p.set_profile(profile.clone(), cx)),
                None => {
                    let p = profile.clone();
                    crate::ui_state::update(cx, |s| *s = s.clone().set_web_profile(&id, p.as_deref()));
                }
            }
            let moved = format!("The page was in the {from} browser profile; it is in {to} now, the one its project uses, and loaded again there.");
            note = Some(match note {
                Some(n) => format!("{n}\n{moved}"),
                None => moved,
            });
        }
    }
    // A page shown before, given a url: it goes there.
    if let (true, Some(url)) = (reused, url) {
        if let Some(p) = super::pages::ensure(&id, &agents, cx) {
            p.update(cx, |p, cx| p.navigate(&url, cx));
        }
        let u = url.clone();
        crate::ui_state::update(cx, |s| *s = s.clone().set_web_url(&id, &u));
    }
    let in_window = crate::windows::web_window(&id, cx).is_some();
    let at = crate::explorer::dock::with_replaced(place::described(&id, cx), gone.as_deref(), cx);
    let (window, place) = if in_window {
        let page_title = super::pages::get(&id, cx).map(|p| p.read(cx).title().to_string()).unwrap_or_default();
        let url = crate::ui_state::get(cx).web_tiles.get(&id).map(|t| t.url.clone()).unwrap_or_default();
        (super::window::title(&page_title, &url), ViewerPlace::Window)
    } else {
        let label = place::holder(&id, cx).map(|(l, _, _)| l).unwrap_or_default();
        (label, req.place.filter(|p| p.is_tile()).unwrap_or_default())
    };
    let profile = super::profiles::name(profile.as_deref(), cx);
    Ok(WebShown { window, in_background: !in_front, place, at, note, reused, profile })
}

/// Page `id` where `req` asks (where it is, when it asks nowhere and it
/// was shown before). The note (it went to a window instead), and the
/// terminal it took the place of.
fn place_it(
    req: &WebShow,
    id: &str,
    reused: bool,
    name: &str,
    name_of: &dyn Fn(&str) -> String,
    cx: &mut App,
) -> Result<(Option<String>, Option<String>), String> {
    let agent = req.agent_id.as_str();
    let tile = reused && place::holder(id, cx).is_some();
    let window = reused && crate::windows::web_window(id, cx).is_some();
    let mut note = None;
    let mut gone = None;
    let missing = |of: Option<&str>| {
        let whose = of.map_or_else(|| name.to_string(), name_of);
        format!("{whose}'s terminal isn't shown in any open Pitwall window, so the page opened in a window of its own.")
    };
    let to_window = match goal(req.place, tile, window) {
        Goal::Tile | Goal::InWindow => false,
        Goal::Window => true,
        Goal::Next(side) => {
            let of = req.of.as_deref().filter(|o| *o != agent);
            match place::spot_for(id, FilesPlace::Next(side), Some(of.unwrap_or(agent)), cx) {
                Ok((label, space, spot)) => {
                    // Where it is already: left there.
                    let there = tile && place::whereabouts_of(id, cx) == Some(Whereabouts::Next(side, of.unwrap_or(agent).to_string()));
                    if !there {
                        place::dock(id, &label, &space, &spot, cx)?;
                    }
                    false
                }
                Err(_) => {
                    note = Some(missing(of));
                    true
                }
            }
        }
        Goal::Replace => {
            let of = req.of.as_deref();
            match place::spot_for(id, FilesPlace::Replace, Some(of.unwrap_or(agent)), cx) {
                Ok((label, space, spot)) => {
                    gone = crate::explorer::dock::replaced_by(&space, &spot, cx);
                    place::dock(id, &label, &space, &spot, cx)?;
                    false
                }
                // In that terminal's place already: left there.
                Err(_) if tile => false,
                Err(_) => {
                    note = Some(missing(of));
                    true
                }
            }
        }
    };
    if to_window && crate::windows::web_window(id, cx).is_none() {
        // Never in front of the user's work, nor with the keyboard.
        place::pop_out(id, None, false, cx)?;
    }
    Ok((note, gone))
}
