//! OS glue for the app shell (architecture.md §9 decision 7). What the UI
//! also sees — shortcut modifier, Dock or tray, menu bar, badge — is decided
//! by `pitwall_core::host::HostInfo` capabilities (`lib.rs`, `menu.rs`); this
//! module holds the OS calls behind them and what has no capability of its
//! own: the process environment and where shipped helper binaries run from.
//!
//! - `unix.rs` (macOS, Linux): the Dock / launcher badge; no tray.
//! - `windows.rs`: the tray icon (the way back to a hidden window; there is
//!   no Dock) and the blocked count as a taskbar overlay icon (`badge.rs`)
//!   and in the tray tooltip.

#[cfg_attr(not(windows), allow(dead_code))]
pub mod badge;

#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

use std::io;
use std::path::{Path, PathBuf};

/// Before Tauri starts (no threads yet).
pub fn prepare_env() {
    // PATH as the user's terminal has it, for everything Pitwall starts. An
    // app opened from the Dock, Finder or a desktop launcher gets a minimal
    // PATH, so a program found through the login shell (agw) would run
    // without Homebrew's dirs and fail to find its own tools (limactl).
    // Never blocks: last run's login PATH (or common dirs) at once, the login
    // shell asked in the background (Windows: the registry's PATH).
    let paths = pitwall_core::paths::Paths::new(pitwall_core::paths::Paths::default_root());
    let _probe = pitwall_core::shell::adopt_login_path(Some(paths.login_path_file()));
    // WebKitGTK's DMA-BUF renderer leaves the window blank on some GPU
    // drivers (NVIDIA, some Wayland compositors); the fallback renders
    // everywhere. Set the variable to 0 to opt back in.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

/// A helper the app ships (`pitwall-hold`, `pitwall-cli`), at a path that
/// stays valid while agents run and after the app quits. An AppImage runs
/// from a mount that disappears when the app exits, so its helpers are copied
/// to Pitwall's data folder (`bin/`) first; everywhere else they are used
/// where they are.
pub fn stable_sidecar(bin: PathBuf) -> PathBuf {
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_some() {
        if let Some(mount) = std::env::var_os("APPDIR") {
            let dest = pitwall_core::paths::Paths::default_root().join("bin");
            return stabilize(&bin, Path::new(&mount), &dest);
        }
    }
    bin
}

/// `bin` copied into `dest_dir` when it lives under `mount` (unchanged
/// copies are left alone); `bin` itself otherwise or when copying fails.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn stabilize(bin: &Path, mount: &Path, dest_dir: &Path) -> PathBuf {
    let (true, Some(name)) = (bin.starts_with(mount), bin.file_name()) else { return bin.to_path_buf() };
    let dest = dest_dir.join(name);
    match copy_if_changed(bin, &dest) {
        Ok(()) => dest,
        Err(e) => {
            eprintln!("pitwall: could not copy {} out of the AppImage: {e}", bin.display());
            bin.to_path_buf()
        }
    }
}

/// Write `to` only when its bytes differ. Replaced by rename, so holders
/// still running the old copy keep their (unlinked) file.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn copy_if_changed(from: &Path, to: &Path) -> io::Result<()> {
    let new = std::fs::read(from)?;
    if std::fs::read(to).ok().as_deref() == Some(&new[..]) {
        return Ok(());
    }
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = to.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, &new)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_inside_a_mount_are_copied_out_once() {
        let root = std::env::temp_dir().join(format!("pw-sidecar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mount = root.join("mount/usr/bin");
        std::fs::create_dir_all(&mount).unwrap();
        let bin = mount.join("pitwall-hold");
        std::fs::write(&bin, "v1").unwrap();
        let dest = root.join("data/bin");

        let got = stabilize(&bin, &root.join("mount"), &dest);
        assert_eq!(got, dest.join("pitwall-hold"));
        assert_eq!(std::fs::read_to_string(&got).unwrap(), "v1");
        // A newer app: replaced; the same app again: left alone.
        std::fs::write(&bin, "v2").unwrap();
        assert_eq!(std::fs::read_to_string(stabilize(&bin, &root.join("mount"), &dest)).unwrap(), "v2");
        let before = std::fs::metadata(&got).unwrap().modified().unwrap();
        stabilize(&bin, &root.join("mount"), &dest);
        assert_eq!(std::fs::metadata(&got).unwrap().modified().unwrap(), before);
        // Not in the mount (a .deb install, dev builds): used where it is.
        let elsewhere = root.join("usr-bin-pitwall-hold");
        assert_eq!(stabilize(&elsewhere, &root.join("mount"), &dest), elsewhere);
        let _ = std::fs::remove_dir_all(&root);
    }
}
