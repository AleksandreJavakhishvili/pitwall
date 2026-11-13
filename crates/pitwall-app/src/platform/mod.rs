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

/// The user is in Pitwall: it is the app in front (macOS: the active
/// app, whichever of its windows; elsewhere: one of its main windows is
/// the active window). The floating control, which never becomes the
/// active window, doesn't count.
pub fn app_in_front(cx: &App) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = cx;
        macos::app_active()
    }
    #[cfg(not(target_os = "macos"))]
    {
        cx.active_window().is_some_and(|w| w.downcast::<crate::ui::MainView>().is_some())
    }
}

/// Give the floating control a new size, keeping the corner it sits on:
/// its bottom right on macOS (it opens upward and to the left), its top
/// left elsewhere (the caller opens it again when its width changes).
pub fn resize_float(window: &mut Window, size: gpui::Size<gpui::Pixels>, cx: &mut App) {
    // Test windows have no native view (their handle panics).
    #[cfg(all(target_os = "macos", not(test)))]
    macos::resize_from_bottom_right(window, size.width.into(), size.height.into(), cx);
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = cx;
        window.resize(size);
    }
}

/// The floating control draws its own shadow (none from the system) and
/// never takes the keyboard from the app the user is in.
pub fn float_setup(window: &mut Window) {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        macos::no_shadow(window);
        macos::never_key(window);
    }
    #[cfg(any(not(target_os = "macos"), test))]
    let _ = window;
}

/// The floating control takes the keyboard (the user clicked a terminal
/// in it), without Pitwall coming forward.
pub fn float_take_keys(window: &mut Window) {
    #[cfg(all(target_os = "macos", not(test)))]
    macos::take_keys(window);
    #[cfg(any(not(target_os = "macos"), test))]
    let _ = window;
}

/// Move the floating control with the pointer until the button is
/// released (called once the press has travelled far enough).
pub fn float_drag(window: &mut Window, cx: &mut App) {
    #[cfg(all(target_os = "macos", not(test)))]
    macos::drag_with_pointer(window, cx);
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = cx;
        window.start_window_move();
    }
}

/// What the system says about Pitwall being active, for the floating
/// control's log (macOS: `NSApplication.isActive`, which is not what
/// [`app_in_front`] goes by).
pub fn app_active_raw() -> bool {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        macos::ns_app_active()
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        false
    }
}

/// The window a glass pop-up (`crate::kit::pop`) belongs to, as the OS
/// knows it.
pub struct PopParent(#[cfg(all(target_os = "macos", not(test)))] macos::PopParent);

/// Whether floating surfaces can be windows of their own here (a
/// transparent child window with the system's glass behind it): macOS.
pub fn pop_supported() -> bool {
    cfg!(target_os = "macos")
}

/// `window` as a pop-up's parent. `None`: it has no native window.
pub fn pop_parent(window: &Window) -> Option<PopParent> {
    // Test windows have no native view (their handle panics).
    #[cfg(all(target_os = "macos", not(test)))]
    {
        macos::pop_parent(window).map(PopParent)
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = window;
        Some(PopParent())
    }
}

/// A new pop-up window: nothing of the system's is drawn around it.
pub fn pop_setup(window: &mut Window) {
    #[cfg(all(target_os = "macos", not(test)))]
    macos::pop_setup(window);
    #[cfg(any(not(target_os = "macos"), test))]
    let _ = window;
}

/// The usable area of the screen that shows the point `at` of the
/// parent's content, in the parent's own coordinates (they may be
/// negative: the screen starts left of and above the window). `None`
/// where that isn't known (the pop-up is then placed as asked).
pub fn pop_area(parent: &PopParent, at: gpui::Point<gpui::Pixels>) -> Option<gpui::Bounds<gpui::Pixels>> {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        use gpui::{point, px, size};
        let (x, y, w, h) = macos::pop_area(&parent.0, at.x.into(), at.y.into())?;
        Some(gpui::Bounds::new(point(px(x), px(y)), size(px(w), px(h))))
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = (parent, at);
        None
    }
}

/// Put the pop-up `window` at `rect` of its parent's content, show it
/// above the parent (it then moves with it) and give it the keyboard.
pub fn pop_place(window: &mut Window, parent: &PopParent, rect: gpui::Bounds<gpui::Pixels>, cx: &mut App) {
    #[cfg(all(target_os = "macos", not(test)))]
    macos::pop_place(
        window,
        &parent.0,
        (rect.origin.x.into(), rect.origin.y.into(), rect.size.width.into(), rect.size.height.into()),
        cx,
    );
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = (parent, cx);
        window.resize(rect.size);
    }
}

/// What the system says about the pop-up `window` (its frame, who has
/// the keyboard, which window is the main one), for `kit::pop::debug`.
pub fn pop_debug(window: &mut Window) -> String {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        macos::pop_debug(window)
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = window;
        String::new()
    }
}

/// The pop-up `window` is about to close: the parent takes the keyboard
/// back if the pop-up had it.
pub fn pop_close(window: &mut Window, cx: &mut App) {
    #[cfg(all(target_os = "macos", not(test)))]
    macos::pop_close(window, cx);
    #[cfg(any(not(target_os = "macos"), test))]
    let _ = (window, cx);
}

/// Debug builds on macOS: a left click in `window` at (`x`, `y`) from
/// its top left, delivered as AppKit delivers one (in this process only).
pub fn debug_click(window: &mut Window, x: f32, y: f32, cx: &mut App) {
    #[cfg(all(target_os = "macos", not(test)))]
    macos::debug_click(window, x, y, cx);
    #[cfg(any(not(target_os = "macos"), test))]
    let _ = (window, x, y, cx);
}

/// Debug builds on macOS: `front` notes the app the user is in,
/// `deactivate` gives it its place back (as when the user goes to another
/// app); anything else only reports.
/// What the system then says of the app and its windows.
pub fn debug_app(what: &str) -> String {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        macos::debug_app(what)
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = what;
        String::new()
    }
}

/// The pointer over the floating control: the pointing hand over what
/// takes a click, the arrow otherwise. macOS only: the control shows while
/// another app is active, where GPUI's cursor styles (which follow the
/// active app's key window) don't apply. Elsewhere GPUI's do.
pub fn float_cursor(pointing: bool) {
    #[cfg(all(target_os = "macos", not(test)))]
    macos::set_cursor(pointing);
    #[cfg(any(not(target_os = "macos"), test))]
    let _ = pointing;
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
