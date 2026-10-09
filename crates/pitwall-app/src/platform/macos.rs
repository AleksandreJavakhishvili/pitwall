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
