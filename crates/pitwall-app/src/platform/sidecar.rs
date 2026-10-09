//! Helpers the app ships (`pitwall-hold`, `pitwall-cli`) at a path that
//! outlives the app (ported from `src-tauri/src/platform/mod.rs`).

use std::io;
use std::path::{Path, PathBuf};

/// A helper the app ships (`pitwall-hold`, `pitwall-cli`), at a path that
/// stays valid while agents run and after the app quits. An AppImage runs
/// from a mount that disappears when the app exits, so its helpers are copied
/// to Pitwall's data folder (`data_root/bin/`) first; everywhere else they
/// are used where they are.
#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
pub fn stable(bin: PathBuf, data_root: &Path) -> PathBuf {
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_some() {
        if let Some(mount) = std::env::var_os("APPDIR") {
            let dest = data_root.join("bin");
            return stabilize(&bin, Path::new(&mount), &dest);
        }
    }
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
        assert_eq!(
            std::fs::read_to_string(stabilize(&bin, &root.join("mount"), &dest)).unwrap(),
            "v2"
        );
        let before = std::fs::metadata(&got).unwrap().modified().unwrap();
        stabilize(&bin, &root.join("mount"), &dest);
        assert_eq!(std::fs::metadata(&got).unwrap().modified().unwrap(), before);
        // Not in the mount (a .deb install, dev builds): used where it is.
        let elsewhere = root.join("usr-bin-pitwall-hold");
        assert_eq!(stabilize(&elsewhere, &root.join("mount"), &dest), elsewhere);
        let _ = std::fs::remove_dir_all(&root);
    }
}
