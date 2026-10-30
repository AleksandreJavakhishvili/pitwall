//! The Windows backend's pure parts, no Win32 calls: which keys are
//! Pitwall's and how GPUI names them, GPUI's logical pixels to the
//! controller's physical ones, and what the view was asked before WebView2
//! finished creating it. Virtual-key codes are plain numbers here (winuser.h)
//! so this can be tested anywhere.

use gpui::{Bounds, Keystroke, Modifiers, Pixels};

/// A rectangle in the window's client area, physical pixels (Win32's RECT).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// GPUI's logical bounds (origin top left of the client area, which is
/// GPUI's whole drawing surface, custom title bar included) to the
/// controller's: GPUI's Windows scale factor is the window's DPI / 96, and
/// the process is per-monitor DPI aware, so WebView2 takes raw pixels. Each
/// edge is rounded on its own so neighbouring views don't overlap or gap.
pub fn physical(bounds: Bounds<Pixels>, scale: f32) -> Rect {
    let edge = |v: Pixels| (f32::from(v) * scale).round() as i32;
    let right = edge(bounds.origin.x + bounds.size.width);
    let bottom = edge(bounds.origin.y + bounds.size.height);
    let left = edge(bounds.origin.x).min(right);
    let top = edge(bounds.origin.y).min(bottom);
    Rect {
        left,
        top,
        right,
        bottom,
    }
}

const VK_CONTROL: u16 = 0x11;
const VK_A: u16 = 0x41;
const VK_C: u16 = 0x43;
const VK_V: u16 = 0x56;
const VK_X: u16 = 0x58;
const VK_Y: u16 = 0x59;
const VK_Z: u16 = 0x5A;

/// Modifier and lock keys: never a shortcut on their own.
fn modifier_key(vk: u16) -> bool {
    matches!(
        vk,
        0x10..=0x12 // shift, control, alt
            | 0xA0..=0xA5 // left/right shift, control, alt
            | 0x5B | 0x5C // windows keys
            | 0x14 | 0x90 | 0x91 // caps, num, scroll lock
            | 0xE5 | 0xE7 // IME process key, packet
    )
}

/// Copy, paste, cut, select all, undo, redo: the page's
/// ([`crate::webview::editing_key`]).
fn editing(vk: u16, m: Modifiers) -> bool {
    let key = match vk {
        VK_A => "a",
        VK_C => "c",
        VK_V => "v",
        VK_X => "x",
        VK_Y => "y",
        VK_Z => "z",
        _ => return false,
    };
    !m.platform && crate::webview::editing_key(key, m.control, m.shift, m.alt)
}

/// A plain Esc: the page has it, then Pitwall takes the keys back.
pub fn plain_escape(vk: u16, m: Modifiers) -> bool {
    vk == 0x1B && !(m.control || m.alt || m.shift || m.platform)
}

/// Keys GPUI's Windows platform names whatever the layout
/// (`parse_immutable` in gpui's `platform/windows/events.rs`).
fn named(vk: u16) -> Option<String> {
    let name = match vk {
        0x20 => "space",
        0x08 => "backspace",
        0x0D => "enter",
        0x09 => "tab",
        0x26 => "up",
        0x28 => "down",
        0x27 => "right",
        0x25 => "left",
        0x24 => "home",
        0x23 => "end",
        0x21 => "pageup",
        0x22 => "pagedown",
        0xA6 => "back",
        0xA7 => "forward",
        0x1B => "escape",
        0x2D => "insert",
        0x2E => "delete",
        0x5D => "menu",
        0x70..=0x87 => return Some(format!("f{}", vk - 0x70 + 1)),
        _ => return None,
    };
    Some(name.into())
}

/// Keys GPUI names by their shifted character when shift is held, dropping
/// shift (`need_to_convert_to_shifted_key` in gpui's `keyboard.rs`): digits
/// and punctuation.
fn shifts(vk: u16) -> bool {
    matches!(vk, 0x30..=0x39 | 0xBA..=0xC1 | 0xDB..=0xDF | 0xE2)
}

/// A key pressed in the page that is Pitwall's: Ctrl with anything but the
/// editing keys, as the GPUI keystroke it would have been in the window.
/// `layout(shifted)` is the current layout's character for the key, shifted
/// or not (Win32: `ToUnicode` / `MapVirtualKeyW`).
pub fn shortcut(
    vk: u16,
    modifiers: Modifiers,
    layout: impl FnOnce(bool) -> Option<String>,
) -> Option<Keystroke> {
    if !modifiers.control || vk == VK_CONTROL || modifier_key(vk) || editing(vk, modifiers) {
        return None;
    }
    let mut modifiers = modifiers;
    let key = match named(vk) {
        Some(key) => key,
        None if modifiers.shift && shifts(vk) => {
            let key = layout(true)?;
            modifiers.shift = false;
            key
        }
        None => layout(false)?.to_ascii_lowercase(),
    };
    Some(Keystroke {
        modifiers,
        key,
        key_char: None,
    })
}

/// Only web pages go to the system browser: handing it any other scheme
/// (`file:`, `ms-settings:`…) would let a page launch things.
pub fn browsable(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Where the view sits and whether it shows, kept so each frame's
/// `set_bounds` only reaches WebView2 when something changed.
#[derive(Debug, Default)]
pub struct Frame {
    pub bounds: Option<Rect>,
    pub visible: bool,
}

impl Frame {
    /// Whether this moved it.
    pub fn set_bounds(&mut self, rect: Rect) -> bool {
        let changed = self.bounds != Some(rect);
        self.bounds = Some(rect);
        changed
    }

    /// Whether this showed or hid it.
    pub fn set_visible(&mut self, visible: bool) -> bool {
        let changed = self.visible != visible;
        self.visible = visible;
        changed
    }
}

/// What the view was asked while WebView2 was still creating it, applied
/// once it is ready: the last URL (the first one, or a later navigate),
/// whether it should take focus, whether the inspector was asked for.
/// Reload, back and forward before the first page have nothing to act on.
#[derive(Debug)]
pub struct Pending {
    pub url: String,
    pub focus: bool,
    pub inspector: bool,
}

impl Pending {
    pub fn new(url: String) -> Self {
        Self {
            url,
            focus: false,
            inspector: false,
        }
    }

    pub fn navigate(&mut self, url: &str) {
        self.url = url.into();
    }

    /// Reload: the first load is still coming; back and forward: no history
    /// yet. Nothing to remember.
    pub fn history(&mut self) {}

    pub fn focus(&mut self, focus: bool) {
        self.focus = focus;
    }

    pub fn open_inspector(&mut self) {
        self.inspector = true;
    }
}

#[cfg(test)]
mod tests {
    use gpui::{point, px, size};

    use super::*;

    fn ctrl() -> Modifiers {
        Modifiers {
            control: true,
            ..Default::default()
        }
    }

    fn ks(vk: u16, m: Modifiers) -> Option<String> {
        shortcut(vk, m, |shifted| {
            Some(match (vk, shifted) {
                (0x31, true) => "!".into(),
                (0xBB, false) => "=".into(),
                (0xBB, true) => "+".into(),
                (v, _) => char::from_u32(v as u32)?.to_string(),
            })
        })
        .map(|k| k.unparse())
    }

    #[test]
    fn ctrl_keys_are_pitwalls() {
        assert_eq!(ks(0x4B, ctrl()).as_deref(), Some("ctrl-k"));
        assert_eq!(ks(0x09, ctrl()).as_deref(), Some("ctrl-tab"));
        assert_eq!(ks(0x74, ctrl()).as_deref(), Some("ctrl-f5"));
        assert_eq!(ks(0x87, ctrl()).as_deref(), Some("ctrl-f24"));
        assert_eq!(ks(0xBB, ctrl()).as_deref(), Some("ctrl-="));
        let shift = Modifiers { shift: true, ..ctrl() };
        assert_eq!(ks(0x50, shift).as_deref(), Some("ctrl-shift-p"));
        // Shifted digits and punctuation are named by what they type.
        assert_eq!(ks(0x31, shift).as_deref(), Some("ctrl-!"));
        assert_eq!(ks(0xBB, shift).as_deref(), Some("ctrl-+"));
        let alt = Modifiers { alt: true, ..ctrl() };
        assert_eq!(ks(0x43, alt).as_deref(), Some("ctrl-alt-c"));
    }

    #[test]
    fn editing_and_plain_keys_are_the_pages() {
        for vk in [VK_C, VK_V, VK_X, VK_A, VK_Z, VK_Y] {
            assert_eq!(ks(vk, ctrl()), None, "{vk:#x}");
        }
        let shift = Modifiers { shift: true, ..ctrl() };
        assert_eq!(ks(VK_Z, shift), None);
        assert_eq!(ks(VK_V, shift), None, "paste as plain text");
        assert!(plain_escape(0x1B, Modifiers::default()));
        assert!(!plain_escape(0x1B, ctrl()));
        assert_eq!(ks(VK_C, shift).as_deref(), Some("ctrl-shift-c"));
        // No Ctrl, or only a modifier key: not a shortcut.
        assert_eq!(ks(0x4B, Modifiers::default()), None);
        assert_eq!(ks(0x4B, Modifiers { alt: true, ..Default::default() }), None);
        assert_eq!(ks(0x74, Modifiers::default()), None);
        for vk in [0x10, 0x11, 0x12, 0xA2, 0xA3, 0x5B, 0x14] {
            assert_eq!(ks(vk, ctrl()), None, "{vk:#x}");
        }
        // A key the layout can't name.
        assert_eq!(shortcut(0xFF, ctrl(), |_| None), None);
    }

    #[test]
    fn logical_to_physical() {
        let b = Bounds::new(point(px(10.), px(20.5)), size(px(300.), px(200.)));
        assert_eq!(
            physical(b, 1.),
            Rect { left: 10, top: 21, right: 310, bottom: 221 }
        );
        assert_eq!(
            physical(b, 1.5),
            Rect { left: 15, top: 31, right: 465, bottom: 331 }
        );
        // Side by side views share an edge at any scale.
        let a = Bounds::new(point(px(0.), px(0.)), size(px(100.3), px(10.)));
        let c = Bounds::new(point(px(100.3), px(0.)), size(px(50.), px(10.)));
        assert_eq!(physical(a, 1.25).right, physical(c, 1.25).left);
        // Never inside out.
        let empty = Bounds::new(point(px(5.), px(5.)), size(px(0.), px(0.)));
        let r = physical(empty, 2.);
        assert!(r.right >= r.left && r.bottom >= r.top);
    }

    #[test]
    fn only_web_pages_go_to_the_browser() {
        assert!(browsable("https://example.com"));
        assert!(browsable("HTTP://Example.com/x"));
        for no in ["file:///C:/x", "ms-settings:privacy", "javascript:1", "about:blank", "http"] {
            assert!(!browsable(no), "{no}");
        }
    }

    #[test]
    fn calls_before_ready_keep_the_last_word() {
        let mut p = Pending::new("http://localhost:3000".into());
        p.history();
        assert_eq!(p.url, "http://localhost:3000");
        p.navigate("http://localhost:5173/a");
        p.navigate("http://localhost:5173/b");
        assert_eq!(p.url, "http://localhost:5173/b");
        assert!(!p.focus && !p.inspector);
        p.focus(true);
        p.focus(false);
        assert!(!p.focus);
        p.focus(true);
        p.open_inspector();
        assert!(p.focus && p.inspector);
    }

    #[test]
    fn frame_reports_changes_only() {
        let mut f = Frame::default();
        let r = Rect { left: 1, top: 2, right: 3, bottom: 4 };
        assert!(f.set_bounds(r));
        assert!(!f.set_bounds(r));
        assert!(f.set_bounds(Rect { right: 5, ..r }));
        assert!(!f.set_visible(false));
        assert!(f.set_visible(true));
        assert!(!f.set_visible(true));
    }
}
