//! macOS: a WKWebView, a subview of the window's content view right above
//! GPUI's own (Metal) view.
//!
//! - Placement: GPUI's coordinates (top-left origin, logical px) turned into
//!   the content view's (bottom-left unless flipped); it keeps its distance
//!   from the top while the window resizes, until the next frame places it.
//! - Storage: `dataStoreForIdentifier:` (macOS 14+) with an identifier made
//!   from the project's data folder, so each project keeps its own cookies
//!   across restarts (WebKit keeps the files under its own folder; it takes
//!   no path). Older macOS: the app's default persistent store, shared.
//! - Events: key-value observing of `URL`, `title`, `loading`, `canGoBack`
//!   and `canGoForward` (link clicks, redirects and history pushes alike);
//!   the navigation delegate for failed loads and for keeping the page on
//!   local dev servers ([`super::allowed`]): another main-frame address, or
//!   a new window, opens in the browser.
//! - Keys: the view's subclass sends ⌘-keys other than the editing ones as
//!   [`WebEvent::Shortcut`] (GPUI's view sees most of them first, as a key
//!   equivalent, and handles Pitwall's there), Esc too (after the page), and
//!   [`WebEvent::Focused`] when it becomes first responder.
//! - Hidden: `setHidden:` (WebKit throttles a hidden page's timers and
//!   drawing) and its media suspended.

use std::cell::Cell;
use std::ffi::c_void;
use std::path::Path;
use std::rc::Rc;

use gpui::{Bounds, Keystroke, Modifiers, Pixels, Window};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ProtocolObject, Sel};
use objc2::runtime::Bool;
use objc2::{define_class, msg_send, sel, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSEvent, NSEventModifierFlags, NSImage, NSResponder, NSView,
    NSWindowOrderingMode, NSWorkspace,
};
use objc2_foundation::{
    NSError, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURLRequest, NSURL,
    NSUUID,
};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKWebView,
    WKWebViewConfiguration, WKWebsiteDataStore,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{Options, Snapshot, WebEvent, WebView};

type Events = Rc<dyn Fn(WebEvent)>;

/// What the view reports, observed by key-value observing.
const OBSERVED: [&str; 5] = ["URL", "title", "loading", "canGoBack", "canGoForward"];

/// macOS virtual key codes the keystrokes need by name.
const ESCAPE: u16 = 53;

pub struct ViewIvars {
    events: Events,
}

define_class!(
    // SAFETY: WKWebView may be subclassed; the overrides call `super`.
    #[unsafe(super(WKWebView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PitwallPreviewWebView"]
    #[ivars = ViewIvars]
    struct PreviewView;

    impl PreviewView {
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> Bool {
            // The window offers key equivalents to every view: only the
            // page's own (it has the keys) are looked at.
            if self.has_keys() {
                let k = keystroke(event);
                if k.modifiers.platform && !super::editing_key(&k.key, true, k.modifiers.shift, k.modifiers.alt) {
                    (self.ivars().events)(WebEvent::Shortcut(k));
                    return Bool::YES;
                }
            }
            // SAFETY: NSView's method, with its argument.
            unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            // SAFETY: NSResponder's method, with its argument.
            let _: () = unsafe { msg_send![super(self), keyDown: event] };
            let plain = !event.modifierFlags().intersects(
                NSEventModifierFlags::Command | NSEventModifierFlags::Control | NSEventModifierFlags::Option,
            );
            if event.keyCode() == ESCAPE && plain {
                (self.ivars().events)(WebEvent::Shortcut(Keystroke {
                    modifiers: Modifiers::none(),
                    key: "escape".into(),
                    key_char: None,
                }));
            }
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> Bool {
            // SAFETY: NSResponder's method.
            let ok: Bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if ok.as_bool() {
                (self.ivars().events)(WebEvent::Focused);
            }
            ok
        }
    }
);

impl PreviewView {
    /// The window's first responder is this view or inside it.
    fn has_keys(&self) -> bool {
        let Some(responder) = self.window().and_then(|w| w.firstResponder()) else {
            return false;
        };
        let view_class = NSView::class();
        // SAFETY: `isKindOfClass:` and `isDescendantOf:` are NSObject's
        // and NSView's; the responder is checked to be a view first.
        unsafe {
            let is_view: bool = msg_send![&*responder, isKindOfClass: view_class];
            is_view && msg_send![&*responder, isDescendantOf: self]
        }
    }
}

pub struct DelegateIvars {
    events: Events,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PitwallPreviewDelegate"]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl WKNavigationDelegate for Delegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide(
            &self,
            web_view: &WKWebView,
            action: &WKNavigationAction,
            decision: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            // SAFETY: WebKit's own objects, on the main thread.
            let (request, frame) = unsafe { (action.request(), action.targetFrame()) };
            let url = request.URL().and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default();
            let local = super::allowed(&url) || url == "about:blank";
            match frame {
                // A sub-frame (an embed) loads what it likes.
                // SAFETY: as above.
                Some(f) if unsafe { !f.isMainFrame() } => decision.call((WKNavigationActionPolicy::Allow,)),
                Some(_) if local => decision.call((WKNavigationActionPolicy::Allow,)),
                // A new window (`target=_blank`): here when local.
                None if local => {
                    decision.call((WKNavigationActionPolicy::Cancel,));
                    // SAFETY: as above.
                    unsafe { web_view.loadRequest(&request) };
                }
                // Somewhere else: the browser.
                _ => {
                    decision.call((WKNavigationActionPolicy::Cancel,));
                    if url.starts_with("http://") || url.starts_with("https://") {
                        if let Some(u) = request.URL() {
                            NSWorkspace::sharedWorkspace().openURL(&u);
                        }
                    }
                }
            }
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn failed_to_start(&self, web_view: &WKWebView, _nav: Option<&WKNavigation>, error: &NSError) {
            self.failed(web_view, error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed_to_finish(&self, web_view: &WKWebView, _nav: Option<&WKNavigation>, error: &NSError) {
            self.failed(web_view, error);
        }
    }

    impl Delegate {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            key: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&AnyObject>,
            _context: *mut c_void,
        ) {
            let (Some(key), Some(object)) = (key, object) else { return };
            // SAFETY: only the web view is observed (`create`).
            let wv: &WKWebView = unsafe { &*(object as *const AnyObject as *const WKWebView) };
            let events = &self.ivars().events;
            // SAFETY: WKWebView's properties, on the main thread.
            unsafe {
                match key.to_string().as_str() {
                    "URL" => {
                        if let Some(u) = wv.URL().and_then(|u| u.absoluteString()) {
                            events(WebEvent::Url(u.to_string()));
                        }
                    }
                    "title" => events(WebEvent::Title(wv.title().map(|t| t.to_string()).unwrap_or_default())),
                    "loading" => events(WebEvent::Loading(wv.isLoading())),
                    _ => events(WebEvent::CanGo { back: wv.canGoBack(), forward: wv.canGoForward() }),
                }
            }
        }
    }
);

impl Delegate {
    fn failed(&self, web_view: &WKWebView, error: &NSError) {
        // Cancelled (a newer load, or a navigation sent to the browser) and
        // "interrupted by policy change": not failures.
        const NSURL_ERROR_CANCELLED: isize = -999;
        const FRAME_LOAD_INTERRUPTED: isize = 102;
        if matches!(error.code(), NSURL_ERROR_CANCELLED | FRAME_LOAD_INTERRUPTED) {
            return;
        }
        // SAFETY: WKWebView's property, on the main thread.
        let url = unsafe { web_view.URL() }.and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default();
        let events = &self.ivars().events;
        events(WebEvent::Failed { url, error: error.localizedDescription().to_string() });
        events(WebEvent::Loading(false));
    }
}

/// GPUI's view and the window's content view.
fn views(window: &Window) -> Option<(Retained<NSView>, Retained<NSView>)> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return None;
    };
    // SAFETY: GPUI hands out its live NSView; we only retain it.
    let gpui_view: Retained<NSView> = unsafe { Retained::retain(h.ns_view.as_ptr().cast())? };
    let content = gpui_view.window()?.contentView()?;
    Some((gpui_view, content))
}

/// A stable identifier for the data folder (the store `dataStoreForIdentifier:` keeps).
fn store_id(dir: &Path) -> [u8; 16] {
    let text = dir.to_string_lossy();
    let fnv = |seed: u64| {
        text.bytes().fold(seed, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
    };
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&fnv(0xcbf2_9ce4_8422_2325).to_be_bytes());
    id[8..].copy_from_slice(&fnv(0x6c62_272e_07bb_0142).to_be_bytes());
    // A version-4, RFC 4122 UUID (and never the nil one WebKit rejects).
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

fn store(dir: &Path, mtm: MainThreadMarker) -> Retained<WKWebsiteDataStore> {
    let per_project = AnyClass::get(c"WKWebsiteDataStore")
        .is_some_and(|c| c.responds_to(sel!(dataStoreForIdentifier:)));
    // SAFETY: WKWebsiteDataStore's class methods, on the main thread; the
    // identifier one only where the system has it (macOS 14).
    unsafe {
        if per_project {
            WKWebsiteDataStore::dataStoreForIdentifier(&NSUUID::from_bytes(store_id(dir)), mtm)
        } else {
            WKWebsiteDataStore::defaultDataStore(mtm)
        }
    }
}

fn responds(obj: &AnyObject, sel: Sel) -> bool {
    // SAFETY: NSObject's `respondsToSelector:`.
    unsafe { msg_send![obj, respondsToSelector: sel] }
}

struct MacWebView {
    view: Retained<PreviewView>,
    delegate: Retained<Delegate>,
    content: Retained<NSView>,
    gpui_view: Retained<NSView>,
    /// Shown, as last asked.
    shown: Cell<bool>,
}

pub fn create(window: &mut Window, opts: Options) -> Result<Box<dyn WebView>, String> {
    let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
    let (gpui_view, content) = views(window).ok_or("no native window to put it in")?;
    if AnyClass::get(c"WKWebView").is_none() {
        return Err("WebKit isn't available".into());
    }
    let events: Events = Rc::from(opts.events);
    // SAFETY: WebKit's classes, made and configured on the main thread
    // before the view uses them.
    let view = unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        config.setWebsiteDataStore(&store(&opts.data_dir, mtm));
        // Right-click → Inspect Element, and the inspector's own window.
        let prefs = config.preferences();
        let _: () = msg_send![&*prefs, setValue: &*objc2_foundation::NSNumber::new_bool(true), forKey: &*NSString::from_str("developerExtrasEnabled")];
        let this = PreviewView::alloc(mtm).set_ivars(ViewIvars { events: events.clone() });
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
        let view: Retained<PreviewView> = msg_send![super(this), initWithFrame: frame, configuration: &*config];
        view
    };
    let delegate: Retained<Delegate> = {
        let this = Delegate::alloc(mtm).set_ivars(DelegateIvars { events });
        // SAFETY: NSObject's `init`.
        unsafe { msg_send![super(this), init] }
    };
    // SAFETY: WKWebView's setters and NSView's hierarchy, on the main thread.
    unsafe {
        view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        if responds(&view, sel!(setInspectable:)) {
            view.setInspectable(true);
        }
        for key in OBSERVED {
            let _: () = msg_send![
                &*view,
                addObserver: &*delegate,
                forKeyPath: &*NSString::from_str(key),
                options: 0usize,
                context: std::ptr::null_mut::<c_void>()
            ];
        }
        view.setHidden(true);
        // Keeps its distance from the top while the window resizes (the
        // next frame places it exactly).
        view.setAutoresizingMask(if content.isFlipped() {
            NSAutoresizingMaskOptions::ViewMaxYMargin
        } else {
            NSAutoresizingMaskOptions::ViewMinYMargin
        });
        content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Above, Some(&gpui_view));
    }
    let mut v = MacWebView { view, delegate, content, gpui_view, shown: Cell::new(false) };
    v.navigate(&opts.url);
    Ok(Box::new(v))
}

impl MacWebView {
    fn wk(&self) -> &WKWebView {
        &self.view
    }
}

impl WebView for MacWebView {
    fn navigate(&mut self, url: &str) {
        let Some(u) = NSURL::URLWithString(&NSString::from_str(url)) else { return };
        // SAFETY: WKWebView's method, on the main thread.
        unsafe { self.wk().loadRequest(&NSURLRequest::requestWithURL(&u)) };
    }

    fn reload(&mut self) {
        // SAFETY: as above.
        unsafe { self.wk().reload() };
    }

    fn back(&mut self) {
        // SAFETY: as above.
        unsafe { self.wk().goBack() };
    }

    fn forward(&mut self) {
        // SAFETY: as above.
        unsafe { self.wk().goForward() };
    }

    fn set_bounds(&mut self, b: Bounds<Pixels>) {
        let (x, y) = (f64::from(f32::from(b.origin.x)), f64::from(f32::from(b.origin.y)));
        let (w, h) = (f64::from(f32::from(b.size.width)), f64::from(f32::from(b.size.height)));
        let y = if self.content.isFlipped() { y } else { self.content.bounds().size.height - y - h };
        self.view.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
    }

    fn set_visible(&mut self, visible: bool) {
        if self.shown.replace(visible) == visible && self.view.isHidden() != visible {
            return;
        }
        if !visible && self.has_keys() {
            self.blur();
        }
        self.view.setHidden(!visible);
        // Media paused while hidden (macOS 12+).
        if responds(&self.view, sel!(setAllMediaPlaybackSuspended:completionHandler:)) {
            // SAFETY: WKWebView's method; a nil completion handler is allowed.
            let _: () = unsafe {
                msg_send![&*self.view, setAllMediaPlaybackSuspended: !visible, completionHandler: std::ptr::null::<c_void>()]
            };
        }
    }

    fn focus(&mut self) {
        if let Some(w) = self.view.window() {
            w.makeFirstResponder(Some(&self.view));
        }
    }

    fn blur(&mut self) {
        if let Some(w) = self.view.window() {
            if self.has_keys() {
                w.makeFirstResponder(Some(&self.gpui_view));
            }
        }
    }

    fn open_inspector(&mut self) {
        // No public call opens it (what wry does too): `_inspector` and
        // `show`, where they exist; else right-click → Inspect Element.
        if !responds(&self.view, sel!(_inspector)) {
            return;
        }
        // SAFETY: checked above; the inspector object answers `show`.
        unsafe {
            let inspector: *mut AnyObject = msg_send![&*self.view, _inspector];
            if let Some(i) = inspector.as_ref() {
                if responds(i, sel!(show)) {
                    let _: () = msg_send![i, show];
                }
            }
        }
    }

    fn snapshot(&mut self, done: Box<dyn FnOnce(Option<Snapshot>) + 'static>) {
        let frame = self.view.frame();
        let scale = self.view.window().map_or(2., |w| w.backingScaleFactor());
        let (w, h) = ((frame.size.width * scale).round() as usize, (frame.size.height * scale).round() as usize);
        if w == 0 || h == 0 {
            done(None);
            return;
        }
        let done = Cell::new(Some(done));
        let block = block2::RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
            let Some(done) = done.take() else { return };
            // SAFETY: WebKit hands the image (or nil) for the call's length.
            let shot = unsafe { image.as_ref() }.and_then(|i| rgba(i, w, h)).map(|rgba| Snapshot {
                width: w as u32,
                height: h as u32,
                rgba,
            });
            done(shot);
        });
        // SAFETY: WKWebView's method; nil configuration = what it shows now.
        unsafe { self.wk().takeSnapshotWithConfiguration_completionHandler(None, &block) };
    }
}

impl MacWebView {
    fn has_keys(&self) -> bool {
        self.view.has_keys()
    }
}

impl Drop for MacWebView {
    fn drop(&mut self) {
        self.blur();
        // SAFETY: the observers `create` added, removed before the view
        // goes; then out of the window.
        unsafe {
            for key in OBSERVED {
                let _: () = msg_send![&*self.view, removeObserver: &*self.delegate, forKeyPath: &*NSString::from_str(key)];
            }
            self.view.setNavigationDelegate(None);
            self.view.stopLoading();
        }
        self.view.removeFromSuperview();
    }
}

/// `image` drawn into a `w`×`h` RGBA bitmap (8 bits a channel).
fn rgba(image: &NSImage, w: usize, h: usize) -> Option<Vec<u8>> {
    let rep_class = AnyClass::get(c"NSBitmapImageRep")?;
    let ctx_class = AnyClass::get(c"NSGraphicsContext")?;
    // SAFETY: AppKit's documented bitmap and graphics-context calls, on the
    // main thread; the bitmap's own buffer is read for its known size.
    unsafe {
        let rep: *mut AnyObject = msg_send![rep_class, alloc];
        let rep: *mut AnyObject = msg_send![
            rep,
            initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
            pixelsWide: w as isize,
            pixelsHigh: h as isize,
            bitsPerSample: 8isize,
            samplesPerPixel: 4isize,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: &*NSString::from_str("NSDeviceRGBColorSpace"),
            bytesPerRow: (w * 4) as isize,
            bitsPerPixel: 32isize
        ];
        let rep: Retained<AnyObject> = Retained::from_raw(rep)?;
        let ctx: *mut AnyObject = msg_send![ctx_class, graphicsContextWithBitmapImageRep: &*rep];
        if ctx.is_null() {
            return None;
        }
        let _: () = msg_send![ctx_class, saveGraphicsState];
        let _: () = msg_send![ctx_class, setCurrentContext: ctx];
        let rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(w as f64, h as f64));
        let _: () = msg_send![image, drawInRect: rect];
        let _: () = msg_send![ctx_class, restoreGraphicsState];
        let data: *const u8 = msg_send![&*rep, bitmapData];
        if data.is_null() {
            return None;
        }
        Some(std::slice::from_raw_parts(data, w * h * 4).to_vec())
    }
}

/// A key event as GPUI names it (`cmd-shift-n`).
fn keystroke(event: &NSEvent) -> Keystroke {
    let flags = event.modifierFlags();
    let modifiers = Modifiers {
        control: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        shift: flags.contains(NSEventModifierFlags::Shift),
        platform: flags.contains(NSEventModifierFlags::Command),
        function: false,
    };
    let key = match event.keyCode() {
        36 | 76 => "enter".to_string(),
        48 => "tab".into(),
        49 => "space".into(),
        51 => "backspace".into(),
        53 => "escape".into(),
        117 => "delete".into(),
        123 => "left".into(),
        124 => "right".into(),
        125 => "down".into(),
        126 => "up".into(),
        115 => "home".into(),
        119 => "end".into(),
        116 => "pageup".into(),
        121 => "pagedown".into(),
        122 => "f1".into(),
        120 => "f2".into(),
        99 => "f3".into(),
        118 => "f4".into(),
        96 => "f5".into(),
        97 => "f6".into(),
        98 => "f7".into(),
        100 => "f8".into(),
        101 => "f9".into(),
        109 => "f10".into(),
        103 => "f11".into(),
        111 => "f12".into(),
        // The key's own character, without Shift or Option (⌘⇧N is `n`
        // with shift, ⌘= is `=`), as GPUI names it.
        _ => event
            .charactersByApplyingModifiers(NSEventModifierFlags::empty())
            .or_else(|| event.charactersIgnoringModifiers())
            .map(|s| s.to_string().to_lowercase())
            .unwrap_or_default(),
    };
    Keystroke { modifiers, key, key_char: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The subclass and the delegate register (debug builds check each
    /// override's types against WebKit's and AppKit's own there).
    #[test]
    fn the_classes_register() {
        assert_eq!(PreviewView::class().name().to_str().unwrap(), "PitwallPreviewWebView");
        assert!(Delegate::class().conforms_to(<dyn WKNavigationDelegate as objc2::ProtocolType>::protocol().unwrap()));
    }

    /// A picture becomes RGBA bytes (no window, no WebKit: an empty image).
    #[test]
    fn an_image_becomes_rgba() {
        // SAFETY: NSImage's documented initialiser.
        let image: Retained<NSImage> = unsafe {
            let i: *mut AnyObject = msg_send![AnyClass::get(c"NSImage").unwrap(), alloc];
            let i: *mut AnyObject = msg_send![i, initWithSize: NSSize::new(3., 2.)];
            Retained::from_raw(i.cast()).unwrap()
        };
        let px = rgba(&image, 3, 2).expect("drawn");
        assert_eq!(px.len(), 3 * 2 * 4);
    }

    #[test]
    fn a_project_keeps_its_store() {
        let a = store_id(std::path::Path::new("/data/webview/app-1"));
        assert_eq!(a, store_id(std::path::Path::new("/data/webview/app-1")));
        assert_ne!(a, store_id(std::path::Path::new("/data/webview/app-2")));
        assert_eq!(a[6] >> 4, 4, "a version-4 UUID");
        assert_ne!(a, [0; 16]);
    }
}
