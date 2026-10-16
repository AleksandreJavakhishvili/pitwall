//! The macOS backend of `sandbox::SessionSandbox`: a Seatbelt profile per
//! session, run with `sandbox-exec` (docs/spec/collaboration/sessions.md
//! §6.3). Deny by default; then only:
//!
//! - **files**: the system's read-only parts (below), the policy's `read`
//!   and `write` subtrees, the tty the holder gave it, a few `/dev` nodes,
//!   and `stat` of each allowed path's parent folders (so path lookups
//!   work) and nothing else;
//! - **programs**: `process-exec` for the policy's allowlist (or anything
//!   but its denylist), each path also as its resolved target;
//! - **network**: outbound to the egress proxy's localhost port and the
//!   policy's local endpoints (the hook socket), nothing else; no inbound;
//! - **IPC**: the Mach services in [`BASE_SERVICES`] (user and group
//!   lookups, notifications, logging, TLS trust evaluation), the Keychain's
//!   only with `credential_store`; signals and process info only within
//!   the sandbox.
//!
//! The sandbox is inherited by every descendant (fork, exec, a detached
//! double fork) and can't be lifted from inside: Seatbelt refuses to apply
//! a second profile in a sandboxed process, so an agent's own Seatbelt
//! sandbox (Claude Code's Bash sandbox, Codex's, Gemini's
//! `SEATBELT_PROFILE`) fails inside this one and must be left off
//! (checked on macOS 26.5: `sandbox_apply: Operation not permitted`).
//!
//! **Unverified** (C1a smoke runs per kind): the Mach services each real
//! agent needs (the list below is what `sh`, `git` and `node` needed on
//! macOS 26.5), the Keychain services for Claude Code's login, and how each
//! agent behaves behind the proxy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::error::{ErrorCode, PwError, Result};
use crate::sandbox::{ExecRule, LocalEndpoint, SandboxPolicy, SessionSandbox, Support, Wrapped};

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
/// Starts `sandbox-exec` with exactly the session's environment, whatever
/// the holder inherited.
const ENV: &str = "/usr/bin/env";
/// The profile's file name in `SandboxPolicy::state_dir`.
pub(crate) const PROFILE_FILE: &str = "seatbelt.sb";
/// The Command Line Tools, where Apple's `/usr/bin` developer shims point
/// when Xcode isn't selected.
const CLT: &str = "/Library/Developer/CommandLineTools";

/// Read-only parts of the system every process needs: programs, libraries,
/// the dyld cache, `/etc` (public configuration: hosts, zone info, shells),
/// Homebrew's and `/usr/local`'s program folders (not their `etc` or `var`,
/// which hold configuration and databases). Missing ones are left out.
const SYSTEM_READ: &[&str] = &[
    "/bin",
    "/sbin",
    "/usr/bin",
    "/usr/sbin",
    "/usr/lib",
    "/usr/libexec",
    "/usr/share",
    "/usr/standalone",
    "/System",
    "/private/etc",
    "/private/var/db/dyld",
    "/private/var/db/timezone",
    "/private/var/select",
    CLT,
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/opt/homebrew/lib",
    "/opt/homebrew/libexec",
    "/opt/homebrew/share",
    "/opt/homebrew/opt",
    "/opt/homebrew/Cellar",
    "/opt/homebrew/Caskroom",
    "/opt/homebrew/Frameworks",
    "/usr/local/bin",
    "/usr/local/sbin",
    "/usr/local/lib",
    "/usr/local/libexec",
    "/usr/local/share",
    "/usr/local/opt",
    "/usr/local/Cellar",
    "/usr/local/Caskroom",
];

/// Symlinks at the top of the disk that paths go through.
const ROOT_LINKS: &[&str] = &["/etc", "/var", "/tmp"];

/// Device nodes any program may open.
const DEVICES: &[&str] = &[
    "/dev/null",
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
    "/dev/tty",
    "/dev/dtracehelper",
    "/dev/autofs_nowait",
];

/// Mach services every session may look up. Each one was needed by `sh`,
/// `git` or `node` on macOS 26.5 except the TLS trust ones, which system
/// TLS stacks (Security.framework) use to verify certificates (taken from
/// Codex's network-enabled Seatbelt policy; **unverified** per agent).
pub(crate) const BASE_SERVICES: &[&str] = &[
    // getpwuid/getgrgid (git, node's os.userInfo)
    "com.apple.system.opendirectoryd.libinfo",
    "com.apple.system.opendirectoryd.membership",
    // notify(3), used by libc on startup
    "com.apple.system.notification_center",
    // os_log
    "com.apple.logd",
    // confstr(_CS_DARWIN_USER_TEMP_DIR); the folders it names stay unreadable
    "com.apple.bsd.dirhelper",
    // certificate trust evaluation
    "com.apple.trustd.agent",
    "com.apple.ocspd",
];

/// The Keychain, for `credential_store` (Claude Code keeps a subscription
/// login there). **Unverified** which of these each agent needs.
const KEYCHAIN_SERVICES: &[&str] = &[
    "com.apple.SecurityServer",
    "com.apple.securityd.xpc",
    "com.apple.security.agent",
    "com.apple.secd",
];

/// sysctls a process may read: hardware and OS facts, and its own
/// process info. Not `kern.procargs2` (other processes' arguments and
/// environment). From Codex's base policy, trimmed.
const SYSCTL_NAMES: &[&str] = &[
    "kern.argmax",
    "kern.hostname",
    "kern.maxfilesperproc",
    "kern.maxproc",
    "kern.osproductversion",
    "kern.osrelease",
    "kern.ostype",
    "kern.osvariant_status",
    "kern.osversion",
    "kern.secure_kernel",
    "kern.usrstack64",
    "kern.version",
    "kern.boottime",
    "kern.ngroups",
    "kern.iossupportversion",
    "security.mac.lockdown_mode_state",
    "sysctl.proc_cputype",
    "sysctl.proc_translated",
    "vm.loadavg",
    "vm.pagesize",
];
const SYSCTL_PREFIXES: &[&str] = &["hw.", "kern.proc.pid.", "kern.proc.pgrp.", "machdep.cpu."];

pub struct Seatbelt {
    /// `sandbox-exec` can apply a profile here (not already inside one).
    works: OnceLock<std::result::Result<(), String>>,
    developer_dir: OnceLock<Option<PathBuf>>,
}

impl Seatbelt {
    pub fn new() -> Seatbelt {
        Seatbelt { works: OnceLock::new(), developer_dir: OnceLock::new() }
    }

    fn probe(&self) -> &std::result::Result<(), String> {
        self.works.get_or_init(|| {
            if !Path::new(SANDBOX_EXEC).exists() {
                return Err(format!("{SANDBOX_EXEC} is missing"));
            }
            let out = Command::new(SANDBOX_EXEC)
                .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .map_err(|e| format!("{SANDBOX_EXEC}: {e}"))?;
            if out.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
        })
    }

    /// The selected Xcode or Command Line Tools folder, where Apple's
    /// `/usr/bin` developer shims (`git`, `make`, `clang`) run the real
    /// tool from. Read from xcode-select's link, else asked once.
    fn developer_dir(&self) -> Option<&Path> {
        self.developer_dir
            .get_or_init(|| {
                let linked = std::fs::read_link("/private/var/db/xcode_select_link").ok();
                let asked = || {
                    Command::new("/usr/bin/xcode-select")
                        .arg("-p")
                        .stdin(Stdio::null())
                        .stderr(Stdio::null())
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
                };
                linked
                    .or_else(asked)
                    .filter(|p| p.is_dir())
                    .or_else(|| Some(PathBuf::from(CLT)).filter(|p| p.is_dir()))
            })
            .as_deref()
    }

    /// Everything the profile needs, resolved on this machine.
    fn resolve(&self, policy: &SandboxPolicy, program: &Path) -> Result<Resolved> {
        let dev = self.developer_dir().map(real);
        let shim_targets = |p: &Path| -> Vec<PathBuf> {
            let (Some(dev), Some(name)) = (&dev, p.file_name()) else { return vec![] };
            if p.parent() != Some(Path::new("/usr/bin")) {
                return vec![];
            }
            [dev.join("usr/bin").join(name), Path::new(CLT).join("usr/bin").join(name)]
                .into_iter()
                .filter(|t| t.is_file())
                .collect()
        };
        let mut shimmed = false;
        let mut expand = |paths: &[PathBuf]| -> Vec<PathBuf> {
            let mut out = Vec::new();
            for p in paths {
                out.push(p.clone());
                out.push(real(p));
                let targets = shim_targets(p);
                shimmed |= !targets.is_empty();
                out.extend(targets.iter().map(|t| real(t)));
            }
            out.sort();
            out.dedup();
            out
        };
        let exec = match &policy.exec {
            ExecRule::Allow(list) => ExecRule::Allow(expand(list)),
            ExecRule::Deny(list) => ExecRule::Deny(expand(list)),
        };
        let real_program = real(program);
        let covers = |p: &PathBuf| p == program || *p == real_program || (p.is_dir() && real_program.starts_with(p));
        let allowed = match &exec {
            ExecRule::Allow(list) => list.iter().any(covers),
            ExecRule::Deny(list) => !list.iter().any(covers),
        };
        if !allowed {
            return Err(PwError::new(
                ErrorCode::Denied,
                format!("{} isn't in the session's programs list", program.display()),
            ));
        }
        let mut system: Vec<PathBuf> = SYSTEM_READ.iter().map(PathBuf::from).filter(|p| p.exists()).map(|p| real(&p)).collect();
        if let Some(dev) = &dev {
            system.push(dev.clone());
        }
        let mut env = policy.env.clone();
        if shimmed {
            // Apple's shims run `xcrun`, which needs the owner's per-user
            // cache folder; the real tools come first on PATH instead.
            if let Some(dev) = &dev {
                let bin = dev.join("usr/bin").to_string_lossy().into_owned();
                let path = env.get("PATH").cloned().unwrap_or_default();
                if !path.split(':').any(|d| d == bin) {
                    env.insert("PATH".into(), if path.is_empty() { bin } else { format!("{bin}:{path}") });
                }
            }
        }
        Ok(Resolved {
            read: policy.read.iter().map(|p| real(p)).collect(),
            write: policy.write.iter().map(|p| real(p)).collect(),
            system,
            exec,
            proxy_port: policy.egress.proxy_port,
            connect: policy
                .connect
                .iter()
                .map(|c| match c {
                    LocalEndpoint::UnixSocket(p) => LocalEndpoint::UnixSocket(real(p)),
                    LocalEndpoint::LocalPort(n) => LocalEndpoint::LocalPort(*n),
                })
                .collect(),
            credential_store: policy.credential_store,
            home: crate::platform::home_dir(),
            env,
        })
    }
}

impl Default for Seatbelt {
    fn default() -> Seatbelt {
        Seatbelt::new()
    }
}

impl SessionSandbox for Seatbelt {
    fn name(&self) -> &str {
        "seatbelt"
    }

    fn support(&self) -> Support {
        match self.probe() {
            Ok(()) => Support::Ready,
            Err(why) => Support::Missing {
                what: format!("macOS's sandbox can't be applied here ({why})"),
                fix: "run Pitwall as a normal app, not from inside another sandbox".into(),
            },
        }
    }

    fn wrap(&self, policy: &SandboxPolicy, argv: &[String]) -> Result<Wrapped> {
        if let Err(why) = self.probe() {
            return Err(PwError::unsupported(format!("can't sandbox the session: {why}")));
        }
        let Some(first) = argv.first() else {
            return Err(PwError::other("nothing to run"));
        };
        let program = find_program(first, policy.env.get("PATH").map(String::as_str))
            .ok_or_else(|| PwError::not_found(format!("{first}: not found on the session's PATH")))?;
        if !policy.state_dir.is_absolute() {
            return Err(PwError::other("the sandbox's state folder must be an absolute path"));
        }
        for key in policy.env.keys() {
            if key.is_empty() || key.contains('=') || key.contains('\0') {
                return Err(PwError::other(format!("bad environment variable name {key:?}")));
            }
        }
        let resolved = self.resolve(policy, &program)?;
        let profile = render(&resolved);
        create_private_dir(&policy.state_dir).map_err(|e| PwError::other(format!("{}: {e}", policy.state_dir.display())))?;
        let path = policy.state_dir.join(PROFILE_FILE);
        std::fs::write(&path, profile).map_err(|e| PwError::other(format!("{}: {e}", path.display())))?;

        let mut out = vec![ENV.to_string(), "-i".to_string()];
        out.extend(resolved.env.iter().map(|(k, v)| format!("{k}={v}")));
        out.extend([SANDBOX_EXEC.to_string(), "-f".to_string(), path.to_string_lossy().into_owned()]);
        out.push(program.to_string_lossy().into_owned());
        out.extend(argv[1..].iter().cloned());
        Ok(Wrapped { argv: out, env: resolved.env })
    }
}

/// The policy with every path resolved (Seatbelt matches real paths:
/// `/var/folders/…` is `/private/var/folders/…`).
struct Resolved {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
    system: Vec<PathBuf>,
    exec: ExecRule,
    proxy_port: Option<u16>,
    connect: Vec<LocalEndpoint>,
    credential_store: bool,
    home: PathBuf,
    env: BTreeMap<String, String>,
}

/// `path` with symlinks resolved; for a path that doesn't exist yet, its
/// nearest existing parent resolved plus the rest.
fn real(path: &Path) -> PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p;
    }
    let mut rest = Vec::new();
    let mut cur = path;
    while let Some(parent) = cur.parent() {
        if let Some(name) = cur.file_name() {
            rest.push(name.to_owned());
        }
        if let Ok(p) = std::fs::canonicalize(parent) {
            return rest.iter().rev().fold(p, |acc, n| acc.join(n));
        }
        cur = parent;
    }
    path.to_path_buf()
}

/// `name` as an absolute program path: as given when it has a `/`, else
/// the first executable match on `path` (the session's `PATH`).
fn find_program(name: &str, path: Option<&str>) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return (p.is_absolute() && super::is_executable(&p)).then_some(p);
    }
    path?
        .split(':')
        .filter(|d| d.starts_with('/'))
        .map(|d| Path::new(d).join(name))
        .find(|p| super::is_executable(p))
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// A string in the profile language (a Scheme string literal).
fn q(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn qp(p: &Path) -> String {
    q(&p.to_string_lossy())
}

/// Every parent folder of `paths` (not the paths themselves).
fn ancestors<'a>(paths: impl Iterator<Item = &'a PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = paths.flat_map(|p| p.ancestors().skip(1).map(Path::to_path_buf).collect::<Vec<_>>()).collect();
    out.sort();
    out.dedup();
    out
}

fn rule(out: &mut String, head: &str, filters: impl IntoIterator<Item = String>) {
    let filters: Vec<String> = filters.into_iter().collect();
    if filters.is_empty() {
        return;
    }
    out.push_str(head);
    for f in filters {
        out.push_str("\n  ");
        out.push_str(&f);
    }
    out.push_str(")\n");
}

/// The profile text.
fn render(r: &Resolved) -> String {
    let mut s = String::from(
        "; Pitwall shared session (docs/spec/collaboration/sessions.md §6.3). Generated; do not edit.\n\
         (version 1)\n\
         (deny default)\n\n\
         ; Processes: children inherit this profile; signals and process info stay inside it.\n\
         (allow process-fork)\n\
         (allow signal (target same-sandbox))\n\
         (allow process-info* (target same-sandbox))\n",
    );
    let lit = |p: &PathBuf| format!("(literal {})", qp(p));
    let sub = |p: &PathBuf| format!("(subpath {})", qp(p));
    // A folder in the programs list stands for the programs inside it.
    let program = |p: &PathBuf| if p.is_dir() { sub(p) } else { lit(p) };
    match &r.exec {
        ExecRule::Allow(list) => rule(&mut s, "(allow process-exec", list.iter().map(program)),
        ExecRule::Deny(list) => {
            s.push_str("(allow process-exec)\n");
            rule(&mut s, "(deny process-exec", list.iter().map(program));
        }
    }

    s.push_str("\n; sysctls: hardware and OS facts, own process info.\n");
    rule(
        &mut s,
        "(allow sysctl-read",
        SYSCTL_NAMES
            .iter()
            .map(|n| format!("(sysctl-name {})", q(n)))
            .chain(SYSCTL_PREFIXES.iter().map(|n| format!("(sysctl-name-prefix {})", q(n)))),
    );

    s.push_str("\n; Files: the system, read only.\n");
    rule(&mut s, "(allow file-read*", r.system.iter().map(sub).chain(["(literal \"/\")".into()]));
    rule(&mut s, "(allow file-read*", ROOT_LINKS.iter().map(|p| format!("(literal {})", q(p))));
    s.push_str("; What the session may read and write.\n");
    rule(&mut s, "(allow file-read*", r.read.iter().map(sub));
    rule(&mut s, "(allow file-read* file-write*", r.write.iter().map(sub));
    s.push_str("; Path lookups through the parents of allowed paths (stat only, no listing).\n");
    let exec_paths: Vec<PathBuf> = match &r.exec {
        ExecRule::Allow(l) => l.clone(),
        ExecRule::Deny(_) => vec![],
    };
    let sockets: Vec<PathBuf> = r
        .connect
        .iter()
        .filter_map(|c| match c {
            LocalEndpoint::UnixSocket(p) => Some(p.clone()),
            LocalEndpoint::LocalPort(_) => None,
        })
        .collect();
    let all: Vec<&PathBuf> = r.system.iter().chain(&r.read).chain(&r.write).chain(&exec_paths).chain(&sockets).collect();
    rule(&mut s, "(allow file-read-metadata", ancestors(all.into_iter()).iter().map(lit));
    rule(&mut s, "(allow file-read-metadata", exec_paths.iter().map(program));

    s.push_str("\n; Devices, and the terminal the holder gave it (opened before the sandbox).\n");
    rule(&mut s, "(allow file-read* file-write-data file-ioctl", DEVICES.iter().map(|p| format!("(literal {})", q(p))));
    s.push_str(
        "(allow file-read-metadata (subpath \"/dev\"))\n\
         (allow file-read* (literal \"/dev\"))\n\
         (allow file-read* file-write-data (subpath \"/dev/fd\"))\n\
         (allow pseudo-tty)\n\
         (allow file-read* file-write* file-ioctl (literal \"/dev/ptmx\"))\n\
         (allow file-read* file-write* (require-all (regex #\"^/dev/ttys[0-9]+$\") (extension \"com.apple.sandbox.pty\")))\n\
         (allow file-ioctl (regex #\"^/dev/ttys[0-9]+$\"))\n",
    );

    s.push_str("\n; Network: only the egress proxy and Pitwall's hook endpoint, on this machine.\n");
    let mut net = Vec::new();
    if let Some(port) = r.proxy_port {
        net.push(format!("(remote ip {})", q(&format!("localhost:{port}"))));
    }
    for c in &r.connect {
        match c {
            LocalEndpoint::LocalPort(port) => net.push(format!("(remote ip {})", q(&format!("localhost:{port}")))),
            LocalEndpoint::UnixSocket(p) => net.push(format!("(remote unix-socket (path-literal {}))", qp(p))),
        }
    }
    rule(&mut s, "(allow network-outbound", net);

    s.push_str("\n; IPC: the system services programs need at startup.\n");
    let mut services: Vec<&str> = BASE_SERVICES.to_vec();
    if r.credential_store {
        services.extend(KEYCHAIN_SERVICES);
    }
    rule(&mut s, "(allow mach-lookup", services.iter().map(|n| format!("(global-name {})", q(n))));
    s.push_str("(allow ipc-posix-shm-read-data (ipc-posix-name \"apple.shm.notification_center\"))\n");
    if r.credential_store {
        let keychains = r.home.join("Library/Keychains");
        s.push_str("; The agent's own login in the Keychain (credential_store).\n");
        rule(&mut s, "(allow file-read* file-write*", [sub(&real(&keychains))]);
        rule(&mut s, "(allow file-read-metadata", ancestors([real(&keychains)].iter()).iter().map(lit));
    }
    s
}

#[cfg(test)]
mod tests;
