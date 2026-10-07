//! The desktop app adopts the login shell's PATH at start-up
//! (`shell::adopt_login_path`), so programs it starts find what a terminal
//! finds even when the app itself was given a minimal PATH (Dock, Finder,
//! desktop launchers). A process of its own: this test changes PATH.
//!
//! Safety: only fake shells in a temp folder and `/bin/sh` run.

// The fake login shells are POSIX scripts.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pitwall_core::exec::{Cmd, Exec, LocalExec};
use pitwall_core::shell::{self, LoginShell};

const MINIMAL: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pitwall-login-path-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A "login shell" that runs `-l -i -c <script>` with `path` as PATH, after
/// printing rc-file noise.
fn fake_shell(dir: &Path, name: &str, body: &str) -> String {
    let sh = dir.join(name);
    std::fs::write(&sh, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
    sh.to_string_lossy().into_owned()
}

fn login_shell(dir: &Path, path: &str) -> String {
    fake_shell(dir, "login-sh", &format!("echo 'welcome back'\nPATH='{path}'; export PATH\nshift 3\nexec /bin/sh -c \"$1\""))
}

#[test]
fn children_get_the_login_path_without_blocking_start_up() {
    let dir = temp_dir("probe");
    let home = dir.join("home");
    std::fs::create_dir_all(home.join(".local/bin")).unwrap();
    let login = "/fake/login/bin:/opt/homebrew/bin:/usr/bin:/bin";

    // The shell's PATH first, then what the app had that it lacks.
    let sh = LoginShell::new(login_shell(&dir, login));
    assert_eq!(shell::probe_path(&sh, Duration::from_secs(10)).as_deref(), Some(login));
    let path = shell::resolve_path(&sh, Some("/app/own/bin:/usr/bin"), &home, Duration::from_secs(10));
    assert_eq!(path, format!("{login}:/app/own/bin"));

    // A failing or hanging shell: the current PATH plus the common dirs that
    // exist (here `~/.local/bin` of the fake home, maybe Homebrew's).
    let failing = LoginShell::new(fake_shell(&dir, "failing-sh", "exit 1"));
    let fallback = shell::resolve_path(&failing, Some(MINIMAL), &home, Duration::from_secs(10));
    assert!(fallback.starts_with(&format!("{MINIMAL}:")), "{fallback}");
    assert!(fallback.contains(&format!("{}/.local/bin", home.display())), "{fallback}");
    assert!(!fallback.contains(".cargo/bin"), "missing dirs left out: {fallback}");
    let hanging = LoginShell::new(fake_shell(&dir, "hanging-sh", "exec sleep 30"));
    let started = Instant::now();
    assert_eq!(shell::resolve_path(&hanging, Some(MINIMAL), &home, Duration::from_millis(300)), fallback);
    assert!(started.elapsed() < Duration::from_secs(5), "time-boxed");

    // The app at start-up, with a minimal PATH, last run's login PATH cached
    // and a login shell that takes a while: the cached PATH at once, the
    // shell's once it answers (and cached for next time).
    std::env::set_var("PATH", MINIMAL);
    std::env::set_var("HOME", &home);
    let slow = fake_shell(&dir, "slow-sh", &format!("sleep 1\nPATH='{login}'; export PATH\nshift 3\nexec /bin/sh -c \"$1\""));
    std::env::set_var(shell::SHELL_ENV, slow);
    let cache = dir.join("data/login-path");
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, "/cached/bin:/usr/bin\n").unwrap();
    let echo = || LocalExec.run(&Cmd::new(&["sh", "-c", "printf %s \"$PATH\""])).unwrap().stdout_text();

    let started = Instant::now();
    let probe = shell::adopt_login_path(Some(cache.clone())).expect("a probe thread");
    assert!(started.elapsed() < Duration::from_millis(500), "start-up isn't held up");
    let quick = echo();
    assert!(quick.starts_with("/cached/bin:/usr/bin:/bin:/usr/sbin:/sbin:"), "{quick}");
    assert!(quick.contains(".local/bin"), "{quick}");

    probe.join().unwrap();
    let want = format!("{login}:/usr/sbin:/sbin");
    assert_eq!(echo(), want, "children see the login PATH");
    assert_eq!(shell::spawn_path().as_deref(), Some(want.as_str()));
    assert_eq!(std::env::var("PATH").unwrap(), MINIMAL, "the process environment is never changed");
    assert_eq!(std::fs::read_to_string(&cache).unwrap(), want, "cached for the next start");
    assert_eq!(shell::login_env_path(), want, "resolved once");

    let _ = std::fs::remove_dir_all(&dir);
}
