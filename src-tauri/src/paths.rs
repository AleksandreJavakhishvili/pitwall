use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Everything Pitwall owns lives under ~/.pitwall.
pub fn pitwall_dir() -> PathBuf {
    home().join(".pitwall")
}

pub fn state_file() -> PathBuf {
    pitwall_dir().join("state.json")
}

pub fn socket_path() -> PathBuf {
    pitwall_dir().join("run").join("pitwall.sock")
}

/// Agents' terminal holders: `<agentId>.sock` each (pitwall-hold).
pub fn hold_dir() -> PathBuf {
    pitwall_dir().join("run").join("hold")
}

pub fn hook_script() -> PathBuf {
    pitwall_dir().join("bin").join("pitwall-hook")
}

pub fn user_agents_dir() -> PathBuf {
    pitwall_dir().join("agents")
}

/// Where versions before 0.1 made agent worktrees. Pitwall no longer creates
/// any; agents still working in one keep their stored path.
pub fn legacy_worktrees_dir() -> PathBuf {
    pitwall_dir().join("worktrees")
}

/// Show paths under the home directory as `~/...`.
pub fn tildify(path: &str) -> String {
    let home = home();
    let home = home.to_string_lossy();
    match path.strip_prefix(home.as_ref()) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}
