//! Files dragged by their grip: from a window of their own into a Pitwall
//! window's layout, or from a files tile to another place in it, into
//! another Pitwall window, or out of every one (a window of their own,
//! where they were let go). GPUI's drags stay in one window, so the
//! pointer is followed on the screen (`platform::track_pointer`); each
//! Pitwall window under it shows where the files would land
//! (`MainScreen::files_hover`), and letting go puts them there
//! ([`super::dock::dock`]). Where the pointer can't be followed so (not
//! macOS), the "Dock…" / "Move…" menu does it.
//!
//! A web page or an extension's inbox in a window of its own is dragged
//! the same way by its grip ([`start_what`] with [`Content::Web`] or
//! [`Content::Inbox`]).

use gpui::{px, AnyWindowHandle, App, Global, Pixels, Point, Window};

use crate::main_screen::tree::Content;
use crate::main_screen::workspace::FilesSpot;

/// Where the files (or page) come from.
#[derive(Clone, Copy)]
pub enum From {
    Tile,
    Window(AnyWindowHandle),
}

/// Where they would land now: window, space, spot.
#[derive(Default)]
struct Over(Option<(String, String, FilesSpot)>);

impl Global for Over {}

/// The grip of agent `owner`'s files was pressed in `window`.
pub fn start(owner: &str, from: From, window: &mut Window, cx: &mut App) {
    start_what(Content::Files(owner.to_string()), from, window, cx)
}

/// The grip of `what` (files, a web page) was pressed in `window`.
pub fn start_what(what: Content, from: From, window: &mut Window, cx: &mut App) {
    let begin = on_screen(window.mouse_position(), window);
    if let From::Window(_) = from {
        // The window follows the pointer (the system's move elsewhere).
        crate::platform::float_drag(window, cx);
    }
    let (o, o2) = (what.clone(), what);
    let last = std::rc::Rc::new(std::cell::Cell::new(begin));
    let last2 = last.clone();
    let tracked = crate::platform::track_pointer(
        window,
        cx,
        move |dx, dy, cx| {
            let at = gpui::point(begin.x + px(dx), begin.y + px(dy));
            last.set(at);
            hover_what(&o, at, cx);
        },
        move |cx| {
            let at = last2.get();
            cx.defer(move |cx| {
                if let Err(e) = finish_what(&o2, at, from, cx) {
                    eprintln!("pitwall: could not move it: {e}");
                }
            });
        },
    );
    if !tracked {
        clear(cx);
    }
}

/// `at` in `window`'s content on the screen (its title bar, when the
/// system draws one, counted).
pub fn on_screen(at: Point<Pixels>, window: &Window) -> Point<Pixels> {
    let b = window.bounds();
    let title = (b.size.height - window.viewport_size().height).max(px(0.));
    gpui::point(b.origin.x + at.x, b.origin.y + title + at.y)
}

/// The files of agent `owner` dragged to `at` on the screen: the Pitwall
/// window in front there shows where they'd land, the others nothing.
/// Returns that (window, space, spot).
pub fn hover(owner: &str, at: Point<Pixels>, cx: &mut App) -> Option<(String, String, FilesSpot)> {
    hover_what(&Content::Files(owner.to_string()), at, cx)
}

/// [`hover`] for files or a web page.
pub fn hover_what(what: &Content, at: Point<Pixels>, cx: &mut App) -> Option<(String, String, FilesSpot)> {
    let frames = crate::windows::frames_front_to_back(cx);
    let under = frames.iter().find(|(_, _, f)| f.contains(&at)).map(|(l, h, _)| (l.clone(), *h));
    let mut found = None;
    for (label, h, _) in &frames {
        let mine = under.as_ref().is_some_and(|(l, _)| l == label);
        let o = what.clone();
        let r = h.update(cx, |view, window, cx| {
            let screen = view.screen()?;
            if !mine {
                screen.update(cx, |s, cx| s.clear_files_hover(cx));
                return None;
            }
            let b = window.bounds();
            let title = (b.size.height - window.viewport_size().height).max(px(0.));
            let local = gpui::point(at.x - b.origin.x, at.y - b.origin.y - title);
            screen.update(cx, |s, cx| s.tile_hover(&o, local, cx))
        });
        if let Ok(Some((space, spot))) = r {
            found = Some((label.clone(), space, spot));
        }
    }
    cx.set_global(Over(found.clone()));
    found
}

/// No files dragged any more: every window's hint goes.
pub fn clear(cx: &mut App) {
    cx.set_global(Over(None));
    for (_, h, _) in crate::windows::frames_front_to_back(cx) {
        let _ = h.update(cx, |view, _, cx| {
            if let Some(s) = view.screen() {
                s.update(cx, |s, cx| s.clear_files_hover(cx));
            }
        });
    }
}

/// Let go at `at` on the screen: where the last hover showed, else (a
/// tile let go outside every Pitwall window) a window of their own there.
/// Returns where they went, in words (`None`: they stayed).
pub fn finish(owner: &str, at: Point<Pixels>, from: From, cx: &mut App) -> Result<Option<String>, String> {
    finish_what(&Content::Files(owner.to_string()), at, from, cx)
}

/// [`finish`] for files or a web page.
pub fn finish_what(what: &Content, at: Point<Pixels>, from: From, cx: &mut App) -> Result<Option<String>, String> {
    let over = cx.try_global::<Over>().and_then(|o| o.0.clone());
    let outside = !crate::windows::frames_front_to_back(cx).iter().any(|(_, _, f)| f.contains(&at));
    clear(cx);
    match (over, from, what) {
        (Some((label, space, spot)), _, Content::Files(owner)) => {
            super::dock::dock(owner, &label, &space, &spot, None, cx)?;
            Ok(Some(super::dock::described(owner, cx)))
        }
        (Some((label, space, spot)), _, Content::Web(id)) => {
            crate::web::place::dock(id, &label, &space, &spot, cx)?;
            Ok(Some(crate::web::place::described(id, cx)))
        }
        (Some((label, space, spot)), _, Content::Inbox(key) | Content::View(key)) => {
            crate::main_screen::inbox_window::dock(key, &label, &space, &spot, cx)?;
            Ok(Some(crate::main_screen::inbox_window::described(key, cx)))
        }
        (Some((label, space, spot)), _, Content::Settings) => {
            let key = crate::main_screen::tree::SETTINGS_KEY;
            crate::main_screen::inbox_window::dock(key, &label, &space, &spot, cx)?;
            Ok(Some(crate::main_screen::inbox_window::described(key, cx)))
        }
        (Some((label, space, spot)), _, Content::Device(d)) => {
            let key = crate::main_screen::tree::Surface::Device(d.clone()).key();
            crate::main_screen::inbox_window::dock(&key, &label, &space, &spot, cx)?;
            Ok(Some(crate::main_screen::inbox_window::described(&key, cx)))
        }
        (None, From::Tile, Content::Files(owner)) if outside => {
            super::dock::pop_out(owner, Some(at), cx)?;
            Ok(Some("a window of its own".into()))
        }
        (None, From::Tile, Content::Web(id)) if outside => {
            let front = crate::platform::app_in_front(cx);
            crate::web::place::pop_out(id, Some(at), front, cx)?;
            Ok(Some("a window of its own".into()))
        }
        (None, From::Tile, Content::Inbox(key) | Content::View(key)) if outside => {
            let front = crate::platform::app_in_front(cx);
            crate::main_screen::inbox_window::pop_out(key, Some(at), front, cx)?;
            Ok(Some("a window of its own".into()))
        }
        (None, From::Tile, Content::Settings) if outside => {
            let front = crate::platform::app_in_front(cx);
            crate::main_screen::inbox_window::pop_out(crate::main_screen::tree::SETTINGS_KEY, Some(at), front, cx)?;
            Ok(Some("a window of its own".into()))
        }
        (None, From::Tile, Content::Device(d)) if outside => {
            let front = crate::platform::app_in_front(cx);
            let key = crate::main_screen::tree::Surface::Device(d.clone()).key();
            crate::main_screen::inbox_window::pop_out(&key, Some(at), front, cx)?;
            Ok(Some("a window of its own".into()))
        }
        _ => Ok(None),
    }
}

/// Where a drag would land now (debug).
pub fn over(cx: &App) -> Option<(String, String, FilesSpot)> {
    cx.try_global::<Over>().and_then(|o| o.0.clone())
}

/// Debug builds, `files-place …` (bench commands; nothing is moved or
/// clicked on the screen, the same code a real drag or menu pick runs):
///
/// - `<agent> right|left|above|below|replace [of]`, `edge-right|edge-left|
///   edge-top|edge-bottom`, `own`, `window`: the "Dock…" / "Move…" menu's
///   places (`of`: next to, or in place of, that agent's terminal, by id).
/// - `drag <agent> <window> <x> <y> [drop]`: the grip dragged to (`x`,
///   `y`) in Pitwall window `<window>`'s content (`out`: that far below
///   the main window, outside it), and let go there with `drop`.
/// - `state <agent>`: where its files are (on stderr).
/// - `orphan <agent> save|discard`: a removed agent's kept files' Save or
///   Discard.
pub fn debug(what: &str, cx: &mut App) {
    use super::dock::FilesPlace;
    use crate::main_screen::tree::Side;
    let words: Vec<&str> = what.split_whitespace().collect();
    let say = |s: String| eprintln!("pitwall-debug: files {s}");
    match words.as_slice() {
        ["state", agent] => {
            let tile = super::dock::holder(agent, cx);
            let window = crate::windows::viewer_window(agent, cx).is_some();
            let kept = super::dock::orphaned(agent, cx);
            // The tree's top folder, with each entry's git status.
            if let Some(v) = super::dock::viewer(agent, cx) {
                let tree = v.read(cx).tree();
                let top = tree.read(cx).state().listings.get("").map(|l| {
                    l.entries.iter().map(|e| format!("{}{}", e.name, e.status.map(|s| format!(" {s:?}")).unwrap_or_default())).collect::<Vec<_>>()
                });
                say(format!("{agent}: tree {top:?}"));
            }
            let key = super::dock::key(agent, cx)
                .or_else(|| crate::windows::viewer_window(agent, cx).and_then(|h| h.read(cx).ok().map(|w| w.key().to_string())));
            say(format!(
                "{agent}: tile {:?} ({}), window {window}, key {key:?}, kept for edits {kept}, over {:?}",
                tile,
                super::dock::described(agent, cx),
                over(cx)
            ));
        }
        ["orphan", agent, "save"] => super::dock::save_orphan(agent, cx),
        ["orphan", agent, "discard"] => super::dock::discard_orphan(agent, cx),
        ["drag", agent, label, x, y, rest @ ..] => {
            let from = match crate::windows::viewer_window(agent, cx) {
                Some(h) if super::dock::viewer(agent, cx).is_none() => From::Window(h.into()),
                _ => From::Tile,
            };
            let (x, y) = (px(x.parse::<f32>().unwrap_or(0.)), px(y.parse::<f32>().unwrap_or(0.)));
            let at = if *label == "out" {
                // Below every Pitwall window's bottom (the main one's).
                let Some(h) = crate::windows::main_window("main", cx) else { return };
                let Ok(b) = h.update(cx, |_, window, _| window.bounds()) else { return };
                gpui::point(b.left() + x, b.bottom() + px(40.) + y)
            } else {
                let Some(h) = crate::windows::main_window(label, cx) else { return say(format!("no window {label}")) };
                match h.update(cx, |_, window, _| on_screen(gpui::point(x, y), window)) {
                    Ok(p) => p,
                    Err(_) => return,
                }
            };
            let landing = hover(agent, at, cx);
            // What the drop preview says.
            let preview = crate::windows::main_window(label, cx)
                .and_then(|h| h.update(cx, |view, _, cx| view.screen().and_then(|s| s.read(cx).files_hint().map(|h| h.label.clone()))).ok())
                .flatten();
            say(format!("{agent} over {at:?}: {landing:?}, preview {preview:?}"));
            if rest.first() == Some(&"drop") {
                match finish(agent, at, from, cx) {
                    Ok(r) => say(format!("{agent} dropped: {r:?}")),
                    Err(e) => say(format!("{agent} not dropped: {e}")),
                }
            }
        }
        [agent, place, rest @ ..] => {
            let p = match *place {
                "right" => FilesPlace::Next(Side::Right),
                "left" => FilesPlace::Next(Side::Left),
                "above" => FilesPlace::Next(Side::Top),
                "below" => FilesPlace::Next(Side::Bottom),
                "edge-right" => FilesPlace::Edge(Side::Right),
                "edge-left" => FilesPlace::Edge(Side::Left),
                "edge-top" => FilesPlace::Edge(Side::Top),
                "edge-bottom" => FilesPlace::Edge(Side::Bottom),
                "own" => FilesPlace::Own,
                "window" => FilesPlace::Window,
                "replace" => FilesPlace::Replace,
                _ => return say(format!("no place {place}")),
            };
            match super::dock::place(agent, p, rest.first().copied(), cx) {
                Ok(at) => say(format!("{agent} placed: {at}")),
                Err(e) => say(format!("{agent} not placed: {e}")),
            }
        }
        _ => say(format!("what? {what}")),
    }
}
