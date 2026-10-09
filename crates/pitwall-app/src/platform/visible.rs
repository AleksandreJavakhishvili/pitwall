//! Is a window on screen? The React app pauses every poll while
//! `document.hidden` (the window minimised, hidden or fully covered) and
//! catches up when it shows again (`src/lib/freshness.ts`). GPUI 0.2.2 has
//! no visibility event, so this asks the OS: macOS the window's occlusion
//! state, Windows whether it is shown and not minimised. Linux can't tell
//! (Wayland doesn't say): a window counts as visible there.

use std::time::Duration;

use gpui::{App, AsyncApp, Window};

/// How often a paused poll looks again (the catch-up delay).
const RECHECK: Duration = Duration::from_secs(1);

/// The window is on screen (not minimised, hidden or fully covered).
pub fn window_visible(window: &Window) -> bool {
    imp::visible(window)
}

/// Any of the app's windows is on screen.
pub fn any_visible(cx: &mut App) -> bool {
    let windows = cx.windows();
    windows.is_empty()
        || windows
            .into_iter()
            .any(|w| w.update(cx, |_, window, _| window_visible(window)).unwrap_or(true))
}

/// Returns once one of the app's windows is on screen (at once when one
/// is): the polls behind the window pause while it is hidden, minimised or
/// covered, and catch up within a second of it showing.
pub async fn until_any_visible(cx: &mut AsyncApp) {
    loop {
        match cx.update(any_visible) {
            Ok(true) | Err(_) => return,
            Ok(false) => cx.background_executor().timer(RECHECK).await,
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use gpui::Window;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindowOcclusionState};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub fn visible(window: &Window) -> bool {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return true;
        };
        let RawWindowHandle::AppKit(h) = handle.as_raw() else {
            return true;
        };
        // SAFETY: GPUI hands out its live NSView; we only retain it.
        let Some(view): Option<Retained<NSView>> =
            (unsafe { Retained::retain(h.ns_view.as_ptr().cast()) })
        else {
            return true;
        };
        let Some(ns) = view.window() else {
            return true;
        };
        ns.occlusionState().contains(NSWindowOcclusionState::Visible) && !ns.isMiniaturized()
    }
}

#[cfg(windows)]
mod imp {
    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};

    pub fn visible(window: &Window) -> bool {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return true;
        };
        let RawWindowHandle::Win32(h) = handle.as_raw() else {
            return true;
        };
        let hwnd = h.hwnd.get() as _;
        // SAFETY: plain queries on GPUI's live window.
        unsafe { IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use gpui::Window;

    pub fn visible(_: &Window) -> bool {
        true
    }
}
