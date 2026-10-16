//! A shared session's environment, built from nothing
//! (docs/spec/collaboration/sessions.md §6.3, "Environment"): `PATH`,
//! `HOME` and `TMPDIR` in the session's folder, `LANG`, `TERM`, the proxy
//! variables, the agent account's config variable, and what the owner
//! chose to pass. Nothing of the owner's environment is read here, so no
//! `SSH_AUTH_SOCK`, no tokens, no `PITWALL_*` can leak in by accident; the
//! owner's explicit additions are checked against the same list.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{PwError, Result};

/// What goes into a session's environment.
#[derive(Debug, Clone, Default)]
pub struct EnvSpec {
    /// The session's own `HOME` (inside its folder, outside the copy).
    pub home: PathBuf,
    /// The session's own temp folder.
    pub tmp: PathBuf,
    /// `PATH`, in order: the system's folders and the agent's install.
    pub path: Vec<PathBuf>,
    /// The egress proxy's port; `None`: no proxy variables (no network).
    pub proxy_port: Option<u16>,
    /// The agent account's config variable and folder
    /// (`CLAUDE_CONFIG_DIR` = the account's folder), from the kind's
    /// `account_env`.
    pub account: Option<(String, PathBuf)>,
    /// `SHELL`, for agents that run commands through it.
    pub shell: Option<PathBuf>,
    /// `LANG`; `None`: `en_US.UTF-8`.
    pub lang: Option<String>,
    /// Variables the owner chose to pass (the share's settings and the
    /// kind's fixed ones, e.g. `DISABLE_AUTOUPDATER`).
    pub pass: BTreeMap<String, String>,
}

/// Names Pitwall sets itself; the owner can't override them.
const OWN: &[&str] = &[
    "PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "TERM", "SHELL", "HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY",
    "NO_PROXY", "https_proxy", "http_proxy", "all_proxy", "no_proxy",
];

/// Never passed, whoever asks: the owner's SSH agent and askpass helpers,
/// Pitwall's own variables (sockets, data folder), and loader injection.
fn refused(name: &str) -> Option<&'static str> {
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("PITWALL_") {
        Some("Pitwall's own variables never reach a shared session")
    } else if ["SSH_AUTH_SOCK", "SSH_AGENT_PID", "GIT_ASKPASS", "SSH_ASKPASS", "SUDO_ASKPASS"].contains(&upper.as_str()) {
        Some("the owner's SSH agent and password helpers never reach a shared session")
    } else if upper.starts_with("DYLD_") || upper.starts_with("LD_") {
        Some("loader variables could change what runs inside the sandbox")
    } else if OWN.contains(&name) || OWN.iter().any(|o| o.eq_ignore_ascii_case(name)) {
        Some("Pitwall sets this one itself")
    } else {
        None
    }
}

/// Whether the owner may pass `name` to a shared session; `Err` says why
/// not (for the Share dialog).
pub fn check_passed(name: &str) -> std::result::Result<(), String> {
    if name.is_empty() || name.contains('=') || name.contains('\0') {
        return Err(format!("{name:?} isn't a variable name"));
    }
    match refused(name) {
        Some(why) => Err(why.to_string()),
        None => Ok(()),
    }
}

/// The session's whole environment.
pub fn build(spec: &EnvSpec) -> Result<BTreeMap<String, String>> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    if !spec.home.is_absolute() || !spec.tmp.is_absolute() {
        return Err(PwError::other("the session's HOME and TMPDIR must be absolute paths"));
    }
    let mut env = BTreeMap::new();
    for (k, v) in &spec.pass {
        check_passed(k).map_err(|why| PwError::other(format!("{k}: {why}")))?;
        env.insert(k.clone(), v.clone());
    }
    if let Some((var, dir)) = &spec.account {
        check_passed(var).map_err(|why| PwError::other(format!("{var}: {why}")))?;
        env.insert(var.clone(), s(dir));
    }
    let mut path: Vec<String> = Vec::new();
    for p in &spec.path {
        let p = s(p);
        if !p.is_empty() && !p.contains(':') && !path.contains(&p) {
            path.push(p);
        }
    }
    env.insert("PATH".into(), path.join(":"));
    env.insert("HOME".into(), s(&spec.home));
    env.insert("TMPDIR".into(), format!("{}/", s(&spec.tmp).trim_end_matches('/')));
    env.insert("LANG".into(), spec.lang.clone().unwrap_or_else(|| "en_US.UTF-8".into()));
    env.insert("TERM".into(), "xterm-256color".into());
    if let Some(shell) = &spec.shell {
        env.insert("SHELL".into(), s(shell));
    }
    if let Some(port) = spec.proxy_port {
        let url = super::proxy::proxy_url(port);
        for k in ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY", "https_proxy", "http_proxy", "all_proxy"] {
            env.insert(k.into(), url.clone());
        }
        // Everything goes through the proxy, local names included.
        env.insert("NO_PROXY".into(), String::new());
        env.insert("no_proxy".into(), String::new());
    }
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> EnvSpec {
        EnvSpec {
            home: "/s/home".into(),
            tmp: "/s/tmp".into(),
            path: vec!["/usr/bin".into(), "/bin".into(), "/usr/bin".into(), "/a:b".into()],
            proxy_port: Some(4100),
            account: Some(("CLAUDE_CONFIG_DIR".into(), "/accounts/share".into())),
            shell: Some("/bin/zsh".into()),
            lang: None,
            pass: BTreeMap::from([("DISABLE_AUTOUPDATER".into(), "1".into())]),
        }
    }

    #[test]
    fn built_from_nothing() {
        std::env::set_var("PITWALL_ENV_TEST_CANARY", "owner-secret");
        let env = build(&spec()).unwrap();
        let keys: Vec<&str> = env.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "ALL_PROXY", "CLAUDE_CONFIG_DIR", "DISABLE_AUTOUPDATER", "HOME", "HTTPS_PROXY", "HTTP_PROXY", "LANG",
                "NO_PROXY", "PATH", "SHELL", "TERM", "TMPDIR", "all_proxy", "http_proxy", "https_proxy", "no_proxy"
            ]
        );
        assert_eq!(env["PATH"], "/usr/bin:/bin");
        assert_eq!(env["HTTPS_PROXY"], "http://127.0.0.1:4100");
        assert_eq!(env["TMPDIR"], "/s/tmp/");
        assert!(!env.values().any(|v| v.contains("owner-secret")));
    }

    #[test]
    fn no_proxy_means_no_proxy_variables() {
        let env = build(&EnvSpec { proxy_port: None, ..spec() }).unwrap();
        assert!(!env.contains_key("HTTPS_PROXY") && !env.contains_key("NO_PROXY"));
    }

    #[test]
    fn owner_cannot_pass_what_must_never_reach_a_session() {
        for name in ["SSH_AUTH_SOCK", "PITWALL_HOME", "pitwall_sock", "DYLD_INSERT_LIBRARIES", "HOME", "https_proxy", "A=B", ""] {
            assert!(check_passed(name).is_err(), "{name}");
            let mut s = spec();
            s.pass.insert(name.into(), "x".into());
            assert!(build(&s).is_err(), "{name}");
        }
        assert!(check_passed("NPM_CONFIG_REGISTRY").is_ok());
        assert!(build(&EnvSpec { home: "rel".into(), ..spec() }).is_err());
    }
}
