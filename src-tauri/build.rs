//! Builds the app's helper binaries and hands them to Tauri as
//! `externalBin`s (sidecars), so they sit next to the app executable both in
//! `pnpm tauri dev` (`target/<profile>/…`) and in the bundle
//! (`Pitwall.app/Contents/MacOS/…`): the `pitwall-hold` terminal holder and
//! the `pitwall-cli` command-line tool (linked as `pitwall` from Settings;
//! the app's own executable is already called `pitwall`). See
//! docs/spec/backend.md.

use std::path::{Path, PathBuf};
use std::process::Command;

/// (package, binary, env var that bakes in where it was built)
const SIDECARS: &[(&str, &str, &str)] = &[("pitwall-hold", "pitwall-hold", "PITWALL_HOLD_BUILT"), ("pitwall-cli", "pitwall-cli", "PITWALL_CLI_BUILT")];

fn main() {
    build_sidecars();
    tauri_build::build()
}

fn build_sidecars() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.parent().unwrap().to_path_buf();
    let target = std::env::var("TARGET").unwrap();
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    for dir in ["pitwall-hold", "pitwall-cli", "pitwall-client", "pitwall-proto"] {
        println!("cargo:rerun-if-changed=../crates/{dir}");
    }
    println!("cargo:rerun-if-changed=build.rs");

    // A separate target dir, so this nested build never waits on the lock the
    // outer build holds.
    let target_dir = root.join("target").join("hold");
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(&root).arg("build");
    for (package, bin, _) in SIDECARS {
        cmd.args(["-p", package, "--bin", bin]);
    }
    cmd.args(["--target", &target, "--target-dir"]).arg(&target_dir);
    if release {
        cmd.arg("--release");
    }
    // Don't inherit the outer build's wrappers (clippy) or per-crate settings.
    for key in ["RUSTC_WORKSPACE_WRAPPER", "RUSTC_WRAPPER", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET"] {
        cmd.env_remove(key);
    }
    let status = cmd.status().expect("run cargo to build the sidecars");
    assert!(status.success(), "building the sidecars (pitwall-hold, pitwall-cli) failed");

    let exe = if target.contains("windows") { ".exe" } else { "" };
    for (_, bin, env) in SIDECARS {
        let built = target_dir.join(&target).join(if release { "release" } else { "debug" }).join(format!("{bin}{exe}"));
        println!("cargo:rustc-env={env}={}", built.display());
        // Tauri's sidecar convention: binaries/<name>-<target triple>.
        let sidecar = manifest_dir.join("binaries").join(format!("{bin}-{target}{exe}"));
        copy_if_changed(&built, &sidecar);
    }
}

/// Only touch the file when it differs, so the dev watcher isn't woken up.
fn copy_if_changed(from: &Path, to: &Path) {
    let new = std::fs::read(from).expect("read the built sidecar");
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
