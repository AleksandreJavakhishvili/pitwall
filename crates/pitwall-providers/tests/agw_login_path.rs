//! agw runs with the PATH the desktop app adopted from the login shell
//! (`shell::adopt_login_path`), so what agw starts itself (limactl from
//! Homebrew) is found even when the app was opened from the Dock with a
//! minimal PATH. A process of its own: this test changes PATH.
//!
//! Safety: the fake agw in `tests/fake_agw/` (wrapped to record its PATH)
//! and `/bin/sh` only, in a temp folder. The real agw is never run.

// The fake agw and the fake login shell are shell scripts.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use pitwall_core::exec::{Cmd, Exec, LocalExec};
use pitwall_core::shell;
use pitwall_core::testing::TempDir;
use pitwall_providers::agw::{AgwExec, User};

fn script(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn agw_and_its_commands_get_the_login_path() {
    let dir = TempDir::new("agw-login-path");
    let state = dir.path().join("agw");
    std::fs::create_dir_all(&state).unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(state.join("ws.ws"), ws.to_string_lossy().as_bytes()).unwrap();
    let body = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_agw/agw")).unwrap();
    let real = state.join("agw-real");
    script(&real, &body.replace("__DIR__", &state.to_string_lossy()));
    // agw itself: records the PATH it was started with.
    let agw = state.join("agw");
    let seen = state.join("agw-path");
    script(&agw, &format!("#!/bin/sh\nprintf %s \"$PATH\" > '{}'\nexec '{}' \"$@\"\n", seen.display(), real.display()));

    // The app at start-up: a minimal PATH, then the login shell's.
    let login = "/fake/login/bin:/opt/homebrew/bin:/usr/bin:/bin";
    let sh = dir.path().join("login-sh");
    script(&sh, &format!("#!/bin/sh\nPATH='{login}'; export PATH\nshift 3\nexec /bin/sh -c \"$1\"\n"));
    std::env::set_var("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
    std::env::set_var(shell::SHELL_ENV, &sh);
    let want = format!("{login}:/usr/sbin:/sbin");
    shell::adopt_login_path(None).expect("a probe thread").join().unwrap();
    assert_eq!(shell::spawn_path().as_deref(), Some(want.as_str()));

    let exec = AgwExec::new(&agw.to_string_lossy(), Arc::new(LocalExec), User::Admin { vm: "fakevm".into() }, Some("ws"));
    let out = exec.run(&Cmd::new(&["sh", "-c", "printf %s \"$PATH\""])).unwrap();
    assert_eq!(std::fs::read_to_string(&seen).unwrap(), want, "agw's PATH");
    // The fake runs commands on this machine, so they inherit it too.
    assert_eq!(out.stdout_text(), want);
}
