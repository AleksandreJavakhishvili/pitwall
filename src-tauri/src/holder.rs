//! Where the app ships the `pitwall-hold` terminal holder. Release: next to
//! the app executable (`Contents/MacOS/`, bundled as a Tauri `externalBin`).
//! Dev: `tauri_build` copies it next to `target/<profile>/pitwall`; tests run
//! from `deps/`, one level down. `build.rs` also bakes in where it built it,
//! and `PITWALL_HOLD_BIN` overrides everything.

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
        .ok_or_else(|| "the terminal holder (pitwall-hold) is missing next to the app".into())
}

/// The core's terminal session against the holder this app ships: start,
/// sizes, output, input, re-attach after the app side goes away, stop.
/// Only harmless `sh` commands run, in temp dirs.
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use pitwall_core::clock::{Clock, SystemClock};
    use pitwall_core::session::{clamp_size, Session, Spawn, DEFAULT_COLS, DEFAULT_ROWS};

    use super::*;

    /// A temp holder socket dir (never the real `~/.pitwall/run/hold`).
    struct HoldDir(PathBuf);

    impl HoldDir {
        fn new(tag: &str) -> HoldDir {
            static N: AtomicU64 = AtomicU64::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("pws-{}-{tag}{n}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            HoldDir(dir)
        }
    }

    impl Drop for HoldDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn clock() -> Arc<dyn Clock> {
        Arc::new(SystemClock::new())
    }

    fn spawn(dir: &HoldDir, id: &str, command_line: &str, size: Option<(u16, u16)>) -> Arc<Session> {
        Session::spawn(Spawn {
            agent_id: id,
            cwd: "/",
            command_line: command_line.into(),
            size,
            hold_dir: &dir.0,
            holder: &holder_bin().unwrap(),
            hook_socket: &dir.0.join("hook.sock"),
            clock: clock(),
        })
        .unwrap()
    }

    fn text(s: &Session) -> String {
        String::from_utf8_lossy(&s.history()).into_owned()
    }

    fn wait_for(s: &Session, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !text(s).contains(needle) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        text(s)
    }

    fn output_of(s: &Session) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !s.poll_exit() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        text(s)
    }

    #[test]
    fn spawns_and_captures_output() {
        let dir = HoldDir::new("cap");
        let s = spawn(&dir, "test", "printf 'pw-%s' \"$PITWALL_AGENT_ID\"", None);
        assert!(output_of(&s).contains("pw-test"));
        assert!(s.poll_exit());
        assert!(s.output_seq.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn subscribers_get_the_replay_and_live_output() {
        let dir = HoldDir::new("sub");
        let s = spawn(&dir, "sub", "sh -c 'echo before; read l; echo after:$l; sleep 0.2'", None);
        wait_for(&s, "before");
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let id = s.attach(Box::new(move |b: &[u8]| {
            g.lock().unwrap().extend_from_slice(b);
            true
        }));
        s.write(b"x\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !String::from_utf8_lossy(&got.lock().unwrap()).contains("after:x") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let seen = String::from_utf8_lossy(&got.lock().unwrap()).into_owned();
        assert!(seen.contains("before") && seen.contains("after:x"), "{seen:?}");
        s.detach(id);
    }

    #[test]
    fn process_starts_at_the_requested_size() {
        let dir = HoldDir::new("size");
        let s = spawn(&dir, "size-test", "sh -c 'echo size=$(stty size); sleep 0.2'", Some((77, 23)));
        assert_eq!(s.size(), (77, 23));
        assert!(output_of(&s).contains("size=23 77"));
    }

    #[test]
    fn default_and_clamped_spawn_sizes() {
        let dir = HoldDir::new("dflt");
        let s = spawn(&dir, "size-default", "sh -c 'echo size=$(stty size); sleep 0.2'", None);
        assert_eq!(s.size(), (DEFAULT_COLS, DEFAULT_ROWS));
        assert!(output_of(&s).contains(&format!("size={DEFAULT_ROWS} {DEFAULT_COLS}")));
        assert_eq!(clamp_size(0, 5000), (2, 1000));
    }

    /// The app goes away and comes back: the agent keeps running in its
    /// holder, and the new app re-attaches to the same process with its
    /// history, without starting anything.
    #[test]
    fn reattach_after_the_app_side_restarts() {
        let dir = HoldDir::new("re");
        let first = spawn(&dir, "re", "sh -c 'echo first-run; while IFS= read -r l; do echo \"got:$l\"; done'", Some((90, 30)));
        assert!(wait_for(&first, "first-run").contains("first-run"));
        let pid = first.pid;
        // What a restart leaves behind: nothing on our side.
        drop(first);

        let again = Session::reattach(&dir.0, "re", clock()).expect("holder still there");
        assert_eq!(again.pid, pid, "same process, no respawn");
        assert_eq!(again.size(), (90, 30), "size comes from the holder");
        assert!(!again.poll_exit());
        assert!(wait_for(&again, "first-run").matches("first-run").count() == 1, "history replayed once");
        again.write(b"hello\n").unwrap();
        assert!(wait_for(&again, "got:hello").contains("got:hello"));

        // An explicit stop still ends it; then there is nothing to re-attach.
        again.terminate(Duration::from_millis(500));
        assert!(again.poll_exit());
        let deadline = Instant::now() + Duration::from_secs(5);
        while Session::reattach(&dir.0, "re", clock()).is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(Session::reattach(&dir.0, "re", clock()).is_none());
        assert!(Session::reattach(&dir.0, "never-started", clock()).is_none());
    }
}
