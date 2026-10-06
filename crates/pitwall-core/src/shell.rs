//! Running programs. Mac apps started from the Dock don't inherit the
//! terminal's PATH, so agents are launched through the user's login +
//! interactive shell. (A `LoginShell` abstraction replaces the Unix
//! specifics here for the Windows port, architecture.md §9 decision 7.)

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn user_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
}

/// How agents are launched: `$SHELL -l -i -c <script>`, as (program, args).
pub fn login_invocation(script: &str) -> (String, Vec<String>) {
    (user_shell(), ["-l", "-i", "-c", script].map(String::from).to_vec())
}

pub fn quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// Whether `program` resolves in the user's login shell.
pub fn which(program: &str) -> Option<String> {
    let out = Command::new(user_shell())
        .args(["-l", "-i", "-c", &format!("command -v {}", quote(program))])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .rfind(|l| l.starts_with('/'))
        .map(String::from)
}

/// stdout of `cmd` if it finishes within `timeout` (killed otherwise).
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(30)),
            _ => {
                // Only ever the child we spawned ourselves.
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    reader.join().ok()
}
