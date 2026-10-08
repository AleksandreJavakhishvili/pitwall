//! What the packaged app needs at run time (docs/spec/gpui/packaging.md).
//!
//! The sidecars (`pitwall-hold`, `pitwall-cli`) sit next to the app's
//! executable in every bundle (`Contents/MacOS/`, `usr/bin/`, the Windows
//! install folder), so [`crate::host::holder_bin`] finds them there. Only an
//! AppImage needs more: it runs from a mount that disappears when the app
//! exits, while holders outlive the app, so its helpers are copied to the
//! data folder's `bin/` first (as `src-tauri/src/platform/mod.rs` does).

use std::io;
use std::path::{Path, PathBuf};

/// `bin` at a path that stays valid while agents run and after the app
/// quits: copied into `data_bin` when the app runs from an AppImage mount,
/// `bin` itself everywhere else.
pub fn stable_sidecar(bin: PathBuf, data_bin: &Path) -> PathBuf {
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_some() {
        if let Some(mount) = std::env::var_os("APPDIR") {
            return stabilize(&bin, Path::new(&mount), data_bin);
        }
    }
    let _ = data_bin;
    bin
}

/// `bin` copied into `dest_dir` when it lives under `mount` (unchanged
/// copies are left alone); `bin` itself otherwise or when copying fails.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn stabilize(bin: &Path, mount: &Path, dest_dir: &Path) -> PathBuf {
    let (true, Some(name)) = (bin.starts_with(mount), bin.file_name()) else {
        return bin.to_path_buf();
    };
    let dest = dest_dir.join(name);
    match copy_if_changed(bin, &dest) {
        Ok(()) => dest,
        Err(e) => {
            eprintln!(
                "pitwall: could not copy {} out of the AppImage: {e}",
                bin.display()
            );
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
        let root = std::env::temp_dir().join(format!("pw-app-sidecar-{}", std::process::id()));
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
        let again = stabilize(&bin, &root.join("mount"), &dest);
        assert_eq!(std::fs::read_to_string(again).unwrap(), "v2");
        let before = std::fs::metadata(&got).unwrap().modified().unwrap();
        stabilize(&bin, &root.join("mount"), &dest);
        assert_eq!(std::fs::metadata(&got).unwrap().modified().unwrap(), before);
        // Not in the mount (a .deb install, a .app, dev builds): used where it is.
        let elsewhere = root.join("usr-bin-pitwall-hold");
        assert_eq!(stabilize(&elsewhere, &root.join("mount"), &dest), elsewhere);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn outside_an_appimage_the_sidecar_is_used_where_it_is() {
        if std::env::var_os("APPIMAGE").is_some() {
            return;
        }
        let bin = PathBuf::from("/opt/x/pitwall-hold");
        assert_eq!(stable_sidecar(bin.clone(), Path::new("/tmp/none")), bin);
    }
}
