//! The Linux backend's plain decisions, apart from GTK: which key presses
//! are Pitwall's (named as GPUI's Linux platform names them), the GTK
//! window's rectangle and the page zoom, cairo's pixels as RGBA, and how
//! often GLib's loop is pumped. No dependencies, so it is tested anywhere.

use std::time::Duration;

/// GDK's modifier bits (`GdkModifierType`).
pub const SHIFT: u32 = 1 << 0;
pub const CONTROL: u32 = 1 << 2;
/// Mod1: Alt on every common keymap.
pub const ALT: u32 = 1 << 3;
pub const SUPER: u32 = 1 << 26;

/// GPUI's modifiers, without depending on gpui.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub platform: bool,
}

/// A key press in the page that is Pitwall's: Ctrl with any key but the
/// editing ones (copy, paste, cut, select all, undo, redo), named as GPUI
/// names it. `None`: the page's (no Ctrl, an editing key, a lone modifier,
/// a key GPUI has no name for).
pub fn shortcut(keyval: u32, state: u32) -> Option<(String, Mods)> {
    if state & CONTROL == 0 {
        return None;
    }
    let key = key_name(keyval)?;
    let mut mods = Mods {
        control: true,
        alt: state & ALT != 0,
        shift: state & SHIFT != 0,
        platform: state & SUPER != 0,
    };
    // GPUI's convention: shift only with letters and named keys (tab,
    // enter…), not with the symbols it already chose ("!" not shift-1).
    if mods.shift && key.chars().count() == 1 && key.to_lowercase() == key.to_uppercase() {
        mods.shift = false;
    }
    if is_editing(&key, mods) {
        return None;
    }
    Some((key, mods))
}

/// Ctrl+C/V/X/A/Z/Y and Ctrl+Shift+Z: the page's own.
fn is_editing(key: &str, mods: Mods) -> bool {
    if mods.alt || mods.platform {
        return false;
    }
    matches!((key, mods.shift), ("c" | "v" | "x" | "a" | "z" | "y", false) | ("z", true))
}

/// An X keysym (GDK's keyval) as GPUI's Linux platform names the key:
/// "enter", "pageup", "f5", lowercase letters, the symbol itself.
pub fn key_name(keyval: u32) -> Option<String> {
    let named = match keyval {
        0xff0d | 0xff8d => "enter",
        0xff09 | 0xfe20 => "tab",
        0xff08 => "backspace",
        0xffff => "delete",
        0xff1b => "escape",
        0xff51 => "left",
        0xff52 => "up",
        0xff53 => "right",
        0xff54 => "down",
        0xff50 => "home",
        0xff57 => "end",
        0xff55 => "pageup",
        0xff56 => "pagedown",
        0xff63 => "insert",
        0x20 => "space",
        // F1…F24
        0xffbe..=0xffd5 => return Some(format!("f{}", keyval - 0xffbe + 1)),
        // The keypad's digits (num lock on): GPUI drops the "kp_".
        0xffb0..=0xffb9 => return Some(char::from((keyval - 0xffb0) as u8 + b'0').to_string()),
        // Latin-1 keysyms are the characters themselves.
        0x21..=0x7e | 0xa1..=0xff => return char::from_u32(keyval).map(lower),
        // Unicode keysyms.
        0x0100_0000..=0x0110_ffff => {
            return char::from_u32(keyval - 0x0100_0000).filter(|c| !c.is_control()).map(lower)
        }
        _ => return None,
    };
    Some(named.to_owned())
}

fn lower(c: char) -> String {
    c.to_lowercase().collect()
}

/// The GTK window's place in its parent, in GDK units, for `x, y, w, h` in
/// GPUI's logical pixels: GPUI's scale gives device pixels, GDK's integer
/// scale (`GDK_SCALE`) its own units. Edges are rounded (so a view meets
/// its neighbours) and it is never empty (GTK won't size a window to 0).
pub fn gtk_rect(x: f32, y: f32, w: f32, h: f32, gpui_scale: f32, gdk_scale: i32) -> (i32, i32, i32, i32) {
    let k = gpui_scale / gdk_scale.max(1) as f32;
    let (left, top) = ((x * k).round(), (y * k).round());
    let (right, bottom) = (((x + w) * k).round(), ((y + h) * k).round());
    (left as i32, top as i32, ((right - left) as i32).max(1), ((bottom - top) as i32).max(1))
}

/// The page zoom that makes a CSS pixel as big as GPUI's logical pixel:
/// WebKitGTK only knows GDK's integer scale, GPUI's can be 1.5 (Xft.dpi).
pub fn page_zoom(gpui_scale: f32, gdk_scale: i32) -> f64 {
    let zoom = gpui_scale as f64 / gdk_scale.max(1) as f64;
    if zoom.is_finite() && zoom > 0.0 {
        zoom
    } else {
        1.0
    }
}

/// cairo's image (ARGB32: native-endian, so B G R A bytes on every Linux
/// target; premultiplied; `stride` bytes a row) as straight RGBA8, row-major
/// and tightly packed. `opaque`: RGB24, whose alpha byte is undefined.
pub fn cairo_to_rgba(data: &[u8], width: usize, height: usize, stride: usize, opaque: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let start = row * stride;
        let Some(line) = data.get(start..start + width * 4) else {
            break;
        };
        for px in line.chunks_exact(4) {
            let (b, g, r) = (px[0], px[1], px[2]);
            let a = if opaque { 255 } else { px[3] };
            let straight = |c: u8| match a {
                0 => 0,
                255 => c,
                a => ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
            };
            out.extend_from_slice(&[straight(r), straight(g), straight(b), a]);
        }
    }
    out
}

/// How long the GPUI side waits between turns of GLib's loop: a frame
/// while a view shows (input, drawing, loads), slowly while every view is
/// hidden (the web process's messages still arrive), not at all without
/// views (the pump stops; no cost when no preview is open).
pub fn pump_interval(views: usize, visible: usize) -> Option<Duration> {
    match (views, visible) {
        (0, _) => None,
        (_, 0) => Some(Duration::from_millis(250)),
        _ => Some(Duration::from_millis(16)),
    }
}

/// At most this many GLib dispatches per turn, so a busy page can't hold
/// GPUI's thread.
pub const MAX_DISPATCHES: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    fn key(keyval: u32, state: u32) -> Option<(String, Mods)> {
        shortcut(keyval, state)
    }

    #[test]
    fn ctrl_keys_are_pitwalls_but_editing_keys_stay_with_the_page() {
        let ctrl = Mods { control: true, ..Mods::default() };
        assert_eq!(key('k' as u32, CONTROL), Some(("k".into(), ctrl)));
        assert_eq!(key('1' as u32, CONTROL), Some(("1".into(), ctrl)));
        assert_eq!(key(0xff09, CONTROL), Some(("tab".into(), ctrl)));
        assert_eq!(key(0xff09, CONTROL | SHIFT), Some(("tab".into(), Mods { shift: true, ..ctrl })));
        for c in ['c', 'v', 'x', 'a', 'z', 'y'] {
            assert_eq!(key(c as u32, CONTROL), None, "{c}");
        }
        // Caps lock: an upper-case keyval without shift is still Ctrl+C.
        assert_eq!(key('C' as u32, CONTROL), None);
        // Redo is Ctrl+Shift+Z; Ctrl+Shift+C is Pitwall's.
        assert_eq!(key('Z' as u32, CONTROL | SHIFT), None);
        assert_eq!(key('C' as u32, CONTROL | SHIFT), Some(("c".into(), Mods { shift: true, ..ctrl })));
        // With Alt, even C is a shortcut.
        assert_eq!(key('c' as u32, CONTROL | ALT), Some(("c".into(), Mods { alt: true, ..ctrl })));
    }

    #[test]
    fn plain_keys_and_lone_modifiers_stay_with_the_page() {
        assert_eq!(key('k' as u32, 0), None);
        assert_eq!(key('k' as u32, SHIFT | ALT), None);
        // Control_L, Shift_L, ISO_Level3_Shift.
        for m in [0xffe3, 0xffe1, 0xfe03] {
            assert_eq!(key(m, CONTROL), None);
        }
    }

    #[test]
    fn keys_are_named_as_gpui_names_them() {
        assert_eq!(key_name(0xff0d).as_deref(), Some("enter"));
        assert_eq!(key_name(0xff55).as_deref(), Some("pageup"));
        assert_eq!(key_name(0xffbe).as_deref(), Some("f1"));
        assert_eq!(key_name(0xffc9).as_deref(), Some("f12"));
        assert_eq!(key_name(0xffb7).as_deref(), Some("7"));
        assert_eq!(key_name(',' as u32).as_deref(), Some(","));
        assert_eq!(key_name('`' as u32).as_deref(), Some("`"));
        assert_eq!(key_name(' ' as u32).as_deref(), Some("space"));
        assert_eq!(key_name('P' as u32).as_deref(), Some("p"));
        // Unicode keysym: Cyrillic Ж.
        assert_eq!(key_name(0x0100_0416).as_deref(), Some("ж"));
        // Shift with a symbol: GPUI says "!", not shift-!.
        let (k, m) = shortcut('!' as u32, CONTROL | SHIFT).unwrap();
        assert_eq!((k.as_str(), m.shift), ("!", false));
    }

    #[test]
    fn rect_scales_to_gdk_units() {
        assert_eq!(gtk_rect(10., 20., 300., 200., 1., 1), (10, 20, 300, 200));
        assert_eq!(gtk_rect(10., 20., 300., 200., 2., 1), (20, 40, 600, 400));
        assert_eq!(gtk_rect(10., 20., 300., 200., 2., 2), (10, 20, 300, 200));
        // 1.5: edges rounded, so the far edge lands where GPUI draws it.
        assert_eq!(gtk_rect(10.3, 0., 100.3, 0., 1.5, 1), (15, 0, 151, 1));
        assert_eq!(gtk_rect(0., 0., 0., 0., 1., 0), (0, 0, 1, 1));
    }

    #[test]
    fn zoom_matches_gpuis_scale() {
        assert_eq!(page_zoom(1.5, 1), 1.5);
        assert_eq!(page_zoom(2., 2), 1.);
        assert_eq!(page_zoom(1., 2), 0.5);
        assert_eq!(page_zoom(f32::NAN, 1), 1.);
    }

    #[test]
    fn cairo_pixels_become_straight_rgba() {
        // Two pixels a row, one row of padding in the stride:
        // opaque blue (B G R A) and half-transparent premultiplied red.
        let data = [255, 0, 0, 255, 0, 0, 64, 128, 9, 9, 9, 9];
        let rgba = cairo_to_rgba(&data, 2, 1, 12, false);
        assert_eq!(rgba, vec![0, 0, 255, 255, 128, 0, 0, 128]);
        // Transparent stays zero; RGB24's alpha byte is ignored.
        assert_eq!(cairo_to_rgba(&[7, 7, 7, 0], 1, 1, 4, false), vec![0, 0, 0, 0]);
        assert_eq!(cairo_to_rgba(&[1, 2, 3, 0], 1, 1, 4, true), vec![3, 2, 1, 255]);
        // A short buffer stops instead of panicking.
        assert_eq!(cairo_to_rgba(&[1, 2, 3, 4], 1, 2, 4, false).len(), 4);
    }

    #[test]
    fn the_pump_runs_only_while_views_exist() {
        assert_eq!(pump_interval(0, 0), None);
        assert_eq!(pump_interval(2, 0), Some(Duration::from_millis(250)));
        assert_eq!(pump_interval(2, 1), Some(Duration::from_millis(16)));
    }
}
