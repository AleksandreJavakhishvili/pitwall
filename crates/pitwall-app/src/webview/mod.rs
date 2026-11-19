//! A native web view inside a Pitwall window: a web tile of the layout
//! (docs/spec/web.md), either a page the user opened or an agent's Preview
//! of the dev server its terminal printed (docs/spec/terminals.md §4).
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
    /// The page asked to download this (a file the engine can't show):
    /// declined, the tile says so and offers the browser (macOS).
    Download(String),
}

/// Where a view's pages may go ([`stays`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// An agent's Preview: local dev servers only ([`allowed`]); another
    /// web address opens in the browser.
    Local,
    /// A web page the user opened: any http(s) address.
    Web,
}

/// A main-frame address `policy` keeps in the view. Anything else is
/// cancelled: an http(s) one opens in the browser ([`to_browser`]), any
/// other scheme (file:, custom app schemes like `msteams:`) isn't opened.
pub fn stays(policy: Policy, url: &str) -> bool {
    match policy {
        Policy::Local => allowed(url),
        Policy::Web => is_http(url),
    }
}

/// A cancelled main-frame address that opens in the browser instead.
pub fn to_browser(policy: Policy, url: &str) -> bool {
    is_http(url) && !stays(policy, url)
}

use pitwall_proto::web::is_http;
/// The address field's rules (also what agents may ask for).
pub use pitwall_proto::web::{address, allowed};

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
    /// Where its pages may go.
    pub policy: Policy,
    /// Persistent cookies and storage: a folder under Pitwall's data
    /// folder, one per profile ([`crate::preview::data_dir`]), so a login
    /// survives restarts and isn't shared with the system's browsers.
    pub data_dir: PathBuf,
    pub events: Box<dyn Fn(WebEvent) + 'static>,
}

/// One web view. Dropping it removes it from the window and frees it.
pub trait WebView {
    fn navigate(&mut self, url: &str);
    fn reload(&mut self);
    /// Stop loading.
    fn stop(&mut self);
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
    /// Moved into `window` (a page that went to another window), keeping
    /// the page as it is, hidden until placed there. `false`: this engine
    /// can't; the caller makes a new view there (the page loads again).
    fn reparent(&mut self, window: &mut Window) -> bool {
        let _ = window;
        false
    }
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

/// Each data folder can have a persistent store of its own (macOS 14+,
/// WebView2's user data folders, WebKitGTK's data managers). `false`
/// (macOS before 14): every view shares one, so only the Default browser
/// profile can be used.
pub fn separate_stores() -> bool {
    #[cfg(target_os = "macos")]
    return macos::separate_stores();
    #[allow(unreachable_code)]
    true
}

/// Delete the engine's store of data folder `dir` (a browser profile
/// removed; no view uses it any more), then the folder. `done` runs on
/// the main thread (macOS) or the calling thread's next turn.
pub fn erase_store(dir: &std::path::Path, done: Box<dyn FnOnce(Result<(), String>)>) {
    let folder = dir.to_path_buf();
    let then: Box<dyn FnOnce(Result<(), String>)> = Box::new(move |res| {
        // The folder only once the engine's store is gone (macOS keeps it
        // elsewhere): a folder left says its id isn't free yet.
        let res = res.and_then(|()| match std::fs::remove_dir_all(&folder) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", folder.display())),
            _ => Ok(()),
        });
        done(res)
    });
    #[cfg(target_os = "macos")]
    return macos::erase_store(dir, then);
    // WebView2 and WebKitGTK keep everything in the folder.
    #[allow(unreachable_code)]
    then(Ok(()))
}

/// The identifier of data folder `dir`'s store on macOS 14+
/// (`dataStoreForIdentifier:`): stable across runs, different for every
/// folder (each project's Preview, each browser profile, each Pitwall data
/// folder). A version-4 UUID's bytes.
pub(crate) fn store_id(dir: &std::path::Path) -> [u8; 16] {
    let text = dir.to_string_lossy();
    let fnv = |seed: u64| text.bytes().fold(seed, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&fnv(0xcbf2_9ce4_8422_2325).to_be_bytes());
    id[8..].copy_from_slice(&fnv(0x6c62_272e_07bb_0142).to_be_bytes());
    // A version-4, RFC 4122 UUID (and never the nil one WebKit rejects).
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

/// The platform modifier's keys the page keeps (copy, paste, cut, select
/// all, undo, redo); every other one goes to Pitwall as
/// [`WebEvent::Shortcut`]. `platform`: the key has ⌘ on macOS, Ctrl
/// elsewhere.
pub fn editing_key(key: &str, platform: bool, shift: bool, alt: bool) -> bool {
    platform && !alt && matches!((key, shift), ("c" | "v" | "x" | "a" | "z", false) | ("z" | "v", true) | ("y", false))
}

#[cfg(test)]
mod tests {
    use super::{editing_key, stays, store_id, to_browser, Policy};
    use pitwall_core::accounts::{web_profile_dir, DEFAULT};
    use std::path::Path;

    /// Every data folder has its own store, the same at every start:
    /// Default's identifier is the one it had before profiles (the user's
    /// sign-ins stay), each profile's differs from it and from each other,
    /// and another Pitwall data folder (an isolated test instance) never
    /// gets the user's.
    #[test]
    fn each_profile_keeps_a_store_of_its_own() {
        let root = Path::new("/Users/sam/.pitwall");
        let default = store_id(&web_profile_dir(root, DEFAULT));
        assert_eq!(web_profile_dir(root, DEFAULT), Path::new("/Users/sam/.pitwall/webview/profile-default"));
        // Pinned: what the folder gave before profiles (FNV-1a of the path).
        let hex: String = default.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "ea71f8ffed36480694617c2a9816b71b", "Default's store must not move");
        assert_eq!(default, store_id(Path::new("/Users/sam/.pitwall/webview/profile-default")));
        let work = store_id(&web_profile_dir(root, "web-work"));
        let personal = store_id(&web_profile_dir(root, "web-personal"));
        assert!(default != work && work != personal && default != personal);
        assert_ne!(store_id(&web_profile_dir(Path::new("/tmp/bench/pw"), DEFAULT)), default, "another data folder, another store");
        assert_ne!(store_id(&crate::preview::data_dir(root, "/work/alpha")), default, "a Preview's isn't a profile's");
        for id in [default, work, personal] {
            assert_eq!(id[6] >> 4, 4, "a version-4 UUID");
            assert_ne!(id, [0; 16]);
        }
    }

    #[test]
    fn where_each_kind_of_view_may_go() {
        assert!(stays(Policy::Web, "https://example.com/"));
        assert!(stays(Policy::Web, "http://localhost:3000"));
        assert!(!stays(Policy::Web, "file:///etc/passwd"));
        assert!(!stays(Policy::Web, "msteams:/l/chat"));
        assert!(!to_browser(Policy::Web, "msteams:/l/chat"), "custom schemes aren't handed on");
        assert!(!to_browser(Policy::Web, "https://example.com/"));
        assert!(stays(Policy::Local, "http://localhost:5173/"));
        assert!(!stays(Policy::Local, "https://example.com/"));
        assert!(to_browser(Policy::Local, "https://example.com/"));
        assert!(!to_browser(Policy::Local, "file:///tmp/x"));
    }

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
}
