//! The OS sandbox around a shared session (docs/spec/collaboration/
//! sessions.md §6.3). Every shared session's whole process tree runs inside
//! one, for every agent kind: the agent keeps its own tools, and the OS
//! limits what its processes can reach (paths, programs, network). One
//! policy, one backend per OS, all behind [`SessionSandbox`]:
//!
//! - macOS: a Seatbelt profile (`sandbox-exec`), C1a;
//! - Linux: bubblewrap plus Landlock where available, C3;
//! - Windows: the Linux backend inside WSL2, C5.
//!
//! The backends live in `src/platform/` with the rest of the OS-specific
//! code; nothing outside it knows which one runs. [`session_sandbox`] gives
//! this machine's.
//!
//! Around the sandbox, OS-neutral pieces the session needs:
//! - [`proxy`]: the per-share egress proxy (the only network a session has);
//! - [`env`]: the session's environment, built from nothing;
//! - [`snapshot`]: the session's copy, a one-commit repository of the scope;
//! - [`scan`]: the sensitive-file scan of a share's scope;
//! - [`kind`]: a kind's `[share]` data resolved on this machine.

pub mod env;
pub mod kind;
pub mod proxy;
pub mod scan;
pub mod snapshot;

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::Result;

/// What a session may do, resolved to concrete paths by the caller (the
/// session's copy, its `HOME` and temp, the system and the agent's install
/// from kind data, the agent account's folder). Everything else is denied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxPolicy {
    /// Readable subtrees.
    pub read: Vec<PathBuf>,
    /// Writable subtrees (also readable). The copy only at "Can change code".
    pub write: Vec<PathBuf>,
    pub exec: ExecRule,
    /// Outbound network: only Pitwall's per-share egress proxy.
    pub egress: Egress,
    /// The whole environment, built from nothing (§6.3 "Environment"):
    /// nothing of the owner's is inherited.
    pub env: BTreeMap<String, String>,
    /// Local endpoints it may connect to besides the proxy (the hook
    /// endpoint, for hook events only). Never `pitwalld.sock`.
    pub connect: Vec<LocalEndpoint>,
    /// A folder for the backend's own files (a Seatbelt profile), outside
    /// everything above.
    pub state_dir: PathBuf,
    /// The agent may use the OS credential store for its own login (macOS:
    /// the Keychain, where Claude Code keeps a subscription login). Off by
    /// default: kinds that keep their login in their config folder (Codex
    /// `auth.json`) don't need it. From kind data (`[share] keychain`).
    pub credential_store: bool,
    /// A tag the backend attaches to every denial it reports, so this
    /// session's can be read back afterwards ([`SessionSandbox::denials`]):
    /// letters, digits, `-` and `_` only.
    pub tag: Option<String>,
}

/// One thing a session tried and its sandbox refused, as the OS reported
/// it: the process, the operation (`file-read-data`, `process-exec`,
/// `network-outbound`) and its target (a path, an address).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denial {
    pub process: String,
    pub operation: String,
    pub target: String,
}

/// What a guest probe's "tried" lines say about the sandbox's denials:
/// reads, writes, programs and connections, each once, the owner's home
/// shown as `~`. File lookups (`stat`), devices, IPC and system queries
/// that every program makes on its way are left out.
pub fn tried_lines(denials: &[Denial], home: &std::path::Path) -> Vec<String> {
    let home = home.to_string_lossy();
    let tilde = |t: &str| match t.strip_prefix(home.as_ref()) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => t.to_string(),
    };
    let mut out: Vec<String> = Vec::new();
    for d in denials {
        let verb = match d.operation.as_str() {
            "file-read-data" => "read",
            op if op.starts_with("file-write") => "write",
            op if op.starts_with("process-exec") => "run",
            op if op.starts_with("network-outbound") => "connect to",
            _ => continue,
        };
        // Devices, and the system log's socket every program tries on its own.
        if d.target.starts_with("/dev/") || d.target.is_empty() || d.target.contains("/var/run/syslog") {
            continue;
        }
        let line = format!("{verb} {}: denied by the sandbox", tilde(&d.target));
        if !out.contains(&line) {
            out.push(line);
        }
    }
    out.truncate(20);
    out
}

/// Which programs may be executed (absolute paths, resolved by the caller
/// from the share's "Programs it may run"; a folder stands for the programs
/// inside it, e.g. an agent's install folder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecRule {
    /// Only these (plus nothing else).
    Allow(Vec<PathBuf>),
    /// Anything but these.
    Deny(Vec<PathBuf>),
}

impl Default for ExecRule {
    fn default() -> ExecRule {
        ExecRule::Allow(Vec::new())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Egress {
    /// The proxy's port on localhost; `None`: no network at all.
    pub proxy_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalEndpoint {
    UnixSocket(PathBuf),
    LocalPort(u16),
}

/// Whether this machine can host shared sessions, for Settings and the
/// Share dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    Ready,
    /// Fixable by the user, e.g. `bubblewrap` not installed, or Ubuntu's
    /// AppArmor user-namespace restriction (§6.3). `fix` says exactly what
    /// to do; Pitwall never weakens a system setting itself.
    Missing { what: String, fix: String },
    /// Not on this OS (yet): this machine can ask but not host.
    Unsupported { why: String },
}

/// An agent's launch, wrapped for the holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapped {
    /// Run this instead of the agent's argv (`sandbox-exec -f <profile> …`).
    pub argv: Vec<String>,
    /// The process's whole environment: the holder starts it with nothing
    /// else.
    pub env: BTreeMap<String, String>,
}

/// One OS's sandbox. Every descendant of the wrapped process stays inside
/// it, including one that double-forks and detaches.
pub trait SessionSandbox: Send + Sync {
    /// For display and logs (`seatbelt`, `bubblewrap`).
    fn name(&self) -> &str;

    fn support(&self) -> Support;

    /// Wrap `argv` so it runs under `policy`, writing what the backend
    /// needs into `policy.state_dir`. Fails rather than run unconfined.
    fn wrap(&self, policy: &SandboxPolicy, argv: &[String]) -> Result<Wrapped>;

    /// What sessions wrapped with `tag` tried and were refused since
    /// `since`, as the OS reported it (macOS: the sandbox's violation
    /// reports in the unified log). Empty where the backend can't tell.
    fn denials(&self, tag: &str, since: std::time::SystemTime) -> Vec<Denial> {
        let _ = (tag, since);
        Vec::new()
    }
}

/// This machine's sandbox backend: Seatbelt on macOS; elsewhere one whose
/// [`support`](SessionSandbox::support) is `Unsupported` and that never
/// wraps (Linux in C3, Windows in C5).
pub fn session_sandbox() -> Box<dyn SessionSandbox> {
    crate::platform::session_sandbox()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(op: &str, target: &str) -> Denial {
        Denial { process: "sh".into(), operation: op.into(), target: target.into() }
    }

    #[test]
    fn tried_lines_say_what_was_refused_once_each() {
        let home = std::path::Path::new("/Users/alice");
        let lines = tried_lines(
            &[
                d("file-read-metadata", "/opt/homebrew/opt/openjdk@21"),
                d("file-write-data", "/dev/dtracehelper"),
                d("file-read-data", "/Users/alice/.ssh/id_ed25519"),
                d("file-read-data", "/Users/alice/.ssh/id_ed25519"),
                d("ipc-posix-shm-read-data", "apple.cfprefs.501v1"),
                d("file-write-create", "/Users/alice/code/api/README.md"),
                d("process-exec*", "/usr/bin/curl"),
                d("network-outbound", "93.184.216.34:443"),
                d("network-outbound", "/private/var/run/syslog"),
                d("file-read-data", "/Users/alicex/notes"),
            ],
            home,
        );
        assert_eq!(
            lines,
            [
                "read ~/.ssh/id_ed25519: denied by the sandbox",
                "write ~/code/api/README.md: denied by the sandbox",
                "run /usr/bin/curl: denied by the sandbox",
                "connect to 93.184.216.34:443: denied by the sandbox",
                "read /Users/alicex/notes: denied by the sandbox",
            ]
        );
    }
}
