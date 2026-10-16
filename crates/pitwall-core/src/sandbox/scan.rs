//! The sensitive-file scan of a share's scope
//! (docs/spec/collaboration/sharing.md §5.4 step 3): file names that
//! usually hold secrets, plus files whose content looks like a secret
//! (private key headers, AWS and GitHub token prefixes). The Share dialog
//! lists what it finds, ticked for exclusion, with the reasons; the owner
//! edits the list both ways.

use std::sync::OnceLock;

use regex::bytes::Regex;

use crate::exec::{self, Exec};

/// How much of each file is read for the content checks.
pub const READ_MAX: u64 = 256 * 1024;

/// One file the scan flagged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Relative to the scanned folder, `/`-separated.
    pub path: String,
    /// Why, in plain words: "matches *.pem", "contains a private key".
    pub reasons: Vec<String>,
}

/// Name patterns (sharing.md §5.4 step 3), as shown in reasons.
const NAMES: &[&str] = &[
    ".env*",
    "*.pem",
    "*.key",
    "id_*",
    "*.p12",
    "*.pfx",
    "*.keystore",
    "*.jks",
    ".npmrc",
    ".pypirc",
    ".netrc",
    "credentials*",
    "secrets*",
    "*.tfstate",
    "*.tfstate.backup",
    "*.local.*",
];

/// Folders whose whole content counts (anywhere in the path).
const DIRS: &[&str] = &[".aws", ".ssh", ".kube", ".gnupg", ".docker"];

/// Full paths (relative) flagged as they are.
const PATHS: &[&str] = &["config/master.key", "config/credentials.yml.enc"];

/// `name` matches `pattern`, where `*` stands for any run of characters.
fn glob(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((pre, rest)) => {
            let Some(tail) = name.strip_prefix(pre) else { return false };
            (0..=tail.len()).filter(|i| tail.is_char_boundary(*i)).any(|i| glob(rest, &tail[i..]))
        }
    }
}

/// Reasons from the path alone.
pub fn by_name(path: &str) -> Vec<String> {
    let path = path.trim_start_matches("./");
    let mut out = Vec::new();
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let Some(name) = parts.last() else { return out };
    for p in PATHS {
        if path == *p || path.ends_with(&format!("/{p}")) {
            out.push(format!("matches {p}"));
        }
    }
    for d in DIRS {
        if parts[..parts.len() - 1].contains(d) {
            out.push(format!("inside {d}/"));
        }
    }
    let lower = name.to_ascii_lowercase();
    for p in NAMES {
        // `.env*`, `id_*`, `credentials*`, `secrets*` are names, matched as
        // written; extensions ignore case (`KEY.PEM`).
        let hit = if p.starts_with('*') { glob(p, &lower) } else { glob(p, name) };
        if hit && !(p == &"id_*" && lower.ends_with(".pub")) {
            out.push(format!("matches {p}"));
        }
    }
    out
}

struct Pattern {
    re: Regex,
    reason: &'static str,
}

fn patterns() -> &'static [Pattern] {
    static P: OnceLock<Vec<Pattern>> = OnceLock::new();
    P.get_or_init(|| {
        let p = |re: &str, reason| Pattern { re: Regex::new(re).expect("valid pattern"), reason };
        vec![
            p(r"-----BEGIN ((RSA|DSA|EC|OPENSSH|ENCRYPTED|PGP) )?PRIVATE KEY( BLOCK)?-----", "contains a private key"),
            p(r"\b(AKIA|ASIA)[0-9A-Z]{16}\b", "contains an AWS access key id"),
            p(r"\bgh[pousr]_[A-Za-z0-9]{36}\b", "contains a GitHub token"),
            p(r"\bgithub_pat_[A-Za-z0-9_]{22,}\b", "contains a GitHub token"),
        ]
    })
}

/// Reasons from the content.
pub fn by_content(bytes: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in patterns() {
        if p.re.is_match(bytes) && !out.iter().any(|r| r == p.reason) {
            out.push(p.reason.to_string());
        }
    }
    out
}

/// Scan `paths` (relative to `root`, e.g. a scope's tracked files) on the
/// machine `exec` reaches. Files that can't be read are judged by name.
pub fn scan(exec: &dyn Exec, root: &str, paths: &[String]) -> Vec<Finding> {
    let full: Vec<String> = paths.iter().map(|p| exec::join(root, p)).collect();
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    for (chunk_paths, chunk_full) in paths.chunks(256).zip(refs.chunks(256)) {
        let contents = exec.read_files(chunk_full, READ_MAX);
        for (path, content) in chunk_paths.iter().zip(contents) {
            let mut reasons = by_name(path);
            if let Ok(Some(bytes)) = content {
                reasons.extend(by_content(&bytes));
            }
            if !reasons.is_empty() {
                out.push(Finding { path: path.clone(), reasons });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExec;
    use crate::testing::TempDir;

    #[test]
    fn names() {
        let r = |p: &str| by_name(p);
        assert_eq!(r(".env"), ["matches .env*"]);
        assert_eq!(r("web/.env.production"), ["matches .env*"]);
        assert_eq!(r("certs/server.PEM"), ["matches *.pem"]);
        assert_eq!(r("config/master.key"), ["matches config/master.key", "matches *.key"]);
        assert_eq!(r("deploy/id_ed25519"), ["matches id_*"]);
        assert!(r("deploy/id_ed25519.pub").is_empty());
        assert_eq!(r("infra/prod.tfstate"), ["matches *.tfstate"]);
        assert_eq!(r("home/.aws/config"), ["inside .aws/"]);
        assert_eq!(r("settings.local.json"), ["matches *.local.*"]);
        assert_eq!(r("credentials.json"), ["matches credentials*"]);
        assert_eq!(r(".npmrc"), ["matches .npmrc"]);
        for fine in ["src/main.rs", "README.md", "docs/keys.md", "local.json", ".environment-notes/x", "src/identity.rs"] {
            assert!(r(fine).is_empty(), "{fine}: {:?}", r(fine));
        }
    }

    #[test]
    fn contents() {
        let key = b"x\n-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n";
        assert_eq!(by_content(key), ["contains a private key"]);
        assert_eq!(by_content(b"-----BEGIN PRIVATE KEY-----"), ["contains a private key"]);
        assert_eq!(by_content(b"key = AKIAABCDEFGHIJKLMNOP"), ["contains an AWS access key id"]);
        let gh = format!("token: ghp_{}", "a".repeat(36));
        assert_eq!(by_content(gh.as_bytes()), ["contains a GitHub token"]);
        let pat = format!("github_pat_{}", "B".repeat(40));
        assert_eq!(by_content(pat.as_bytes()), ["contains a GitHub token"]);
        assert!(by_content(b"-----BEGIN PUBLIC KEY-----\nAKIA-not-a-key ghp_short").is_empty());
    }

    /// A fixture tree: flagged by name, by content, by both; clean files
    /// stay out.
    #[test]
    fn scans_a_tree() {
        let dir = TempDir::new("scan");
        let root = dir.path();
        let files: &[(&str, String)] = &[
            ("src/main.rs", "fn main() {}".into()),
            ("src/fixtures/deploy.txt", "-----BEGIN RSA PRIVATE KEY-----\n...".into()),
            (".env", "A=1".into()),
            ("certs/site.pem", "-----BEGIN EC PRIVATE KEY-----".into()),
            ("ci/notes.md", format!("use ghs_{} here", "Z".repeat(36))),
            ("README.md", "hello".into()),
        ];
        for (p, c) in files {
            let path = root.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        let paths: Vec<String> = files.iter().map(|(p, _)| p.to_string()).chain(["gone.key".to_string()]).collect();
        let found = scan(&LocalExec, &root.to_string_lossy(), &paths);
        let got: Vec<(&str, Vec<&str>)> = found.iter().map(|f| (f.path.as_str(), f.reasons.iter().map(String::as_str).collect())).collect();
        assert_eq!(
            got,
            [
                ("src/fixtures/deploy.txt", vec!["contains a private key"]),
                (".env", vec!["matches .env*"]),
                ("certs/site.pem", vec!["matches *.pem", "contains a private key"]),
                ("ci/notes.md", vec!["contains a GitHub token"]),
                ("gone.key", vec!["matches *.key"]),
            ]
        );
    }
}
