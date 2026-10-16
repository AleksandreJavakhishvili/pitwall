//! The sensitive-file scan of a share's scope (collaboration/sharing.md
//! §5.4 step 3): files a session's copy should leave out, each with why,
//! ticked for exclusion in the Share dialog and excluded by default by
//! `share.create`. Names (`.env*`, `*.pem`, `id_*`, …) and, for small text
//! files, content (private key headers, AWS and GitHub token prefixes).
//!
//! The Core track's snapshot builder decides what a copy really holds; this
//! is only the list the owner reviews. A [`Scanner`] is the seam
//! (`CollabBackend::scan` uses [`PatternScan`] unless it has its own).

use std::path::{Path, PathBuf};

use pitwall_proto::api::collab::SensitiveFile;
use pitwall_proto::collab::Scope;

/// What a scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanResult {
    pub sensitive: Vec<SensitiveFile>,
    /// Files in scope after the scope's exclusions (`None`: too many to count).
    pub files: Option<u32>,
}

/// Scans a scope (blocking; may read the disk).
pub trait Scanner: Send + Sync {
    fn scan(&self, scope: &Scope) -> ScanResult;
}

/// Walk the scope's folders: names, then the first bytes of small files.
pub struct PatternScan {
    /// Stop counting after this many files.
    pub max_files: u32,
}

impl Default for PatternScan {
    fn default() -> PatternScan {
        PatternScan { max_files: 50_000 }
    }
}

/// Folders never walked (dependencies, build output, the repository's own).
const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", ".venv", "venv", "__pycache__", "dist", "build", ".next"];
/// Folders whose whole content is sensitive.
const SECRET_DIRS: &[&str] = &[".aws", ".ssh", ".kube", ".gnupg"];
/// Content read from each small file.
const PEEK: usize = 4096;
const PEEK_MAX_FILE: u64 = 512 * 1024;

/// Why a file name is sensitive, if it is (`matches *.pem`).
pub fn by_name(rel: &str) -> Option<String> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let lower = name.to_ascii_lowercase();
    let ext = |e: &str| lower.ends_with(e);
    let why = |pat: &str| Some(format!("matches {pat}"));
    if rel.split('/').any(|part| SECRET_DIRS.contains(&part)) {
        let dir = rel.split('/').find(|p| SECRET_DIRS.contains(p)).unwrap_or_default();
        return why(&format!("{dir}/"));
    }
    if lower == ".env" || lower.starts_with(".env.") {
        return why(".env*");
    }
    for e in [".pem", ".key", ".p12", ".pfx", ".keystore", ".jks", ".tfstate", ".tfstate.backup"] {
        if ext(e) {
            return why(&format!("*{e}"));
        }
    }
    // SSH private keys (`id_ed25519`); `id_ed25519.pub` is public.
    if lower.starts_with("id_") && !lower.contains('.') {
        return why("id_*");
    }
    for exact in [".npmrc", ".pypirc", ".netrc", ".git-credentials", ".htpasswd"] {
        if lower == exact {
            return why(exact);
        }
    }
    if lower.starts_with("credentials") {
        return why("credentials*");
    }
    if lower.starts_with("secrets") || lower.starts_with("secret.") {
        return why("secrets*");
    }
    if rel.ends_with("config/master.key") {
        return why("config/master.key");
    }
    // Local config: `settings.local.json`, `app.local.yml`.
    if lower.contains(".local.") {
        return why("*.local.*");
    }
    None
}

/// Why a file's first bytes are sensitive, if they are.
pub fn by_content(head: &str) -> Option<String> {
    if head.contains("-----BEGIN") && head.contains("PRIVATE KEY") {
        return Some("contains a private key".into());
    }
    let has_word = |prefix: &str, min: usize| {
        head.match_indices(prefix).any(|(i, _)| {
            head[i + prefix.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').count() >= min
        })
    };
    if has_word("AKIA", 16) || has_word("ASIA", 16) {
        return Some("contains an AWS access key".into());
    }
    for p in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"] {
        if has_word(p, 20) {
            return Some("contains a GitHub token".into());
        }
    }
    if has_word("xox", 10) && (head.contains("xoxb-") || head.contains("xoxp-")) {
        return Some("contains a Slack token".into());
    }
    if has_word("sk-ant-", 20) || has_word("sk-proj-", 20) {
        return Some("contains an API key".into());
    }
    None
}

fn expand(repo: &str) -> PathBuf {
    match repo.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None => PathBuf::from(repo),
    }
}

/// A path is under one of `excluded` (an exact file or a folder prefix).
pub fn is_excluded(rel: &str, excluded: &[String]) -> bool {
    excluded.iter().any(|x| {
        let x = x.trim_end_matches('/');
        rel == x || rel.starts_with(&format!("{x}/"))
    })
}

impl Scanner for PatternScan {
    fn scan(&self, scope: &Scope) -> ScanResult {
        let root = expand(&scope.repo);
        let starts: Vec<PathBuf> = if scope.paths.is_empty() {
            vec![root.clone()]
        } else {
            scope.paths.iter().map(|p| root.join(p.trim_end_matches('/'))).collect()
        };
        let mut out = ScanResult { sensitive: Vec::new(), files: Some(0) };
        let mut count: u32 = 0;
        let mut stack = starts;
        while let Some(p) = stack.pop() {
            let Ok(meta) = std::fs::symlink_metadata(&p) else { continue };
            let rel = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            if meta.is_dir() {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if SKIP_DIRS.contains(&name) {
                    continue;
                }
                let Ok(rd) = std::fs::read_dir(&p) else { continue };
                let mut kids: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
                kids.sort();
                kids.reverse();
                stack.extend(kids);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let why = by_name(&rel).or_else(|| peek(&p, meta.len()).as_deref().and_then(by_content));
            if let Some(why) = why {
                out.sensitive.push(SensitiveFile { path: rel.clone(), why });
            }
            if !is_excluded(&rel, &scope.exclude) {
                count += 1;
                if count >= self.max_files {
                    out.files = None;
                }
            }
        }
        if out.files.is_some() {
            out.files = Some(count);
        }
        out.sensitive.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }
}

fn peek(path: &Path, len: u64) -> Option<String> {
    if len == 0 || len > PEEK_MAX_FILE {
        return None;
    }
    use std::io::Read;
    let mut buf = vec![0u8; PEEK];
    let n = std::fs::File::open(path).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_why() {
        let cases = [
            (".env", Some("matches .env*")),
            ("app/.env.production", Some("matches .env*")),
            ("deploy/id_ed25519", Some("matches id_*")),
            ("certs/server.pem", Some("matches *.pem")),
            ("config/master.key", Some("matches *.key")),
            ("home/.ssh/config", Some("matches .ssh/")),
            ("infra/terraform.tfstate", Some("matches *.tfstate")),
            (".npmrc", Some("matches .npmrc")),
            ("aws/credentials", Some("matches credentials*")),
            ("config/settings.local.json", Some("matches *.local.*")),
            ("src/main.rs", None),
            ("docs/environment.md", None),
            ("src/identity.rs", None),
        ];
        for (path, want) in cases {
            assert_eq!(by_name(path).as_deref(), want, "{path}");
        }
        assert_eq!(by_content("-----BEGIN OPENSSH PRIVATE KEY-----\nabc").as_deref(), Some("contains a private key"));
        assert_eq!(by_content("key = AKIAIOSFODNN7EXAMPLE").as_deref(), Some("contains an AWS access key"));
        assert_eq!(by_content("token: ghp_0123456789abcdefghijABCDEFGHIJ").as_deref(), Some("contains a GitHub token"));
        assert_eq!(by_content("nothing to see"), None);
        assert!(is_excluded("deploy/keys/a", &["deploy/".into()]));
        assert!(!is_excluded("deployment", &["deploy".into()]));
    }

    #[test]
    fn scans_a_fixture_tree() {
        let dir = std::env::temp_dir().join(format!("pw-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let put = |rel: &str, text: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        put("src/main.rs", "fn main() {}\n");
        put("src/keys.txt", "-----BEGIN RSA PRIVATE KEY-----\n");
        put("docs/readme.md", "hi\n");
        put(".env", "SECRET=1\n");
        put("deploy/id_ed25519", "x");
        put("node_modules/x/.env", "ignored\n");
        put(".git/config", "ignored\n");
        let scope = |paths: &[&str], exclude: &[&str]| Scope {
            repo: dir.to_string_lossy().into_owned(),
            git_ref: "main".into(),
            paths: paths.iter().map(|s| s.to_string()).collect(),
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let all = PatternScan::default().scan(&scope(&[], &[]));
        let found: Vec<(&str, &str)> = all.sensitive.iter().map(|f| (f.path.as_str(), f.why.as_str())).collect();
        assert_eq!(found, [(".env", "matches .env*"), ("deploy/id_ed25519", "matches id_*"), ("src/keys.txt", "contains a private key")]);
        assert_eq!(all.files, Some(5));
        let some = PatternScan::default().scan(&scope(&["src/", "docs/"], &["src/keys.txt"]));
        assert_eq!(some.sensitive.len(), 1);
        assert_eq!(some.files, Some(2), "main.rs and readme.md; keys.txt is excluded");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
