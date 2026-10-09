//! OS integration (docs/spec/gpui/platform.md; Tauri: plugins and
//! `src-tauri/src/platform/`). The UI calls these functions; OS checks stay
//! in here.
//!
//! - Badge: the Dock badge (macOS), a taskbar overlay icon and the tray
//!   tooltip (Windows), the Unity launcher badge (Linux).
//! - Tray (Windows): Open Pitwall / Settings… / Quit / Quit and Stop Agents;
//!   closing the main window then hides it there.
//! - Notifications when an agent needs the user and Pitwall isn't in front.
//! - Attention: the Dock bounce / taskbar flash for a new approval.
//! - Single instance, the `pitwall` CLI install, shipped helpers, and the
//!   clipboard / folder picker / opener seams ([`files`]).
//! - The in-window File menu for Windows ([`app_menu`]); the macOS menu bar
//!   is `crate::menu`.

pub mod app_menu;
pub mod badge;
pub mod cli_install;
pub mod decorations;
pub mod files;
pub mod glass;
pub mod notify;
pub mod sidecar;
pub mod single_instance;
pub mod visible;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use gpui::{AnyWindowHandle, App, Entity, Global, Window};

use crate::agents::{AgentStore, StoreEvent};

/// The bundle id this build ships as: `dev.pitwall.app` (the stable
/// channel, the default), or the channel's id that `scripts/package-app.sh`
/// bakes in (`dev.pitwall.app.preview` for `PITWALL_CHANNEL=preview`).
/// Windows: the AppUserModelID the installers give the Start menu shortcut,
/// which toasts must name to be shown.
pub const BUNDLE_ID: &str = match option_env!("PITWALL_BUNDLE_ID") {
    Some(id) => id,
    None => "dev.pitwall.app",
};

/// The main window's app id. Linux: the `.desktop` file's name (Wayland
/// `app_id`, X11 `WM_CLASS`), so docks match the window to its launcher, icon
/// and badge (`linux::DESKTOP_ID`); the bundle id elsewhere.
pub const WINDOW_APP_ID: &str = if cfg!(target_os = "linux") {
    "pitwall"
} else {
    BUNDLE_ID
};

/// What the OS glue remembers.
#[derive(Default)]
struct PlatformState {
    /// Windows: the tray icon is up, so the main window hides on close.
    tray: bool,
    /// Windows that hide instead of closing (the main window, with a tray).
    hide_on_close: Vec<AnyWindowHandle>,
    /// The badge's last count (re-applied to new windows on Windows).
    blocked: usize,
}

impl Global for PlatformState {}

/// Once, at start: notifications' identity and (Windows) the tray.
pub fn init(cx: &mut App) {
    notify::init();
    #[cfg(windows)]
    let tray = windows::setup_tray(cx);
    #[cfg(not(windows))]
    let tray = false;
    cx.set_global(PlatformState {
        tray,
        ..Default::default()
    });
}

fn state(cx: &mut App) -> &mut PlatformState {
    if !cx.has_global::<PlatformState>() {
        cx.set_global(PlatformState::default());
    }
    cx.global_mut::<PlatformState>()
}

/// Keep the badge on the blocked count and notify on attention events
/// (Tauri: `attention::set_badge`, `attention::notify`).
pub fn follow(store: &Entity<AgentStore>, cx: &mut App) {
    let initial = store.read(cx).blocked;
    set_badge(initial, cx);
    cx.observe(store, |store, cx| {
        let blocked = store.read(cx).blocked;
        if blocked != state(cx).blocked {
            set_badge(blocked, cx);
        }
    })
    .detach();
    cx.subscribe(store, |_, event: &StoreEvent, cx| {
        let StoreEvent::Attention {
            name,
            reason,
            detail,
            ..
        } = event
        else {
            return;
        };
        if notify::wanted(pitwall_in_front(cx), notify::bench()) {
            notify::send(name.clone(), notify::body(reason, detail.as_deref()));
        }
    })
    .detach();
}

/// The user is looking at Pitwall: its main window is the active one
/// (Tauri `attention.rs`: main visible and focused).
fn pitwall_in_front(cx: &App) -> bool {
    let main = cx.try_global::<crate::AppState>().and_then(|s| s.main);
    match (cx.active_window(), main) {
        (Some(active), Some(main)) => active.window_id() == main.window_id(),
        _ => false,
    }
}

/// Show `count` agents needing the user on the Dock / taskbar / launcher.
pub fn set_badge(count: usize, cx: &mut App) {
    state(cx).blocked = count;
    #[cfg(target_os = "macos")]
    macos::set_badge(count);
    #[cfg(target_os = "linux")]
    linux::set_badge(count);
    #[cfg(windows)]
    {
        windows::set_tray_tooltip(count);
        for w in cx.windows() {
            let _ = w.update(cx, |_, window, _| windows::set_overlay(window, count));
        }
    }
}

/// Make sure the user notices (a new approval): the Dock bounces until
/// Pitwall is activated; on Windows the taskbar button flashes. Linux: gpui
/// has no attention request, the notification and badge carry it.
pub fn request_attention(cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        let _ = &cx;
        macos::request_attention();
    }
    #[cfg(windows)]
    for w in cx.windows() {
        let _ = w.update(cx, |_, window, _| windows::flash(window));
    }
    #[cfg(target_os = "linux")]
    let _ = cx;
}

/// The About panel (macOS menu "About Pitwall").
pub fn about(_cx: &mut App) {
    #[cfg(target_os = "macos")]
    macos::about();
}

/// The main window: closing it hides it (agents keep running) on macOS,
/// where the Dock brings it back (`on_reopen`), and on Windows with a tray,
/// as the Tauri app does (`hidesOnClose`); Linux quits.
pub fn keep_on_close(window: &mut Window, cx: &mut App) {
    let handle = window.window_handle();
    let blocked = state(cx).blocked;
    #[cfg(windows)]
    windows::set_overlay(window, blocked);
    let _ = blocked;
    if !cfg!(target_os = "macos") && !state(cx).tray {
        return;
    }
    state(cx).hide_on_close.push(handle);
    window.on_window_should_close(cx, |window, _| {
        hide(window);
        false
    });
}

/// Settings for a new window's renderer (macOS: two drawables, see
/// `macos::two_drawables`). Call once per window, as it opens.
pub fn lean_renderer(window: &Window) {
    // Test windows have no native view (their handle panics).
    #[cfg(all(target_os = "macos", not(test)))]
    macos::two_drawables(window);
    #[cfg(all(target_os = "macos", test))]
    let _ = window;
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// Close (or, for a window kept by [`keep_on_close`], hide) a window.
pub fn close_window(handle: AnyWindowHandle, cx: &mut App) {
    let hides = state(cx).hide_on_close.contains(&handle);
    let _ = handle.update(cx, |_, window, _| {
        if hides {
            hide(window);
        } else {
            window.remove_window();
        }
    });
}

pub(crate) fn hide(window: &mut Window) {
    #[cfg(windows)]
    windows::hide(window);
    #[cfg(target_os = "macos")]
    macos::hide(window);
    #[cfg(not(any(windows, target_os = "macos")))]
    window.minimize_window();
}

/// Before activating a window that may be hidden in the tray.
pub fn unhide(window: &mut Window) {
    #[cfg(windows)]
    windows::unhide(window);
    #[cfg(not(windows))]
    let _ = window;
}
