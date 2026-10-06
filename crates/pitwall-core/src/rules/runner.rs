//! Finding and running the `rulesync` CLI. Pitwall never installs it: it uses
//! `rulesync` from the user's login-shell PATH, or `npx -y rulesync` only when
//! the user turned that on in Settings.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::exec::{self, Cmd, Exec, LocalExec};
use crate::shell;

#[derive(Debug, Clone)]
pub struct Runner {
    /// Program and leading args, e.g. ["/opt/homebrew/bin/rulesync"] or
    /// ["/usr/local/bin/npx", "-y", "rulesync"].
    pub argv: Vec<String>,
    /// PATH to run with (npm-installed rulesync needs `node` on it).
    pub path_env: Option<String>,
    pub via: &'static str,
}

const MARK: &str = "__PITWALL_PATH__";

/// PATH as the user's login + interactive shell sees it (Dock apps get a
/// minimal PATH). Resolved once.
pub fn login_path() -> Option<String> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        let (shell, script) = (shell::user_shell(), format!("printf '\\n{MARK}%s\\n' \"$PATH\""));
        let text = exec::local_stdout(&[&shell, "-l", "-i", "-c", &script], exec::LONG)?;
        text.lines()
            .rev()
            .find_map(|l| l.strip_prefix(MARK))
            .map(str::to_string)
            .filter(|p| !p.is_empty())
    })
    .clone()
}

fn find_in(path_env: &str, program: &str) -> Option<PathBuf> {
    std::env::split_paths(path_env)
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(program))
        .find(|p| crate::platform::is_executable(p))
}

fn search_path() -> String {
    login_path()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default()
}

pub struct Detected {
    pub rulesync: Option<PathBuf>,
    pub npx: Option<PathBuf>,
}

pub fn detect() -> Detected {
    let path = search_path();
    Detected {
        rulesync: find_in(&path, "rulesync"),
        npx: find_in(&path, "npx"),
    }
}

/// The runner to use, honouring the user's npx opt-in.
pub fn runner(allow_npx: bool) -> Option<Runner> {
    let found = detect();
    let path_env = Some(search_path());
    if let Some(bin) = found.rulesync {
        return Some(Runner {
            argv: vec![bin.to_string_lossy().into_owned()],
            path_env,
            via: "rulesync",
        });
    }
    match (allow_npx, found.npx) {
        (true, Some(npx)) => Some(Runner {
            argv: vec![npx.to_string_lossy().into_owned(), "-y".into(), "rulesync".into()],
            path_env,
            via: "npx",
        }),
        _ => None,
    }
}

pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Runner {
    /// rulesync runs on this Mac (its staging folder and library are here).
    pub fn run(&self, cwd: &Path, args: &[String]) -> Result<Output, String> {
        let argv: Vec<&str> = self.argv.iter().chain(args).map(String::as_str).collect();
        let cwd = cwd.to_string_lossy();
        let mut env = vec![("NO_COLOR", "1"), ("CI", "1")];
        if let Some(p) = &self.path_env {
            env.push(("PATH", p));
        }
        let out = LocalExec
            .run(&Cmd::new(&argv).cwd(&cwd).env(&env))
            .map_err(|e| format!("could not run rulesync: {e}"))?;
        Ok(Output {
            ok: out.ok(),
            stdout: out.stdout_text(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// Run with the global `--json` flag and return the result document.
    /// Failures come back as `Err` with rulesync's own message.
    pub fn run_json(&self, cwd: &Path, args: &[String]) -> Result<serde_json::Value, String> {
        let mut full = args.to_vec();
        full.insert(1, "--json".into());
        let out = self.run(cwd, &full)?;
        let doc = parse_json_doc(&out.stdout);
        match doc {
            Some(doc) if doc.get("success").and_then(|v| v.as_bool()) == Some(true) => Ok(doc),
            Some(doc) => {
                let msg = doc
                    .pointer("/error/message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("rulesync failed");
                Err(format!("rulesync: {msg}"))
            }
            None => {
                let detail = if out.stderr.trim().is_empty() { out.stdout } else { out.stderr };
                let detail: String = detail.trim().chars().take(600).collect();
                Err(if out.ok {
                    format!("rulesync printed no JSON result: {detail}")
                } else {
                    format!("rulesync failed: {detail}")
                })
            }
        }
    }

    pub fn version(&self) -> Option<String> {
        let out = self.run(Path::new("/"), &["--version".into()]).ok()?;
        out.ok
            .then(|| out.stdout.lines().map(str::trim).rfind(|l| !l.is_empty()).map(String::from))
            .flatten()
    }
}

/// The JSON document rulesync prints (tolerates stray lines around it).
fn parse_json_doc(stdout: &str) -> Option<serde_json::Value> {
    if let Ok(v) = serde_json::from_str(stdout.trim()) {
        return Some(v);
    }
    let start = stdout.find('{')?;
    let end = stdout.rfind('}')?;
    serde_json::from_str(stdout.get(start..=end)?).ok()
}

/// `warnings` from a result document.
pub fn warnings(doc: &serde_json::Value) -> Vec<String> {
    doc.get("warnings")
        .and_then(|w| w.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_executables_on_path() {
        let dir = std::env::temp_dir().join(format!("pw-runner-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("rulesync");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        assert_eq!(find_in(&dir.to_string_lossy(), "rulesync"), None, "not executable yet");
        crate::platform::make_executable(&bin).unwrap();
        assert_eq!(find_in(&format!("/nonexistent:{}", dir.display()), "rulesync"), Some(bin));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn json_doc_with_noise() {
        let v = parse_json_doc("motd\n{\"success\":true}\n").unwrap();
        assert_eq!(v["success"], true);
        assert!(parse_json_doc("nothing").is_none());
    }
}
