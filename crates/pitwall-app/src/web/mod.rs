//! Web pages in Pitwall (docs/spec/web.md), wherever they are: a page
//! ([`page::WebPage`]) kept by tile id ([`pages`]), drawn by a tile of a
//! Pitwall window's layout (`crate::main_screen::web_tile`) or by a window
//! of its own ([`window`]); where it goes ([`place`]); and what agents ask
//! for with `show_web` ([`show`]).

pub mod page;
pub mod pages;
pub mod place;
pub mod show;
pub mod window;

#[cfg(test)]
mod tests;

/// Debug builds, `web …` (bench commands; nothing is clicked on the screen,
/// the same code the tool, a menu pick or a button runs):
///
/// - `show <agent> <url|-> [right|left|above|below|replace|window] [of]`:
///   what `show_web` does for agent `<agent>` (any id; one no window
///   shows gets a window).
/// - `place <agent> right|left|above|below|replace|edge-right|edge-bottom|own|window [of]`:
///   agent `<agent>`'s page moved there ("Move…" / "Dock…").
/// - `close <agent>`: its page closed.
/// - `state`: every page (on stderr): where, its address and title, how
///   many loads it started, whether it has a view.
pub fn debug(what: &str, cx: &mut gpui::App) {
    use crate::explorer::dock::FilesPlace;
    use crate::main_screen::tree::Side;
    use pitwall_proto::ViewerPlace;
    let say = |s: String| eprintln!("pitwall-debug: web {s}");
    let words: Vec<&str> = what.split_whitespace().collect();
    let page_of = |agent: &str, cx: &gpui::App| crate::ui_state::get(cx).web_by(agent);
    match words.as_slice() {
        ["show", agent, url, rest @ ..] => {
            let place = match rest.first().copied() {
                None => None,
                Some(w) => match ViewerPlace::parse(w) {
                    Some(p) => Some(p),
                    None => return say(format!("no place {w}")),
                },
            };
            let req = pitwall_proto::WebShow {
                agent_id: agent.to_string(),
                url: (*url != "-").then(|| url.to_string()),
                place,
                of: rest.get(1).map(|s| s.to_string()),
            };
            match show::show(&req, cx) {
                Ok(r) => say(format!("{agent} shown: {r:?}")),
                Err(e) => say(format!("{agent} not shown: {} {}", e.code, e.message)),
            }
        }
        ["place", agent, place, rest @ ..] => {
            let Some(id) = page_of(agent, cx) else { return say(format!("{agent} has no page")) };
            let p = match *place {
                "right" => FilesPlace::Next(Side::Right),
                "left" => FilesPlace::Next(Side::Left),
                "above" => FilesPlace::Next(Side::Top),
                "below" => FilesPlace::Next(Side::Bottom),
                "edge-right" => FilesPlace::Edge(Side::Right),
                "edge-bottom" => FilesPlace::Edge(Side::Bottom),
                "own" => FilesPlace::Own,
                "window" => FilesPlace::Window,
                "replace" => FilesPlace::Replace,
                _ => return say(format!("no place {place}")),
            };
            match place::place(&id, p, rest.first().copied(), cx) {
                Ok(at) => say(format!("{agent} placed: {at}")),
                Err(e) => say(format!("{agent} not placed: {e}")),
            }
        }
        ["close", agent] => match page_of(agent, cx) {
            Some(id) => pages::close(&id, cx),
            None => say(format!("{agent} has no page")),
        },
        ["state"] => {
            let ui = crate::ui_state::get(cx);
            for (id, t) in &ui.web_tiles {
                let page = pages::get(id, cx);
                let (title, loads, view, host) = page.map_or((String::new(), 0, false, None), |p| {
                    let p = p.read(cx);
                    (p.title().to_string(), p.loads(), p.has_view(), p.host().map(|h| h.kind))
                });
                say(format!(
                    "{id}: by {:?} preview {:?} url {:?} title {title:?} loads {loads} view {view} held {host:?} at {}",
                    t.by,
                    t.agent,
                    t.url,
                    place::described(id, cx)
                ));
            }
        }
        _ => say(format!("what? {what}")),
    }
}
