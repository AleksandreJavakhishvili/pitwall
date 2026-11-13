//! macOS: the Dock badge, the attention bounce and the About panel, through
//! AppKit (gpui has none of them). Everything here runs on the main thread,
//! where gpui calls us.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSRequestUserAttentionType};
use objc2_foundation::{NSBundle, NSString};

fn app() -> Option<objc2::rc::Retained<NSApplication>> {
    MainThreadMarker::new().map(NSApplication::sharedApplication)
}

/// The running bundle's identifier (`None` for an unbundled dev build).
pub fn bundle_id() -> Option<String> {
    NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|s| s.to_string())
}

/// The Dock badge: the number of agents needing the user; none at 0.
pub fn set_badge(count: usize) {
    let Some(app) = app() else { return };
    let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
    app.dockTile().setBadgeLabel(label.as_deref());
}

/// Pitwall is the app the user is in: the one whose menu bar is up.
/// Not `NSApplication.isActive`: that is also true while the floating
/// control (a panel that doesn't activate the app) is being pressed, when
/// the user is still in another app.
pub fn app_active() -> bool {
    use objc2_app_kit::NSWorkspace;
    match NSWorkspace::sharedWorkspace().menuBarOwningApplication() {
        Some(owner) => {
            // SAFETY: `processIdentifier` is NSRunningApplication's pid_t property.
            let pid: i32 = unsafe { objc2::msg_send![&*owner, processIdentifier] };
            pid as u32 == std::process::id()
        }
        None => app().is_some_and(|app| app.isActive()),
    }
}

/// `NSApplication.isActive`, for the floating control's log.
#[cfg_attr(test, allow(dead_code))]
pub fn ns_app_active() -> bool {
    app().is_some_and(|app| app.isActive())
}

/// Resize a window to `width` × `height` points, keeping its bottom right
/// corner where it is, so it opens upward and to the left: the floating
/// control, which starts in the screen's bottom right corner. A window
/// that would then leave its screen's usable area at the top or the left
/// is moved back into it. Done once the caller has returned: AppKit tells
/// GPUI about the new size while it is set, and GPUI can't hear it during
/// an update of that window (GPUI's own `resize` waits the same way).
#[cfg_attr(test, allow(dead_code))]
pub fn resize_from_bottom_right(window: &gpui::Window, width: f32, height: f32, cx: &mut gpui::App) {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSView;
    use objc2_foundation::NSRect;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: GPUI hands out its live NSView; we only retain it.
    let view: Option<Retained<NSView>> = unsafe { Retained::retain(h.ns_view.as_ptr().cast()) };
    let Some(w) = view.and_then(|v| v.window()) else { return };
    cx.spawn(async move |_| {
        let mut frame = w.frame();
        // AppKit measures from the bottom left: the bottom stays, the left moves.
        frame.origin.x += frame.size.width - width as f64;
        frame.size.width = width as f64;
        frame.size.height = height as f64;
        // SAFETY: `screen` and `visibleFrame` are NSWindow / NSScreen
        // methods; the screen is checked before it is asked.
        unsafe {
            let screen: *mut AnyObject = msg_send![&*w, screen];
            if !screen.is_null() {
                let area: NSRect = msg_send![screen, visibleFrame];
                let top = area.origin.y + area.size.height;
                if frame.origin.y + frame.size.height > top {
                    frame.origin.y = top - frame.size.height;
                }
                if frame.origin.x < area.origin.x {
                    frame.origin.x = area.origin.x;
                }
            }
        }
        w.setFrame_display(frame, true);
    })
    .detach();
}

/// The window behind a GPUI window.
fn ns_window(window: &gpui::Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2::rc::Retained;
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
    // SAFETY: GPUI hands out its live NSView; we only retain it.
    let view: Option<Retained<NSView>> = unsafe { Retained::retain(h.ns_view.as_ptr().cast()) };
    view?.window()
}

/// No system shadow or frame line around a window that draws its own
/// (the floating control: the system's follow the window's rectangle,
/// not what is drawn in it).
#[cfg_attr(test, allow(dead_code))]
pub fn no_shadow(window: &gpui::Window) {
    if let Some(w) = ns_window(window) {
        w.setHasShadow(false);
        // GPUI's windows are titled with the content under the title
        // bar: no line under that (invisible) bar either.
        w.setTitlebarAppearsTransparent(true);
        w.setTitlebarSeparatorStyle(objc2_app_kit::NSTitlebarSeparatorStyle::None);
    }
}

/// A panel that takes clicks without becoming the key window (the
/// floating control): the app the user is in keeps the keyboard.
/// `becomesKeyOnlyIfNeeded` is NSPanel's own switch for that; GPUI's
/// pop-ups are panels, and anything else is left alone.
#[cfg_attr(test, allow(dead_code))]
pub fn never_key(window: &gpui::Window) {
    use objc2::{class, msg_send};
    let Some(w) = ns_window(window) else { return };
    // SAFETY: an NSPanel property, set only on an NSPanel.
    unsafe {
        let panel: bool = msg_send![&*w, isKindOfClass: class!(NSPanel)];
        if panel {
            let _: () = msg_send![&*w, setBecomesKeyOnlyIfNeeded: true];
        }
    }
}

/// Give a window the keyboard without bringing its app forward (the
/// floating control, once the user clicks a terminal in it to type).
#[cfg_attr(test, allow(dead_code))]
pub fn take_keys(window: &gpui::Window) {
    if let Some(w) = ns_window(window) {
        w.makeKeyWindow();
    }
}

/// Move a window with the pointer until the left button is released: it
/// keeps the spot under the pointer it has now. Our own loop rather than
/// AppKit's window drag: that one needs the press to reach the window as
/// a title bar press, and a window that never becomes the active app's
/// doesn't get it reliably.
#[cfg_attr(test, allow(dead_code))]
pub fn drag_with_pointer(window: &gpui::Window, cx: &mut gpui::App) {
    use objc2_app_kit::NSEvent;
    use objc2_foundation::NSPoint;
    let Some(w) = ns_window(window) else { return };
    let (at, frame) = (NSEvent::mouseLocation(), w.frame());
    let grab = (at.x - frame.origin.x, at.y - frame.origin.y);
    cx.spawn(async move |cx| {
        // At most a minute: a release that was never seen must not leave
        // the window glued to the pointer.
        for _ in 0..7500 {
            if NSEvent::pressedMouseButtons() & 1 == 0 {
                break;
            }
            let at = NSEvent::mouseLocation();
            let to = NSPoint::new(at.x - grab.0, at.y - grab.1);
            if to != w.frame().origin {
                w.setFrameOrigin(to);
            }
            cx.background_executor().timer(std::time::Duration::from_millis(8)).await;
        }
    })
    .detach();
}

/// Debug builds: hand the window a left click at (`x`, `y`) points from
/// its top left, as AppKit would deliver one (`-[NSWindow sendEvent:]`):
/// through the window's own handling of its title bar area, first clicks
/// and its views. In this process only: no pointer moves, nothing is
/// posted to the system. For `float::debug_hook`.
#[cfg_attr(test, allow(dead_code))]
pub fn debug_click(window: &gpui::Window, x: f32, y: f32, cx: &mut gpui::App) {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_app_kit::{NSEventModifierFlags, NSEventType, NSView};
    use objc2_foundation::NSPoint;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: GPUI hands out its live NSView; we only retain it.
    let view: Option<Retained<NSView>> = unsafe { Retained::retain(h.ns_view.as_ptr().cast()) };
    let Some(w) = view.and_then(|v| v.window()) else { return };
    // After the caller's update: GPUI handles the events as it does real ones.
    cx.spawn(async move |_| {
        let frame = w.frame();
        let at = NSPoint::new(x as f64, frame.size.height - y as f64);
        for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
            // SAFETY: the documented NSEvent constructor and NSWindow
            // methods, with their argument types, on the main thread.
            unsafe {
                let number: isize = msg_send![&*w, windowNumber];
                let event: *mut AnyObject = msg_send![
                    class!(NSEvent),
                    mouseEventWithType: kind,
                    location: at,
                    modifierFlags: NSEventModifierFlags::empty(),
                    timestamp: 0f64,
                    windowNumber: number,
                    context: std::ptr::null_mut::<AnyObject>(),
                    eventNumber: 0isize,
                    clickCount: 1isize,
                    pressure: 1f32
                ];
                if !event.is_null() {
                    let _: () = msg_send![&*w, sendEvent: event];
                }
            }
        }
    })
    .detach();
}

/// Set the pointer: the pointing hand or the arrow. For a window shown
/// while another app is active (the floating control): AppKit's cursor
/// rects, which GPUI uses, only work for the active app's key window.
#[cfg_attr(test, allow(dead_code))]
pub fn set_cursor(pointing: bool) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    if MainThreadMarker::new().is_none() {
        return;
    }
    allow_cursor_in_background();
    // SAFETY: NSCursor class methods returning a shared cursor, and its
    // `set`, on the main thread.
    unsafe {
        let cursor: *mut AnyObject =
            if pointing { msg_send![class!(NSCursor), pointingHandCursor] } else { msg_send![class!(NSCursor), arrowCursor] };
        if !cursor.is_null() {
            let _: () = msg_send![cursor, set];
        }
    }
}

/// Let this app set the pointer while it isn't the active one (once).
/// The window server ignores a background app's cursor unless its
/// connection says `SetsCursorInBackground`; that property has no public
/// API, so the two functions are looked up at run time and skipped when a
/// macOS doesn't have them (the pointer then stays an arrow).
fn allow_cursor_in_background() {
    use std::ffi::{c_char, c_int, c_void};
    use std::sync::Once;
    static ONCE: Once = Once::new();
    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    // `RTLD_DEFAULT` on macOS.
    const DEFAULT: *mut c_void = -2isize as *mut c_void;
    ONCE.call_once(|| {
        // SAFETY: both symbols, when present, have had these signatures
        // since macOS 10.x (`CGSConnectionID` is an int, the key a
        // CFString, the value a CFBoolean; NSString and NSNumber are
        // toll-free bridged to them). Nothing is called if either is absent.
        unsafe {
            let main = dlsym(DEFAULT, c"CGSMainConnectionID".as_ptr());
            let set = dlsym(DEFAULT, c"CGSSetConnectionProperty".as_ptr());
            if main.is_null() || set.is_null() {
                return;
            }
            let main: extern "C" fn() -> c_int = std::mem::transmute(main);
            let set: extern "C" fn(c_int, c_int, *const c_void, *const c_void) -> c_int = std::mem::transmute(set);
            let key = NSString::from_str("SetsCursorInBackground");
            let yes = objc2_foundation::NSNumber::new_bool(true);
            let cid = main();
            set(cid, cid, objc2::rc::Retained::as_ptr(&key).cast(), objc2::rc::Retained::as_ptr(&yes).cast());
        }
    });
}

/// The window a glass pop-up (`kit::pop`) belongs to, kept while it is open.
#[cfg_attr(test, allow(dead_code))]
pub struct PopParent(objc2::rc::Retained<objc2_app_kit::NSWindow>);

thread_local! {
    /// The pop-ups that are open (their `NSWindow`s): what was asked of
    /// one before it closed and runs after is dropped.
    static POPS: std::cell::RefCell<std::collections::HashSet<usize>> = std::cell::RefCell::new(std::collections::HashSet::new());
}

fn pop_key(w: &objc2_app_kit::NSWindow) -> usize {
    std::ptr::from_ref(w) as usize
}

#[cfg_attr(test, allow(dead_code))]
pub fn pop_parent(window: &gpui::Window) -> Option<PopParent> {
    ns_window(window).map(PopParent)
}

/// A new pop-up: no shadow or title bar line of the system's, and never
/// the main window.
#[cfg_attr(test, allow(dead_code))]
pub fn pop_setup(window: &gpui::Window) {
    no_shadow(window);
    if let Some(w) = ns_window(window) {
        pops_never_main();
        POPS.with_borrow_mut(|all| all.insert(pop_key(&w)));
    }
}

/// A pop-up takes the keyboard without becoming the main window: its
/// parent stays that, and keeps its active look (title bar, traffic
/// lights). GPUI's windows all answer yes to `canBecomeMainWindow`, and
/// AppKit makes a window that can be main the main one when it becomes
/// key. So the answer of GPUI's panel class (pop-ups are panels) is
/// replaced, once, by one that says no for a window in [`POPS`] and yes
/// for any other, as before. (Not a subclass set on the pop-up: AppKit
/// observes its windows by key path, which changing an object's class
/// breaks.)
fn pops_never_main() {
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::sel;
    use std::sync::Once;
    extern "C-unwind" fn can_be_main(this: &AnyObject, _: Sel) -> Bool {
        let pop = POPS.with_borrow(|all| all.contains(&(std::ptr::from_ref(this) as usize)));
        Bool::new(!pop)
    }
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(method) = AnyClass::get(c"GPUIPanel").and_then(|c| c.instance_method(sel!(canBecomeMainWindow))) else {
            return;
        };
        // SAFETY: the same signature as the method it replaces (no
        // arguments, a BOOL back); called by AppKit on the main thread,
        // where `POPS` lives.
        unsafe {
            let imp: Imp = std::mem::transmute(can_be_main as extern "C-unwind" fn(&AnyObject, Sel) -> Bool);
            method.set_implementation(imp);
        }
    });
}

/// The parent's content area on screen (AppKit's coordinates: from the
/// bottom left).
fn content_rect(w: &objc2_app_kit::NSWindow) -> objc2_foundation::NSRect {
    w.contentRectForFrameRect(w.frame())
}

/// The usable area (no menu bar, no Dock) of the screen that shows the
/// point (`x`, `y`) of the parent's content, in the parent's own
/// coordinates (points from its content's top left, y down): (x, y,
/// width, height). The parent's own screen when the point is on none.
#[cfg_attr(test, allow(dead_code))]
pub fn pop_area(parent: &PopParent, x: f32, y: f32) -> Option<(f32, f32, f32, f32)> {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::NSPoint;
    let mtm = MainThreadMarker::new()?;
    let c = content_rect(&parent.0);
    let top = c.origin.y + c.size.height;
    let at = NSPoint::new(c.origin.x + f64::from(x), top - f64::from(y));
    let inside = |r: objc2_foundation::NSRect| {
        at.x >= r.origin.x && at.x < r.origin.x + r.size.width && at.y >= r.origin.y && at.y < r.origin.y + r.size.height
    };
    let screen = NSScreen::screens(mtm).iter().find(|s| inside(s.frame())).or_else(|| parent.0.screen())?;
    let v = screen.visibleFrame();
    Some((
        (v.origin.x - c.origin.x) as f32,
        (top - (v.origin.y + v.size.height)) as f32,
        v.size.width as f32,
        v.size.height as f32,
    ))
}

/// Put a pop-up at (`x`, `y`, `width`, `height`) of its parent's content
/// (points from its top left, y down), show it as the parent's child (it
/// moves with it and stays above it) and give it the keyboard; Pitwall's
/// window stays the main one, so it keeps its active look. Done once the
/// caller has returned: AppKit tells GPUI about the frame and the keyboard
/// while they are set, and GPUI can't hear it during an update.
#[cfg_attr(test, allow(dead_code))]
pub fn pop_place(window: &gpui::Window, parent: &PopParent, (x, y, width, height): (f32, f32, f32, f32), cx: &mut gpui::App) {
    use objc2_app_kit::{NSWindowOrderingMode, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let Some(w) = ns_window(window) else { return };
    let parent = parent.0.clone();
    cx.spawn(async move |_| {
        if !POPS.with_borrow(|all| all.contains(&pop_key(&w))) {
            return;
        }
        let c = content_rect(&parent);
        let frame = NSRect::new(
            NSPoint::new(c.origin.x + f64::from(x), c.origin.y + c.size.height - f64::from(y) - f64::from(height)),
            NSSize::new(f64::from(width), f64::from(height)),
        );
        // GPUI's pop-ups are titled windows with the content under the
        // title bar; the system draws a rim of light along the top of one
        // that has the keyboard. A bare panel has none. (Here, not in
        // `pop_setup`: the window is laid out again when its style changes.)
        let bare = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        if w.styleMask() != bare {
            w.setStyleMask(bare);
        }
        if w.frame() != frame {
            w.setFrame_display(frame, true);
        }
        if w.parentWindow().is_none() {
            // SAFETY: both are live windows, on the main thread; the
            // parent lets go of its child in `pop_close`.
            unsafe { parent.addChildWindow_ordered(&w, NSWindowOrderingMode::Above) };
        }
        if !w.isKeyWindow() {
            w.makeKeyWindow();
        }
    })
    .detach();
}

/// What AppKit says about a pop-up, for `kit::pop::debug`.
#[cfg_attr(test, allow(dead_code))]
pub fn pop_debug(window: &gpui::Window) -> String {
    let Some(w) = ns_window(window) else { return "no native window".into() };
    let f = w.frame();
    let parent = w.parentWindow();
    format!(
        "frame ({}, {}) {}×{}, visible {}, key {}, style {:#x}, shadow {}, level {}, parent: {}, app active {}",
        f.origin.x,
        f.origin.y,
        f.size.width,
        f.size.height,
        w.isVisible(),
        w.isKeyWindow(),
        w.styleMask().0,
        w.hasShadow(),
        w.level(),
        parent.map_or("none".to_string(), |p| {
            let pf = p.frame();
            format!("main {}, key {}, frame ({}, {}) {}×{}", p.isMainWindow(), p.isKeyWindow(), pf.origin.x, pf.origin.y, pf.size.width, pf.size.height)
        }),
        ns_app_active(),
    )
}

/// A pop-up is closing: it leaves its parent, which gets the keyboard
/// back if the pop-up had it. (After the caller has returned, as
/// [`pop_place`].)
#[cfg_attr(test, allow(dead_code))]
pub fn pop_close(window: &gpui::Window, cx: &mut gpui::App) {
    let Some(w) = ns_window(window) else { return };
    POPS.with_borrow_mut(|all| all.remove(&pop_key(&w)));
    let had_keys = w.isKeyWindow();
    cx.spawn(async move |_| {
        let Some(parent) = w.parentWindow() else { return };
        parent.removeChildWindow(&w);
        w.orderOut(None);
        if had_keys && parent.isVisible() {
            parent.makeKeyWindow();
        }
    })
    .detach();
}

/// Bounce the Dock icon until the app is activated (a new approval).
pub fn request_attention() {
    if let Some(app) = app() {
        app.requestUserAttention(NSRequestUserAttentionType::CriticalRequest);
    }
}

/// The standard About panel (name, version and icon from the bundle).
pub fn about() {
    if let Some(app) = app() {
        app.orderFrontStandardAboutPanel(None);
    }
}

/// Take a window off screen without closing it (`orderOut:`): the main
/// window on close, as the Tauri app hides it; activating shows it again.
pub fn hide(window: &gpui::Window) {
    use objc2::rc::Retained;
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: GPUI hands out its live NSView; we only retain it.
    let view: Option<Retained<NSView>> = unsafe { Retained::retain(h.ns_view.as_ptr().cast()) };
    if let Some(w) = view.and_then(|v| v.window()) {
        w.orderOut(None);
    }
}

/// Two drawables for the window's Metal layer instead of GPUI's three:
/// one on screen, one being drawn. The app draws at most at the display's
/// rate and mostly on a 30 fps grid, so a third buffer only holds memory
/// (a full window of pixels: ~16 MB at 1280×800 on a Retina display).
#[cfg_attr(test, allow(dead_code))]
pub fn two_drawables(window: &gpui::Window) {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, sel};
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: GPUI hands out its live NSView; we only retain it.
    let Some(view): Option<Retained<NSView>> = (unsafe { Retained::retain(h.ns_view.as_ptr().cast()) }) else {
        return;
    };
    // SAFETY: `layer` is an NSView method; the layer is GPUI's
    // CAMetalLayer, checked to answer the setter before it is called.
    unsafe {
        let layer: *mut AnyObject = msg_send![&*view, layer];
        if layer.is_null() {
            return;
        }
        let ok: bool = msg_send![layer, respondsToSelector: sel!(setMaximumDrawableCount:)];
        if ok {
            let _: () = msg_send![layer, setMaximumDrawableCount: 2usize];
        }
    }
}

thread_local! {
    /// Debug builds: the app that was in front when `debug_app("front")`
    /// was asked, to give its place back.
    static WAS_FRONT: std::cell::RefCell<Option<objc2::rc::Retained<objc2_app_kit::NSRunningApplication>>> =
        const { std::cell::RefCell::new(None) };
}

/// Debug builds: `front` notes the app the user is in (before this one is
/// activated for a check), `deactivate` gives that app its place back (as
/// when the user goes to another app; this app is hidden when none was
/// noted), `above` lifts the app's windows over floating ones; then what
/// AppKit says of the app and of its windows that are on screen.
#[cfg_attr(test, allow(dead_code))]
pub fn debug_app(what: &str) -> String {
    use objc2_app_kit::{NSApplicationActivationOptions, NSWorkspace};
    let Some(app) = app() else { return "not on the main thread".into() };
    if what == "front" && !app.isActive() {
        WAS_FRONT.set(NSWorkspace::sharedWorkspace().frontmostApplication());
    }
    // `above`: the app's ordinary windows above other apps' floating ones
    // (test instances side by side), to be seen in a screenshot.
    if what == "above" {
        for w in app.windows().iter().filter(|w| w.isVisible() && w.parentWindow().is_none()) {
            w.setLevel(102);
        }
    }
    if what == "deactivate" {
        match WAS_FRONT.take() {
            Some(other) => {
                other.activateWithOptions(NSApplicationActivationOptions::empty());
            }
            None => app.hide(None),
        }
    }
    let windows: Vec<String> = app
        .windows()
        .iter()
        .filter(|w| w.isVisible())
        .map(|w| {
            let f = w.frame();
            format!(
                "[#{} ({}, {}) {}×{} key {} main {} child {}]",
                w.windowNumber(),
                f.origin.x,
                f.origin.y,
                f.size.width,
                f.size.height,
                w.isKeyWindow(),
                w.isMainWindow(),
                w.parentWindow().is_some()
            )
        })
        .collect();
    format!("active {}, menu bar ours {}, windows {}", app.isActive(), app_active(), windows.join(" "))
}
