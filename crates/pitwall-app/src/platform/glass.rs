//! The window material behind GPUI's content, per OS (docs/spec/gpui/platform.md
//! "Glass"; Tauri: `src-tauri/src/platform/glass.rs`).
//!
//! - macOS 26+ (Liquid): an `NSGlassEffectView` under GPUI's Metal view, the
//!   window made non-opaque (`WindowBackgroundAppearance::Transparent`).
//!   Chrome regions can get their own glass pieces ([`sync_region`]).
//! - Older macOS (Vibrancy): GPUI's `Blurred` (an `NSVisualEffectView`).
//! - Windows 11 (Mica): `Transparent` + `DWMWA_SYSTEMBACKDROP_TYPE`.
//! - Windows 10, Linux: nothing native; the UI paints "Glass lite".
//!
//! Also: the native window appearance (title bar) when the theme is forced,
//! and the OS accessibility options (Reduce transparency, Increase contrast,
//! Reduce motion). Everything here runs on the main thread.

#[cfg(not(target_os = "macos"))]
use gpui::Window;
use gpui::WindowBackgroundAppearance;
#[cfg(not(target_os = "macos"))]
use gpui::{Bounds, Pixels};

use crate::theme::{GlassOffer, GlassTier, OsPrefs};
use crate::theme::Mode;

/// The best this OS offers without an override.
pub fn default_offer() -> GlassOffer {
    if cfg!(target_os = "macos") {
        if liquid_available() {
            GlassOffer::Liquid
        } else {
            GlassOffer::Vibrancy
        }
    } else {
        use pitwall_core::host::{HostInfo, WindowGlass};
        use pitwall_core::paths::Paths;
        match HostInfo::current(&Paths::new(Paths::default_root())).glass {
            WindowGlass::Mica => GlassOffer::Mica,
            WindowGlass::Vibrancy | WindowGlass::None => GlassOffer::None,
        }
    }
}

/// The background appearance GPUI should use for a tier.
pub fn background_for(tier: Option<GlassTier>) -> WindowBackgroundAppearance {
    match tier {
        Some(GlassTier::Native) => WindowBackgroundAppearance::Blurred,
        Some(GlassTier::Liquid | GlassTier::Mica) => WindowBackgroundAppearance::Transparent,
        Some(GlassTier::Lite) | None => WindowBackgroundAppearance::Opaque,
    }
}

#[cfg(target_os = "macos")]
pub use mac::{apply, liquid_available, os_prefs, sweep_regions, sync_region};

#[cfg(windows)]
pub use win::{apply, os_prefs};

#[cfg(not(target_os = "macos"))]
pub fn liquid_available() -> bool {
    false
}

/// No native pieces here: only the bookkeeping (what tests read).
#[cfg(not(target_os = "macos"))]
pub fn sync_region(window: &mut Window, id: &'static str, _: Bounds<Pixels>, _: f32) {
    let (wid, owner) = (window.window_handle().window_id(), window.current_view());
    regions::with_book(wid, |b| b.place(owner, id));
}

#[cfg(not(target_os = "macos"))]
pub fn sweep_regions(window: &mut Window) {
    let wid = window.window_handle().window_id();
    regions::with_book(wid, |b| b.sweep());
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn apply(window: &mut Window, tier: Option<GlassTier>, _forced: Option<Mode>, _mode: Mode) {
    window.set_background_appearance(background_for(tier));
}

/// GNOME's `enable-animations` (best effort, read once per run).
#[cfg(not(any(target_os = "macos", windows)))]
pub fn os_prefs() -> OsPrefs {
    static REDUCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let reduce_motion = *REDUCE.get_or_init(|| {
        std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
    });
    OsPrefs {
        reduce_motion,
        ..OsPrefs::default()
    }
}

/// Which chrome glass pieces a window shows (all OSes; the native views
/// are macOS's, in `mac`).
///
/// A piece is placed by a region's canvas while its view prepaints
/// ([`sync_region`]) and hidden after a frame that drew that view again
/// without it ([`sweep_regions`]). The view that placed it (its owner) can
/// be a cached one: a frame that replays its last paint runs none of its
/// canvases, yet shows the same regions, so its pieces stay as they are.
/// (Sweeping them there hid the sidebar, top bar and right panel glass on
/// every frame that replayed the main screen, a working dot's ring for
/// one, and made them again on the next: the window flickered.) A piece
/// that goes is hidden, not removed: showing it again (back to the
/// agents from Review) doesn't change the window's views.
pub(crate) mod regions {
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};

    use gpui::{EntityId, WindowId};

    /// What a placement asks of the native piece.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Placed {
        /// Not there yet: make it.
        New,
        /// Hidden since an earlier frame: show it.
        Shown,
        /// Already showing.
        Kept,
    }

    #[derive(Clone, Copy, Debug)]
    struct Region {
        owner: EntityId,
        shown: bool,
        /// Placed in the frame being drawn.
        seen: bool,
    }

    /// How often pieces were made, hidden and shown again (tests: churn).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Counts {
        pub made: usize,
        pub hidden: usize,
        pub reshown: usize,
    }

    /// One window's pieces.
    #[derive(Default)]
    pub struct Book {
        regions: HashMap<&'static str, Region>,
        /// The views that placed a piece in the frame being drawn.
        drawn: HashSet<EntityId>,
        pub counts: Counts,
    }

    impl Book {
        /// `owner` placed piece `id` in this frame.
        pub fn place(&mut self, owner: EntityId, id: &'static str) -> Placed {
            self.drawn.insert(owner);
            let placed = match self.regions.get_mut(id) {
                None => {
                    self.counts.made += 1;
                    self.regions.insert(id, Region { owner, shown: true, seen: true });
                    return Placed::New;
                }
                Some(r) => {
                    r.owner = owner;
                    r.seen = true;
                    if r.shown {
                        Placed::Kept
                    } else {
                        r.shown = true;
                        Placed::Shown
                    }
                }
            };
            if placed == Placed::Shown {
                self.counts.reshown += 1;
            }
            placed
        }

        /// The frame is drawn: the pieces to hide (their owner drew again
        /// without them), and the next frame's bookkeeping starts.
        pub fn sweep(&mut self) -> Vec<&'static str> {
            let mut hide = Vec::new();
            for (id, r) in &mut self.regions {
                if !r.seen && r.shown && self.drawn.contains(&r.owner) {
                    r.shown = false;
                    hide.push(*id);
                }
                r.seen = false;
            }
            self.counts.hidden += hide.len();
            self.drawn.clear();
            hide
        }

        /// The pieces showing, sorted.
        #[cfg(test)]
        pub fn shown(&self) -> Vec<&'static str> {
            let mut v: Vec<_> = self.regions.iter().filter(|(_, r)| r.shown).map(|(id, _)| *id).collect();
            v.sort_unstable();
            v
        }

        /// The native pieces were all removed (Liquid Glass went off).
        #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
        pub fn clear(&mut self) {
            self.regions.clear();
            self.drawn.clear();
        }
    }

    thread_local! {
        static BOOKS: RefCell<HashMap<WindowId, Book>> = RefCell::new(HashMap::new());
    }

    pub fn with_book<R>(window: WindowId, f: impl FnOnce(&mut Book) -> R) -> R {
        BOOKS.with_borrow_mut(|all| f(all.entry(window).or_default()))
    }

    /// The pieces `window` shows and how often they changed (tests).
    #[cfg(test)]
    pub fn of(window: WindowId) -> (Vec<&'static str>, Counts) {
        with_book(window, |b| (b.shown(), b.counts))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use gpui::{AppContext as _, TestAppContext};

        fn ids(cx: &mut TestAppContext, n: usize) -> Vec<EntityId> {
            (0..n).map(|_| cx.new(|_| ()).entity_id()).collect()
        }

        #[gpui::test]
        fn a_replayed_view_keeps_its_pieces(cx: &mut TestAppContext) {
            let v = ids(cx, 1);
            let screen = v[0];
            let mut b = Book::default();
            for id in ["topbar", "sidebar", "right-panel"] {
                assert_eq!(b.place(screen, id), Placed::New);
            }
            assert!(b.sweep().is_empty());
            // A frame that replays the screen's paint: nothing placed.
            for _ in 0..3 {
                assert!(b.sweep().is_empty(), "nothing hidden");
            }
            assert_eq!(b.shown(), vec!["right-panel", "sidebar", "topbar"]);
            // Drawn again: kept as they are.
            for id in ["topbar", "sidebar", "right-panel"] {
                assert_eq!(b.place(screen, id), Placed::Kept);
            }
            assert!(b.sweep().is_empty());
            assert_eq!(b.counts, Counts { made: 3, hidden: 0, reshown: 0 });
        }

        #[gpui::test]
        fn a_piece_left_out_hides_and_comes_back(cx: &mut TestAppContext) {
            let v = ids(cx, 2);
            let (screen, other) = (v[0], v[1]);
            let mut b = Book::default();
            b.place(screen, "topbar");
            b.place(screen, "right-panel");
            b.place(other, "elsewhere");
            b.sweep();
            // Review: the screen draws again without the right panel.
            b.place(screen, "topbar");
            assert_eq!(b.sweep(), vec!["right-panel"]);
            assert_eq!(b.shown(), vec!["elsewhere", "topbar"], "another view's piece stays");
            // Back to the agents: the same piece shows again, none is made.
            b.place(screen, "topbar");
            assert_eq!(b.place(screen, "right-panel"), Placed::Shown);
            assert!(b.sweep().is_empty());
            assert_eq!(b.counts, Counts { made: 3, hidden: 1, reshown: 1 });
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use gpui::{Bounds, Pixels, Window, WindowId};
    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
        NSAutoresizingMaskOptions, NSGlassEffectView, NSView, NSWindowOrderingMode, NSWorkspace,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::{background_for, GlassTier, Mode, OsPrefs};

    /// `NSGlassEffectView` exists (macOS 26+). Checked at runtime: naming a
    /// missing class would abort.
    pub fn liquid_available() -> bool {
        AnyClass::get(c"NSGlassEffectView").is_some()
    }

    pub fn os_prefs() -> OsPrefs {
        let ws = NSWorkspace::sharedWorkspace();
        OsPrefs {
            reduce_transparency: ws.accessibilityDisplayShouldReduceTransparency(),
            increase_contrast: ws.accessibilityDisplayShouldIncreaseContrast(),
            reduce_motion: ws.accessibilityDisplayShouldReduceMotion(),
        }
    }

    /// The glass views Pitwall added to each window.
    #[derive(Default)]
    struct WindowGlass {
        /// Behind the whole window.
        window: Option<Retained<NSGlassEffectView>>,
        /// Chrome pieces, by region id (shown or hidden as
        /// [`super::regions`] says).
        regions: HashMap<&'static str, Retained<NSGlassEffectView>>,
    }

    thread_local! {
        static GLASS: RefCell<HashMap<WindowId, WindowGlass>> = RefCell::new(HashMap::new());
    }

    /// GPUI's own view (the Metal one) and the window's content view.
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

    fn new_glass(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSGlassEffectView> {
        NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), frame)
    }

    pub fn apply(window: &mut Window, tier: Option<GlassTier>, forced: Option<Mode>, _mode: Mode) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let liquid = tier == Some(GlassTier::Liquid) && liquid_available();
        window.set_background_appearance(background_for(if liquid {
            tier
        } else if tier == Some(GlassTier::Liquid) {
            Some(GlassTier::Native)
        } else {
            tier
        }));
        let Some((gpui_view, content)) = views(window) else {
            return;
        };
        // Title bar and native controls follow a forced theme.
        if let Some(ns_window) = content.window() {
            let appearance = forced.and_then(|m| {
                // SAFETY: AppKit's appearance name constants.
                let name = unsafe {
                    match m {
                        Mode::Dark => NSAppearanceNameDarkAqua,
                        Mode::Light => NSAppearanceNameAqua,
                    }
                };
                NSAppearance::appearanceNamed(name)
            });
            ns_window.setAppearance(appearance.as_deref());
            // Test scripts: `PITWALL_DEBUG_ALL_SPACES=1` shows the window on
            // every Space (full-screen ones too), so screenshots and smoke
            // runs find it on screen.
            if std::env::var_os("PITWALL_DEBUG_ALL_SPACES").is_some() {
                use objc2_app_kit::NSWindowCollectionBehavior as B;
                ns_window.setCollectionBehavior(B::CanJoinAllSpaces | B::FullScreenAuxiliary);
            }
        }
        let id = Window::window_handle(window).window_id();
        GLASS.with_borrow_mut(|all| {
            let g = all.entry(id).or_default();
            match (liquid, &g.window) {
                (true, None) => {
                    let glass = new_glass(mtm, content.bounds());
                    glass.setAutoresizingMask(
                        NSAutoresizingMaskOptions::ViewWidthSizable
                            | NSAutoresizingMaskOptions::ViewHeightSizable,
                    );
                    // Below everything, GPUI's view included.
                    content.addSubview_positioned_relativeTo(
                        &glass,
                        NSWindowOrderingMode::Below,
                        None,
                    );
                    g.window = Some(glass);
                }
                (false, Some(_)) => {
                    if let Some(v) = g.window.take() {
                        v.removeFromSuperview();
                    }
                    for (_, v) in g.regions.drain() {
                        v.removeFromSuperview();
                    }
                    super::regions::with_book(id, |b| b.clear());
                }
                _ => {}
            }
            let _ = gpui_view;
        });
    }

    /// Place (or move) the glass piece `id` under GPUI's view at `bounds`
    /// (window coordinates, top-left origin), with `radius` corners. Only
    /// while the window has Liquid Glass; GPUI must leave that region
    /// unpainted (or tinted) for the glass to show. Call it while the
    /// region's view prepaints (see [`super::regions`]).
    pub fn sync_region(window: &mut Window, id: &'static str, bounds: Bounds<Pixels>, radius: f32) {
        let wid = Window::window_handle(window).window_id();
        super::regions::with_book(wid, |b| b.place(window.current_view(), id));
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let has_window_glass =
            GLASS.with_borrow(|all| all.get(&wid).is_some_and(|g| g.window.is_some()));
        if !has_window_glass {
            return;
        }
        let Some((gpui_view, content)) = views(window) else {
            return;
        };
        let height = content.bounds().size.height;
        let frame = NSRect::new(
            NSPoint::new(
                f64::from(f32::from(bounds.origin.x)),
                height - f64::from(f32::from(bounds.origin.y + bounds.size.height)),
            ),
            NSSize::new(
                f64::from(f32::from(bounds.size.width)),
                f64::from(f32::from(bounds.size.height)),
            ),
        );
        GLASS.with_borrow_mut(|all| {
            let g = all.entry(wid).or_default();
            let v = g.regions.entry(id).or_insert_with(|| {
                let v = new_glass(mtm, frame);
                // Right under GPUI's view: above the window-wide glass.
                content.addSubview_positioned_relativeTo(
                    &v,
                    NSWindowOrderingMode::Below,
                    Some(&gpui_view),
                );
                v
            });
            if v.frame() != frame {
                v.setFrame(frame);
            }
            if v.cornerRadius() != radius as f64 {
                v.setCornerRadius(radius as f64);
            }
            if v.isHidden() {
                v.setHidden(false);
            }
        });
    }

    /// After a frame: hide the pieces whose view drew again without them
    /// (kept for when they come back), and start the next frame's
    /// bookkeeping.
    pub fn sweep_regions(window: &mut Window) {
        let wid = Window::window_handle(window).window_id();
        let hide = super::regions::with_book(wid, |b| b.sweep());
        if hide.is_empty() {
            return;
        }
        GLASS.with_borrow(|all| {
            if let Some(g) = all.get(&wid) {
                for id in hide {
                    if let Some(v) = g.regions.get(id) {
                        v.setHidden(true);
                    }
                }
            }
        });
    }
}

#[cfg(windows)]
mod win {
    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
    };

    use super::{background_for, GlassTier, Mode, OsPrefs};

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
    const DWMSBT_NONE: i32 = 1;
    const DWMSBT_MAINWINDOW: i32 = 2;

    pub fn os_prefs() -> OsPrefs {
        let mut animate: i32 = 1;
        // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&mut animate as *mut i32).cast(),
                0,
            )
        };
        OsPrefs {
            reduce_motion: ok != 0 && animate == 0,
            ..OsPrefs::default()
        }
    }

    pub fn apply(window: &mut Window, tier: Option<GlassTier>, _forced: Option<Mode>, mode: Mode) {
        window.set_background_appearance(background_for(tier));
        let Ok(handle) = HasWindowHandle::window_handle(&*window) else {
            return;
        };
        let RawWindowHandle::Win32(h) = handle.as_raw() else {
            return;
        };
        let hwnd = h.hwnd.get() as *mut core::ffi::c_void;
        let dark: i32 = (mode == Mode::Dark) as i32;
        let backdrop = if tier == Some(GlassTier::Mica) {
            DWMSBT_MAINWINDOW
        } else {
            DWMSBT_NONE
        };
        // SAFETY: documented DWM attributes on our own window; failures
        // (Windows 10) leave the window as it was.
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&dark as *const i32).cast(),
                4,
            );
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                (&backdrop as *const i32).cast(),
                4,
            );
        }
    }
}
