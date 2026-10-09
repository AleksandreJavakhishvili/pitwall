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

#[cfg(not(target_os = "macos"))]
pub fn sync_region(_: &mut Window, _: &'static str, _: Bounds<Pixels>, _: f32) {}

#[cfg(not(target_os = "macos"))]
pub fn sweep_regions(_: &mut Window) {}

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
        /// Chrome pieces, by region id: the view and whether this frame
        /// placed it.
        regions: HashMap<&'static str, (Retained<NSGlassEffectView>, bool)>,
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
                    for (_, (v, _)) in g.regions.drain() {
                        v.removeFromSuperview();
                    }
                }
                _ => {}
            }
            let _ = gpui_view;
        });
    }

    /// Place (or move) the glass piece `id` under GPUI's view at `bounds`
    /// (window coordinates, top-left origin), with `radius` corners. Only
    /// while the window has Liquid Glass; GPUI must leave that region
    /// unpainted (or tinted) for the glass to show.
    pub fn sync_region(window: &mut Window, id: &'static str, bounds: Bounds<Pixels>, radius: f32) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let wid = Window::window_handle(window).window_id();
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
            let entry = g.regions.entry(id).or_insert_with(|| {
                let v = new_glass(mtm, frame);
                // Right under GPUI's view: above the window-wide glass.
                content.addSubview_positioned_relativeTo(
                    &v,
                    NSWindowOrderingMode::Below,
                    Some(&gpui_view),
                );
                (v, true)
            });
            if entry.0.frame() != frame {
                entry.0.setFrame(frame);
            }
            if entry.0.cornerRadius() != radius as f64 {
                entry.0.setCornerRadius(radius as f64);
            }
            entry.1 = true;
        });
    }

    /// After a frame: drop the regions no element placed in it, and start
    /// the next frame's bookkeeping.
    pub fn sweep_regions(window: &mut Window) {
        let wid = Window::window_handle(window).window_id();
        GLASS.with_borrow_mut(|all| {
            if let Some(g) = all.get_mut(&wid) {
                g.regions.retain(|_, (v, seen)| {
                    if !*seen {
                        v.removeFromSuperview();
                    }
                    std::mem::replace(seen, false)
                });
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
