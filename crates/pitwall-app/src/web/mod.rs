//! Web pages in Pitwall (docs/spec/web.md), wherever they are: a page
//! ([`page::WebPage`]) kept by tile id ([`pages`]), drawn by a tile of a
//! Pitwall window's layout (`crate::main_screen::web_tile`) or by a window
//! of its own ([`window`]); where it goes ([`place`]); and what agents ask
//! for with `show_web` ([`show`]).

pub mod page;
pub mod pages;
pub mod place;
pub mod profiles;
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
/// - `profile add <name>` / `remove <id>` / `list`: browser profiles (list:
///   each one's folder and store identifier).
/// - `profile project <path> <id|default>`: a project's profile.
/// - `profile switch <agent> <id|default>`: its page's profile (the bar's
///   profile menu).
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
        ["profile", "add", name @ ..] => match profiles::add(&name.join(" "), cx) {
            Ok(a) => say(format!("profile added: {} {}", a.id, a.dir)),
            Err(e) => say(format!("profile not added: {e}")),
        },
        ["profile", "project", project, id] => {
            let m = crate::accounts::model(cx);
            let id = (*id != "default").then_some(*id);
            match m.update(cx, |m, cx| m.set_project_default(project, pitwall_core::accounts::WEB, id, cx)) {
                Ok(()) => say(format!("profile of {project}: {:?}", profiles::for_project(project, cx))),
                Err(e) => say(format!("profile not set: {e}")),
            }
        }
        ["profile", "switch", agent, id] => {
            let Some(page) = page_of(agent, cx).and_then(|p| pages::get(&p, cx)) else { return say(format!("{agent} has no page made")) };
            let id = (*id != "default").then(|| id.to_string());
            page.update(cx, |p, cx| p.set_profile(id, cx));
            say(format!("{agent}'s page in {:?}", page.read(cx).profile()));
        }
        ["profile", "remove", id] => match profiles::remove(id, cx) {
            Ok(()) => say(format!("profile removed: {id}")),
            Err(e) => say(format!("profile not removed: {e}")),
        },
        ["profile", "list"] => {
            let sep = profiles::separate(cx);
            for a in profiles::all(cx) {
                let dir = profiles::dir(Some(&a.id), cx);
                say(format!("profile {} {:?} dir {} store {}", a.id, a.name, dir.display(), store_hex(&dir)));
            }
            let dir = profiles::dir(None, cx);
            say(format!("profile default dir {} store {} separate {sep}", dir.display(), store_hex(&dir)));
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
                    "{id}: by {:?} preview {:?} url {:?} profile {:?} title {title:?} loads {loads} view {view} held {host:?} at {}",
                    t.by,
                    t.agent,
                    t.url,
                    t.profile,
                    place::described(id, cx)
                ));
            }
        }
        _ => say(format!("what? {what}")),
    }
}

/// The store identifier of data folder `dir` (macOS 14+), as hex.
fn store_hex(dir: &std::path::Path) -> String {
    crate::webview::store_id(dir).iter().map(|b| format!("{b:02x}")).collect()
}
