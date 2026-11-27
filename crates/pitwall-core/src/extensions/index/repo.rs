//! The index repository (marketplace.md §7.1): the TOML files its pull
//! requests change, the checks its CI runs on them, and the build of
//! `dist/v1/index.json` and `blocked.json` from them. The
//! `pitwall-index` tool (crates/pitwall-index) runs these in the index
//! repository's workflows; `pitwall extension publish` writes an entry.
//!
//! ```text
//! extensions/<id>.toml   one per extension: author, repository, versions
//! authors/<handle>.toml  GitHub login, public keys, contact
//! maintainers.toml       who may review, with their review keys
//! categories.toml        the fixed list of categories
//! blocked.toml           takedowns
//! dist/v1/               built by CI on main, never edited by hand
//! ```

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{check_release_url, check_repository, check_version, is_commit, is_sha256, review_statement, verify_by, Block, Blocked, Delisted, Index, ListedPermission, Listing, Release, Review, CATEGORIES, FORMAT, MAX_CATEGORIES, MAX_KEYS, MAX_VERSIONS};
use crate::extensions::npm::{GetError, Http};
use crate::extensions::package::{self, zip, Signer};
use crate::extensions::{permissions, Manifest};

/// The world a version is built for when its tiles don't say.
pub const DEFAULT_WORLD: &str = "pitwall:tile@0.1.0";
/// The largest TOML file the index repository may hold.
pub const MAX_TOML_BYTES: u64 = 256 << 10;

/// `extensions/<id>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    /// The author's handle (`authors/<handle>.toml`).
    pub author: String,
    /// `https://github.com/<owner>/<name>`: the source, and where its
    /// packages are released.
    pub repository: String,
    pub categories: Vec<String>,
    /// Its licence, as SPDX (`MIT`, `Apache-2.0`); its packages hold the
    /// text.
    pub license: String,
    /// Set by a maintainer to take it out of Browse: why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delisted: Option<String>,
    pub versions: Vec<EntryVersion>,
}

/// One `[[versions]]` of an entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryVersion {
    pub version: String,
    /// The package, a release asset of the repository.
    pub url: String,
    pub sha256: String,
    /// The commit of the repository the component was built from.
    pub commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitwall: Option<String>,
    pub world: String,
    /// Withdrawn by its author (or a maintainer).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub yanked: bool,
    /// Set by a maintainer: the version's review (Verified).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<EntryReview>,
}

/// `[versions.review]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryReview {
    /// The maintainer's handle (`maintainers.toml`).
    pub by: String,
    pub date: String,
    /// The component rebuilt from `commit` gave the same hash.
    pub reproducible: bool,
    /// minisign signature of [`super::review_statement`] by the
    /// maintainer's review key.
    pub signature: String,
}

/// `authors/<handle>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Author {
    pub handle: String,
    /// The GitHub login whose pull requests speak for this author.
    pub github: String,
    /// minisign public keys (base64): each version is signed by one.
    pub keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
}

/// `maintainers.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Maintainers {
    #[serde(default)]
    pub maintainers: Vec<Maintainer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Maintainer {
    pub handle: String,
    pub github: String,
    /// The public key their reviews are signed with; Pitwall ships the
    /// same keys (`OFFICIAL_REVIEW_KEYS`).
    pub review_key: String,
}

/// `categories.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Categories {
    pub categories: Vec<CategoryDef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryDef {
    pub id: String,
    pub name: String,
}

/// `blocked.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BlockedFile {
    #[serde(default)]
    pub blocked: Vec<Block>,
}

/// The index repository, read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Repo {
    /// By id.
    pub entries: BTreeMap<String, Entry>,
    /// By handle.
    pub authors: BTreeMap<String, Author>,
    pub maintainers: Maintainers,
    pub blocked: BlockedFile,
}

/// The `categories.toml` the index repository holds: Pitwall's list.
pub fn categories_toml() -> String {
    let c = Categories { categories: CATEGORIES.iter().map(|(id, name)| CategoryDef { id: (*id).into(), name: (*name).into() }).collect() };
    toml::to_string(&c).unwrap_or_default()
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let shown = path.display();
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{shown}: {e}"))?;
    if !meta.is_file() || meta.len() > MAX_TOML_BYTES {
        return Err(format!("{shown}: not a plain file, or larger than 256 KiB"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{shown}: {e}"))?;
    toml::from_str(&text).map_err(|e| format!("{shown}: {}", e.to_string().trim()))
}

/// The `.toml` files in folder `dir`, by stem.
fn tomls(dir: &Path) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let stem = name.strip_suffix(".toml").ok_or_else(|| format!("{}: only .toml files belong here", e.path().display()))?;
        out.push((stem.to_string(), e.path()));
    }
    out.sort();
    Ok(out)
}

/// Read the index repository at `dir` and check every file on its own and
/// against the others (offline): ids and handles match their file names,
/// entries name known authors and categories, versions are well formed
/// and released from the entry's repository, keys are minisign keys and
/// no key is two authors', reviews are by maintainers and their
/// signatures check, `categories.toml` is Pitwall's list. Every problem
/// found, or the repository.
pub fn load(dir: &Path) -> Result<Repo, Vec<String>> {
    let mut problems = Vec::new();
    let mut repo = Repo::default();
    match read_toml::<Categories>(&dir.join("categories.toml")) {
        Ok(c) if c.categories.iter().map(|c| (c.id.as_str(), c.name.as_str())).eq(CATEGORIES.iter().copied()) => {}
        Ok(_) => problems.push("categories.toml isn't Pitwall's list of categories (it changes with Pitwall).".into()),
        Err(e) => problems.push(e),
    }
    if dir.join("maintainers.toml").exists() {
        match read_toml::<Maintainers>(&dir.join("maintainers.toml")) {
            Ok(m) => repo.maintainers = m,
            Err(e) => problems.push(e),
        }
    }
    for m in &repo.maintainers.maintainers {
        if Signer::from_public_key(&m.review_key).is_err() {
            problems.push(format!("maintainers.toml: {}'s review_key isn't a minisign public key", m.handle));
        }
    }
    if dir.join("blocked.toml").exists() {
        match read_toml::<BlockedFile>(&dir.join("blocked.toml")) {
            Ok(b) => repo.blocked = b,
            Err(e) => problems.push(e),
        }
    }
    for b in &repo.blocked.blocked {
        let ok = crate::extensions::check_id(&b.id).is_ok() && !b.reason.trim().is_empty() && super::parse_time(&b.date).is_some() && b.versions.iter().all(|v| check_version(v).is_ok()) && b.sha256.iter().all(|h| is_sha256(h));
        if !ok {
            problems.push(format!("blocked.toml: the rule for “{}” needs an id, a reason, a date (YYYY-MM-DD), and versions like 1.2.3 or lower-case sha256s", b.id));
        }
    }
    let mut key_owner: BTreeMap<String, String> = BTreeMap::new();
    match tomls(&dir.join("authors")) {
        Ok(files) => {
            for (stem, path) in files {
                match read_toml::<Author>(&path) {
                    Ok(a) => {
                        let at = |e: &str| format!("authors/{stem}.toml: {e}");
                        if a.handle != stem {
                            problems.push(at(&format!("its handle is “{}”, but the file is named for “{stem}”", a.handle)));
                        }
                        if crate::extensions::check_id(&a.handle).is_err() {
                            problems.push(at("a handle is lower-case letters, digits and -, starting with a letter"));
                        }
                        if a.github.is_empty() || a.github.len() > 39 || !a.github.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                            problems.push(at("github isn't a GitHub login"));
                        }
                        if a.keys.is_empty() || a.keys.len() > MAX_KEYS {
                            problems.push(at(&format!("it needs 1 to {MAX_KEYS} keys")));
                        }
                        for k in &a.keys {
                            match Signer::from_public_key(k) {
                                Err(_) => problems.push(at(&format!("{k} isn't a minisign public key"))),
                                Ok(s) => {
                                    if let Some(other) = key_owner.insert(s.public_key.clone(), a.handle.clone()) {
                                        problems.push(at(&format!("key {} is also {other}'s: a key belongs to one author", s.key_id)));
                                    }
                                }
                            }
                        }
                        repo.authors.insert(stem, a);
                    }
                    Err(e) => problems.push(e),
                }
            }
        }
        Err(e) => problems.push(e),
    }
    match tomls(&dir.join("extensions")) {
        Ok(files) => {
            for (stem, path) in files {
                match read_toml::<Entry>(&path) {
                    Ok(e) => {
                        problems.extend(check_entry(&stem, &e, &repo).into_iter().map(|p| format!("extensions/{stem}.toml: {p}")));
                        repo.entries.insert(stem, e);
                    }
                    Err(e) => problems.push(e),
                }
            }
        }
        Err(e) => problems.push(e),
    }
    if problems.is_empty() {
        Ok(repo)
    } else {
        Err(problems)
    }
}

/// What's wrong with entry `e` (in `<stem>.toml`) against the rest of
/// `repo` (its authors and maintainers).
fn check_entry(stem: &str, e: &Entry, repo: &Repo) -> Vec<String> {
    let mut p = Vec::new();
    if e.id != stem {
        p.push(format!("its id is “{}”, but the file is named for “{stem}”", e.id));
    }
    if let Err(why) = crate::extensions::check_id(&e.id) {
        p.push(why);
    }
    if !repo.authors.contains_key(&e.author) {
        p.push(format!("its author “{}” has no authors/{}.toml", e.author, e.author));
    }
    if let Err(why) = check_repository(&e.repository) {
        p.push(why);
    }
    if e.categories.is_empty() || e.categories.len() > MAX_CATEGORIES {
        p.push(format!("it needs 1 to {MAX_CATEGORIES} categories"));
    }
    for c in e.categories.iter().filter(|c| super::category_name(c).is_none()) {
        p.push(format!("“{c}” isn't one of the categories ({})", CATEGORIES.map(|c| c.0).join(", ")));
    }
    if e.license.trim().is_empty() || e.license.len() > 100 {
        p.push("it needs its licence, as SPDX (MIT, Apache-2.0, …)".into());
    }
    if e.versions.is_empty() || e.versions.len() > MAX_VERSIONS {
        p.push(format!("it needs 1 to {MAX_VERSIONS} versions"));
    }
    let mut seen = HashSet::new();
    for v in &e.versions {
        let at = |s: String| format!("version {}: {s}", v.version);
        if let Err(why) = check_version(&v.version) {
            p.push(why);
        }
        if !seen.insert(v.version.clone()) {
            p.push(at("listed twice".into()));
        }
        if let Err(why) = check_release_url(&v.url, &e.repository) {
            p.push(at(why));
        }
        if !is_sha256(&v.sha256) {
            p.push(at("sha256 isn't 64 lower-case hex digits".into()));
        }
        if !is_commit(&v.commit) {
            p.push(at("commit isn't a full commit id (40 lower-case hex digits)".into()));
        }
        if v.pitwall.as_deref().is_some_and(|r| !r.strip_prefix(">=").is_some_and(|x| check_version(x).is_ok())) {
            p.push(at("pitwall isn't like >=0.14.0".into()));
        }
        if !v.world.starts_with("pitwall:tile@") {
            p.push(at(format!("world isn't like {DEFAULT_WORLD}")));
        }
        if let Some(r) = &v.review {
            match repo.maintainers.maintainers.iter().find(|m| m.handle == r.by) {
                None => p.push(at(format!("its review is by “{}”, who isn't in maintainers.toml", r.by))),
                Some(m) => {
                    let keys: Vec<Signer> = Signer::from_public_key(&m.review_key).into_iter().collect();
                    if verify_by(review_statement(&e.id, &v.version, &v.sha256).as_bytes(), &r.signature, &keys).is_err() {
                        p.push(at(format!("its review's signature isn't {}'s over “{}”", r.by, review_statement(&e.id, &v.version, &v.sha256).trim())));
                    }
                }
            }
            if !r.reproducible {
                p.push(at("a Verified version must build reproducibly (reproducible = true)".into()));
            }
            if super::parse_time(&r.date).is_none() {
                p.push(at("its review's date isn't YYYY-MM-DD".into()));
            }
        }
    }
    p
}

/// Ids that look like `id` among `others`: the same once `-` is dropped
/// and look-alike characters folded (`l`/`1`/`i`, `o`/`0`, `rn`/`m`), or
/// one edit apart. A new id like these needs a maintainer's look
/// (typosquatting).
pub fn look_alikes<'a>(id: &str, others: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let fold = |s: &str| s.replace('-', "").replace("rn", "m").replace(['1', 'l'], "i").replace('0', "o").replace("vv", "w");
    let f = fold(id);
    others.into_iter().filter(|o| *o != id && (fold(o) == f || edit_distance(o, id) <= 1)).collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// What a pull request changes, judged (marketplace.md §7.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Change {
    /// Why it can't be merged as it is.
    pub problems: Vec<String>,
    /// Why a maintainer must look before it is merged (a new extension, a
    /// key, a review, a takedown…).
    pub needs_maintainer: Vec<String>,
    /// The entries it adds versions to: (id, version).
    pub new_versions: Vec<(String, String)>,
}

impl Change {
    /// Whether a bot may merge it once every check passes and its new
    /// versions change no permission, host or key: nothing needs a
    /// maintainer and every entry it touches is `login`'s.
    pub fn auto_mergeable(&self) -> bool {
        self.problems.is_empty() && self.needs_maintainer.is_empty()
    }
}

/// Judge the change from `base` (the repository at the pull request's
/// base) to `head`, made by GitHub user `login`: ids keep their author,
/// listed versions are never changed or removed (only yanked), and what
/// only maintainers decide (reviews, delisting, un-yanking, blocks,
/// maintainers, new extensions, new keys) is marked for them.
pub fn judge(base: &Repo, head: &Repo, login: &str) -> Change {
    let mut c = Change::default();
    let owner_login = |r: &Repo, handle: &str| r.authors.get(handle).map(|a| a.github.to_ascii_lowercase());
    let by_author = |r: &Repo, handle: &str| owner_login(r, handle).is_some_and(|g| g == login.to_ascii_lowercase());
    for id in base.entries.keys().filter(|id| !head.entries.contains_key(*id)) {
        c.needs_maintainer.push(format!("{id} is removed: only maintainers remove an extension (delisting keeps it in the file)"));
    }
    for (id, e) in &head.entries {
        match base.entries.get(id) {
            None => {
                c.needs_maintainer.push(format!("{id} is new: a maintainer checks its name, author and licence"));
                let alike = look_alikes(id, base.entries.keys().map(String::as_str));
                if !alike.is_empty() {
                    c.needs_maintainer.push(format!("{id} looks like {}: check it isn't pretending to be one", alike.join(", ")));
                }
                c.new_versions.extend(e.versions.iter().map(|v| (id.clone(), v.version.clone())));
            }
            Some(old) => {
                if old.author != e.author {
                    c.problems.push(format!("{id} belongs to {}: another author can't take it over", old.author));
                }
                if old.repository != e.repository {
                    c.needs_maintainer.push(format!("{id}'s repository changes from {} to {}", old.repository, e.repository));
                }
                if old.delisted != e.delisted {
                    c.needs_maintainer.push(format!("{id}'s delisting changes: a maintainer's call"));
                }
                for ov in &old.versions {
                    match e.versions.iter().find(|v| v.version == ov.version) {
                        None => c.problems.push(format!("{id} {} is removed: a listed version is never removed, set yanked = true instead", ov.version)),
                        Some(v) => {
                            let same = v.url == ov.url && v.sha256 == ov.sha256 && v.commit == ov.commit && v.pitwall == ov.pitwall && v.world == ov.world;
                            if !same {
                                c.problems.push(format!("{id} {} changed: a listed version's file is never changed, add a new version", ov.version));
                            }
                            if ov.yanked && !v.yanked {
                                c.needs_maintainer.push(format!("{id} {} is un-yanked: a maintainer's call", ov.version));
                            }
                            if v.review != ov.review {
                                c.needs_maintainer.push(format!("{id} {}'s review changes: only maintainers review", ov.version));
                            }
                        }
                    }
                }
                for v in e.versions.iter().filter(|v| !old.versions.iter().any(|o| o.version == v.version)) {
                    if v.review.is_some() {
                        c.needs_maintainer.push(format!("{id} {} comes with a review: only maintainers review", v.version));
                    }
                    let newest_old = old.versions.iter().map(|o| o.version.as_str()).max_by(|a, b| crate::extensions::install::compare_versions(a, b));
                    if newest_old.is_some_and(|n| crate::extensions::install::compare_versions(&v.version, n) != std::cmp::Ordering::Greater) {
                        c.problems.push(format!("{id} {} isn't newer than {}: new versions go after the listed ones", v.version, newest_old.unwrap_or_default()));
                    }
                    c.new_versions.push((id.clone(), v.version.clone()));
                }
            }
        }
        if !by_author(head, &e.author) && head.entries.get(id) != base.entries.get(id) {
            c.needs_maintainer.push(format!("{id} is changed by {login}, who isn't its author's GitHub login"));
        }
    }
    for (h, a) in &head.authors {
        match base.authors.get(h) {
            None => {}
            Some(old) if old.keys != a.keys => c.needs_maintainer.push(format!("{h}'s keys change: a maintainer checks the author asked (a key rotation)")),
            Some(old) if old.github != a.github => c.needs_maintainer.push(format!("{h}'s GitHub login changes")),
            Some(_) => {}
        }
    }
    if base.maintainers != head.maintainers {
        c.needs_maintainer.push("maintainers.toml changes".into());
    }
    if base.blocked != head.blocked {
        c.needs_maintainer.push("blocked.toml changes (a takedown)".into());
    }
    c
}

/// A package downloaded and checked for the index.
#[derive(Debug, Clone)]
pub struct Checked {
    pub id: String,
    pub version: String,
    pub manifest: Manifest,
    pub files: Vec<zip::Entry>,
    pub size: u64,
    /// The author's key that signed it.
    pub signer: Signer,
}

/// What a build gives.
#[derive(Debug, Clone)]
pub struct Built {
    pub index: Index,
    pub blocked: Blocked,
    /// `index.json`'s bytes, as signed.
    pub index_json: Vec<u8>,
    /// `blocked.json`'s.
    pub blocked_json: Vec<u8>,
    /// New screenshots and media (READMEs, icons, images): (path under
    /// `dist/v1/`, bytes).
    pub screens: Vec<(String, Vec<u8>)>,
    /// The packages downloaded and checked for new versions, for the
    /// tool's headless run.
    pub checked: Vec<Checked>,
    /// What each new version changes against the version before it, for
    /// auto-merge: (id, version, what changed in words).
    pub sensitive: Vec<(String, String, String)>,
}

/// The release asset `v` of entry `e`, downloaded with `http` and checked
/// against the index: its size, its SHA-256, a signature (`<url>.minisig`)
/// by one of `keys`, then the package's own rules
/// ([`package::open_bytes`]), its manifest's id and version the entry's,
/// a licence in it, its tiles' worlds and Pitwall requirement the entry's.
pub fn fetch_checked(http: &dyn Http, e: &Entry, v: &EntryVersion, keys: &[Signer]) -> Result<Checked, String> {
    let at = |s: String| format!("{} {}: {s}", e.id, v.version);
    let get = |url: &str, limit: u64| http.get(url, "application/octet-stream", limit, Duration::from_secs(20), Duration::from_secs(120));
    let bytes = get(&v.url, package::MAX_BYTES).map_err(|err| at(match err {
        GetError::TooBig => format!("{} is larger than {}", v.url, package::size_words(package::MAX_BYTES)),
        GetError::Failed(w) => format!("{} can't be downloaded: {w}", v.url),
    }))?;
    let sha = package::sha256(&bytes);
    if sha != v.sha256 {
        return Err(at(format!("its SHA-256 is {sha}, not the {} the entry lists", v.sha256)));
    }
    let sig = get(&format!("{}.minisig", v.url), package::MAX_SIGNATURE_BYTES).map_err(|_| at(format!("{}.minisig can't be downloaded: release the signature beside the package", v.url)))?;
    let signer = verify_by(&bytes, &String::from_utf8_lossy(&sig), keys).map_err(|w| at(format!("its signature didn't check against the author's keys: {w}")))?;
    let (manifest, files) = package::open_bytes(&bytes).map_err(at)?;
    if manifest.id != e.id || manifest.version.as_deref() != Some(&v.version) {
        return Err(at(format!("the package is {} {}, not the entry's", manifest.id, manifest.version.clone().unwrap_or_default())));
    }
    if !files.iter().any(|f| f.name.starts_with("LICENSE")) {
        return Err(at("the package has no LICENSE: one is required for the index".into()));
    }
    if manifest.description.as_deref().is_none_or(|d| d.trim().is_empty()) {
        return Err(at("its extension.json has no description: the index shows one".into()));
    }
    for t in &manifest.contributes.tiles {
        let w = t.world.as_deref().unwrap_or(DEFAULT_WORLD);
        if w != v.world {
            return Err(at(format!("tile {} is built for {w}, not the entry's {}", t.id, v.world)));
        }
    }
    if manifest.pitwall != v.pitwall {
        return Err(at(format!("its extension.json needs Pitwall {}, the entry says {}", manifest.pitwall.clone().unwrap_or("(any)".into()), v.pitwall.clone().unwrap_or("(any)".into()))));
    }
    Ok(Checked { id: e.id.clone(), version: v.version.clone(), manifest, files, size: bytes.len() as u64, signer })
}

/// The section of `changelog` (a CHANGELOG.md) for `version`: from the
/// heading naming it to the next heading of its level or higher, at most
/// [`super::MAX_CHANGELOG`] bytes.
pub fn changelog_section(changelog: &str, version: &str) -> Option<String> {
    let mut out = Vec::new();
    let mut level = None;
    for line in changelog.lines() {
        let hashes = line.bytes().take_while(|b| *b == b'#').count();
        if hashes > 0 {
            if let Some(l) = level {
                if hashes <= l {
                    break;
                }
            } else if line[hashes..].split(|c: char| c.is_whitespace() || c == '[' || c == ']').any(|w| w.trim_start_matches('v') == version) {
                level = Some(hashes);
                continue;
            }
        }
        if level.is_some() {
            out.push(line);
        }
    }
    let text = out.join("\n").trim().to_string();
    let mut cut = text.len().min(super::MAX_CHANGELOG);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    (!text.is_empty()).then(|| text[..cut].to_string())
}

/// The listed permissions of manifest `m`.
pub fn listed_permissions(m: &Manifest) -> Vec<ListedPermission> {
    m.permissions.iter().map(|p| ListedPermission { name: p.name.clone(), reason: p.reason.clone(), hosts: if p.name == permissions::NET { permissions::listed(m, &p.name) } else { Vec::new() }, amount: permissions::amount(m, &p.name) }).collect()
}

/// Build `index.json` and `blocked.json` from `repo` at time `now`
/// (`YYYY-MM-DDTHH:MM:SSZ`). Versions already in `previous` (the index
/// built last, same id, version and SHA-256) keep what was recorded then
/// and aren't downloaded again; every other version is downloaded with
/// `http` and checked ([`fetch_checked`]), with the version before it,
/// to say what it changes. A listing's name, description, icon,
/// permissions and screenshots are its newest version's.
pub fn build(repo: &Repo, previous: Option<&Index>, http: &dyn Http, now: &str) -> Result<Built, Vec<String>> {
    let mut problems = Vec::new();
    let mut extensions = Vec::new();
    let mut delisted = Vec::new();
    let mut screens = Vec::new();
    let mut checked_all = Vec::new();
    let mut sensitive = Vec::new();
    let today = &now[..now.len().min(10)];
    for (id, e) in &repo.entries {
        if let Some(why) = &e.delisted {
            delisted.push(Delisted { id: id.clone(), reason: why.clone() });
            continue;
        }
        let Some(author) = repo.authors.get(&e.author) else {
            problems.push(format!("{id}: its author {} isn't there", e.author));
            continue;
        };
        let keys: Vec<Signer> = author.keys.iter().filter_map(|k| Signer::from_public_key(k).ok()).collect();
        let old = previous.and_then(|p| p.listing(id));
        let mut versions: Vec<&EntryVersion> = e.versions.iter().collect();
        versions.sort_by(|a, b| crate::extensions::install::compare_versions(&a.version, &b.version));
        let mut releases = Vec::new();
        let mut checked: BTreeMap<String, Checked> = BTreeMap::new();
        let mut failed = false;
        for (i, v) in versions.iter().enumerate() {
            let kept = old.and_then(|l| l.release(&v.version)).filter(|r| r.sha256 == v.sha256 && r.url == v.url);
            let (size, date, changelog) = match kept {
                Some(r) => (r.size, r.date.clone(), r.changelog.clone()),
                None => match fetch_checked(http, e, v, &keys) {
                    Ok(c) => {
                        let log = c.files.iter().find(|f| f.name == "CHANGELOG.md").and_then(|f| changelog_section(&String::from_utf8_lossy(&f.bytes), &v.version));
                        let out = (c.size, today.to_string(), log);
                        // What it changes against the version before it.
                        if let Some(prev) = i.checked_sub(1).map(|j| versions[j]) {
                            let before = match checked.get(&prev.version) {
                                Some(b) => Ok(b.clone()),
                                None => fetch_checked(http, e, prev, &keys),
                            };
                            match before {
                                Ok(b) => {
                                    let mut what: Vec<String> = package::permission_changes(&b.manifest, &c.manifest).iter().map(|p| format!("permission {} changes", p.name())).collect();
                                    if !b.signer.same(&c.signer) {
                                        what.push(format!("signed by key {} instead of {}", c.signer.key_id, b.signer.key_id));
                                    }
                                    sensitive.extend(what.into_iter().map(|w| (id.clone(), v.version.clone(), w)));
                                    checked.insert(prev.version.clone(), b);
                                }
                                Err(why) => sensitive.push((id.clone(), v.version.clone(), format!("the version before it can't be compared: {why}"))),
                            }
                        }
                        checked.insert(v.version.clone(), c.clone());
                        checked_all.push(c);
                        out
                    }
                    Err(why) => {
                        problems.push(why);
                        failed = true;
                        continue;
                    }
                },
            };
            let review = v.review.as_ref().map(|r| Review { by: r.by.clone(), date: r.date.clone(), reproducible: r.reproducible, signature: r.signature.clone() });
            releases.push(Release { version: v.version.clone(), url: v.url.clone(), sha256: v.sha256.clone(), size, date, pitwall: v.pitwall.clone(), world: v.world.clone(), commit: v.commit.clone(), yanked: v.yanked, review, changelog });
        }
        if failed {
            continue;
        }
        // The listing's text and screenshots: its newest version's.
        let newest = versions.last().map(|v| v.version.clone()).unwrap_or_default();
        let fresh = match checked.get(&newest) {
            Some(c) => Some(c.clone()),
            None => match old.filter(|l| l.release(&newest).is_some()) {
                Some(_) => None,
                None => match versions.last().map(|v| fetch_checked(http, e, v, &keys)) {
                    Some(Ok(c)) => Some(c),
                    Some(Err(why)) => {
                        problems.push(why);
                        continue;
                    }
                    None => None,
                },
            },
        };
        let listing = match (fresh, old) {
            (Some(c), _) => {
                let (shots, readme, media, files) = match listing_media(id, &c) {
                    Ok(m) => m,
                    Err(why) => {
                        problems.push(format!("{id} {}: {why}", c.version));
                        continue;
                    }
                };
                screens.extend(files);
                Listing {
                    id: id.clone(),
                    name: c.manifest.name.clone(),
                    author: e.author.clone(),
                    github: author.github.clone(),
                    description: c.manifest.description.clone().unwrap_or_default(),
                    icon: c.manifest.icon.clone(),
                    categories: e.categories.clone(),
                    license: e.license.clone(),
                    repository: e.repository.clone(),
                    permissions: listed_permissions(&c.manifest),
                    screens: shots,
                    tagline: c.manifest.tagline.clone(),
                    homepage: c.manifest.homepage.clone(),
                    readme,
                    media,
                    keys: author.keys.clone(),
                    versions: releases,
                }
            }
            (None, Some(l)) => Listing { author: e.author.clone(), github: author.github.clone(), categories: e.categories.clone(), license: e.license.clone(), repository: e.repository.clone(), keys: author.keys.clone(), versions: releases, ..l.clone() },
            (None, None) => continue,
        };
        extensions.push(listing);
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let index = Index { format: FORMAT, generated: now.to_string(), extensions, delisted };
    let blocked = Blocked { format: FORMAT, generated: now.to_string(), blocked: repo.blocked.blocked.clone() };
    let index_json = serde_json::to_vec(&index).map_err(|e| vec![e.to_string()])?;
    let blocked_json = serde_json::to_vec(&blocked).map_err(|e| vec![e.to_string()])?;
    // What Pitwall reads must read back: the same limits.
    super::parse_index(&index_json).map_err(|e| vec![e])?;
    super::parse_blocked(&blocked_json).map_err(|e| vec![e])?;
    Ok(Built { index, blocked, index_json, blocked_json, screens, checked: checked_all, sensitive })
}

/// What the index carries of package `c` for Browse: its legacy
/// `screens` (light files, at most [`super::MAX_SCREENS`]), its README
/// (`media/<id>/<sha256>.md`), its media (icon, screenshots with their
/// dark and light files and captions, the README's images), and the
/// files to write for them under `dist/v1/`, each named by its SHA-256.
/// The package's checks already passed ([`package::open_bytes`]); the
/// README's images that aren't in it are left out (shown as their alt
/// text).
#[allow(clippy::type_complexity)]
pub fn listing_media(id: &str, c: &Checked) -> Result<(Vec<String>, Option<String>, super::ListedMedia, Vec<(String, Vec<u8>)>), String> {
    use crate::extensions::media;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut put = |dir: &str, ext: &str, bytes: &[u8]| -> String {
        let path = format!("{dir}/{id}/{}.{ext}", package::sha256(bytes));
        if !files.iter().any(|(p, _)| *p == path) {
            files.push((path.clone(), bytes.to_vec()));
        }
        path
    };
    let file = |name: &str| c.files.iter().find(|f| f.name == name);
    let ext_of = |name: &str| media::Kind::of_name(name).map(|k| k.ext()).unwrap_or("png");
    let names: Vec<String> = c.files.iter().map(|f| f.name.clone()).collect();
    let shots = media::screenshots(&names, &c.manifest.screenshots)?;
    let mut legacy = Vec::new();
    let mut listed = super::ListedMedia::default();
    for s in &shots {
        let (Some(l), Some(d)) = (file(&s.light), file(&s.dark)) else { continue };
        let light = put("media", ext_of(&s.light), &l.bytes);
        let dark = put("media", ext_of(&s.dark), &d.bytes);
        if legacy.len() < super::MAX_SCREENS {
            legacy.push(put("screens", ext_of(&s.light), &l.bytes));
        }
        listed.screenshots.push(super::ListedShot { light, dark, caption: s.caption.clone() });
    }
    if let Some(i) = c.manifest.icon_file.as_deref().and_then(file) {
        media::check_icon(&i.name, &i.bytes)?;
        listed.icon = Some(put("media", ext_of(&i.name), &i.bytes));
    }
    let mut readme = None;
    if let Some(r) = file(package::README) {
        if r.bytes.len() as u64 > super::MAX_README_BYTES {
            return Err(format!("its README is larger than {}", package::size_words(super::MAX_README_BYTES)));
        }
        let text = String::from_utf8_lossy(&r.bytes).into_owned();
        for src in media::readme_refs(&text) {
            if let media::Ref::Local(rel) = media::local_ref(&src) {
                if let Some(f) = file(&rel) {
                    let kind = media::check_readme_image(&rel, &f.bytes)?;
                    if listed.images.len() < super::MAX_MEDIA_IMAGES {
                        let at = put("media", kind.ext(), &f.bytes);
                        listed.images.insert(rel, at);
                    }
                }
            }
        }
        readme = Some(put("media", "md", text.as_bytes()));
    }
    Ok((legacy, readme, listed, files))
}

/// The entry `pitwall extension publish` writes for a new extension.
pub fn new_entry(m: &Manifest, author: &str, repository: &str, categories: Vec<String>, license: &str, v: EntryVersion) -> Entry {
    Entry { id: m.id.clone(), author: author.into(), repository: repository.into(), categories, license: license.into(), delisted: None, versions: vec![v] }
}

/// An `extensions/<id>.toml`, read (not yet checked against the rest of
/// the repository).
pub fn parse_entry(text: &str) -> Result<Entry, String> {
    toml::from_str(text).map_err(|e| format!("The entry can't be read: {}", e.to_string().trim()))
}

/// An `authors/<handle>.toml`, read.
pub fn parse_author(text: &str) -> Result<Author, String> {
    toml::from_str(text).map_err(|e| format!("The author file can't be read: {}", e.to_string().trim()))
}

/// `text` (an `extensions/<id>.toml`) with version `v` added: refused when
/// it is listed already or isn't newer than the listed ones.
pub fn add_version(text: &str, v: EntryVersion) -> Result<Entry, String> {
    let mut e = parse_entry(text)?;
    if e.versions.iter().any(|o| o.version == v.version) {
        return Err(format!("{} {} is in the index already: a listed version is never changed, so publish a new version.", e.id, v.version));
    }
    if let Some(n) = e.versions.iter().map(|o| o.version.as_str()).max_by(|a, b| crate::extensions::install::compare_versions(a, b)) {
        if crate::extensions::install::compare_versions(&v.version, n) != std::cmp::Ordering::Greater {
            return Err(format!("{} isn't newer than {n}, the newest listed.", v.version));
        }
    }
    e.versions.push(v);
    Ok(e)
}

/// An entry as its file holds it.
pub fn entry_toml(e: &Entry) -> String {
    toml::to_string(e).unwrap_or_default()
}

/// `authors/<handle>.toml` for a first submission.
pub fn author_toml(a: &Author) -> String {
    toml::to_string(a).unwrap_or_default()
}

/// The address that opens GitHub's "new file" page on index repository
/// `repo` with file `path` holding `contents`: saving it there makes a
/// fork and a pull request. Nothing is pushed by Pitwall.
pub fn new_file_url(repo: &str, path: &str, contents: &str) -> String {
    format!("https://github.com/{repo}/new/main?{}", super::query(&[("filename", path), ("value", contents)]))
}

/// The address of GitHub's edit page for `path` on index repository
/// `repo` (an existing entry: GitHub can't prefill an edit).
pub fn edit_url(repo: &str, path: &str) -> String {
    format!("https://github.com/{repo}/edit/main/{path}")
}
