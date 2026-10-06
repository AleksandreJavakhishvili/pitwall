//! Mac apps started from the Dock don't inherit the terminal's PATH, so agents
//! are launched through the user's login + interactive shell.

use std::process::Command;

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
        .stdin(std::process::Stdio::null())
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
