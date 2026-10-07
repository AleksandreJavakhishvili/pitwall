//! Where the app ships the `pitwall-hold` terminal holder. Release: next to
//! the app executable (`Contents/MacOS/`, bundled as a Tauri `externalBin`).
//! Dev: `tauri_build` copies it next to `target/<profile>/pitwall`; tests run
//! from `deps/`, one level down. `build.rs` also bakes in where it built it,
//! and `PITWALL_HOLD_BIN` overrides everything. Linux: `/usr/bin/` (.deb), or
//! copied out of an AppImage's mount (`platform::stable_sidecar`).

use std::path::{Path, PathBuf};

pub fn holder_bin() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("PITWALL_HOLD_BIN") {
        return Ok(PathBuf::from(p));
    }
    let name = format!("pitwall-hold{}", std::env::consts::EXE_SUFFIX);
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join(&name));
        if let Some(up) = dir.parent() {
            candidates.push(up.join(&name));
        }
    }
    if let Some(built) = option_env!("PITWALL_HOLD_BUILT") {
        candidates.push(PathBuf::from(built));
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .map(crate::platform::stable_sidecar)
        .ok_or_else(|| "the terminal holder (pitwall-hold) is missing next to the app".into())
}

/// The holder this app ships is the one the local provider runs (its
/// contract tests start their own build of it, in temp dirs).
#[cfg(test)]
mod tests {
    #[test]
    fn the_shipped_holder_is_found() {
        let bin = super::holder_bin().expect("pitwall-hold next to the app or built by build.rs");
        assert!(bin.is_file());
    }
}
