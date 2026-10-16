//! The "Isolation" tests (docs/spec/collaboration/plan.md §12), on the real
//! `sandbox-exec`, with canary files in temp folders standing in for the
//! owner's home, projects and Pitwall's data (never the real ones).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::sandbox::env::{self, EnvSpec};
use crate::sandbox::proxy::EgressProxy;
use crate::sandbox::Egress;
use crate::testing::TempDir;

const CANARY: &str = "PITWALL-CANARY-7f3a";

/// A session's folders beside a fake owner's.
struct World {
    _dir: TempDir,
    root: PathBuf,
    work: PathBuf,
    home: PathBuf,
    tmp: PathBuf,
    state: PathBuf,
    /// The fake owner's home: `.ssh/id_test`, a project, Pitwall's data.
    owner: PathBuf,
    other_session: PathBuf,
}

impl World {
    fn new(tag: &str) -> World {
        let dir = TempDir::new(tag);
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let s = root.join("collab/shares/api/s/1");
        let (work, home, tmp, state) = (s.join("work"), s.join("home"), s.join("tmp"), root.join("collab/state/1"));
        let owner = root.join("owner");
        let other_session = root.join("collab/shares/api/s/2/work");
        for d in [&work, &home, &tmp, &other_session, &owner.join(".ssh"), &owner.join("code/api"), &owner.join(".pitwall")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(work.join("README.md"), "the copy\n").unwrap();
        for f in [".ssh/id_test", "code/api/.env", ".pitwall/state.json", ".gitconfig"] {
            std::fs::write(owner.join(f), CANARY).unwrap();
        }
        std::fs::write(other_session.join("notes.md"), CANARY).unwrap();
        World { _dir: dir, root, work, home, tmp, state, owner, other_session }
    }

    fn policy(&self, write_copy: bool, programs: ExecRule, proxy: Option<u16>, connect: Vec<LocalEndpoint>) -> SandboxPolicy {
        let env = env::build(&EnvSpec {
            home: self.home.clone(),
            tmp: self.tmp.clone(),
            path: vec!["/usr/bin".into(), "/bin".into()],
            proxy_port: proxy,
            account: None,
            shell: Some("/bin/sh".into()),
            lang: None,
            pass: BTreeMap::new(),
        })
        .unwrap();
        let mut write = vec![self.home.clone(), self.tmp.clone()];
        if write_copy {
            write.push(self.work.clone());
        }
        SandboxPolicy {
            read: vec![self.work.clone()],
            write,
            exec: programs,
            egress: Egress { proxy_port: proxy },
            env,
            connect,
            state_dir: self.state.clone(),
            credential_store: false,
            tag: None,
        }
    }

    /// Run `script` with `/bin/sh -c` inside the sandbox, in the copy, with
    /// an owner variable set outside to prove it doesn't get in.
    fn run(&self, policy: &SandboxPolicy, script: &str) -> (bool, String, String) {
        let argv = vec!["sh".to_string(), "-c".to_string(), script.to_string()];
        let wrapped = Seatbelt::new().wrap(policy, &argv).expect("wrap");
        let out = Command::new(&wrapped.argv[0])
            .args(&wrapped.argv[1..])
            .current_dir(&self.work)
            .env("OWNER_TOKEN", CANARY)
            .env("SSH_AUTH_SOCK", self.owner.join("agent.sock"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

fn tools() -> Vec<PathBuf> {
    ["/bin/sh", "/bin/bash", "/bin/cat", "/bin/ls", "/bin/echo", "/bin/sleep", "/usr/bin/env", "/usr/bin/nc", "/usr/bin/touch"]
        .iter()
        .map(PathBuf::from)
        .collect()
}

fn with(extra: &[&str]) -> ExecRule {
    let mut t = tools();
    t.extend(extra.iter().map(PathBuf::from));
    ExecRule::Allow(t)
}

#[test]
fn profile_denies_by_default_and_names_only_the_session() {
    let w = World::new("sb-profile");
    let p = w.policy(false, with(&[]), Some(4100), vec![LocalEndpoint::UnixSocket(w.root.join("hook.sock"))]);
    let r = Seatbelt::new().resolve(&p, Path::new("/bin/sh")).unwrap();
    let text = render(&r);
    assert!(text.contains("(deny default)"));
    assert!(!text.contains("(allow default)"));
    assert!(!text.contains("(allow network-outbound)\n") && !text.contains("(allow network*"));
    assert!(text.contains("(remote ip \"localhost:4100\")"));
    assert!(text.contains(&format!("(subpath {})", qp(&w.work))));
    // The owner's folders appear only as parents (stat), never as subtrees.
    assert!(!text.contains(&format!("(subpath {})", qp(&w.owner))));
    assert!(!text.contains(&format!("(subpath {})", qp(&w.root))));
    assert!(!text.contains("com.apple.SecurityServer"), "no Keychain without credential_store");
    let keychain = SandboxPolicy { credential_store: true, ..p };
    assert!(render(&Seatbelt::new().resolve(&keychain, Path::new("/bin/sh")).unwrap()).contains("com.apple.SecurityServer"));
}

#[test]
fn wrap_refuses_programs_outside_the_list() {
    let w = World::new("sb-refuse");
    let p = w.policy(false, with(&[]), None, vec![]);
    let err = Seatbelt::new().wrap(&p, &["/usr/bin/curl".into()]).unwrap_err();
    assert!(err.is(ErrorCode::Denied), "{err}");
    assert!(Seatbelt::new().wrap(&p, &["no-such-program".into()]).is_err());
    assert!(Seatbelt::new().wrap(&p, &[]).is_err());
    // A bare name is found on the session's PATH; the env goes through `env -i`.
    let ok = Seatbelt::new().wrap(&p, &["cat".into(), "x".into()]).unwrap();
    assert_eq!(ok.argv[..2], ["/usr/bin/env".to_string(), "-i".to_string()]);
    let tail = &ok.argv[ok.argv.len() - 5..];
    assert_eq!(tail[0], "/usr/bin/sandbox-exec");
    assert_eq!(&tail[3..], ["/bin/cat".to_string(), "x".to_string()]);
    assert!(w.state.join(PROFILE_FILE).is_file());
}

#[test]
fn reads_outside_the_copy_fail_and_the_copy_reads() {
    let w = World::new("sb-read");
    let p = w.policy(false, with(&[]), None, vec![]);
    let (ok, out, _) = w.run(&p, "cat README.md && ls");
    assert!(ok, "reading the copy works");
    assert!(out.contains("the copy") && out.contains("README.md"));
    for path in [
        w.owner.join(".ssh/id_test"),
        w.owner.join("code/api/.env"),
        w.owner.join(".pitwall/state.json"),
        w.owner.join(".gitconfig"),
        w.other_session.join("notes.md"),
        // the same file through the data volume's firmlink (`/System` is readable)
        Path::new("/System/Volumes/Data").join(w.owner.join(".ssh/id_test").strip_prefix("/").unwrap()),
    ] {
        let (ok, out, err) = w.run(&p, &format!("cat '{}'", path.display()));
        assert!(!ok && !out.contains(CANARY), "{} was readable: {out}", path.display());
        assert!(err.contains("Operation not permitted"), "{err}");
    }
    for dir in [&w.owner, &w.owner.join(".ssh"), &w.root, &w.other_session] {
        let (ok, out, _) = w.run(&p, &format!("ls -a '{}'", dir.display()));
        assert!(!ok && !out.contains("id_test") && !out.contains("notes.md"), "{} was listable: {out}", dir.display());
    }
}

#[test]
fn writes_fail_at_read_only_and_outside_the_copy() {
    let w = World::new("sb-write");
    let ro = w.policy(false, with(&[]), None, vec![]);
    let (ok, _, _) = w.run(&ro, "echo x > new.txt");
    assert!(!ok && !w.work.join("new.txt").exists(), "created a file in a read-only copy");
    let (ok, _, _) = w.run(&ro, "echo x >> README.md");
    assert!(!ok);
    assert_eq!(std::fs::read_to_string(w.work.join("README.md")).unwrap(), "the copy\n");
    let (ok, _, _) = w.run(&ro, "echo x > \"$HOME/scratch\" && echo y > \"$TMPDIR/t\"");
    assert!(ok, "its own HOME and TMPDIR are writable");

    let rw = w.policy(true, with(&[]), None, vec![]);
    let (ok, _, _) = w.run(&rw, "echo x > new.txt && echo more >> README.md");
    assert!(ok && w.work.join("new.txt").exists(), "writable at Can change code");
    for target in [w.owner.join("planted"), w.other_session.join("planted"), w.root.join("planted"), w.owner.join(".ssh/id_test")] {
        let (ok, _, _) = w.run(&rw, &format!("echo x > '{}'", target.display()));
        assert!(!ok, "wrote {}", target.display());
    }
    assert!(!w.owner.join("planted").exists() && !w.root.join("planted").exists());
    assert_eq!(std::fs::read_to_string(w.owner.join(".ssh/id_test")).unwrap(), CANARY);
}

#[test]
fn only_listed_programs_run() {
    let w = World::new("sb-exec");
    let allow = w.policy(false, with(&[]), None, vec![]);
    let (ok, _, err) = w.run(&allow, "/usr/bin/curl --version");
    assert!(!ok && err.contains("Operation not permitted"), "{err}");
    let (ok, out, _) = w.run(&allow, "/bin/echo allowed");
    assert!(ok && out.contains("allowed"));

    let deny = w.policy(false, ExecRule::Deny(vec!["/usr/bin/curl".into(), "/usr/bin/nc".into()]), None, vec![]);
    let (ok, _, _) = w.run(&deny, "/usr/bin/curl --version");
    assert!(!ok);
    let (ok, out, _) = w.run(&deny, "/usr/bin/uname");
    assert!(ok && !out.is_empty(), "everything else runs with a denylist");
}

/// A local server that counts connections.
fn tcp_server() -> (u16, Arc<AtomicUsize>) {
    let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = l.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    std::thread::spawn(move || {
        for mut c in l.incoming().flatten() {
            h.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 2048];
            let _ = c.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = c.read(&mut buf);
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nreached");
        }
    });
    (port, hits)
}

#[test]
fn network_only_through_the_proxy_and_only_to_allowed_hosts() {
    let w = World::new("sb-net");
    let (target, hits) = tcp_server();
    let proxy = EgressProxy::start(&[format!("127.0.0.1:{target}")], None).unwrap();
    let p = w.policy(false, with(&["/usr/bin/curl"]), Some(proxy.port()), vec![]);

    // Direct: refused by the sandbox, whatever the address.
    let (ok, _, _) = w.run(&p, &format!("/usr/bin/nc -z -w 2 127.0.0.1 {target}"));
    assert!(!ok);
    let (ok, _, _) = w.run(&p, &format!("/usr/bin/curl -sS --noproxy '*' -m 5 http://127.0.0.1:{target}/"));
    assert!(!ok);
    let (ok, _, _) = w.run(&p, "/usr/bin/curl -sS --noproxy '*' -m 5 https://example.com/");
    assert!(!ok);
    assert_eq!(hits.load(Ordering::SeqCst), 0, "a direct connection got through");

    // Through the proxy (HTTPS_PROXY/HTTP_PROXY from the environment).
    let (ok, out, err) = w.run(&p, &format!("/usr/bin/curl -sS -m 5 http://127.0.0.1:{target}/"));
    assert!(ok && out == "reached", "{out} {err}");
    let (ok, out, err) = w.run(&p, &format!("/usr/bin/curl -sS -m 5 -p http://127.0.0.1:{target}/"));
    assert!(ok && out == "reached", "CONNECT tunnel: {out} {err}");
    // Not listed: the proxy says no.
    let (ok, out, _) = w.run(&p, "/usr/bin/curl -s -m 5 -o /dev/null -w '%{http_code}' http://example.com/");
    assert!(!ok || out == "403", "{out}");
    let (ok, _, _) = w.run(&p, "/usr/bin/curl -sS -m 5 https://example.com/");
    assert!(!ok);
    let refused: Vec<String> = proxy.refusals().into_iter().map(|r| r.target).collect();
    assert!(refused.iter().any(|t| t == "example.com:80") && refused.iter().any(|t| t == "example.com:443"), "{refused:?}");

    // No proxy at all: no network at all.
    let none = w.policy(false, with(&["/usr/bin/curl"]), None, vec![]);
    let (ok, _, _) = w.run(&none, &format!("/usr/bin/curl -sS -m 5 -x {} http://127.0.0.1:{target}/", proxy.url()));
    assert!(!ok);
}

#[test]
fn the_environment_holds_nothing_of_the_owners() {
    let w = World::new("sb-env");
    let p = w.policy(false, with(&[]), Some(4100), vec![]);
    let (ok, out, _) = w.run(&p, "/usr/bin/env");
    assert!(ok);
    let mut got: Vec<&str> = out.lines().map(|l| l.split('=').next().unwrap()).collect();
    got.sort_unstable();
    // `PWD`, `SHLVL` and `_` are the shell's own.
    got.retain(|k| !["PWD", "SHLVL", "_", "OLDPWD"].contains(k));
    let mut want: Vec<&str> = p.env.keys().map(String::as_str).collect();
    want.sort_unstable();
    assert_eq!(got, want);
    assert!(!out.contains(CANARY) && !out.contains("SSH_AUTH_SOCK") && !out.contains("PITWALL_"));
    assert!(out.contains(&format!("HOME={}", w.home.display())));
}

/// A short folder in /private/tmp, removed on drop.
struct ShortDir(PathBuf);

impl ShortDir {
    fn new() -> ShortDir {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let dir = PathBuf::from(format!("/private/tmp/pwsb-{}", &id[..8]));
        std::fs::create_dir_all(&dir).unwrap();
        ShortDir(dir)
    }
}

impl Drop for ShortDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A detached, double-forked child is still sandboxed: it can't reach a
/// fake `pitwalld.sock`, while the hook socket (allowed) works.
#[test]
fn a_double_forked_child_stays_inside() {
    let w = World::new("sb-fork");
    // Socket paths must be short (104 bytes on macOS).
    let short = ShortDir::new();
    let daemon_sock = short.0.join("pitwalld.sock");
    let hook_sock = short.0.join("hook.sock");
    let count = |path: &Path| {
        let l = UnixListener::bind(path).unwrap();
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                n2.fetch_add(1, Ordering::SeqCst);
                drop(c);
            }
        });
        n
    };
    let daemon = count(&daemon_sock);
    let hook = count(&hook_sock);
    let p = w.policy(false, with(&[]), None, vec![LocalEndpoint::UnixSocket(hook_sock.clone())]);
    let script = format!(
        "( ( sleep 1; /usr/bin/nc -U '{d}' </dev/null; echo \"daemon=$?\" > \"$HOME/daemon\"; \
           cat '{secret}' > \"$HOME/secret\" 2>/dev/null; \
           /usr/bin/nc -U '{h}' </dev/null; echo \"hook=$?\" > \"$HOME/hook\" ) & ) ; exit 0",
        d = daemon_sock.display(),
        h = hook_sock.display(),
        secret = w.owner.join(".ssh/id_test").display(),
    );
    let (ok, _, err) = w.run(&p, &script);
    assert!(ok, "{err}");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !w.home.join("hook").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(200));
    let read = |f: &str| std::fs::read_to_string(w.home.join(f)).unwrap_or_default();
    assert_eq!(read("hook").trim(), "hook=0", "the detached child ran and could use the hook socket");
    assert_ne!(read("daemon").trim(), "daemon=0");
    assert_eq!(daemon.load(Ordering::SeqCst), 0, "reached pitwalld.sock");
    assert_eq!(hook.load(Ordering::SeqCst), 1);
    assert!(!read("secret").contains(CANARY));
}

/// Opt-in smoke run of the installed agents' `--version` (and `node`)
/// inside the sandbox, with their `[share]` data:
/// `cargo test -p pitwall-core seatbelt::tests::smoke -- --ignored --nocapture`.
/// Starts no conversation and uses no login.
#[test]
#[ignore]
fn smoke_installed_agents() {
    let w = World::new("sb-smoke");
    let home = crate::platform::home_dir();
    let search = std::env::var("PATH").unwrap_or_default();
    let kinds = crate::kind::load_kinds(Path::new("/nonexistent/pitwall-agents"));
    for id in ["claude", "codex", "gemini"] {
        let kind = kinds.iter().find(|k| k.id == id).unwrap();
        let Some(program) = crate::sandbox::kind::which(&kind.command, &search) else {
            println!("{id}: not installed");
            continue;
        };
        let install = crate::sandbox::kind::resolve(kind, &home, Some(&program), &search);
        let mut p = w.policy(false, with(&[]), Some(9), vec![]);
        p.read.extend(install.read.iter().cloned());
        let ExecRule::Allow(list) = &mut p.exec else { unreachable!() };
        list.extend(install.exec.iter().cloned());
        let path: Vec<String> = install.path.iter().map(|d| d.to_string_lossy().into_owned()).collect();
        p.env.insert("PATH".into(), format!("{}:/usr/bin:/bin", path.join(":")));
        let (ok, out, err) = w.run(&p, &format!("'{}' --version", program.display()));
        println!("{id}: ok={ok} out={} err={}", out.trim(), err.trim());
    }
}

/// The kernel's reports, as `log show` gives them (recorded on macOS 26.5
/// from a tagged profile; paths replaced).
#[test]
fn violation_reports_parse_into_denials() {
    let d = parse_report("Sandbox: cat(13306) deny(1) file-read-data /Users/alice/.ssh/id_ed25519\npitwall-c_1").unwrap();
    assert_eq!((d.process.as_str(), d.operation.as_str(), d.target.as_str()), ("cat", "file-read-data", "/Users/alice/.ssh/id_ed25519"));
    let d = parse_report("1 duplicate report for Sandbox: bash(13304) deny(1) file-write-create /x/new file.txt\npitwall-c_1").unwrap();
    assert_eq!((d.operation.as_str(), d.target.as_str()), ("file-write-create", "/x/new file.txt"));
    let d = parse_report("Sandbox: curl(13308) deny(1) network-outbound 93.184.216.34:443\npitwall-c_1").unwrap();
    assert_eq!(d.operation, "network-outbound");
    assert!(parse_report("log run noninteractively, parent: 1 (zsh)").is_none());
    assert!(!valid_tag("a\" OR 1==1") && valid_tag("pitwall-c_1a2b"));
}

/// A tagged session's denials come back from the OS's reports, and only
/// its own: the guest probes' "tried" lines (§5.4).
#[test]
fn a_tagged_sessions_denials_are_read_back() {
    let w = World::new("sb-denials");
    let tag = format!("pitwall-test-{}", uuid::Uuid::new_v4().simple());
    let mut p = w.policy(false, with(&[]), None, vec![]);
    p.tag = Some(tag.clone());
    let profile = {
        Seatbelt::new().wrap(&p, &["sh".to_string()]).unwrap();
        std::fs::read_to_string(w.state.join(PROFILE_FILE)).unwrap()
    };
    assert!(profile.contains(&format!("(deny default (with message \"{tag}\"))")), "{profile}");
    let since = std::time::SystemTime::now() - Duration::from_secs(2);
    let key = w.owner.join(".ssh/id_test");
    w.run(&p, &format!("cat '{}'; echo hi > '{}/x.txt'", key.display(), w.owner.display()));
    // The unified log takes a moment.
    let deadline = Instant::now() + Duration::from_secs(20);
    let real_key = std::fs::canonicalize(&key).unwrap().to_string_lossy().into_owned();
    let lines = loop {
        let lines = crate::sandbox::tried_lines(&Seatbelt::new().denials(&tag, since), Path::new("/nonexistent-home"));
        if lines.iter().any(|l| l.contains(&real_key)) || Instant::now() > deadline {
            break lines;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    assert!(lines.contains(&format!("read {real_key}: denied by the sandbox")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("write ") && l.contains("x.txt")), "{lines:?}");
    // Another tag: nothing.
    assert!(Seatbelt::new().denials("pitwall-test-nobody-0", since).is_empty());
}
