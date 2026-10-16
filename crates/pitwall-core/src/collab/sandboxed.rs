//! The core's session seams backed by the sandbox module (§6.2, §6.3):
//!
//! - [`GitSnapshots`]: a session's copy is [`crate::sandbox::snapshot::build`]
//!   (a one-commit repository of the scope, the owner's blob ids, no link
//!   back);
//! - [`ProxyPool`]: one [`EgressProxy`] per share, its hosts updated as the
//!   share changes; what it refused is kept per share (the guest probes'
//!   "tried" lines).
//!
//! The OS sandbox itself is `sandbox::session_sandbox()`; the policy is
//! built in `core` from kind data (`sandbox::kind::resolve`) and the
//! environment from nothing (`sandbox::env::build`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use super::session::{EgressProxies, Snapshot, Snapshots};
use crate::error::Result;
use crate::exec::Exec;
use crate::sandbox::proxy::{EgressProxy, Refusal};
use crate::sandbox::snapshot::{self, SnapshotSpec};
use pitwall_proto::collab::{Scope, ShareSettings};

/// Copies made with `git` from the owner's repository (read only there).
pub struct GitSnapshots {
    exec: Arc<dyn Exec>,
}

impl GitSnapshots {
    pub fn new(exec: Arc<dyn Exec>) -> GitSnapshots {
        GitSnapshots { exec }
    }
}

/// A share's scope as the snapshot builder takes it (`~/` is the owner's
/// home).
pub fn snapshot_spec(scope: &Scope) -> SnapshotSpec {
    SnapshotSpec {
        repo: crate::paths::expand_tilde(&scope.repo).to_string_lossy().into_owned(),
        git_ref: scope.git_ref.clone(),
        paths: scope.paths.clone(),
        exclude: scope.exclude.clone(),
        uncommitted: scope.uncommitted,
    }
}

impl Snapshots for GitSnapshots {
    fn make(&self, scope: &Scope, dest: &Path) -> Result<Snapshot> {
        let snap = snapshot::build(&*self.exec, &snapshot_spec(scope), &dest.to_string_lossy())?;
        for skipped in &snap.skipped {
            eprintln!("pitwall: a session's copy left out {skipped}");
        }
        Ok(Snapshot { base_commit: snap.source, files: snap.files.len() as u32 })
    }
}

/// What the sensitive-file scan of a share's scope found (§5.4 step 3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeScan {
    /// Flagged files of the scope, before its exclusions (so the owner sees
    /// the ones excluded too), relative to the repository's top.
    pub findings: Vec<crate::sandbox::scan::Finding>,
    /// Files a copy would hold after the exclusions.
    pub files: u32,
}

/// Scan the files a copy of `scope` would hold, as the copy holds them:
/// tracked files at the scope's ref ([`snapshot::scope_files`]), names and
/// content ([`crate::sandbox::scan::scan`]). Untracked files never reach a
/// copy, so they aren't listed.
pub fn scan_scope(exec: &dyn Exec, scope: &Scope) -> Result<ScopeScan> {
    let mut spec = snapshot_spec(scope);
    let exclude = std::mem::take(&mut spec.exclude);
    let entries = snapshot::scope_files(exec, &spec)?;
    let out = exec.run(&crate::exec::Cmd::new(&["git", "-C", &spec.repo, "rev-parse", "--show-toplevel"]))?;
    let top = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
    let findings = crate::sandbox::scan::scan(exec, &top, &paths);
    let excluded = |p: &str| {
        exclude.iter().any(|e| {
            let e = e.trim_start_matches("./").trim_end_matches('/');
            !e.is_empty() && (p == e || p.starts_with(&format!("{e}/")))
        })
    };
    let files = paths.iter().filter(|p| !excluded(p)).count() as u32;
    Ok(ScopeScan { findings, files })
}

/// One egress proxy per share, started when its first session needs the
/// network.
#[derive(Default)]
pub struct ProxyPool {
    proxies: Mutex<HashMap<String, EgressProxy>>,
}

impl ProxyPool {
    pub fn new() -> ProxyPool {
        ProxyPool::default()
    }

    fn map(&self) -> MutexGuard<'_, HashMap<String, EgressProxy>> {
        self.proxies.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// What the proxy of `share` refused since `since`, as "tried" lines
    /// (`fetch example.com:443: host not allowed`).
    pub fn refused_since(&self, share: &str, since: SystemTime) -> Vec<String> {
        let refusals: Vec<Refusal> = self.map().get(share).map(|p| p.refusals()).unwrap_or_default();
        refusals.into_iter().filter(|r| r.at >= since).map(|r| format!("connect to {}: {}", r.target, r.why)).collect()
    }

}

impl EgressProxies for ProxyPool {
    fn port(&self, share: &ShareSettings, hosts: &[String]) -> Result<Option<u16>> {
        let mut map = self.map();
        if hosts.is_empty() {
            // No host to reach: no proxy, so no network at all.
            map.remove(&share.slug);
            return Ok(None);
        }
        if let Some(p) = map.get(&share.slug) {
            p.set_hosts(hosts)?;
            return Ok(Some(p.port()));
        }
        let proxy = EgressProxy::start(hosts, None)?;
        let port = proxy.port();
        map.insert(share.slug.clone(), proxy);
        Ok(Some(port))
    }

    fn forget(&self, share: &str) {
        self.map().remove(share);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn share(slug: &str) -> ShareSettings {
        serde_json::from_value(serde_json::json!({
            "agent": "a1", "slug": slug, "name": slug, "description": "", "scope": {"repo": "/x", "ref": "main"}
        }))
        .unwrap()
    }

    #[test]
    fn one_proxy_per_share_and_none_without_hosts() {
        let pool = ProxyPool::new();
        let a = pool.port(&share("a"), &["api.example.com".into()]).unwrap().unwrap();
        assert_eq!(pool.port(&share("a"), &["api.example.com".into(), "*.example.org".into()]).unwrap(), Some(a), "kept, hosts updated");
        let b = pool.port(&share("b"), &["api.example.com".into()]).unwrap().unwrap();
        assert_ne!(a, b);
        assert_eq!(pool.port(&share("b"), &[]).unwrap(), None);
        assert!(pool.port(&share("c"), &["not a host!".into()]).is_err());
    }

    #[test]
    fn copies_come_from_the_snapshot_builder() {
        let dir = TempDir::new("gitsnap");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/lib.rs"), "fn main() {}\n").unwrap();
        std::fs::write(repo.join(".env"), "CANARY=1\n").unwrap();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "one"]);
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let scope = Scope { repo: repo.to_string_lossy().into_owned(), git_ref: "main".into(), exclude: vec![".env".into()], ..Scope::default() };
        let snap = GitSnapshots::new(Arc::new(crate::exec::LocalExec)).make(&scope, &work).unwrap();
        assert_eq!(snap.files, 1);
        assert_eq!(snap.base_commit.len(), 40);
        assert!(work.join("src/lib.rs").is_file());
        assert!(!work.join(".env").exists());
        // The scan lists the excluded .env too (the owner sees why), and
        // counts what the copy holds.
        let scanned = scan_scope(&crate::exec::LocalExec, &scope).unwrap();
        assert_eq!(scanned.findings.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), [".env"]);
        assert_eq!(scanned.files, 1);
    }
}
