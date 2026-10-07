//! Builds the app's helper binaries and hands them to Tauri as
//! `externalBin`s (sidecars), so they sit next to the app executable both in
//! `pnpm tauri dev` (`target/<profile>/…`) and in the bundle
//! (`Pitwall.app/Contents/MacOS/…`): the `pitwall-hold` terminal holder and
//! the `pitwall-cli` command-line tool (linked as `pitwall` from Settings;
//! the app's own executable is already called `pitwall`), and on Windows the
//! `pitwall-hook` relay (tauri.windows.conf.json lists it). See
//! docs/spec/backend.md.
//!
//! `PITWALL_SIDECAR_STUBS=1` writes empty stand-ins instead of building them,
//! for a `cargo check` of the app for another OS (cross-checking Windows from
//! a Mac: the sidecars would need that OS's linker).

use std::path::{Path, PathBuf};
use std::process::Command;

/// (package, binary, env var that bakes in where it was built)
const SIDECARS: &[(&str, &str, &str)] = &[("pitwall-hold", "pitwall-hold", "PITWALL_HOLD_BUILT"), ("pitwall-cli", "pitwall-cli", "PITWALL_CLI_BUILT")];

fn main() {
    build_sidecars();
    // Cross-checking from another OS: Windows resources (the icon, the
    // manifest) need that OS's resource compiler, so they are left out.
    let host = std::env::var("HOST").unwrap_or_default();
    let target = std::env::var("TARGET").unwrap_or_default();
    if std::env::var_os("PITWALL_SIDECAR_STUBS").is_some() && target.contains("windows") && !host.contains("windows") {
        for alias in ["desktop", "mobile", "dev"] {
            println!("cargo:rustc-check-cfg=cfg({alias})");
        }
        println!("cargo:rustc-cfg=desktop");
        println!("cargo:rustc-env=TAURI_ENV_TARGET_TRIPLE={target}");
        return;
    }
    tauri_build::build()
}

/// The `pitwall-hook` relay ships on Windows only (Unix keeps the sh script).
const WINDOWS_SIDECARS: &[(&str, &str, &str)] = &[("pitwall-hook", "pitwall-hook", "PITWALL_HOOK_BUILT")];

fn build_sidecars() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.parent().unwrap().to_path_buf();
    let target = std::env::var("TARGET").unwrap();
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    for dir in ["pitwall-hold", "pitwall-cli", "pitwall-client", "pitwall-proto", "pitwall-hook"] {
        println!("cargo:rerun-if-changed=../crates/{dir}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PITWALL_SIDECAR_STUBS");
    let exe = if target.contains("windows") { ".exe" } else { "" };
    let sidecars: Vec<_> = SIDECARS.iter().chain(if exe.is_empty() { &[][..] } else { WINDOWS_SIDECARS }).collect();

    if std::env::var_os("PITWALL_SIDECAR_STUBS").is_some() {
        for (_, bin, _) in &sidecars {
            let sidecar = manifest_dir.join("binaries").join(format!("{bin}-{target}{exe}"));
            if !sidecar.exists() {
                std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
                std::fs::write(&sidecar, b"").unwrap();
            }
        }
        return;
    }

    // A separate target dir, so this nested build never waits on the lock the
    // outer build holds.
    let target_dir = root.join("target").join("hold");
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(&root).arg("build");
    for (package, bin, _) in &sidecars {
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
    assert!(status.success(), "building the sidecars (pitwall-hold, pitwall-cli, pitwall-hook) failed");

    for (_, bin, env) in &sidecars {
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
