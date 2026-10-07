//! XDG base directories (freedesktop.org), as Linux desktops lay out a
//! user's files: `$XDG_DATA_HOME` (default `~/.local/share`) and
//! `$XDG_CONFIG_HOME` (default `~/.config`). Pure, so it is tested on any Unix.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// `value` when it is an absolute path (the spec says relative values are
/// invalid and must be ignored), else `home/default`.
pub fn base_dir(value: Option<OsString>, home: &Path, default: &str) -> PathBuf {
    match value.map(PathBuf::from) {
        Some(p) if p.is_absolute() => p,
        _ => home.join(default),
    }
}

pub fn data_home(home: &Path) -> PathBuf {
    base_dir(std::env::var_os("XDG_DATA_HOME"), home, ".local/share")
}

pub fn config_home(home: &Path) -> PathBuf {
    base_dir(std::env::var_os("XDG_CONFIG_HOME"), home, ".config")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_values_win_and_others_fall_back() {
        let home = Path::new("/home/dev");
        assert_eq!(base_dir(None, home, ".local/share"), PathBuf::from("/home/dev/.local/share"));
        assert_eq!(base_dir(Some("".into()), home, ".config"), PathBuf::from("/home/dev/.config"));
        assert_eq!(base_dir(Some("rel/data".into()), home, ".local/share"), PathBuf::from("/home/dev/.local/share"));
        assert_eq!(base_dir(Some("/data/dev".into()), home, ".local/share"), PathBuf::from("/data/dev"));
    }
}
