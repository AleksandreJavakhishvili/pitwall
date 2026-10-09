//! App actions, their shortcuts and the native menu (Tauri:
//! `src-tauri/src/menu.rs`, chosen by `HostInfo::menu`).
//!
//! - macOS: the menu bar Tauri's default menu gives the app (About, Settings…
//!   ⌘,, Services, Hide, Quit ⌘Q, Quit and Stop Agents; File; Edit; View;
//!   Window).
//! - Windows: GPUI draws no native menu bar there; the same File menu
//!   (Settings… Ctrl+Shift+,, Close Window, Quit, Quit and Stop Agents) is an
//!   in-window menu ([`crate::platform::app_menu`]) and the tray's menu. No
//!   Edit menu: its Ctrl accelerators belong to the terminals.
//! - Linux: no menu (GTK menus take F10 and Ctrl keys); Settings is
//!   Ctrl+Shift+, and quitting lives in the command palette.
//!
//! The Edit actions ([`Copy`], [`Paste`], …) are what text inputs and
//! terminals handle for ⌘C/⌘V on macOS; on Windows and Linux terminals bind
//! their own Ctrl+Shift+C/V.

use gpui::{actions, App, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType};

actions!(
    pitwall,
    [
        /// Quit the UI; agents keep running in their holders.
        Quit,
        /// End every agent, then quit.
        QuitAndStopAgents,
        /// Open Settings (a placeholder until phase 6).
        OpenSettings,
        /// Close whatever overlay is open (Settings, dialogs).
        Dismiss,
        /// Show (and focus) the main window: the tray, a second launch.
        ShowMain,
        /// macOS: the standard About panel.
        About,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        /// Close the focused window (main hides where a Dock or tray brings
        /// it back).
        CloseWindow,
        ToggleFullScreen,
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
        SelectAll,
    ]
);

/// The keystrokes of the app's own actions on this OS (`HostInfo::shortcuts`:
/// ⌘ on macOS, Ctrl+Shift elsewhere, so Ctrl stays the terminal's).
pub fn bindings() -> Vec<KeyBinding> {
    let mut b = vec![KeyBinding::new("escape", Dismiss, None)];
    if cfg!(target_os = "macos") {
        b.extend([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
            KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
            KeyBinding::new("cmd-z", Undo, None),
            KeyBinding::new("shift-cmd-z", Redo, None),
            KeyBinding::new("cmd-x", Cut, None),
            KeyBinding::new("cmd-c", Copy, None),
            KeyBinding::new("cmd-v", Paste, None),
            KeyBinding::new("cmd-a", SelectAll, None),
        ]);
    } else {
        b.push(KeyBinding::new("ctrl-shift-,", OpenSettings, None));
        // What Ctrl+Shift+, types on Windows and Linux (gpui 0.2.2 drops the
        // Shift of a shifted symbol).
        b.push(KeyBinding::new("ctrl-<", OpenSettings, None));
    }
    b
}

/// The native menu bar (shown on macOS; the order of Tauri's default menu).
pub fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Pitwall".into(),
            items: vec![
                MenuItem::action("About Pitwall", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Pitwall", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Pitwall", Quit),
                MenuItem::action("Quit and Stop Agents", QuitAndStopAgents),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![MenuItem::action("Close Window", CloseWindow)],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![MenuItem::action("Enter Full Screen", ToggleFullScreen)],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
                MenuItem::separator(),
                MenuItem::action("Close Window", CloseWindow),
            ],
        },
    ]
}

/// The app-wide handlers that need no window.
pub fn register(cx: &mut App) {
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| {
        if let Some(w) = cx.active_window() {
            let _ = w.update(cx, |_, window, _| window.minimize_window());
        }
    });
    cx.on_action(|_: &Zoom, cx| {
        if let Some(w) = cx.active_window() {
            let _ = w.update(cx, |_, window, _| window.zoom_window());
        }
    });
    cx.on_action(|_: &ToggleFullScreen, cx| {
        if let Some(w) = cx.active_window() {
            let _ = w.update(cx, |_, window, _| window.toggle_fullscreen());
        }
    });
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(w) = cx.active_window() {
            crate::platform::close_window(w, cx);
        }
    });
    cx.bind_keys(bindings());
    cx.set_menus(menus());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(menu: &Menu) -> Vec<String> {
        menu.items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Action { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn settings_has_a_shortcut_on_every_os() {
        let b = bindings();
        assert!(b.iter().any(|k| k.action().partial_eq(&OpenSettings)));
        if cfg!(target_os = "macos") {
            assert!(b.iter().any(|k| k.action().partial_eq(&Quit)));
            assert!(b.iter().any(|k| k.action().partial_eq(&Copy)));
        } else {
            // Ctrl alone belongs to terminals there (HostInfo::shortcuts).
            assert!(!b.iter().any(|k| k
                .keystrokes()
                .iter()
                .any(|s| s.modifiers().control && !s.modifiers().shift)));
        }
    }

    #[test]
    fn the_app_menu_has_settings_and_both_quits() {
        let m = menus();
        let names = names(&m[0]);
        for want in [
            "About Pitwall",
            "Settings…",
            "Quit Pitwall",
            "Quit and Stop Agents",
        ] {
            assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
        }
        assert_eq!(names[1], "Settings…", "Settings right after About");
    }

    #[test]
    fn the_menu_bar_is_tauris_default_order() {
        let m = menus();
        let bar: Vec<_> = m.iter().map(|m| m.name.to_string()).collect();
        assert_eq!(bar, ["Pitwall", "File", "Edit", "View", "Window"]);
        let edit = names(&m[2]);
        assert_eq!(
            edit,
            ["Undo", "Redo", "Cut", "Copy", "Paste", "Select All"]
        );
    }
}
