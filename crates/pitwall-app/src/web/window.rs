//! A web page in a window of its own (docs/spec/web.md, "In a window"):
//! "Open in a window of its own" on a web tile's bar, `where: window` from
//! an agent's `show_web`, or a tile dragged out of every Pitwall window. A
//! normal OS window titled with the page ("<title> — Web"), with Pitwall's
//! chrome; its place and size are kept in `ui.json` (`webWindow`) for the
//! next one, and the page itself (`webTiles`, `window: true`) opens again
//! in one at the next start. Its bar's grip drags it into a Pitwall
//! window's layout and "Dock…" places it from a menu: the same page moves
//! into a tile and the window closes. Pin keeps it above other apps where
//! the system lets Pitwall (as a files window).
//!
//! The page ([`super::page::WebPage`]) is the same one a tile drew: on
//! macOS its native view is moved into this window, so it doesn't load
//! again; elsewhere it is made again here.

use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, size, App, Bounds, Context, FocusHandle, Point, Pixels, Task, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions,
};

use super::page::{Host, HostKind};

const DEFAULT_SIZE: (f32, f32) = (1100., 760.);
const MIN_SIZE: (f32, f32) = (420., 300.);

pub struct WebWindow {
    /// The page's tile id.
    id: String,
    focus: FocusHandle,
    save: Option<Task<()>>,
    /// The page moved out or went: its closing isn't the user's.
    leaving: bool,
    _subs: Vec<gpui::Subscription>,
}

/// What a window title says: the page's title (else its host) and "Web".
pub fn title(page_title: &str, url: &str) -> String {
    let name = if !page_title.trim().is_empty() {
        page_title.trim().to_string()
    } else if !url.is_empty() {
        super::page::host(url)
    } else {
        "Web page".to_string()
    };
    format!("{name} — Web")
}

impl WebWindow {
    pub fn new(id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subs = vec![
            cx.observe_window_appearance(window, |_, window, cx| {
                crate::theme::os_changed(cx);
                crate::theme::apply_window(window, cx);
            }),
            crate::settings::attach_window(window, cx),
            cx.observe_window_bounds(window, |this, window, cx| this.bounds_changed(window, cx)),
        ];
        // The user closing it closes the page (not when it moves into a
        // layout, nor when Pitwall quits: it opens again next time).
        let me = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let Some(w) = me.upgrade() else { return true };
            let (leaving, id) = (w.read(cx).leaving, w.read(cx).id.clone());
            if !leaving && !crate::windows::quitting(cx) {
                cx.defer(move |cx| super::pages::close(&id, cx));
            }
            true
        });
        // Each frame: the page's view placed where it was drawn.
        let me = cx.entity().downgrade();
        window.on_frame_drawn(move |window, cx| me.update(cx, |w, cx| w.frame(window, cx)).unwrap_or(false));
        WebWindow { id, focus: cx.focus_handle(), save: None, leaving: false, _subs: subs }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// The page is moving out (or went): the window closes next, and that
    /// doesn't close the page.
    pub fn closing(&mut self) {
        self.leaving = true;
    }

    /// A frame was drawn: the page's view follows its element. `true`: keep
    /// listening.
    fn frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.leaving {
            return false;
        }
        let Some(page) = super::pages::get(&self.id, cx) else { return !self.leaving };
        // Held by another window now (docked): nothing to place here.
        let mine = page.read(cx).host().is_some_and(|h| h.window.window_id() == window.window_handle().window_id());
        if !mine {
            return true;
        }
        let placed = page.read(cx).slot().and_then(|slot| crate::preview::placed(window.rendered_hitboxes(), window.rendered_tooltip_bounds(), slot));
        let dragging = cx.has_active_drag();
        if page.update(cx, |p, cx| p.frame(placed, dragging, window, cx)) {
            window.refresh();
        }
        true
    }

    /// Kept for the next window, half a second after the last change.
    fn bounds_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kept = crate::explorer::window::kept_bounds(window);
        self.save = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            cx.update(|cx| crate::ui_state::update(cx, |s| s.web_window = Some(kept)));
        }));
    }
}

impl Render for WebWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).clone();
        let host = Host { window: window.window_handle(), focus: self.focus.clone(), kind: HostKind::Window };
        let agents = super::pages::agents(cx);
        let page = if self.leaving { None } else { super::pages::hold(&self.id, host, &agents, window, cx) };
        let root = div()
            .id("web-window")
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
            .child(match page {
                Some(p) => div().flex_1().min_h_0().flex().bg(t.bg).child(p).into_any_element(),
                None => div().flex_1().into_any_element(),
            });
        crate::platform::decorations::frame(root, window, cx)
    }
}

/// Open page `id` (its `ui.json` entry made already) in a window of its
/// own, its top left at `at` on screen (a tile let go outside) or where
/// the last one was. It takes the keyboard only when `focus` (Pitwall is
/// in front); otherwise it is ordered in without activating Pitwall.
pub fn open(id: &str, at: Option<Point<Pixels>>, focus: bool, cx: &mut App) -> Result<WindowHandle<WebWindow>, String> {
    if let Some(h) = crate::windows::web_window(id, cx) {
        return Ok(h);
    }
    let kept = crate::ui_state::get(cx);
    let url = kept.web_tiles.get(id).map(|t| t.url.clone()).unwrap_or_default();
    let page_title = super::pages::get(id, cx).map(|p| p.read(cx).title().to_string()).unwrap_or_default();
    let nth = crate::windows::web_window_ids(cx).len();
    let options = options(kept.web_window, nth, at, focus, title(&page_title, &url), cx);
    let i = id.to_string();
    let handle = cx
        .open_window(options, move |window, cx| {
            crate::platform::lean_renderer(window);
            cx.new(|cx| WebWindow::new(i, window, cx))
        })
        .map_err(|e| e.to_string())?;
    crate::windows::add_web_window(id, handle, cx);
    Ok(handle)
}

/// A window of its own for a tile's content (a web page, an inbox): its
/// top left at `at` on screen (a tile let go outside), else where the
/// last one was (`kept`), cascaded from it as the `nth`; titled `title`.
/// It takes the keyboard only when `focus`; a bench instance's is off
/// screen and never focused.
pub fn options(kept: Option<[f64; 4]>, nth: usize, at: Option<Point<Pixels>>, focus: bool, title: String, cx: &mut App) -> WindowOptions {
    let screens: Vec<_> = cx.displays().iter().map(|d| d.bounds()).collect();
    let bounds = match (crate::explorer::window::placement(kept, nth, &screens), at) {
        (b, Some(at)) => {
            let size = b.map_or(size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)), |b| b.size);
            WindowBounds::Windowed(Bounds::new(crate::explorer::window::dropped_at(at, size, &screens), size))
        }
        (Some(b), None) => WindowBounds::Windowed(b),
        (None, None) => {
            let b = Bounds::centered(None, size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)), cx);
            let step = px(26. * nth as f32);
            WindowBounds::Windowed(Bounds::new(point(b.origin.x + step, b.origin.y + step), b.size))
        }
    };
    // Bench: shown and drawn, but off-screen, never focused (`bench.rs`),
    // as the main window.
    let (bounds, focus) = if crate::bench::enabled() && !crate::bench::visible() {
        let mut b = bounds.get_bounds();
        b.origin = point(px(crate::bench::OFF_SCREEN.0), px(crate::bench::OFF_SCREEN.1 + 900.));
        (WindowBounds::Windowed(b), false)
    } else {
        (bounds, focus)
    };
    WindowOptions {
        window_bounds: Some(bounds),
        titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
        focus,
        show: true,
        window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        window_decorations: crate::platform::decorations::request(),
        app_id: Some("dev.pitwall.app".into()),
        ..Default::default()
    }
}

/// At start: the pages left in windows of their own open in one again,
/// behind whatever is in front.
pub fn restore(cx: &mut App) {
    let ids: Vec<String> = crate::ui_state::get(cx).web_tiles.iter().filter(|(_, t)| t.window).map(|(id, _)| id.clone()).collect();
    for id in ids {
        if let Err(e) = open(&id, None, false, cx) {
            eprintln!("pitwall: could not open the web page's window again: {e}");
        }
    }
}

/// The window holding page `id` closes without closing the page (it moved
/// into a layout).
pub fn close_for_move(id: &str, cx: &mut App) {
    if let Some(h) = crate::windows::web_window(id, cx) {
        let _ = h.update(cx, |w, _, cx| {
            w.closing();
            cx.notify();
        });
        cx.defer(move |cx| {
            let _ = h.update(cx, |_, window, _| window.remove_window());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_names_the_page() {
        assert_eq!(title("Example Domain", "https://example.com/"), "Example Domain — Web");
        assert_eq!(title("", "https://example.com/docs?x=1"), "example.com — Web");
        assert_eq!(title("  ", ""), "Web page — Web");
    }
}
