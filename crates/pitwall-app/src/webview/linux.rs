//! The Linux backend of [`super::WebView`]: WebKitGTK (webkit2gtk 4.1, or
//! 4.0), the engine GNOME's browser and Tauri use.
//!
//! - Loaded at run time ([`ffi`]), not linked: Pitwall starts where
//!   WebKitGTK isn't installed, and [`create`] says so (the UI opens the
//!   browser instead). Nothing is loaded until the first preview.
//! - X11: the view sits in a borderless GTK window that is reparented into
//!   GPUI's X11 window (as Tauri's child web views do), so it is a real child
//!   window over the pane. GTK talks to the X server over its own
//!   connection; a child window doesn't care whose connection made it.
//! - Wayland: no in-pane view. A Wayland client can only place surfaces of
//!   its own connection inside its window, and GTK has its own; a separate
//!   top-level window can't be placed over the pane either (Wayland gives
//!   clients no global coordinates). [`create`] returns `Err` and the UI
//!   opens the browser.
//! - GLib's loop: GPUI doesn't run it, so a GPUI task turns it every frame
//!   while a view shows, slowly while all are hidden, and stops with the
//!   last view ([`logic::pump_interval`]).
//! - Keyboard: a click in the page gives its window X input focus (no
//!   window manager does that for a child window); [`WebView::blur`] gives
//!   it back to GPUI's window.

mod ffi;
mod logic;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_uint, c_ulong, CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::rc::Rc;

use gpui::{AsyncApp, Bounds, Keystroke, Modifiers, Pixels, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{allowed, Options, Snapshot, WebEvent, WebView};
use ffi::{api, Api, GError, P};

const WAYLAND: &str = "The in-app preview isn't available on Wayland yet";

/// Xlib's `RevertToParent` and `CurrentTime`.
const REVERT_TO_PARENT: c_int = 2;
const CURRENT_TIME: c_ulong = 0;

pub fn create(window: &mut Window, opts: Options) -> Result<Box<dyn WebView>, String> {
    let parent = x11_parent(window)?;
    let api = api()?;
    gtk_ready(api)?;
    let view = unsafe { Linux::new(api, parent, window.scale_factor(), opts)? };
    start_pump(window);
    Ok(Box::new(view))
}

/// GPUI's X11 window (its XID); `Err` on Wayland.
fn x11_parent(window: &Window) -> Result<c_ulong, String> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|e| format!("The in-app preview couldn't find the window: {e}"))?;
    match handle.as_raw() {
        RawWindowHandle::Xcb(h) => Ok(h.window.get() as c_ulong),
        RawWindowHandle::Xlib(h) => Ok(h.window),
        RawWindowHandle::Wayland(_) => Err(WAYLAND.into()),
        _ => Err("The in-app preview needs an X11 window".into()),
    }
}

thread_local! {
    /// GTK initialized (`Some(true)`), or it couldn't be.
    static GTK: Cell<Option<bool>> = const { Cell::new(None) };
    static PUMP: RefCell<Pump> = RefCell::default();
    /// One WebKit context per data folder (each project's cookies and
    /// storage), kept for the app's life: views of a project share it.
    static CONTEXTS: RefCell<HashMap<PathBuf, P>> = RefCell::default();
}

#[derive(Default)]
struct Pump {
    views: usize,
    visible: usize,
    running: bool,
    /// URLs the pages tried to leave for: opened in the browser by the pump
    /// (it has the app context; GTK's callbacks don't).
    open: Vec<String>,
}

/// GTK, on GPUI's main thread, the first time a preview opens. X11 only:
/// GDK would otherwise pick Wayland when GPUI runs on XWayland.
fn gtk_ready(api: &Api) -> Result<(), String> {
    let ok = GTK.with(|g| {
        if let Some(ok) = g.get() {
            return ok;
        }
        // SAFETY: before any other GTK call, on the main thread. GTK keeps the
        // process's locale as it is (setlocale would change it for everyone).
        let ok = unsafe {
            (api.gdk_set_allowed_backends)(c"x11".as_ptr());
            (api.gtk_disable_setlocale)();
            (api.gtk_init_check)(null_mut(), null_mut()) != 0
        };
        g.set(Some(ok));
        ok
    });
    if ok {
        Ok(())
    } else {
        Err("GTK couldn't open the X display".into())
    }
}

/// Starts turning GLib's loop (once; it stops by itself after the last
/// view). The task needs the app context, which the next frame gives.
fn start_pump(window: &mut Window) {
    if PUMP.with_borrow(|p| p.running) {
        return;
    }
    window.on_next_frame(|_, cx| {
        if PUMP.with_borrow(|p| p.running) {
            return;
        }
        PUMP.with_borrow_mut(|p| p.running = true);
        cx.spawn(async move |cx: &mut AsyncApp| pump(cx).await).detach();
    });
    window.refresh();
}

async fn pump(cx: &mut AsyncApp) {
    let Ok(api) = api() else { return };
    loop {
        let (views, visible) = PUMP.with_borrow(|p| (p.views, p.visible));
        // No views: one last turn (the closed view's teardown), then stop.
        let wait = logic::pump_interval(views, visible).unwrap_or(std::time::Duration::from_millis(50));
        cx.background_executor().timer(wait).await;
        for _ in 0..logic::MAX_DISPATCHES {
            // SAFETY: the main thread, which owns GLib's default context.
            if unsafe { (api.g_main_context_iteration)(null_mut(), 0) } == 0 {
                break;
            }
        }
        let open = PUMP.with_borrow_mut(|p| std::mem::take(&mut p.open));
        if !open.is_empty() {
            let _ = cx.update(|cx| {
                for url in &open {
                    crate::platform::files::open_url(cx, url);
                }
            });
        }
        if PUMP.with_borrow(|p| p.views) == 0 {
            break;
        }
    }
    PUMP.with_borrow_mut(|p| p.running = false);
}

/// The WebKit context for a data folder: cookies, storage and cache there.
unsafe fn context(api: &Api, dir: &Path) -> Result<P, String> {
    if let Some(ctx) = CONTEXTS.with_borrow(|m| m.get(dir).copied()) {
        return Ok(ctx);
    }
    let cache_dir = dir.join("cache");
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("The preview's data folder: {e}"))?;
    let (data, cache, cookies) = (c_path(dir)?, c_path(&cache_dir)?, c_path(&dir.join("cookies.sqlite"))?);
    let ctx = unsafe {
        let manager = (api.webkit_website_data_manager_new)(
            c"base-data-directory".as_ptr(),
            data.as_ptr(),
            c"base-cache-directory".as_ptr(),
            cache.as_ptr(),
            null::<c_char>(),
        );
        if manager.is_null() {
            return Err("WebKitGTK couldn't open the preview's data folder".into());
        }
        let ctx = (api.webkit_web_context_new_with_website_data_manager)(manager);
        (api.g_object_unref)(manager);
        // Cookies are kept in memory unless told where (WEBKIT_COOKIE_PERSISTENT_STORAGE_SQLITE).
        let jar = (api.webkit_web_context_get_cookie_manager)(ctx);
        (api.webkit_cookie_manager_set_persistent_storage)(jar, cookies.as_ptr(), 1);
        ctx
    };
    CONTEXTS.with_borrow_mut(|m| m.insert(dir.to_owned(), ctx));
    Ok(ctx)
}

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| "A path with a NUL byte".to_string())
}

/// A C string from GLib/WebKit (borrowed), as an owned `String`.
unsafe fn string(s: *const c_char) -> String {
    if s.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
    }
}

/// What the signal handlers share with the view; each connection holds a
/// reference, released when GTK frees the object.
struct Inner {
    api: &'static Api,
    /// Off once the view is dropped: late signals (teardown) do nothing.
    alive: Cell<bool>,
    /// The GTK window has X input focus.
    focused: Cell<bool>,
    events: Box<dyn Fn(WebEvent)>,
    view: P,
    xdisplay: P,
    gdk_display: P,
    xid: c_ulong,
    parent: c_ulong,
}

impl Inner {
    fn emit(&self, event: WebEvent) {
        (self.events)(event);
    }

    fn uri(&self) -> String {
        unsafe { string((self.api.webkit_web_view_get_uri)(self.view)) }
    }

    fn can_go(&self) {
        let (back, forward) = unsafe {
            (
                (self.api.webkit_web_view_can_go_back)(self.view) != 0,
                (self.api.webkit_web_view_can_go_forward)(self.view) != 0,
            )
        };
        self.emit(WebEvent::CanGo { back, forward });
    }

    /// X input focus to `window`; errors (it isn't viewable) are ignored
    /// rather than left to GDK's handler, which aborts on unexpected ones.
    fn set_x_focus(&self, window: c_ulong) {
        let api = self.api;
        unsafe {
            (api.gdk_x11_display_error_trap_push)(self.gdk_display);
            (api.XSetInputFocus)(self.xdisplay, window, REVERT_TO_PARENT, CURRENT_TIME);
            (api.XFlush)(self.xdisplay);
            (api.gdk_x11_display_error_trap_pop_ignored)(self.gdk_display);
        }
    }

    fn take_focus(&self) {
        self.set_x_focus(self.xid);
        unsafe { (self.api.gtk_widget_grab_focus)(self.view) };
    }
}

/// `data` of a handler as the view's [`Inner`], unless the view is gone.
unsafe fn inner<'a>(data: P) -> Option<&'a Inner> {
    let inner = unsafe { &*(data as *const Inner) };
    inner.alive.get().then_some(inner)
}

unsafe extern "C" fn release(data: P, _closure: P) {
    drop(unsafe { Rc::from_raw(data as *const Inner) });
}

/// Connects `handler` (an `extern "C"` fn of the signal's signature, as a
/// pointer) to `signal` on `object`.
unsafe fn connect(inner: &Rc<Inner>, object: P, signal: &CStr, handler: *const ()) {
    let data = Rc::into_raw(inner.clone()) as P;
    unsafe {
        let handler = std::mem::transmute::<*const (), unsafe extern "C" fn()>(handler);
        (inner.api.g_signal_connect_data)(object, signal.as_ptr(), Some(handler), data, Some(release), 0);
    }
}

// WebKitLoadEvent
const LOAD_STARTED: c_int = 0;
const LOAD_COMMITTED: c_int = 2;
const LOAD_FINISHED: c_int = 3;

unsafe extern "C" fn on_load_changed(_view: P, event: c_int, data: P) {
    let Some(i) = (unsafe { inner(data) }) else { return };
    match event {
        LOAD_STARTED => i.emit(WebEvent::Loading(true)),
        LOAD_COMMITTED => {
            i.emit(WebEvent::Url(i.uri()));
            i.can_go();
        }
        LOAD_FINISHED => {
            i.emit(WebEvent::Loading(false));
            i.can_go();
        }
        _ => {}
    }
}

unsafe extern "C" fn on_load_failed(_view: P, _event: c_int, uri: *const c_char, error: *mut GError, data: P) -> c_int {
    let Some(i) = (unsafe { inner(data) }) else { return 0 };
    let api = i.api;
    let (domain, code, message) = unsafe { ((*error).domain, (*error).code, string((*error).message)) };
    // Not failures: a load replaced by another (WEBKIT_NETWORK_ERROR_CANCELLED)
    // or stopped by decide-policy (WEBKIT_POLICY_ERROR_FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE).
    let quiet = unsafe {
        (domain == (api.webkit_network_error_quark)() && code == 302)
            || (domain == (api.webkit_policy_error_quark)() && code == 102)
    };
    if quiet {
        return 0;
    }
    i.emit(WebEvent::Failed { url: unsafe { string(uri) }, error: message });
    // No WebKit error page: the pane says what went wrong.
    1
}

/// `notify::uri`: the address changed (also history.pushState). A URL the
/// page is leaving for (opened in the browser instead) isn't shown.
unsafe extern "C" fn on_uri(_view: P, _pspec: P, data: P) {
    let Some(i) = (unsafe { inner(data) }) else { return };
    let uri = i.uri();
    if allowed(&uri) {
        i.emit(WebEvent::Url(uri));
    }
}

unsafe extern "C" fn on_title(_view: P, _pspec: P, data: P) {
    let Some(i) = (unsafe { inner(data) }) else { return };
    let title = unsafe { string((i.api.webkit_web_view_get_title)(i.view)) };
    i.emit(WebEvent::Title(title));
}

/// The back-forward list's `changed`.
unsafe extern "C" fn on_history(_list: P, _added: P, _removed: P, data: P) {
    if let Some(i) = unsafe { inner(data) } {
        i.can_go();
    }
}

unsafe extern "C" fn on_view_focus_in(_widget: P, _event: P, data: P) -> c_int {
    if let Some(i) = unsafe { inner(data) } {
        i.emit(WebEvent::Focused);
    }
    0
}

/// The GTK window's X focus, tracked even after the view is dropped.
unsafe fn on_window_focus(focused: bool, data: P) {
    let i = unsafe { &*(data as *const Inner) };
    i.focused.set(focused);
}

unsafe extern "C" fn on_window_focus_in(_widget: P, _event: P, data: P) -> c_int {
    unsafe { on_window_focus(true, data) };
    0
}

unsafe extern "C" fn on_window_focus_out(_widget: P, _event: P, data: P) -> c_int {
    unsafe { on_window_focus(false, data) };
    0
}

/// A click in the page: X focus to its window, which no window manager
/// gives a child window.
unsafe extern "C" fn on_button_press(_widget: P, _event: P, data: P) -> c_int {
    if let Some(i) = unsafe { inner(data) } {
        if !i.focused.get() {
            i.set_x_focus(i.xid);
        }
    }
    0
}

/// Ctrl+key (not an editing key) is Pitwall's: swallowed, sent as a
/// shortcut. On the window, so it runs before the page sees the key.
unsafe extern "C" fn on_key_press(_widget: P, event: P, data: P) -> c_int {
    let Some(i) = (unsafe { inner(data) }) else { return 0 };
    let (mut keyval, mut state): (c_uint, c_uint) = (0, 0);
    unsafe {
        if (i.api.gdk_event_get_keyval)(event, &mut keyval) == 0
            || (i.api.gdk_event_get_state)(event, &mut state) == 0
        {
            return 0;
        }
    }
    let Some((key, m)) = logic::shortcut(keyval, state) else { return 0 };
    i.emit(WebEvent::Shortcut(Keystroke {
        modifiers: Modifiers {
            control: m.control,
            alt: m.alt,
            shift: m.shift,
            platform: m.platform,
            function: false,
        },
        key,
        key_char: None,
    }));
    1
}

// WebKitPolicyDecisionType, WebKitNavigationType
const NAVIGATION_ACTION: c_int = 0;
const NEW_WINDOW_ACTION: c_int = 1;
const RESPONSE: c_int = 2;
const LINK_CLICKED: c_int = 0;

/// A URL the preview hands to the browser: anything but a local dev server,
/// except the page's own blank, data and blob documents.
fn leaves(uri: &str) -> bool {
    !allowed(uri) && !["about:", "data:", "blob:"].iter().any(|s| uri.starts_with(s))
}

fn open_in_browser(uri: String) {
    PUMP.with_borrow_mut(|p| p.open.push(uri));
}

/// `decide-policy`: the preview stays on local dev servers. Clicked links
/// elsewhere, and main-frame loads that end elsewhere (redirects, scripts),
/// open in the browser; new windows open here if local, else there.
unsafe extern "C" fn on_decide_policy(view: P, decision: P, kind: c_int, data: P) -> c_int {
    let Some(i) = (unsafe { inner(data) }) else { return 0 };
    let api = i.api;
    unsafe {
        match kind {
            NAVIGATION_ACTION | NEW_WINDOW_ACTION => {
                let action = (api.webkit_navigation_policy_decision_get_navigation_action)(decision);
                let uri = string((api.webkit_uri_request_get_uri)((api.webkit_navigation_action_get_request)(action)));
                if kind == NEW_WINDOW_ACTION {
                    (api.webkit_policy_decision_ignore)(decision);
                    if allowed(&uri) {
                        if let Ok(c) = CString::new(uri) {
                            (api.webkit_web_view_load_uri)(view, c.as_ptr());
                        }
                    } else if leaves(&uri) {
                        open_in_browser(uri);
                    }
                    return 1;
                }
                // Subframes (embeds) load what they like; a clicked link
                // leaves now, before its request.
                let clicked = (api.webkit_navigation_action_get_navigation_type)(action) == LINK_CLICKED
                    && (api.webkit_navigation_action_is_user_gesture)(action) != 0;
                if clicked && leaves(&uri) {
                    (api.webkit_policy_decision_ignore)(decision);
                    open_in_browser(uri);
                    return 1;
                }
                0
            }
            RESPONSE => {
                let response = (api.webkit_response_policy_decision_get_response)(decision);
                let uri = string((api.webkit_uri_response_get_uri)(response));
                let main = match api.webkit_response_policy_decision_is_main_frame_main_resource {
                    Some(is_main) => is_main(decision) != 0,
                    // Before 2.40: the main frame's load is the view's URI.
                    None => uri == i.uri(),
                };
                if main && (uri.starts_with("http://") || uri.starts_with("https://")) && !allowed(&uri) {
                    (api.webkit_policy_decision_ignore)(decision);
                    open_in_browser(uri);
                    return 1;
                }
                0
            }
            _ => 0,
        }
    }
}

/// What a snapshot request carries to its callback.
struct SnapshotJob {
    api: &'static Api,
    done: Box<dyn FnOnce(Option<Snapshot>)>,
}

unsafe extern "C" fn on_snapshot(source: P, result: P, data: P) {
    let job = unsafe { Box::from_raw(data as *mut SnapshotJob) };
    let api = job.api;
    let shot = unsafe {
        let mut error: *mut GError = null_mut();
        let surface = (api.webkit_web_view_get_snapshot_finish)(source, result, &mut error);
        if !error.is_null() {
            (api.g_error_free)(error);
        }
        if surface.is_null() {
            None
        } else {
            let shot = read_surface(api, surface);
            (api.cairo_surface_destroy)(surface);
            shot
        }
    };
    (job.done)(shot);
}

/// A cairo image surface (ARGB32 or RGB24) as a [`Snapshot`].
unsafe fn read_surface(api: &Api, surface: P) -> Option<Snapshot> {
    unsafe {
        (api.cairo_surface_flush)(surface);
        let format = (api.cairo_image_surface_get_format)(surface);
        let (w, h) = ((api.cairo_image_surface_get_width)(surface), (api.cairo_image_surface_get_height)(surface));
        let stride = (api.cairo_image_surface_get_stride)(surface);
        let pixels = (api.cairo_image_surface_get_data)(surface);
        // CAIRO_FORMAT_ARGB32 = 0, CAIRO_FORMAT_RGB24 = 1.
        if pixels.is_null() || !(0..=1).contains(&format) || w <= 0 || h <= 0 || stride < w * 4 {
            return None;
        }
        let (w, h, stride) = (w as usize, h as usize, stride as usize);
        let data = std::slice::from_raw_parts(pixels, stride * h);
        Some(Snapshot {
            width: w as u32,
            height: h as u32,
            rgba: logic::cairo_to_rgba(data, w, h, stride, format == 1),
        })
    }
}

struct Linux {
    api: &'static Api,
    /// The GTK window (our reference) and the web view in it (ours too).
    window: P,
    view: P,
    inner: Rc<Inner>,
    /// GPUI's scale (X11: one for the whole screen, from Xft.dpi).
    scale: f32,
    rect: Option<(i32, i32, i32, i32)>,
    visible: bool,
}

impl Linux {
    unsafe fn new(api: &'static Api, parent: c_ulong, scale: f32, opts: Options) -> Result<Self, String> {
        let ctx = unsafe { context(api, &opts.data_dir)? };
        unsafe {
            let view = (api.webkit_web_view_new_with_context)(ctx);
            if view.is_null() {
                return Err("WebKitGTK couldn't make a web view".into());
            }
            (api.g_object_ref_sink)(view);
            // GTK_WINDOW_TOPLEVEL, borderless; mapped inside GPUI's window,
            // so no window manager ever sees it.
            let window = (api.gtk_window_new)(0);
            (api.g_object_ref)(window);
            (api.gtk_window_set_decorated)(window, 0);
            (api.gtk_window_set_skip_taskbar_hint)(window, 1);
            (api.gtk_window_set_skip_pager_hint)(window, 1);
            (api.gtk_container_add)(window, view);
            (api.gtk_widget_grab_focus)(view);
            (api.gtk_widget_realize)(window);
            let gdk_window = (api.gtk_widget_get_window)(window);
            let gdk_display = (api.gdk_window_get_display)(gdk_window);
            let xdisplay = (api.gdk_x11_display_get_xdisplay)(gdk_display);
            let xid = (api.gdk_x11_window_get_xid)(gdk_window);
            (api.gdk_x11_display_error_trap_push)(gdk_display);
            (api.XReparentWindow)(xdisplay, xid, parent, 0, 0);
            (api.XFlush)(xdisplay);
            (api.gdk_x11_display_error_trap_pop_ignored)(gdk_display);

            let zoom = logic::page_zoom(scale, (api.gtk_widget_get_scale_factor)(window));
            if (zoom - 1.0).abs() > 0.01 {
                (api.webkit_web_view_set_zoom_level)(view, zoom);
            }

            let inner = Rc::new(Inner {
                api,
                alive: Cell::new(true),
                focused: Cell::new(false),
                events: opts.events,
                view,
                xdisplay,
                gdk_display,
                xid,
                parent,
            });
            let handlers: [(P, &CStr, *const ()); 11] = [
                (view, c"load-changed", on_load_changed as *const ()),
                (view, c"load-failed", on_load_failed as *const ()),
                (view, c"notify::uri", on_uri as *const ()),
                (view, c"notify::title", on_title as *const ()),
                (view, c"decide-policy", on_decide_policy as *const ()),
                (view, c"focus-in-event", on_view_focus_in as *const ()),
                (view, c"button-press-event", on_button_press as *const ()),
                (window, c"key-press-event", on_key_press as *const ()),
                (window, c"focus-in-event", on_window_focus_in as *const ()),
                (window, c"focus-out-event", on_window_focus_out as *const ()),
                ((api.webkit_web_view_get_back_forward_list)(view), c"changed", on_history as *const ()),
            ];
            for (object, signal, handler) in handlers {
                connect(&inner, object, signal, handler);
            }

            if let Ok(url) = CString::new(opts.url) {
                (api.webkit_web_view_load_uri)(view, url.as_ptr());
            }
            PUMP.with_borrow_mut(|p| p.views += 1);
            Ok(Linux { api, window, view, inner, scale, rect: None, visible: false })
        }
    }

    /// Moves and sizes the GTK window to `rect` (GTK's own calls, so it
    /// doesn't put it back on its next configure).
    fn place(&self, (x, y, w, h): (i32, i32, i32, i32)) {
        unsafe {
            (self.api.gtk_window_move)(self.window, x, y);
            (self.api.gtk_window_resize)(self.window, w, h);
        }
    }
}

impl WebView for Linux {
    fn navigate(&mut self, url: &str) {
        if let Ok(url) = CString::new(url) {
            unsafe { (self.api.webkit_web_view_load_uri)(self.view, url.as_ptr()) };
        }
    }

    fn reload(&mut self) {
        unsafe { (self.api.webkit_web_view_reload)(self.view) };
    }

    fn back(&mut self) {
        unsafe { (self.api.webkit_web_view_go_back)(self.view) };
    }

    fn forward(&mut self) {
        unsafe { (self.api.webkit_web_view_go_forward)(self.view) };
    }

    fn set_bounds(&mut self, bounds: Bounds<Pixels>) {
        let gdk_scale = unsafe { (self.api.gtk_widget_get_scale_factor)(self.window) };
        let rect = logic::gtk_rect(
            bounds.origin.x.into(),
            bounds.origin.y.into(),
            bounds.size.width.into(),
            bounds.size.height.into(),
            self.scale,
            gdk_scale,
        );
        if self.rect != Some(rect) {
            self.rect = Some(rect);
            self.place(rect);
        }
    }

    fn set_visible(&mut self, visible: bool) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        PUMP.with_borrow_mut(|p| {
            if visible {
                p.visible += 1;
            } else {
                p.visible = p.visible.saturating_sub(1);
            }
        });
        unsafe {
            if visible {
                if let Some(rect) = self.rect {
                    self.place(rect);
                }
                (self.api.gtk_widget_show_all)(self.window);
            } else {
                // Unmapped: WebKit marks the page hidden and throttles it; X
                // focus, if it had it, reverts to GPUI's window.
                (self.api.gtk_widget_hide)(self.window);
            }
        }
    }

    fn focus(&mut self) {
        if self.visible {
            self.inner.take_focus();
        }
    }

    fn blur(&mut self) {
        // Only from the view: never pull focus from another app.
        if self.inner.focused.get() {
            self.inner.set_x_focus(self.inner.parent);
        }
    }

    fn open_inspector(&mut self) {
        unsafe {
            let settings = (self.api.webkit_web_view_get_settings)(self.view);
            (self.api.webkit_settings_set_enable_developer_extras)(settings, 1);
            (self.api.webkit_web_inspector_show)((self.api.webkit_web_view_get_inspector)(self.view));
        }
    }

    fn snapshot(&mut self, done: Box<dyn FnOnce(Option<Snapshot>) + 'static>) {
        let job = Box::into_raw(Box::new(SnapshotJob { api: self.api, done })) as P;
        // WEBKIT_SNAPSHOT_REGION_VISIBLE, WEBKIT_SNAPSHOT_OPTIONS_NONE.
        unsafe { (self.api.webkit_web_view_get_snapshot)(self.view, 0, 0, null_mut(), Some(on_snapshot), job) };
    }
}

impl Drop for Linux {
    fn drop(&mut self) {
        self.inner.alive.set(false);
        PUMP.with_borrow_mut(|p| {
            p.views = p.views.saturating_sub(1);
            if self.visible {
                p.visible = p.visible.saturating_sub(1);
            }
        });
        // Destroying the window takes the view with it (and X focus reverts
        // to GPUI's window). Our references keep the pointers valid even if
        // GPUI's window went first (X destroyed ours with it).
        unsafe {
            (self.api.gtk_widget_destroy)(self.window);
            (self.api.g_object_unref)(self.view);
            (self.api.g_object_unref)(self.window);
        }
    }
}
