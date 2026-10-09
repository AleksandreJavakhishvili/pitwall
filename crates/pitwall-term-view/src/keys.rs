//! Keys → bytes, the way xterm encodes them.
//!
//! Only keys that need an escape sequence or a control byte are encoded here.
//! Plain text (letters, digits, symbols, IME commits, Option-composed
//! characters when Option is not Meta) is left to the text input path, so
//! input methods and dead keys keep working.
//!
//! References: xterm's ctlseqs ("PC-Style Function Keys", "Alt and Meta
//! Keys"), and what xterm.js sends for the same keys.

/// Modifier keys held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    /// ⌘ on macOS, the Windows / Super key elsewhere. Never encoded: such keys
    /// belong to the application (shortcuts).
    pub cmd: bool,
}

impl Mods {
    /// xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }
    fn any(self) -> bool {
        self.shift || self.alt || self.ctrl
    }
}

/// Terminal modes and settings that change what a key sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModes {
    /// DECCKM: arrows/Home/End send SS3 (`ESC O A`) instead of CSI.
    pub app_cursor: bool,
    /// Treat Option/Alt as Meta: send ESC + the key instead of the
    /// character the OS composes.
    pub option_as_meta: bool,
}

/// One key press: `key` is the key's name or the character printed on it
/// (GPUI's `Keystroke::key`, e.g. "a", "enter", "up", "f5"); `text` is what
/// it would type (`Keystroke::key_char`).
#[derive(Clone, Copy, Debug)]
pub struct KeyPress<'a> {
    pub key: &'a str,
    pub text: Option<&'a str>,
    pub mods: Mods,
}

const ESC: u8 = 0x1b;

/// The bytes to send for `k`, or `None` when it is not a terminal key (an
/// app shortcut) or is plain text for the text input path.
pub fn encode_key(k: KeyPress, modes: KeyModes) -> Option<Vec<u8>> {
    let m = k.mods;
    if m.cmd {
        return None;
    }
    if let Some(seq) = named_key(k.key, m, modes) {
        return Some(seq);
    }
    // A single character key from here on.
    let mut chars = k.key.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None; // an unknown named key
    }
    if m.ctrl {
        if let Some(c) = ctrl_byte(ch, m.shift) {
            let mut out = Vec::with_capacity(2);
            if m.alt {
                out.push(ESC);
            }
            out.push(c);
            return Some(out);
        }
        // Ctrl with a key that has no control code: send it as text.
        return None;
    }
    if m.alt && modes.option_as_meta {
        let base = if m.shift { shifted(ch) } else { ch };
        let mut out = vec![ESC];
        let mut buf = [0u8; 4];
        out.extend_from_slice(base.encode_utf8(&mut buf).as_bytes());
        return Some(out);
    }
    None
}

fn csi(body: &str) -> Vec<u8> {
    let mut v = vec![ESC, b'['];
    v.extend_from_slice(body.as_bytes());
    v
}

fn ss3(c: u8) -> Vec<u8> {
    vec![ESC, b'O', c]
}

/// Cursor-style keys: `ESC [ X` / `ESC O X`, with modifiers `ESC [ 1 ; m X`.
fn cursor_key(final_byte: u8, m: Mods, modes: KeyModes) -> Vec<u8> {
    if m.any() {
        csi(&format!("1;{}{}", m.param(), final_byte as char))
    } else if modes.app_cursor {
        ss3(final_byte)
    } else {
        vec![ESC, b'[', final_byte]
    }
}

/// `ESC [ n ~`, with modifiers `ESC [ n ; m ~`.
fn tilde_key(n: u8, m: Mods) -> Vec<u8> {
    if m.any() {
        csi(&format!("{n};{}~", m.param()))
    } else {
        csi(&format!("{n}~"))
    }
}

fn named_key(key: &str, m: Mods, modes: KeyModes) -> Option<Vec<u8>> {
    let meta = |bytes: &[u8]| -> Vec<u8> {
        let mut v = Vec::with_capacity(bytes.len() + 1);
        if m.alt {
            v.push(ESC);
        }
        v.extend_from_slice(bytes);
        v
    };
    Some(match key {
        "enter" => meta(b"\r"),
        "tab" if m.shift => csi("Z"),
        "tab" => meta(b"\t"),
        "backspace" if m.ctrl => meta(&[0x08]),
        "backspace" => meta(&[0x7f]),
        "escape" => meta(&[ESC]),
        "space" if m.ctrl => meta(&[0x00]),
        "space" if m.alt && modes.option_as_meta => vec![ESC, b' '],
        "space" => return None,
        "up" => cursor_key(b'A', m, modes),
        "down" => cursor_key(b'B', m, modes),
        "right" => cursor_key(b'C', m, modes),
        "left" => cursor_key(b'D', m, modes),
        "home" => cursor_key(b'H', m, modes),
        "end" => cursor_key(b'F', m, modes),
        "insert" => tilde_key(2, m),
        "delete" => tilde_key(3, m),
        "pageup" => tilde_key(5, m),
        "pagedown" => tilde_key(6, m),
        "f1" | "f2" | "f3" | "f4" => {
            let c = b"PQRS"[(key.as_bytes()[1] - b'1') as usize];
            if m.any() {
                csi(&format!("1;{}{}", m.param(), c as char))
            } else {
                ss3(c)
            }
        }
        _ => {
            let n: u8 = key.strip_prefix('f')?.parse().ok()?;
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                13 => 25,
                14 => 26,
                15 => 28,
                16 => 29,
                17 => 31,
                18 => 32,
                19 => 33,
                20 => 34,
                _ => return None,
            };
            tilde_key(code, m)
        }
    })
}

/// The C0 control byte for Ctrl + `ch` (xterm's table, as xterm.js sends it).
fn ctrl_byte(ch: char, shift: bool) -> Option<u8> {
    let c = ch.to_ascii_lowercase();
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        '@' | '2' | ' ' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '7' | '/' => 0x1f,
        '-' if shift => 0x1f,
        '8' | '?' => 0x7f,
        _ => return None,
    })
}

/// The character Shift gives on a US layout (for Meta + Shift + key, where
/// the OS only reports the unshifted key).
fn shifted(ch: char) -> char {
    match ch {
        'a'..='z' => ch.to_ascii_uppercase(),
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '`' => '~',
        other => other,
    }
}

/// Text to send for a paste: line endings become CR (what a terminal's Enter
/// sends), and with bracketed paste the text is wrapped in `ESC[200~ … ESC[201~`
/// (with any end marker inside removed, so a paste can't end itself early).
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if !bracketed {
        return normalized.into_bytes();
    }
    let body = normalized.replace("\x1b[201~", "");
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: &str, mods: Mods) -> Option<Vec<u8>> {
        encode_key(KeyPress { key, text: None, mods }, KeyModes::default())
    }
    const NONE: Mods = Mods { shift: false, alt: false, ctrl: false, cmd: false };
    const SHIFT: Mods = Mods { shift: true, ..NONE };
    const ALT: Mods = Mods { alt: true, ..NONE };
    const CTRL: Mods = Mods { ctrl: true, ..NONE };

    #[test]
    fn plain_text_goes_to_the_input_path() {
        assert_eq!(key("a", NONE), None);
        assert_eq!(key("a", SHIFT), None);
        assert_eq!(key("space", NONE), None);
        // Option without Meta: the composed character is text.
        assert_eq!(key("s", ALT), None);
        // ⌘ shortcuts are the app's.
        assert_eq!(key("c", Mods { cmd: true, ..NONE }), None);
        assert_eq!(key("up", Mods { cmd: true, ..NONE }), None);
    }

    #[test]
    fn editing_keys() {
        assert_eq!(key("enter", NONE).unwrap(), b"\r");
        assert_eq!(key("enter", SHIFT).unwrap(), b"\r");
        assert_eq!(key("tab", NONE).unwrap(), b"\t");
        assert_eq!(key("tab", SHIFT).unwrap(), b"\x1b[Z");
        assert_eq!(key("backspace", NONE).unwrap(), b"\x7f");
        assert_eq!(key("backspace", CTRL).unwrap(), b"\x08");
        assert_eq!(key("backspace", ALT).unwrap(), b"\x1b\x7f");
        assert_eq!(key("escape", NONE).unwrap(), b"\x1b");
        assert_eq!(key("space", CTRL).unwrap(), b"\x00");
    }

    #[test]
    fn cursor_keys_follow_decckm_and_modifiers() {
        assert_eq!(key("up", NONE).unwrap(), b"\x1b[A");
        assert_eq!(key("left", NONE).unwrap(), b"\x1b[D");
        let app = KeyModes { app_cursor: true, ..Default::default() };
        let k = |name, mods| encode_key(KeyPress { key: name, text: None, mods }, app).unwrap();
        assert_eq!(k("up", NONE), b"\x1bOA");
        assert_eq!(k("home", NONE), b"\x1bOH");
        // Modifiers always use the CSI form.
        assert_eq!(k("up", SHIFT), b"\x1b[1;2A");
        assert_eq!(key("right", ALT).unwrap(), b"\x1b[1;3C");
        assert_eq!(key("right", CTRL).unwrap(), b"\x1b[1;5C");
        assert_eq!(key("down", Mods { ctrl: true, shift: true, ..NONE }).unwrap(), b"\x1b[1;6B");
        assert_eq!(key("end", NONE).unwrap(), b"\x1b[F");
    }

    #[test]
    fn function_and_tilde_keys() {
        assert_eq!(key("f1", NONE).unwrap(), b"\x1bOP");
        assert_eq!(key("f4", NONE).unwrap(), b"\x1bOS");
        assert_eq!(key("f1", SHIFT).unwrap(), b"\x1b[1;2P");
        assert_eq!(key("f5", NONE).unwrap(), b"\x1b[15~");
        assert_eq!(key("f12", NONE).unwrap(), b"\x1b[24~");
        assert_eq!(key("f12", CTRL).unwrap(), b"\x1b[24;5~");
        assert_eq!(key("f20", NONE).unwrap(), b"\x1b[34~");
        assert_eq!(key("f21", NONE), None);
        assert_eq!(key("pageup", NONE).unwrap(), b"\x1b[5~");
        assert_eq!(key("pagedown", SHIFT).unwrap(), b"\x1b[6;2~");
        assert_eq!(key("delete", NONE).unwrap(), b"\x1b[3~");
        assert_eq!(key("insert", NONE).unwrap(), b"\x1b[2~");
    }

    #[test]
    fn control_bytes() {
        assert_eq!(key("c", CTRL).unwrap(), [0x03]);
        assert_eq!(key("a", CTRL).unwrap(), [0x01]);
        assert_eq!(key("z", CTRL).unwrap(), [0x1a]);
        assert_eq!(key("[", CTRL).unwrap(), [0x1b]);
        assert_eq!(key("\\", CTRL).unwrap(), [0x1c]);
        assert_eq!(key("]", CTRL).unwrap(), [0x1d]);
        assert_eq!(key("6", CTRL).unwrap(), [0x1e]);
        assert_eq!(key("/", CTRL).unwrap(), [0x1f]);
        assert_eq!(key("2", CTRL).unwrap(), [0x00]);
        assert_eq!(key("8", CTRL).unwrap(), [0x7f]);
        // Ctrl+Alt: ESC prefix.
        assert_eq!(key("x", Mods { ctrl: true, alt: true, ..NONE }).unwrap(), [0x1b, 0x18]);
        // No control code: typed as text.
        assert_eq!(key("1", CTRL), None);
    }

    #[test]
    fn option_as_meta() {
        let meta = KeyModes { option_as_meta: true, ..Default::default() };
        let k = |name, mods| encode_key(KeyPress { key: name, text: Some("ß"), mods }, meta);
        assert_eq!(k("s", ALT).unwrap(), b"\x1bs");
        assert_eq!(k("b", ALT).unwrap(), b"\x1bb");
        assert_eq!(k("f", Mods { alt: true, shift: true, ..NONE }).unwrap(), b"\x1bF");
        assert_eq!(k("1", Mods { alt: true, shift: true, ..NONE }).unwrap(), b"\x1b!");
        assert_eq!(k("enter", ALT).unwrap(), b"\x1b\r");
        assert_eq!(k("space", ALT).unwrap(), b"\x1b ");
        assert_eq!(k("left", ALT).unwrap(), b"\x1b[1;3D");
    }

    #[test]
    fn paste_wrapping() {
        assert_eq!(paste_bytes("a\nb", false), b"a\rb");
        assert_eq!(paste_bytes("a\r\nb", false), b"a\rb");
        assert_eq!(paste_bytes("hi", true), b"\x1b[200~hi\x1b[201~");
        assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~xy\x1b[201~");
    }
}
