//! The extensions index's tool (docs/spec/tile-sdk/marketplace.md §7.2):
//! what the index repository's workflows run. The checks and the build
//! are `pitwall_core::extensions::index::repo`'s; this adds the https
//! downloads, the headless run of each new version in Pitwall's tile
//! runtime, and the signatures (the index key's on `index.json` and
//! `blocked.json`, a maintainer's on a review).
//!
//! Nothing here pushes, comments or merges: the workflows do that with
//! what this prints.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use pitwall_core::extensions::index::repo::{self, Checked, EntryReview};
use pitwall_core::extensions::index::{self, Index};
use pitwall_core::extensions::npm::{GetError, Http};
use pitwall_core::extensions::package;

/// Downloads over https (ureq, rustls): no credentials or cookies,
/// redirects only to https (GitHub's release assets redirect to its
/// storage), at most the limit asked.
pub struct Https;

impl Http for Https {
    fn get(&self, url: &str, accept: &str, limit: u64, connect: Duration, timeout: Duration) -> Result<Vec<u8>, GetError> {
        if !url.starts_with("https://") {
            return Err(GetError::Failed("only https addresses are downloaded".into()));
        }
        let agent = ureq::AgentBuilder::new().timeout_connect(connect).timeout(timeout).redirects(5).https_only(true).user_agent(concat!("pitwall-index/", env!("CARGO_PKG_VERSION"))).build();
        let resp = agent.get(url).set("Accept", accept).call().map_err(|e| GetError::Failed(e.to_string().chars().take(200).collect()))?;
        if resp.header("Content-Length").and_then(|l| l.trim().parse::<u64>().ok()).is_some_and(|l| l > limit) {
            return Err(GetError::TooBig);
        }
        let mut buf = Vec::new();
        resp.into_reader().take(limit + 1).read_to_end(&mut buf).map_err(|e| GetError::Failed(e.to_string()))?;
        if buf.len() as u64 > limit {
            return Err(GetError::TooBig);
        }
        Ok(buf)
    }
}

/// Load package `c` headless in Pitwall's tile runtime with no grants:
/// each of its tiles started (its `init`) and drawn once, under the
/// runtime's limits. It must not trap, run out of time or fail to load
/// (imports outside the world it is built for are refused at load).
pub fn run_headless(c: &Checked) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let into = dir.path().join(&c.id);
    package::unpack(&c.files, &into)?;
    for t in &c.manifest.contributes.tiles {
        let tile = t.id.clone();
        let folder = into.clone();
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let s = pitwall_extension_test::Harness::load(&folder).open(&tile);
            s.stopped().map(|r| r.to_string())
        }));
        match ran {
            Ok(None) => {}
            Ok(Some(why)) => return Err(format!("{} {}: tile {} stopped: {why}", c.id, c.version, t.id)),
            Err(p) => {
                let why = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                return Err(format!("{} {}: tile {} didn't start headless: {}", c.id, c.version, t.id, why.lines().next().unwrap_or("it panicked")));
            }
        }
    }
    Ok(())
}

/// What `check` found on a pull request.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub problems: Vec<String>,
    pub needs_maintainer: Vec<String>,
    /// New versions that change a permission, a host or the key: (id,
    /// version, what).
    pub sensitive: Vec<(String, String, String)>,
    /// The new versions checked (downloaded, signature, package, headless).
    pub checked: Vec<(String, String)>,
}

impl Report {
    /// Every check passed.
    pub fn passed(&self) -> bool {
        self.problems.is_empty()
    }

    /// A bot may merge it: every check passed, nothing needs a
    /// maintainer, and no new version changes a permission, host or key
    /// (marketplace.md §7.2 step 5).
    pub fn auto_merge(&self) -> bool {
        self.passed() && self.needs_maintainer.is_empty() && self.sensitive.is_empty()
    }

    /// As the pull request's comment says it (Markdown).
    pub fn markdown(&self) -> String {
        let mut out = String::from("## Index check\n\n");
        let list = |out: &mut String, title: &str, items: &[String]| {
            if !items.is_empty() {
                out.push_str(&format!("**{title}**\n\n"));
                for i in items {
                    out.push_str(&format!("- {i}\n"));
                }
                out.push('\n');
            }
        };
        list(&mut out, "Problems (fix these first)", &self.problems);
        list(&mut out, "A maintainer looks at", &self.needs_maintainer);
        list(&mut out, "Changes a permission, host or key", &self.sensitive.iter().map(|(i, v, w)| format!("{i} {v}: {w}")).collect::<Vec<_>>());
        list(&mut out, "Checked", &self.checked.iter().map(|(i, v)| format!("{i} {v}: downloaded, SHA-256 and signature match, package rules pass, its tiles start headless")).collect::<Vec<_>>());
        out.push_str(match (self.passed(), self.auto_merge()) {
            (false, _) => "Result: **not mergeable** yet.\n",
            (true, true) => "Result: every check passed and nothing sensitive changed: **can be merged by the bot**.\n",
            (true, false) => "Result: every check passed; **a maintainer merges** it.\n",
        });
        out
    }
}

/// Check the index repository at `head` (a pull request's checkout):
/// every file ([`repo::load`]); against `base` (the base branch's
/// checkout) when given, what the change may do ([`repo::judge`]) as
/// GitHub user `login`; then a build ([`repo::build`]) that downloads
/// and checks each version not in `previous` (the index built last), and
/// with `run`, each new version loaded headless ([`run_headless`]).
pub fn check(head: &Path, base: Option<&Path>, login: &str, previous: Option<&Index>, http: &dyn Http, run: bool, now: &str) -> Report {
    let mut r = Report::default();
    let head = match repo::load(head) {
        Ok(h) => h,
        Err(p) => {
            r.problems = p;
            return r;
        }
    };
    if let Some(b) = base {
        match repo::load(b) {
            Ok(b) => {
                let c = repo::judge(&b, &head, login);
                r.problems.extend(c.problems);
                r.needs_maintainer.extend(c.needs_maintainer);
            }
            Err(p) => r.problems.extend(p.into_iter().map(|p| format!("the base doesn't load: {p}"))),
        }
    }
    match repo::build(&head, previous, http, now) {
        Ok(b) => {
            r.sensitive = b.sensitive;
            for c in &b.checked {
                if run {
                    if let Err(why) = run_headless(c) {
                        r.problems.push(why);
                        continue;
                    }
                }
                r.checked.push((c.id.clone(), c.version.clone()));
            }
        }
        Err(p) => r.problems.extend(p),
    }
    r
}

/// A minisign secret key read from `path` (no password, or the one in
/// environment variable `password_env`).
pub fn secret_key(path: &Path, password_env: &str) -> Result<minisign::SecretKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{} can't be read: {e}", path.display()))?;
    minisign::SecretKeyBox::from_string(&text)
        .and_then(|b| b.into_unencrypted_secret_key())
        .or_else(|_| minisign::SecretKeyBox::from_string(&text).and_then(|b| b.into_secret_key(std::env::var(password_env).ok())))
        .map_err(|e| format!("{} can't be used: {e}", path.display()))
}

/// A minisign signature of `bytes` by `sk`, its trusted comment naming
/// `file`.
pub fn sign(sk: &minisign::SecretKey, bytes: &[u8], file: &str, now: &str) -> Result<String, String> {
    let pk = minisign::PublicKey::from_secret_key(sk).map_err(|e| e.to_string())?;
    let trusted = format!("file:{file}\tgenerated:{now}");
    minisign::sign(Some(&pk), sk, std::io::Cursor::new(bytes), Some(&trusted), Some("pitwall extensions index")).map(|s| s.into_string()).map_err(|e| e.to_string())
}

/// What a build wrote.
#[derive(Debug, Clone)]
pub struct Written {
    pub extensions: usize,
    pub new_versions: Vec<(String, String)>,
    pub screens: usize,
}

/// Build the index repository at `repo_dir` into `out` (its `dist/v1/`):
/// what is there already read as the previous index, every new version
/// downloaded and checked (and run headless with `run`), then
/// `index.json` and `blocked.json` signed by `key` and written with their
/// `.minisig`, and new screenshots under `screens/`. Nothing is written
/// unless all of it checks.
pub fn build(repo_dir: &Path, out: &Path, key: &minisign::SecretKey, http: &dyn Http, now: &str, run: bool) -> Result<Written, Vec<String>> {
    let r = repo::load(repo_dir)?;
    let previous = std::fs::read(out.join("index.json")).ok().and_then(|b| index::parse_index(&b).ok());
    let b = repo::build(&r, previous.as_ref(), http, now)?;
    if run {
        let failed: Vec<String> = b.checked.iter().filter_map(|c| run_headless(c).err()).collect();
        if !failed.is_empty() {
            return Err(failed);
        }
    }
    let err = |e: std::io::Error| vec![e.to_string()];
    let index_sig = sign(key, &b.index_json, "index.json", now).map_err(|e| vec![e])?;
    let blocked_sig = sign(key, &b.blocked_json, "blocked.json", now).map_err(|e| vec![e])?;
    std::fs::create_dir_all(out).map_err(err)?;
    for (path, bytes) in &b.screens {
        let to = out.join(path);
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p).map_err(err)?;
        }
        write_atomic(&to, bytes).map_err(err)?;
    }
    for (name, bytes, sig) in [("blocked.json", &b.blocked_json, &blocked_sig), ("index.json", &b.index_json, &index_sig)] {
        write_atomic(&out.join(format!("{name}.minisig")), sig.as_bytes()).map_err(err)?;
        write_atomic(&out.join(name), bytes).map_err(err)?;
    }
    Ok(Written { extensions: b.index.extensions.len(), new_versions: b.checked.iter().map(|c| (c.id.clone(), c.version.clone())).collect(), screens: b.screens.len() })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let part = path.with_extension("part");
    std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, path))
}

/// Record maintainer `by`'s review of `id` `version` in its entry: the
/// signature by review key `key` over the review statement, with `date`;
/// `reproducible` as the rebuild found. Refused when the key isn't `by`'s
/// in `maintainers.toml`, or a version that didn't rebuild the same.
pub fn review(repo_dir: &Path, id: &str, version: &str, by: &str, key: &minisign::SecretKey, date: &str, reproducible: bool) -> Result<(), String> {
    if !reproducible {
        return Err("A version is Verified only when it rebuilds to the same component (marketplace.md §7.3).".into());
    }
    let path = repo_dir.join(format!("extensions/{id}.toml"));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{} can't be read: {e}", path.display()))?;
    let mut e = repo::parse_entry(&text)?;
    let r = repo::load(repo_dir).map_err(|p| p.join("\n"))?;
    let m = r.maintainers.maintainers.iter().find(|m| m.handle == by).ok_or_else(|| format!("{by} isn't in maintainers.toml."))?;
    let pk = minisign::PublicKey::from_secret_key(key).map_err(|e| e.to_string())?;
    if pk.to_base64() != m.review_key {
        return Err(format!("That key isn't {by}'s review key in maintainers.toml."));
    }
    let v = e.versions.iter_mut().find(|v| v.version == version).ok_or_else(|| format!("{id} has no version {version}."))?;
    let statement = index::review_statement(id, version, &v.sha256);
    let signature = minisign::sign(Some(&pk), key, std::io::Cursor::new(statement.as_bytes()), Some(statement.trim()), None).map_err(|e| e.to_string())?.into_string();
    v.review = Some(EntryReview { by: by.into(), date: date.into(), reproducible, signature });
    write_atomic(&path, repo::entry_toml(&e).as_bytes()).map_err(|e| e.to_string())
}
