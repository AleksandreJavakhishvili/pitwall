//! The Glass look's native window material (`HostInfo::glass`, layout.md
//! "Appearance"): what the compositor draws behind a transparent webview.
//!
//! - `Vibrancy` (macOS): an `NSVisualEffectView` (under-window-background
//!   material, kept active so an unfocused window stays glass too) behind
//!   the webview.
//! - `Mica` (Windows 11): the Mica system backdrop. Not Acrylic: it lags
//!   while windows are dragged or resized.
//! - `None` (Linux, Windows 10, or `PITWALL_GLASS=lite`): nothing native;
//!   the UI paints "Glass lite" itself.
//!
//! The webview stops painting its own background only while Glass is on
//! (`set_background_color` with alpha 0; on macOS this is WebKit's private
//! `drawsBackground` key, which Tauri ≥ 2.12.1 always uses — fine for a
//! notarized app outside the Mac App Store). Off again, the effect is
//! removed and the page paints opaque as before. macOS "Reduce
//! transparency" turns the native layer off (the UI falls back to Flat).

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use pitwall_core::host::WindowGlass;
use serde::Serialize;
use tauri::window::{Color, Effect, EffectState, EffectsBuilder};
use tauri::{Runtime, WebviewWindow};

/// Windows are created transparent (Windows: DWM draws Mica only behind a
/// transparent window; the page paints opaque unless Glass is on). macOS
/// needs no transparent window: the effect view sits behind a webview that
/// stops drawing its background at runtime.
pub const TRANSPARENT_WINDOWS: bool = cfg!(windows);

/// What the UI needs to draw the right Glass tier.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlassState {
    /// The native material is on behind the (now transparent) webview.
    pub native: bool,
    /// The OS asks for less transparency (macOS accessibility).
    pub reduce_transparency: bool,
    /// The OS asks for more contrast (macOS accessibility).
    pub increase_contrast: bool,
}

/// Windows whose material is on (the UI asks again on focus, to pick up
/// accessibility changes; the effect view is only replaced on a change).
fn glassy() -> &'static Mutex<HashSet<String>> {
    static ON: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ON.get_or_init(Default::default)
}

/// Turn the native material on or off for one window.
pub fn set<R: Runtime>(window: &WebviewWindow<R>, glass: WindowGlass, on: bool) -> GlassState {
    let (reduce_transparency, increase_contrast) = accessibility();
    let effect = match glass {
        WindowGlass::Vibrancy => Some(Effect::UnderWindowBackground),
        WindowGlass::Mica => Some(Effect::Mica),
        WindowGlass::None => None,
    };
    let mut glassy = glassy().lock().unwrap_or_else(|e| e.into_inner());
    let label = window.label().to_string();
    let native = match effect {
        Some(_) if on && !reduce_transparency && glassy.contains(&label) => true,
        Some(effect) if on && !reduce_transparency => {
            let effects = EffectsBuilder::new().effect(effect).state(EffectState::Active).build();
            let ok = window.set_effects(effects).is_ok() && window.set_background_color(Some(Color(0, 0, 0, 0))).is_ok();
            if ok {
                glassy.insert(label);
            }
            ok
        }
        Some(_) => {
            if glassy.remove(&label) {
                let _ = window.set_effects(None);
                let _ = window.set_background_color(None);
            }
            false
        }
        None => false,
    };
    GlassState { native, reduce_transparency, increase_contrast }
}

/// A closed window's label may be reused by a new one (`pitwall-N`).
pub fn forget(label: &str) {
    glassy().lock().unwrap_or_else(|e| e.into_inner()).remove(label);
}

/// macOS accessibility display options ("Reduce transparency", "Increase
/// contrast"); elsewhere the UI's media queries are all there is.
#[cfg(target_os = "macos")]
fn accessibility() -> (bool, bool) {
    let ws = objc2_app_kit::NSWorkspace::sharedWorkspace();
    (ws.accessibilityDisplayShouldReduceTransparency(), ws.accessibilityDisplayShouldIncreaseContrast())
}

#[cfg(not(target_os = "macos"))]
fn accessibility() -> (bool, bool) {
    (false, false)
}
