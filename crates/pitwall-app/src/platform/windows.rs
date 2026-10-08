//! Windows: the tray icon (the way back to a hidden window; there is no
//! Dock), the blocked count as a taskbar overlay icon and in the tray
//! tooltip, flashing the taskbar button, and hiding windows to the tray
//! (Tauri: `src-tauri/src/platform/windows.rs`). Win32 calls go to the HWND
//! gpui gives through `raw-window-handle`; all on the main thread.

use std::cell::RefCell;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui::{App, AsyncApp, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIcon, DestroyIcon, FlashWindowEx, ShowWindow, FLASHWINFO, FLASHW_ALL, FLASHW_TIMERNOFG,
    HICON, SW_HIDE, SW_SHOW,
};

use super::badge;
use crate::menu::{OpenSettings, Quit, QuitAndStopAgents, ShowMain};

/// The app icon in the notification area.
const ICON_PNG: &[u8] = include_bytes!("../../packaging/icons/32x32.png");

thread_local! {
    /// The tray icon lives as long as the app (dropping it removes it).
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

/// The window's HWND, if gpui gives one.
pub fn hwnd(window: &Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// What a tray click or tray menu item asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayCommand {
    Show,
    Settings,
    Quit,
    QuitAndStop,
}

const SHOW_ID: &str = "pitwall-show";
const SETTINGS_ID: &str = "pitwall-settings";
const QUIT_ID: &str = "pitwall-quit";
const QUIT_STOP_ID: &str = "pitwall-quit-stop";

fn command(id: &str) -> Option<TrayCommand> {
    Some(match id {
        SHOW_ID => TrayCommand::Show,
        SETTINGS_ID => TrayCommand::Settings,
        QUIT_ID => TrayCommand::Quit,
        QUIT_STOP_ID => TrayCommand::QuitAndStop,
        _ => return None,
    })
}

fn icon() -> Option<Icon> {
    let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).ok()
}

/// The tray icon: left click shows the main window; its menu has Open
/// Pitwall, Settings…, Quit Pitwall and Quit and Stop Agents. Returns
/// whether it is up (closing the main window then only hides it).
pub fn setup_tray(cx: &mut App) -> bool {
    let menu = Menu::new();
    let items = [
        MenuItem::with_id(SHOW_ID, "Open Pitwall", true, None),
        MenuItem::with_id(SETTINGS_ID, "Settings…", true, None),
        MenuItem::with_id(QUIT_ID, "Quit Pitwall", true, None),
        MenuItem::with_id(QUIT_STOP_ID, "Quit and Stop Agents", true, None),
    ];
    let sep = PredefinedMenuItem::separator();
    let appended = menu
        .append_items(&[&items[0], &items[1], &sep, &items[2], &items[3]])
        .is_ok();
    let mut builder = TrayIconBuilder::new()
        .with_id("pitwall-tray")
        .with_tooltip(badge::tooltip(0))
        .with_menu_on_left_click(false);
    if appended {
        builder = builder.with_menu(Box::new(menu));
    }
    if let Some(icon) = icon() {
        builder = builder.with_icon(icon);
    }
    let tray = match builder.build() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("pitwall: no tray icon: {e}");
            return false;
        }
    };
    TRAY.with(|t| *t.borrow_mut() = Some(tray));

    // Tray callbacks come from the message loop; run them on the app.
    let (tx, mut rx) = unbounded::<TrayCommand>();
    let menu_tx: UnboundedSender<TrayCommand> = tx.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if let Some(c) = command(e.id().as_ref()) {
            let _ = menu_tx.unbounded_send(c);
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = e
        {
            let _ = tx.unbounded_send(TrayCommand::Show);
        }
    }));
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(c) = rx.next().await {
            let _ = cx.update(|cx| match c {
                TrayCommand::Show => cx.dispatch_action(&ShowMain),
                TrayCommand::Settings => cx.dispatch_action(&OpenSettings),
                TrayCommand::Quit => cx.dispatch_action(&Quit),
                TrayCommand::QuitAndStop => cx.dispatch_action(&QuitAndStopAgents),
            });
        }
    })
    .detach();
    true
}

/// The tray tooltip follows the blocked count.
pub fn set_tray_tooltip(count: usize) {
    TRAY.with(|t| {
        if let Some(tray) = t.borrow().as_ref() {
            let _ = tray.set_tooltip(Some(badge::tooltip(count)));
        }
    });
}

/// A red disc with the count on the taskbar button (none at 0).
pub fn set_overlay(window: &Window, count: usize) {
    let Some(hwnd) = hwnd(window) else { return };
    // SAFETY: plain COM and GDI calls on the UI thread with a live HWND; the
    // icon is destroyed after the taskbar has copied it.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(list) =
            CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
        else {
            return;
        };
        if list.HrInit().is_err() {
            return;
        }
        if count == 0 {
            let _ = list.SetOverlayIcon(hwnd, HICON::default(), PCWSTR::null());
            return;
        }
        let Some(icon) = overlay_icon(count) else {
            return;
        };
        let _ = list.SetOverlayIcon(hwnd, icon, w!("Agents need you"));
        let _ = DestroyIcon(icon);
    }
}

/// `badge::rgba` as an HICON (32-bit colour with alpha, empty AND mask).
fn overlay_icon(count: usize) -> Option<HICON> {
    let rgba = badge::rgba(count);
    let bgra: Vec<u8> = rgba
        .chunks(4)
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect();
    let n = badge::SIZE as i32;
    // 1 bpp, rows padded to 16 bits.
    let mask = vec![0u8; (badge::SIZE.div_ceil(16) * 2 * badge::SIZE) as usize];
    // SAFETY: both buffers outlive the call and have the sizes CreateIcon reads.
    unsafe { CreateIcon(None, n, n, 1, 32, mask.as_ptr(), bgra.as_ptr()).ok() }
}

/// Flash the taskbar button until the window comes to the front.
pub fn flash(window: &Window) {
    let Some(hwnd) = hwnd(window) else { return };
    let info = FLASHWINFO {
        cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
        hwnd,
        dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
        uCount: 0,
        dwTimeout: 0,
    };
    // SAFETY: a valid FLASHWINFO for a live window.
    unsafe {
        let _ = FlashWindowEx(&info);
    }
}

/// Hide to the tray (agents keep running).
pub fn hide(window: &Window) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: a live HWND owned by this process.
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// Bring a hidden window back (gpui activates it after).
pub fn unhide(window: &Window) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: a live HWND owned by this process.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_ids_map_to_commands() {
        assert_eq!(command(SHOW_ID), Some(TrayCommand::Show));
        assert_eq!(command(QUIT_STOP_ID), Some(TrayCommand::QuitAndStop));
        assert_eq!(command("other"), None);
        assert!(icon().is_some(), "the tray icon decodes");
        assert!(overlay_icon(3).is_some());
    }
}
