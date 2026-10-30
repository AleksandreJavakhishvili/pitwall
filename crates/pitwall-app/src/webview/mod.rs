//! A native web view inside a Pitwall window: the agent pane's Preview of
//! the dev server its terminal printed (docs/spec/terminals.md §4).
//!
//! One interface, one backend per platform, each the system's own engine
//! (no bundled browser):
//! - macOS: WKWebView, a subview of the window's content view ([`macos`]).
//! - Windows: WebView2, a controller hosted in the window's HWND ([`windows`]).
//! - Linux: WebKitGTK ([`linux`]).
//!
//! The view is a native surface above GPUI's drawing: the UI places it with
//! [`WebView::set_bounds`] every frame it is drawn, hides it when its pane
//! isn't, and swaps it for a [`WebView::snapshot`] while a GPUI overlay
//! (menu, popover, palette) covers it. Everything here runs on the main
//! thread; events arrive there through the `events` callback.

use std::path::PathBuf;

use gpui::{Bounds, Keystroke, Pixels, Window};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// What a backend reports, on the main thread.
#[derive(Debug, Clone, PartialEq)]
pub enum WebEvent {
    /// The page committed a new URL (a link, a redirect, history).
    Url(String),
    Title(String),
    /// A load started (`true`) or finished or failed (`false`).
    Loading(bool),
    /// The main frame couldn't load (the server is down, a bad URL…).
    Failed { url: String, error: String },
    CanGo { back: bool, forward: bool },
    /// The page took keyboard focus (a click into it).
    Focused,
    /// A key with the platform modifier (⌘ on macOS, Ctrl elsewhere) that
    /// isn't an editing key (copy, paste, cut, select all, undo, redo; see
    /// [`editing_key`]): it is Pitwall's (pane switching, the palette…), not
    /// the page's. Also a plain Esc, after the page had it too: the keyboard
    /// goes back to Pitwall.
    Shortcut(Keystroke),
}

/// A picture of the page (RGBA8, row-major, `width * height * 4` bytes),
/// drawn in its place while a GPUI overlay covers the view.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Options {
    pub url: String,
    /// Persistent cookies and storage (one folder per project, so a login
    /// survives restarts and projects don't share one).
    pub data_dir: PathBuf,
    pub events: Box<dyn Fn(WebEvent) + 'static>,
}

/// One web view. Dropping it removes it from the window and frees it.
pub trait WebView {
    fn navigate(&mut self, url: &str);
    fn reload(&mut self);
    fn back(&mut self);
    fn forward(&mut self);
    /// Where it sits, in the window's logical pixels (GPUI's coordinates:
    /// origin top left of the window's content).
    fn set_bounds(&mut self, bounds: Bounds<Pixels>);
    /// Shown or hidden; hidden also pauses the page where the engine can
    /// (timers, animations), so a hidden preview costs nothing.
    fn set_visible(&mut self, visible: bool);
    /// Give the page keyboard focus.
    fn focus(&mut self);
    /// Give keyboard focus back to the window (GPUI).
    fn blur(&mut self);
    /// The engine's own inspector (Web Inspector, DevTools).
    fn open_inspector(&mut self);
    /// A picture of what it shows now; `done` runs on the main thread
    /// (`None`: the engine couldn't).
    fn snapshot(&mut self, done: Box<dyn FnOnce(Option<Snapshot>) + 'static>);
}

/// A web view in `window`, hidden until [`WebView::set_bounds`] and
/// [`WebView::set_visible`]. `Err`: this machine can't show one (no engine
/// installed, an unsupported display server): the caller opens the URL in
/// the browser instead and says why.
pub fn create(window: &mut Window, opts: Options) -> Result<Box<dyn WebView>, String> {
    #[cfg(target_os = "macos")]
    return macos::create(window, opts);
    #[cfg(windows)]
    return windows::create(window, opts);
    #[cfg(target_os = "linux")]
    return linux::create(window, opts);
    #[allow(unreachable_code)]
    {
        let _ = (window, opts);
        Err("No web view on this platform".into())
    }
}

/// The platform modifier's keys the page keeps (copy, paste, cut, select
/// all, undo, redo); every other one goes to Pitwall as
/// [`WebEvent::Shortcut`]. `platform`: the key has ⌘ on macOS, Ctrl
/// elsewhere.
pub fn editing_key(key: &str, platform: bool, shift: bool, alt: bool) -> bool {
    platform && !alt && matches!((key, shift), ("c" | "v" | "x" | "a" | "z", false) | ("z" | "v", true) | ("y", false))
}

/// Only local dev servers: localhost, loopback, private LAN and tailnet
/// addresses, over http(s). The preview isn't a general browser.
pub fn allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) else {
        return false;
    };
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
    };
    let host = host.to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host == "::1" {
        return true;
    }
    match host.parse::<std::net::Ipv4Addr>() {
        // 100.64/10: Tailscale (CGNAT).
        Ok(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_unspecified()
                || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{allowed, editing_key};

    #[test]
    fn the_page_keeps_its_editing_keys() {
        for (key, shift) in [("c", false), ("v", false), ("x", false), ("a", false), ("z", false), ("z", true), ("y", false), ("v", true)] {
            assert!(editing_key(key, true, shift, false), "{key} {shift}");
        }
        for (key, shift) in [("k", false), ("1", false), ("n", true), ("r", false), ("t", false), ("c", true)] {
            assert!(!editing_key(key, true, shift, false), "{key} {shift}");
        }
        assert!(!editing_key("c", false, false, false), "no modifier: not a shortcut at all");
        assert!(!editing_key("c", true, false, true), "⌥⌘C is Pitwall's");
    }

    #[test]
    fn only_local_dev_servers() {
        for ok in [
            "http://localhost:3000",
            "http://localhost:5173/app?x=1",
            "https://app.localhost:8443/",
            "http://127.0.0.1:8000",
            "http://0.0.0.0:4000",
            "http://[::1]:3000/",
            "http://192.168.1.20:3000",
            "http://10.0.0.5",
            "http://172.20.1.1:80",
            "http://100.101.2.3:3000",
        ] {
            assert!(allowed(ok), "{ok}");
        }
        for no in [
            "https://example.com",
            "http://8.8.8.8",
            "http://100.200.1.1",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "http://localhost.evil.com",
            "ftp://localhost",
        ] {
            assert!(!allowed(no), "{no}");
        }
    }
}
