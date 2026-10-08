//! App actions, their shortcuts and the native menu (Tauri:
//! `src-tauri/src/menu.rs`, chosen by `HostInfo::menu`).
//!
//! - macOS: the app menu (About, Settings… ⌘,, Services, Hide, Quit ⌘Q,
//!   Quit and Stop Agents) and a Window menu.
//! - Windows and Linux: GPUI draws no native menu bar there, so Settings is
//!   Ctrl+Shift+, (as in the Tauri app) and the in-window menu comes with the
//!   title bar (phase 1, built in-house).

use gpui::{actions, App, KeyBinding, Menu, MenuItem, SystemMenuType};

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
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
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
        ]);
    } else {
        b.push(KeyBinding::new("ctrl-shift-,", OpenSettings, None));
    }
    b
}

/// The native menu bar (shown on macOS).
pub fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Pitwall".into(),
            items: vec![
                MenuItem::action("About Pitwall", crate::settings::About),
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
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
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
    cx.bind_keys(bindings());
    cx.set_menus(menus());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_has_a_shortcut_on_every_os() {
        let b = bindings();
        assert!(b.iter().any(|k| k.action().partial_eq(&OpenSettings)));
        if cfg!(target_os = "macos") {
            assert!(b.iter().any(|k| k.action().partial_eq(&Quit)));
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
        let names: Vec<_> = m[0]
            .items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Action { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect();
        for want in ["Settings…", "Quit Pitwall", "Quit and Stop Agents"] {
            assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
        }
    }
}
