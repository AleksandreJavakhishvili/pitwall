//! Builds the `pitwall-hold` terminal holder and hands it to Tauri as an
//! `externalBin` (sidecar), so it sits next to the app executable both in
//! `pnpm tauri dev` (`target/<profile>/pitwall-hold`) and in the bundle
//! (`Pitwall.app/Contents/MacOS/pitwall-hold`). See docs/spec/backend.md.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    build_holder();
    tauri_build::build()
}

fn build_holder() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.parent().unwrap().to_path_buf();
    let target = std::env::var("TARGET").unwrap();
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    println!("cargo:rerun-if-changed=../crates/pitwall-hold");
    println!("cargo:rerun-if-changed=build.rs");

    // A separate target dir, so this nested build never waits on the lock the
    // outer build holds.
    let target_dir = root.join("target").join("hold");
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(&root)
        .args(["build", "-p", "pitwall-hold", "--bin", "pitwall-hold", "--target", &target, "--target-dir"])
        .arg(&target_dir);
    if release {
        cmd.arg("--release");
    }
    // Don't inherit the outer build's wrappers (clippy) or per-crate settings.
    for key in ["RUSTC_WORKSPACE_WRAPPER", "RUSTC_WRAPPER", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET"] {
        cmd.env_remove(key);
    }
    let status = cmd.status().expect("run cargo to build pitwall-hold");
    assert!(status.success(), "building pitwall-hold failed");

    let exe = if target.contains("windows") { ".exe" } else { "" };
    let built = target_dir
        .join(&target)
        .join(if release { "release" } else { "debug" })
        .join(format!("pitwall-hold{exe}"));
    println!("cargo:rustc-env=PITWALL_HOLD_BUILT={}", built.display());

    // Tauri's sidecar convention: binaries/<name>-<target triple>.
    let sidecar = manifest_dir.join("binaries").join(format!("pitwall-hold-{target}{exe}"));
    copy_if_changed(&built, &sidecar);
}

/// Only touch the file when it differs, so the dev watcher isn't woken up.
fn copy_if_changed(from: &Path, to: &Path) {
    let new = std::fs::read(from).expect("read the built pitwall-hold");
    if std::fs::read(to).ok().as_deref() == Some(&new[..]) {
        return;
    }
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    let tmp = to.with_extension("tmp");
    std::fs::write(&tmp, &new).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::rename(&tmp, to).unwrap();
}
