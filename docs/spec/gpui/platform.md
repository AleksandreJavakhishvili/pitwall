# GPUI port: per-OS integration

What the Tauri app does through Tauri plugins and `src-tauri/src/platform/`,
and how the GPUI app does it. "gpui" means what gpui 0.2.2 offers itself;
everything else is a small module under `crates/pitwall-app/src/platform/`
calling the OS directly, with crates that are already in the tree
(`objc2-app-kit` on macOS, `windows`/`windows-sys` on Windows). The
capability switches stay in `pitwall_core::host::HostInfo` (`menu`, `badge`,
`tray`, `shortcuts`, `glass`), so OS checks don't spread through the UI.
Inventory: §25 (multi-window), §27 (OS integration), §24 (appearance).

| Feature | Tauri today | macOS | Windows | Linux | Phase |
|---|---|---|---|---|---|
| App menu | Default menu + Settings… ⌘, + Quit and Stop Agents (`menu.rs`) | gpui `cx.set_menus` (done: Settings, Services, Hide, Quit, Quit and Stop Agents; Window menu). Add the Edit menu with `MenuItem::os_action` (Copy/Paste/Select All) once text inputs exist | gpui draws no menu bar: an in-window menu in the drawn title bar (in-house), File → Settings Ctrl+Shift+,, Close, Quit, Quit and Stop Agents | No native menu (as today: GTK menus steal F10 and Ctrl keys); same in-window menu or command palette entries | 0 (macOS), 1 |
| Window frame / title bar | native frames (GTK on Linux, Win32 caption + menu bar on Windows); top bar is a `data-tauri-drag-region` | native title bar (unchanged) | native caption (controls, snap layouts, system menu); the in-window File menu bar under it (`app_menu`); dragging / double-clicking the top bar is passed to the caption (`WM_NCLBUTTONDOWN`/`WM_SYSCOMMAND`, `platform/decorations.rs`) | X11 and Wayland compositors with `zxdg_decoration_manager_v1` (KDE, sway): server-side, the top bar still drags the window (on X11 its double-click stays with the window manager's title bar, which keeps the release). Without it (GNOME, Weston) gpui 0.2.2 reports "server" although nothing draws one, so Pitwall probes the registry and asks for client-side: the top bar is the title bar (drag, double-click maximize, right-click window menu, min/max/close at its right end), rounded top corners, hairline border and shadow while floating, resize areas on every edge and corner, none on tiled sides (`platform/decorations.rs`). Debug builds: `PITWALL_DECORATIONS=client\|server` | 1 |
| Shortcut modifier | `HostInfo::shortcuts` | ⌘ | Ctrl+Shift (Ctrl+Shift+Alt for ⌘⇧) | same as Windows | 0–2 |
| Dock / taskbar badge | `set_badge_count` (Dock), overlay icon + tray tooltip (`platform/badge.rs`) | `NSApp.dockTile.setBadgeLabel` via objc2-app-kit (gpui has no badge API) | `ITaskbarList3::SetOverlayIcon` with the existing generated badge icons, via `windows` crate on the HWND from `raw-window-handle` | Unity launcher API over D-Bus (`com.canonical.Unity.LauncherEntry`), best effort, as Tauri does | 7 |
| Tray | Windows only: show/quit menu (`platform/windows.rs`) | none (Dock) | `Shell_NotifyIconW` with a hidden message window and a popup menu (in-house; `tray-icon` crate, MIT/Apache-2.0, is the alternative to evaluate) | none | 7 |
| Close main window | hides (macOS, Windows with tray), quits (Linux) | keep app, `on_reopen` reopens (done) | hide to tray (after tray exists; quits until then) | quit (done) | 0, 7 |
| Notifications | `tauri-plugin-notification`, only when the window isn't focused (`attention.rs`) | `UNUserNotificationCenter` via objc2 (needs a bundled, signed app; dev builds fall back to no-op) | Toast notifications via `windows` (`ToastNotificationManager`, needs an AppUserModelID set by the installer) | `org.freedesktop.Notifications` over D-Bus (`notify-rust`, MIT/Apache-2.0, or `zbus` direct) | 7 |
| Request attention (new approval) | `request_user_attention(Critical)` | `NSApp.requestUserAttention(.criticalRequest)` | `FlashWindowEx` | `_NET_WM_STATE_DEMANDS_ATTENTION` / xdg-activation, best effort | 7 |
| Multi-window | one webview window per moved-out space (`windows.rs`) | gpui `cx.open_window`, all windows share entities | same | same | 2 |
| Drag and drop | HTML5 DnD inside the page | gpui `on_drag`/`on_drop` (in-house component); external file drops via gpui `ExternalPaths` | same | same | 2 |
| Clipboard | web clipboard, xterm | gpui `write_to_clipboard` / `read_from_clipboard` | same | same (X11 and Wayland in gpui) | 1–2 |
| Folder picker / dialogs | `tauri-plugin-dialog` | gpui `cx.prompt_for_paths`, `prompt_for_new_path`, `window.prompt` | same (gpui) | same (gpui via xdg portal, `ashpd`) | 2 |
| Open URL / reveal in Finder / open in editor | `tauri-plugin-opener` | gpui `cx.open_url`, `cx.reveal_path`, `open_with_system` | same | same | 2, 5 |
| Deep links | none today | gpui `on_open_urls` + `register_url_scheme` are there if wanted | — | — | later |
| Single instance | none today (the CLI socket is the de facto lock) | `home.rs` refuses a second app on one data folder (done); for a real "focus the running one", the second launch sends a `focus` request over the CLI socket and exits | same over the named pipe | same | 0, 7 |
| Autostart | none today | — (not in scope) | — | — | — |
| Install the `pitwall` CLI | symlink `pitwall` → bundled `pitwall-cli` in `~/.local/bin` or `/usr/local/bin`; Windows: a copy in `%LOCALAPPDATA%\Pitwall\bin` (`cli_install.rs`) | same logic moved to `platform/cli_install.rs` (nothing in it needs Tauri beyond finding the sidecar) | same (copy + marker file) | same (symlink) | 7 |
| Approvals dialog | `approvals-changed` → modal in every window | `AgentStore.approvals` (bridged in phase 0) → modal (in-house) in the focused window, attention request when the list goes from empty to not empty | same | same | 7 |
| Permissions onboarding (macOS TCC) | `permissions_status`, `open_privacy_settings` | same core calls; open System Settings with `cx.open_url("x-apple.systempreferences:…")` | — | — | 7 |
| Login-shell PATH | `prepare_env` | done (`run()`) | registry PATH (core) | done | 0 |
| Glass | vibrancy (`NSVisualEffectView`), Mica, "Glass lite" (`platform/glass.rs`) | `WindowBackgroundAppearance::Blurred`: gpui adds an `NSVisualEffectView` behind its layer (Big Sur+); translucent surface tokens on top; "Reduce transparency" (`NSWorkspace.accessibilityDisplayShouldReduceTransparency`, objc2 already used) → Flat | gpui's `Blurred` is Acrylic, which lags while dragging (why Tauri chose Mica): use `Transparent` and set Mica with `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE)` on the HWND ourselves; Windows 10 → Glass lite | Wayland: gpui `Blurred` (KDE blur protocol) where available, else Glass lite (painted, opaque) | 6 |
| Reduce motion | setting + `prefers-reduced-motion` | setting + `NSWorkspace.accessibilityDisplayShouldReduceMotion` | setting + `SPI_GETCLIENTAREAANIMATION` | setting + GTK/portal setting, best effort | 6 |
| Light/dark | `prefers-color-scheme` + setting | gpui `window.appearance()` + observer (done) | same | same (portal) | 0, 6 |
| Environment fixes | WebKitGTK DMA-BUF workaround | not needed (no web view) | — | — | — |

Notes:

- macOS bundle identity (`dev.pitwall.app`) matters for notifications, TCC
  and the Dock: released builds keep the Tauri app's id, so they take its
  place; dev bundles built with `PITWALL_CHANNEL=preview` use
  `dev.pitwall.app.preview` so their permissions and notification settings
  don't mix with the installed app's.
- gpui ships an unused macOS `status_item.rs`; it is not public API, so the
  tray (Windows only) does not rely on it.
