//! Running programs. Mac apps started from the Dock don't inherit the
//! terminal's PATH, so agents are launched through the user's login +
//! interactive shell. (A `LoginShell` abstraction replaces the Unix
//! specifics here for the Windows port, architecture.md §9 decision 7.)
//! Programs run through [`LocalExec`]: this is about this machine.

use std::time::Duration;

use crate::exec::{Cmd, Exec, LocalExec};

/// A login shell that hangs (waiting on something in the user's rc files).
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(60);

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
    let (shell, script) = (user_shell(), format!("command -v {}", quote(program)));
    let out = LocalExec
        .run(&Cmd::new(&[&shell, "-l", "-i", "-c", &script]).timeout(LOGIN_SHELL_TIMEOUT))
        .ok()?;
    if !out.ok() {
        return None;
    }
    out.stdout_text()
        .lines()
        .map(str::trim)
        .rfind(|l| l.starts_with('/'))
        .map(String::from)
}
