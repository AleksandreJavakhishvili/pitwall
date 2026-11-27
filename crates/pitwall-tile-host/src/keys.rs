//! Key bindings as a tile writes them (`j`, `cmd-s`, `shift-tab`, `/`;
//! `cmd` is ⌘ on macOS and Ctrl elsewhere) and the keys Pitwall keeps for
//! itself (ui.md §3.5). The host drops a reserved binding from a frame's
//! keymap with a line in the tile's Log; the app draws the rest.

/// A binding's modifiers, whatever order they were written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    /// None of ⌘/Ctrl/Alt: a key that types (Shift alone still types).
    pub fn types(&self) -> bool {
        !(self.cmd || self.ctrl || self.alt)
    }
}

/// `keys` split into its modifiers and its key, lowercase; `None` for an
/// empty binding or an unknown modifier. The key is after the last `-`
/// that has something before it, so `-` and `cmd--` name the minus key.
pub fn parse(keys: &str) -> Option<(Mods, String)> {
    let k = keys.trim().to_ascii_lowercase();
    if k.is_empty() {
        return None;
    }
    let (mods, key) = match k.rfind('-') {
        Some(i) if i > 0 && i + 1 < k.len() => (&k[..i], &k[i + 1..]),
        Some(i) if i > 0 && i + 1 == k.len() && k[..i].ends_with('-') => (&k[..i - 1], "-"),
        _ => ("", k.as_str()),
    };
    let mut m = Mods::default();
    for part in mods.split('-').filter(|p| !p.is_empty()) {
        match part {
            "cmd" | "super" | "win" => m.cmd = true,
            "ctrl" | "control" => m.ctrl = true,
            "alt" | "opt" | "option" => m.alt = true,
            "shift" => m.shift = true,
            _ => return None,
        }
    }
    let key = match key {
        "esc" => "escape",
        "return" => "enter",
        // GPUI on Linux names the plus and minus keys this way.
        "add" | "plus" => "+",
        "subtract" | "minus" => "-",
        other => other,
    };
    Some((m, key.to_string()))
}

/// The keys `cmd-` (and `cmd-shift-`) take in Pitwall: its palette, new
/// agent and terminal, sidebar, wall, review, inbox, web page, font size,
/// agents 1–9, maximise, and the menu's edit and window keys.
const CMD_KEYS: &[&str] = &["enter", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "=", "+", "-", ",", "a", "b", "c", "e", "h", "i", "j", "k", "l", "m", "n", "p", "q", "r", "t", "v", "w", "x", "z"];

/// Every reserved key, as the docs list it (`cmd-` keys alone and with
/// `shift`, then the others).
pub fn reserved_list() -> Vec<String> {
    let mut out: Vec<String> = ["escape", "tab", "shift-tab", "ctrl-enter"].iter().map(|s| s.to_string()).collect();
    for k in CMD_KEYS {
        out.push(format!("cmd-{k}"));
        out.push(format!("cmd-shift-{k}"));
    }
    out.extend(["cmd-shift-f", "cmd-alt-h", "cmd-ctrl-f", "cmd-shift-alt-e", "cmd-shift-alt-n", "cmd-shift-alt-r", "cmd-shift-alt-t"].iter().map(|s| s.to_string()));
    out
}

/// Whether binding `keys` is one of Pitwall's own (never the tile's).
pub fn reserved(keys: &str) -> bool {
    let Some((m, key)) = parse(keys) else { return false };
    let Mods { cmd, ctrl, alt, shift } = m;
    let k = key.as_str();
    match (cmd, ctrl, alt, shift) {
        (false, false, false, false) => matches!(k, "escape" | "tab"),
        (false, false, false, true) => k == "tab",
        (false, true, false, _) => k == "enter",
        (true, false, false, false) => CMD_KEYS.contains(&k),
        (true, false, false, true) => CMD_KEYS.contains(&k) || k == "f",
        (true, false, true, false) => k == "h",
        (true, true, false, false) => k == "f",
        (true, false, true, true) => matches!(k, "e" | "n" | "r" | "t"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_parse_in_any_order_and_pitwalls_keys_are_reserved() {
        assert_eq!(parse("shift-cmd-N"), Some((Mods { cmd: true, shift: true, ..Mods::default() }, "n".into())));
        assert_eq!(parse("cmd--").map(|p| p.1), Some("-".into()));
        assert_eq!(parse("-").map(|p| p.1), Some("-".into()));
        assert_eq!(parse("hyper-x"), None);
        for r in ["cmd-n", "Cmd-Enter", "escape", "esc", "tab", "shift-tab", "cmd-3", "cmd-shift-k", "shift-cmd-k", "alt-cmd-h", "cmd-ctrl-f", "ctrl-enter", "cmd-shift-alt-t"] {
            assert!(reserved(r), "{r}");
        }
        for free in ["n", "/", "j", "cmd-s", "cmd-f", "cmd-alt-n", "ctrl-n", "alt-n", "enter", "shift-enter"] {
            assert!(!reserved(free), "{free}");
        }
        let list = reserved_list();
        assert!(list.iter().all(|k| reserved(k)), "{list:?}");
    }
}
