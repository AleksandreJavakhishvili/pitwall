//! An extension's inbox in a window of its own, and where an inbox goes
//! (docs/spec/extensions.md "The inbox tile"): "Open in a window of its
//! own" on its bar, "Window of its own" in Move…, or its tile dragged out
//! of every Pitwall window (macOS). Back into a layout with "Dock…" or its
//! grip dragged over a Pitwall window. The places, the spots and the drag
//! are the web pages' ([`crate::web::place`], [`crate::explorer::drag`]);
//! the window is a web page's window's look and options
//! ([`crate::web::window::options`]). The inbox itself is one view per
//! extension wherever it is drawn ([`super::inbox_tile::view`]): moving it
//! keeps its thread, its drafts and the adapter (the extension's hub).
//!
//! Kept in `ui.json`: the inbox in a window (`extensions.<id>.window`)
//! opens in one again at the next start, behind the rest, without the
//! keyboard; its place and size (`inboxWindow`) are kept for the next one.
//! Closing the window closes the inbox; quitting doesn't.
//!
//! An extension's views ([`super::view_tile`]) move the same way: each
//! function here takes a surface's key ([`Surface::key`]: the extension's
//! id for its inbox, `<extension>/<view>` for a view), and a view in a
//! window is kept in `extensions.<id>.viewWindows`.

use std::time::Duration;

use gpui::{div, prelude::*, px, App, AppContext, Context, FocusHandle, Pixels, Point, Task, Window, WindowHandle};

use super::inbox_tile;
use super::tree::Surface;
use super::workspace::{whereabouts, FilesSpot};
use crate::explorer::dock::FilesPlace;

pub struct InboxWindow {
    /// What it shows: its surface's key.
    ext: String,
    focus: FocusHandle,
    save: Option<Task<()>>,
    /// The inbox moved into a layout: its window's closing isn't the
    /// user's.
    leaving: bool,
    _subs: Vec<gpui::Subscription>,
}

/// What a window title says: the extension's name and "Inbox", or the
/// view's title.
pub fn title(key: &str, cx: &App) -> String {
    let s = Surface::parse(key);
    let m = crate::extensions::manifest_of(s.ext(), cx);
    let name = m.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| s.ext().to_string());
    match &s {
        Surface::Inbox(_) => format!("{name} — Inbox"),
        Surface::View(_, v) => format!("{name} — {}", m.as_ref().and_then(|m| m.contributes.view(v)).map_or(v.as_str(), |d| d.title.as_str())),
    }
}

impl InboxWindow {
    pub fn new(ext: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hub = crate::extensions::hub::hub(cx);
        let subs = vec![
            cx.observe_window_appearance(window, |_, window, cx| {
                crate::theme::os_changed(cx);
                crate::theme::apply_window(window, cx);
            }),
            crate::settings::attach_window(window, cx),
            cx.observe_window_bounds(window, |this, window, cx| this.bounds_changed(window, cx)),
            // The adapter's threads and Settings: drawn again.
            cx.observe(&hub, |_, _, cx| cx.notify()),
        ];
        // The user closing it closes the inbox (not when it moves into a
        // layout, nor when Pitwall quits: it opens again next time).
        let me = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let Some(w) = me.upgrade() else { return true };
            let (leaving, ext) = (w.read(cx).leaving, w.read(cx).ext.clone());
            if !leaving && !crate::windows::quitting(cx) {
                cx.defer(move |cx| match Surface::parse(&ext) {
                    Surface::Inbox(e) => inbox_tile::close(&e, cx),
                    Surface::View(..) => super::view_tile::close(&ext, cx),
                });
            }
            true
        });
        InboxWindow { ext, focus: cx.focus_handle(), save: None, leaving: false, _subs: subs }
    }

    pub fn ext(&self) -> &str {
        &self.ext
    }

    /// The inbox is moving out: the window closes next, and that doesn't
    /// close the inbox.
    pub fn closing(&mut self) {
        self.leaving = true;
    }

    /// Kept for the next window, half a second after the last change.
    fn bounds_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kept = crate::explorer::window::kept_bounds(window);
        self.save = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            cx.update(|cx| crate::ui_state::update(cx, |s| s.inbox_window = Some(kept)));
        }));
    }
}

impl Render for InboxWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).clone();
        let body = (!self.leaving).then(|| held(&self.ext, cx));
        // Nothing has the keyboard in it: the inbox (or view) takes it.
        if let Some((_, f)) = &body {
            if window.focused(cx).is_none() && window.is_window_active() {
                window.focus(f, cx);
            }
        }
        let body = body.map(|(v, _)| v);
        let root = div()
            .id("inbox-window")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(crate::theme::backdrop(cx))
            .font_family(crate::kit::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(crate::kit::BODY_LINE_HEIGHT))
            .text_color(t.text)
            .child(match body {
                Some(v) => div().flex_1().min_h_0().flex().bg(t.bg).child(v).into_any_element(),
                None => div().flex_1().into_any_element(),
            });
        crate::platform::decorations::frame(root, window, cx)
    }
}

/// The inbox or view of surface `key`, held by a window of its own: what
/// is drawn, and what takes the keyboard.
fn held(key: &str, cx: &mut App) -> (gpui::AnyView, FocusHandle) {
    let agents = crate::web::pages::agents(cx);
    match Surface::parse(key) {
        Surface::Inbox(e) => {
            let v = inbox_tile::view(&e, cx);
            v.update(cx, |v, cx| v.held(true, agents, cx));
            let f = gpui::Focusable::focus_handle(v.read(cx), cx);
            (v.into(), f)
        }
        Surface::View(..) => {
            let v = super::view_tile::view(key, cx);
            v.update(cx, |v, cx| v.held(true, cx));
            let f = gpui::Focusable::focus_handle(v.read(cx), cx);
            (v.into(), f)
        }
    }
}

/// Open extension `ext`'s inbox (its `ui.json` entry made already) in a
/// window of its own, its top left at `at` on screen (a tile let go
/// outside) or where the last one was. It takes the keyboard only when
/// `focus` (Pitwall is in front); otherwise it is ordered in without
/// activating Pitwall.
pub fn open(ext: &str, at: Option<Point<Pixels>>, focus: bool, cx: &mut App) -> Result<WindowHandle<InboxWindow>, String> {
    if let Some(h) = crate::windows::inbox_window(ext, cx) {
        return Ok(h);
    }
    let kept = crate::ui_state::get(cx).inbox_window;
    let nth = crate::windows::inbox_windows(cx);
    let options = crate::web::window::options(kept, nth, at, focus, title(ext, cx), cx);
    let focus = options.focus;
    let e = ext.to_string();
    let handle = cx
        .open_window(options, move |window, cx| {
            crate::platform::lean_renderer(window);
            cx.new(|cx| InboxWindow::new(e, window, cx))
        })
        .map_err(|e| e.to_string())?;
    crate::windows::add_inbox_window(ext, handle, cx);
    if focus {
        let (_, f) = held(ext, cx);
        let _ = handle.update(cx, |_, window, cx| window.focus(&f, cx));
    }
    Ok(handle)
}

/// At start: the inboxes left in windows of their own open in one again,
/// behind whatever is in front (those of extensions no longer found are
/// forgotten).
pub fn restore(cx: &mut App) {
    let ui = crate::ui_state::get(cx);
    let mut kept: Vec<Surface> = Vec::new();
    for (id, e) in &ui.extensions {
        if e.window {
            kept.push(Surface::Inbox(id.clone()));
        }
        kept.extend(e.view_windows.iter().map(|v| Surface::View(id.clone(), v.clone())));
    }
    for s in kept {
        // An extension no longer found, or a view its manifest no longer
        // has: forgotten.
        let m = crate::extensions::manifest_of(s.ext(), cx);
        let there = match &s {
            Surface::Inbox(_) => m.is_some(),
            Surface::View(_, v) => m.is_some_and(|m| m.contributes.view(v).is_some()),
        };
        if !there {
            crate::ui_state::update(cx, |ui| *ui = ui.clone().close_surface(&s));
            continue;
        }
        if let Err(e) = open(&s.key(), None, false, cx) {
            eprintln!("pitwall: could not open the extension's window again: {e}");
        }
    }
}

/// The window holding extension `ext`'s inbox closes without closing the
/// inbox (it moved into a layout, or it closes another way).
pub fn close_for_move(ext: &str, cx: &mut App) {
    if let Some(h) = crate::windows::inbox_window(ext, cx) {
        let _ = h.update(cx, |w, _, cx| {
            w.closing();
            cx.notify();
        });
        cx.defer(move |cx| {
            let _ = h.update(cx, |_, window, _| window.remove_window());
        });
    }
}

/// Extension `ext`'s inbox out of its tile into a window of its own (at
/// `at` on screen, else where the last one was): the same inbox. The
/// window takes the keyboard only with `focus`.
pub fn pop_out(ext: &str, at: Option<Point<Pixels>>, focus: bool, cx: &mut App) -> Result<WindowHandle<InboxWindow>, String> {
    crate::ui_state::update(cx, |ui| *ui = ui.clone().surface_to_window(&Surface::parse(ext)));
    open(ext, at, focus, cx)
}

/// Put extension `ext`'s inbox at `spot` of space `space` (in Pitwall
/// window `label`): its tile moved there, or its window's inbox moved in
/// (the window closes).
pub fn dock(ext: &str, label: &str, space: &str, spot: &FilesSpot, cx: &mut App) -> Result<(), String> {
    let s = Surface::parse(ext);
    crate::web::place::land(&s.content(), label, space, cx, |ui, fit| ui.surface_to_layout(space, &s, spot, fit))?;
    close_for_move(ext, cx);
    Ok(())
}

/// Move extension `ext`'s inbox (a tile or a window) to `place` (`of`: in
/// that agent's terminal's place). Returns where, in words.
pub fn place(ext: &str, place: FilesPlace, of: Option<&str>, cx: &mut App) -> Result<String, String> {
    if place == FilesPlace::Window {
        if crate::windows::inbox_window(ext, cx).is_none() {
            let front = crate::platform::app_in_front(cx);
            pop_out(ext, None, front, cx)?;
        }
        return Ok("a window of its own".into());
    }
    let what = Surface::parse(ext).content();
    let (label, space, spot) = crate::web::place::spot_near(&what, place, of.map(String::from), cx)?;
    let gone = crate::explorer::dock::replaced_by(&space, &spot, cx);
    dock(ext, &label, &space, &spot, cx)?;
    Ok(crate::explorer::dock::with_replaced(described(ext, cx), gone.as_deref(), cx))
}

/// The agents whose terminals extension `ext`'s inbox could take the place
/// of (`(id, name)`).
pub fn replaceable(ext: &str, cx: &mut App) -> Vec<(String, String)> {
    crate::web::place::replaceable_near(&Surface::parse(ext).content(), None, cx)
}

/// Where extension `ext`'s inbox is, in words ("the inbox tile in space
/// All (window main), a column on the right of the space"; "a window of
/// its own").
pub fn described(ext: &str, cx: &mut App) -> String {
    let ui = crate::ui_state::get(cx);
    // (Its tile first: a window it just left closes after this.)
    let s = Surface::parse(ext);
    let Some((space, pane)) = ui.tile_of(&s.content()) else {
        return if crate::windows::inbox_window(ext, cx).is_some() { "a window of its own" } else { "nowhere" }.into();
    };
    let Some(sp) = ui.space(&space) else { return "nowhere".into() };
    let at = whereabouts(&sp.layout, &pane, None).words(&|a: &str| crate::explorer::dock::agent_name(a, cx));
    let tile = match &s {
        Surface::Inbox(_) => "the inbox tile".to_string(),
        Surface::View(_, v) => format!("the view tile “{v}”"),
    };
    format!("{tile} in space {} (window {}), {at}", sp.name, ui.window_of_space(&space))
}

/// The grip of an inbox in a window of its own was pressed: it is dragged
/// (the window with it) and lands in the layout of the Pitwall window it
/// is let go over ([`crate::explorer::drag`], as files and web pages).
pub fn drag_start(ext: &str, window: &mut Window, cx: &mut App) {
    let from = crate::explorer::drag::From::Window(window.window_handle());
    crate::explorer::drag::start_what(Surface::parse(ext).content(), from, window, cx);
}
