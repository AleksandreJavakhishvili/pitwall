//! What the packaged app needs at run time (docs/spec/gpui/packaging.md).
//!
//! The sidecars (`pitwall-hold`, `pitwall-cli`) sit next to the app's
//! executable in every bundle (`Contents/MacOS/`, `usr/bin/`, the Windows
//! install folder), so [`crate::host::holder_bin`] finds them there. Only an
//! AppImage needs more: it runs from a mount that disappears when the app
//! exits, while holders outlive the app, so its helpers are copied to the
//! data folder's `bin/` first (as `src-tauri/src/platform/mod.rs` does).
//!
//! A macOS bundle needs more too: updating the app replaces its bundle
//! under holders that keep running, and macOS then stops trusting them
//! (Desktop and Full Disk Access are refused). So the holder runs from a
//! copy per build in `bin/versions/<hash>/` that no update touches
//! ([`versioned`]); copies no process runs any more are removed ([`prune`]).

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
    #[cfg(target_os = "macos")]
    if in_bundle(&bin) {
        return versioned(&bin, &data_bin.join("versions"));
    }
    let _ = data_bin;
    bin
}

/// `bin` is inside an app bundle (`….app/Contents/MacOS/`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn in_bundle(bin: &Path) -> bool {
    bin.parent().is_some_and(|d| d.ends_with("Contents/MacOS"))
        && bin.ancestors().any(|a| a.extension().is_some_and(|e| e == "app"))
}

/// `bin` copied to `versions/<hash of its bytes>/<name>` (once; the copy
/// keeps its signature), so a later build never changes the file a running
/// holder came from. `bin` itself when copying fails.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn versioned(bin: &Path, versions: &Path) -> PathBuf {
    let copy = || -> io::Result<PathBuf> {
        use std::hash::{Hash, Hasher};
        let bytes = std::fs::read(bin)?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        let name = bin.file_name().ok_or_else(|| io::Error::other("no file name"))?;
        let dest = versions.join(format!("{:016x}", h.finish())).join(name);
        copy_if_changed(bin, &dest)?;
        Ok(dest)
    };
    match copy() {
        Ok(dest) => {
            let (versions, keep) = (versions.to_path_buf(), dest.clone());
            std::thread::spawn(move || prune(&versions, &keep, &running_programs()));
            dest
        }
        Err(e) => {
            eprintln!("pitwall: could not copy {} out of the app: {e}", bin.display());
            bin.to_path_buf()
        }
    }
}

/// Remove the copies in `versions` besides `keep`'s that no process in
/// `running` (their programs' paths) runs.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn prune(versions: &Path, keep: &Path, running: &[PathBuf]) {
    let Ok(dirs) = std::fs::read_dir(versions) else { return };
    for dir in dirs.flatten().map(|d| d.path()) {
        if keep.starts_with(&dir) || running.iter().any(|r| r.starts_with(&dir)) {
            continue;
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The programs every process runs (`ps`'s full paths); empty when `ps`
/// fails, which keeps every copy.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn running_programs() -> Vec<PathBuf> {
    match std::process::Command::new("/bin/ps").args(["-axww", "-o", "comm="]).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).lines().map(|l| PathBuf::from(l.trim())).collect(),
        _ => vec![PathBuf::from("/")],
    }
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
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
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
    fn a_bundled_holder_runs_from_a_copy_per_build() {
        let root = std::env::temp_dir().join(format!("pw-app-versions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let macos = root.join("Pitwall.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let bin = macos.join("pitwall-hold");
        assert!(in_bundle(&bin) && !in_bundle(&root.join("target/debug/pitwall-hold")));
        let versions = root.join("data/bin/versions");

        std::fs::write(&bin, "v1").unwrap();
        let v1 = versioned(&bin, &versions);
        assert!(v1.starts_with(&versions) && v1.ends_with("pitwall-hold"));
        assert_eq!(std::fs::read_to_string(&v1).unwrap(), "v1");
        assert_eq!(versioned(&bin, &versions), v1, "the same build: the same copy");
        // An update: a new copy; the old one stays while a holder runs it.
        std::fs::write(&bin, "v2").unwrap();
        let v2 = versioned(&bin, &versions);
        assert_ne!(v1, v2);
        assert_eq!(std::fs::read_to_string(&v1).unwrap(), "v1", "never changed under a running holder");
        prune(&versions, &v2, std::slice::from_ref(&v1));
        assert!(v1.exists() && v2.exists());
        prune(&versions, &v2, &[]);
        assert!(!v1.exists() && v2.exists(), "unused: removed");
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
