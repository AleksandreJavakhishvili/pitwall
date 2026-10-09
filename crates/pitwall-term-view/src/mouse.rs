//! Mouse reports (xterm mouse tracking: modes 1000 / 1002 / 1003, with the
//! default, UTF-8 (1005) or SGR (1006) encoding).

/// What happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press(Button),
    Release(Button),
    /// Movement; `Some` while a button is held.
    Motion(Option<Button>),
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// Which reports the program asked for and how to encode them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseModes {
    /// 1000: presses and releases (and the wheel).
    pub click: bool,
    /// 1002: also motion while a button is held.
    pub drag: bool,
    /// 1003: all motion.
    pub motion: bool,
    /// 1006: `ESC [ < b ; x ; y M/m`.
    pub sgr: bool,
    /// 1005: coordinates as UTF-8.
    pub utf8: bool,
}

impl MouseModes {
    pub fn any(self) -> bool {
        self.click || self.drag || self.motion
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseMods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// The report for `action` at cell (`col`, `row`) (0-based), or `None` when
/// the current modes don't report it.
pub fn encode_mouse(
    action: MouseAction,
    col: usize,
    row: usize,
    mods: MouseMods,
    modes: MouseModes,
) -> Option<Vec<u8>> {
    if !modes.any() {
        return None;
    }
    let (mut code, release) = match action {
        MouseAction::Press(b) => (button_code(b), false),
        MouseAction::Release(b) => (if modes.sgr { button_code(b) } else { 3 }, true),
        MouseAction::Motion(held) => {
            match held {
                Some(_) if !(modes.drag || modes.motion) => return None,
                None if !modes.motion => return None,
                _ => {}
            }
            (32 + held.map(button_code).unwrap_or(3), false)
        }
        MouseAction::WheelUp => (64, false),
        MouseAction::WheelDown => (65, false),
        MouseAction::WheelLeft => (66, false),
        MouseAction::WheelRight => (67, false),
    };
    if mods.shift {
        code += 4;
    }
    if mods.alt {
        code += 8;
    }
    if mods.ctrl {
        code += 16;
    }
    let (x, y) = (col + 1, row + 1);
    if modes.sgr {
        let end = if release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }
    let mut out = vec![0x1b, b'[', b'M', 32 + code];
    if modes.utf8 {
        for v in [x, y] {
            let c = char::from_u32((32 + v).min(2047) as u32)?;
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    } else {
        // The default encoding can't go past column / row 223.
        if x > 223 || y > 223 {
            return None;
        }
        out.push(32 + x as u8);
        out.push(32 + y as u8);
    }
    Some(out)
}

fn button_code(b: Button) -> u8 {
    match b {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLICK: MouseModes = MouseModes { click: true, drag: false, motion: false, sgr: false, utf8: false };

    #[test]
    fn off_by_default() {
        assert_eq!(
            encode_mouse(MouseAction::Press(Button::Left), 0, 0, MouseMods::default(), MouseModes::default()),
            None
        );
    }

    #[test]
    fn default_encoding() {
        let m = MouseMods::default();
        assert_eq!(encode_mouse(MouseAction::Press(Button::Left), 0, 0, m, CLICK).unwrap(), b"\x1b[M !!");
        assert_eq!(encode_mouse(MouseAction::Release(Button::Left), 2, 3, m, CLICK).unwrap(), b"\x1b[M##$");
        assert_eq!(encode_mouse(MouseAction::WheelUp, 0, 0, m, CLICK).unwrap(), b"\x1b[M`!!");
        assert_eq!(encode_mouse(MouseAction::Press(Button::Left), 300, 0, m, CLICK), None);
        // Motion needs 1002 / 1003.
        assert_eq!(encode_mouse(MouseAction::Motion(Some(Button::Left)), 0, 0, m, CLICK), None);
        let drag = MouseModes { drag: true, ..CLICK };
        assert_eq!(encode_mouse(MouseAction::Motion(Some(Button::Left)), 0, 0, m, drag).unwrap(), b"\x1b[M@!!");
        assert_eq!(encode_mouse(MouseAction::Motion(None), 0, 0, m, drag), None);
        let any = MouseModes { motion: true, ..CLICK };
        assert_eq!(encode_mouse(MouseAction::Motion(None), 0, 0, m, any).unwrap(), b"\x1b[MC!!");
    }

    #[test]
    fn sgr_encoding_and_modifiers() {
        let sgr = MouseModes { sgr: true, ..CLICK };
        let m = MouseMods::default();
        assert_eq!(encode_mouse(MouseAction::Press(Button::Right), 9, 4, m, sgr).unwrap(), b"\x1b[<2;10;5M");
        assert_eq!(encode_mouse(MouseAction::Release(Button::Right), 9, 4, m, sgr).unwrap(), b"\x1b[<2;10;5m");
        assert_eq!(encode_mouse(MouseAction::WheelDown, 300, 0, m, sgr).unwrap(), b"\x1b[<65;301;1M");
        let ctrl_shift = MouseMods { shift: true, ctrl: true, alt: false };
        assert_eq!(encode_mouse(MouseAction::Press(Button::Left), 0, 0, ctrl_shift, sgr).unwrap(), b"\x1b[<20;1;1M");
    }

    #[test]
    fn utf8_coordinates() {
        let utf8 = MouseModes { utf8: true, ..CLICK };
        let out = encode_mouse(MouseAction::Press(Button::Left), 299, 0, MouseMods::default(), utf8).unwrap();
        assert_eq!(out, "\x1b[M \u{14c}!".as_bytes());
    }
}
