//! The extensions index (docs/spec/tile-sdk/marketplace.md §7): the list
//! of tiles Browse shows, which Pitwall reads as one signed file.
//!
//! The index is a public repository of TOML files changed by pull request
//! ([`repo`]: one `extensions/<id>.toml` per extension, `authors/`,
//! `maintainers.toml`, `blocked.toml`). Its CI checks each change and,
//! on `main`, builds `dist/v1/index.json` and `dist/v1/blocked.json`
//! ([`repo::build`]) and signs both with the index key. Pitwall reads
//! those two files ([`source`]: an [`IndexSource`], over https with a
//! cache and ETag, or a fixture), checks their signatures against the
//! index keys it ships with ([`IndexConfig`]) and parses them with strict
//! limits ([`parse_index`], [`parse_blocked`]).
//!
//! Packages are the authors' own release assets, pinned in the index by
//! SHA-256 and signed by one of the author's keys; nothing is re-hosted.
//!
//! **Verified** is per version: a maintainer read that version's source
//! (marketplace.md §7.3) and signed a statement naming the extension, the
//! version and the package's SHA-256 ([`review_statement`]) with a review
//! key. Pitwall checks that signature against the review keys it ships
//! with, not against anything the index says, so a stolen index key alone
//! can't make a version Verified. Browse offers the newest Verified
//! version by default ([`Listing::default_release`]).
//!
//! **Takedown** ([`Blocked`], §7.6): a yanked version isn't offered for
//! new installs or updates; a blocked one (by id, version or hash) is
//! refused at install and update and turned off where it is installed;
//! a delisted extension leaves Browse, installed copies keep working.
//! [`refusal`] says why an install or update from the index is refused.

pub mod repo;
pub mod source;

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::package::Signer;

pub use source::{Answer, Fetched, FixtureSource, HttpsSource, IndexHttp, IndexSource, Origin};

/// The format of `index.json` and `blocked.json` this Pitwall reads.
pub const FORMAT: u64 = 1;
/// The largest `index.json` Pitwall reads.
pub const MAX_INDEX_BYTES: u64 = 16 << 20;
/// The largest `blocked.json`.
pub const MAX_BLOCKED_BYTES: u64 = 2 << 20;
/// The largest signature file.
pub const MAX_SIGNATURE_BYTES: u64 = super::package::MAX_SIGNATURE_BYTES;
/// The most extensions one index lists (beyond it, the index is split as
/// marketplace.md §7.1 says).
pub const MAX_EXTENSIONS: usize = 20_000;
/// The most versions one extension lists.
pub const MAX_VERSIONS: usize = 500;
/// The most keys one author has.
pub const MAX_KEYS: usize = 8;
/// The most categories one extension is in.
pub const MAX_CATEGORIES: usize = 3;
/// The most screenshots one extension shows (as a package holds).
pub const MAX_SCREENS: usize = super::package::MAX_SCREENSHOTS;
/// The most permissions one extension lists.
pub const MAX_PERMISSIONS: usize = 32;
/// The most hosts one permission lists.
pub const MAX_HOSTS: usize = 64;
/// The most block-list rules.
pub const MAX_BLOCKS: usize = 10_000;
/// The longest changelog section kept for one version.
pub const MAX_CHANGELOG: usize = 4 << 10;
/// The longest description.
pub const MAX_DESCRIPTION: usize = 200;
/// The longest name.
pub const MAX_NAME: usize = 60;
/// The longest address, reason or other free text.
pub const MAX_TEXT: usize = 500;
/// Days after which an index that hasn't changed is called stale: its
/// publish job signs a fresh one every day, so an older one means it is
/// being held back (a mirror or an attacker replaying an old file) or
/// the job is broken.
pub const STALE_AFTER_DAYS: u64 = 14;

/// The index repository on GitHub that Pitwall reads by default
/// (`owner/name`). It doesn't exist yet: see marketplace.md §7.8.
pub const OFFICIAL_REPO: &str = "pitwalls/pitwall-extensions";
/// Where the official index's built files are.
pub const OFFICIAL_BASE: &str = "https://raw.githubusercontent.com/pitwalls/pitwall-extensions/main/dist/v1/";
/// The official index's public keys (current and next, packaging.md
/// §6.2), base64 as the second line of a `minisign.pub`. None yet: the
/// owner makes them when the index repository is created
/// (marketplace.md §7.8); until then the official index isn't read.
pub const OFFICIAL_INDEX_KEYS: [&str; 0] = [];
/// The maintainers' review keys whose signatures make a version Verified.
pub const OFFICIAL_REVIEW_KEYS: [&str; 0] = [];

/// The categories Browse filters by (marketplace.md §7.4), by id and name.
pub const CATEGORIES: [(&str, &str); 9] = [("git", "Git"), ("ci", "CI"), ("databases", "Databases"), ("apis", "APIs"), ("notes", "Notes"), ("monitoring", "Monitoring"), ("docs", "Docs"), ("fun", "Fun"), ("other", "Other")];

/// The name of category `id`.
pub fn category_name(id: &str) -> Option<&'static str> {
    CATEGORIES.iter().find(|(c, _)| *c == id).map(|(_, n)| *n)
}

/// Which index Pitwall reads and which keys it trusts for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexConfig {
    /// Where `index.json`, `blocked.json` and `screens/` are (ending in
    /// `/`).
    pub base: String,
    /// The index repository on GitHub (`owner/name`): where reports go.
    pub repo: String,
    /// The keys `index.json` and `blocked.json` may be signed with.
    pub keys: Vec<Signer>,
    /// The keys a Verified review may be signed with.
    pub review_keys: Vec<Signer>,
}

impl IndexConfig {
    /// The official index, with the keys this Pitwall ships.
    pub fn official() -> IndexConfig {
        let keys = |list: &[&str]| list.iter().filter_map(|k| Signer::from_public_key(k).ok()).collect();
        IndexConfig { base: OFFICIAL_BASE.into(), repo: OFFICIAL_REPO.into(), keys: keys(&OFFICIAL_INDEX_KEYS), review_keys: keys(&OFFICIAL_REVIEW_KEYS) }
    }

    /// The address of file `rel` of the index (`index.json`,
    /// `screens/<id>/<hash>.webp`).
    pub fn url(&self, rel: &str) -> String {
        format!("{}{rel}", self.base)
    }
}

/// `dist/v1/index.json`: every listed extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    pub format: u64,
    /// When it was built, `YYYY-MM-DDTHH:MM:SSZ`.
    pub generated: String,
    pub extensions: Vec<Listing>,
    /// Extensions taken out of Browse, so an installed copy can say "no
    /// longer listed" and why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delisted: Vec<Delisted>,
}

/// One extension in the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub id: String,
    /// As its newest version's manifest names it.
    pub name: String,
    /// The author's handle in the index (`authors/<handle>.toml`).
    pub author: String,
    /// The author's GitHub login.
    pub github: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub categories: Vec<String>,
    /// The licence its repository names (SPDX), or "see LICENSE".
    pub license: String,
    /// Its source (an https GitHub repository).
    pub repository: String,
    /// What its newest version asks for, with the reasons it gives.
    pub permissions: Vec<ListedPermission>,
    /// Its screenshots, relative to the index's base
    /// (`screens/<id>/<sha256>.<png|webp>`).
    #[serde(default)]
    pub screens: Vec<String>,
    /// The author's public keys: every version is signed by one.
    pub keys: Vec<String>,
    /// Oldest first.
    pub versions: Vec<Release>,
}

/// A permission as Browse shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedPermission {
    pub name: String,
    pub reason: String,
    /// For `net`, the hosts as the manifest writes them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<String>,
    /// For `memory` and `sql`, the size ("128 MiB").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
}

/// One version of a listed extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    /// The package: the author's release asset (https).
    pub url: String,
    /// The package's SHA-256, lower-case hex.
    pub sha256: String,
    /// Its size in bytes.
    pub size: u64,
    /// When it was added to the index, `YYYY-MM-DD`.
    pub date: String,
    /// The lowest Pitwall it needs (`>=X.Y.Z`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitwall: Option<String>,
    /// The tile world it is built for (`pitwall:tile@0.1.0`).
    pub world: String,
    /// The commit of `repository` it was built from.
    pub commit: String,
    /// Withdrawn by its author or a maintainer: not offered for new
    /// installs or updates; installed copies keep working.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub yanked: bool,
    /// A maintainer's review, when this version is Verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
    /// The section of its CHANGELOG.md for this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
}

/// A maintainer's review of one version (marketplace.md §7.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    /// The maintainer's handle.
    pub by: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// The wasm rebuilt from `commit` matched (required for Verified).
    pub reproducible: bool,
    /// A minisign signature of [`review_statement`] by a review key.
    pub signature: String,
}

/// An extension taken out of Browse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delisted {
    pub id: String,
    pub reason: String,
}

/// `dist/v1/blocked.json`: what Pitwall refuses and turns off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Blocked {
    pub format: u64,
    pub generated: String,
    pub blocked: Vec<Block>,
}

/// One block-list rule: extension `id`, all of it when `versions` and
/// `sha256` are both empty, else the versions and packages named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sha256: Vec<String>,
    /// Why, in words the user is shown.
    pub reason: String,
    /// `YYYY-MM-DD`.
    pub date: String,
}

impl Block {
    /// Whether it covers version `version` of `id`, whose package has
    /// SHA-256 `sha256` (when known).
    pub fn covers(&self, id: &str, version: &str, sha256: Option<&str>) -> bool {
        if self.id != id {
            return false;
        }
        let all = self.versions.is_empty() && self.sha256.is_empty();
        all || self.versions.iter().any(|v| v == version) || sha256.is_some_and(|h| self.sha256.iter().any(|b| b.eq_ignore_ascii_case(h)))
    }
}

impl Blocked {
    /// The rule that blocks version `version` of `id`, if one does.
    pub fn find(&self, id: &str, version: &str, sha256: Option<&str>) -> Option<&Block> {
        self.blocked.iter().find(|b| b.covers(id, version, sha256))
    }

    /// The installed copies (id, version, package SHA-256) the list turns
    /// off, with the rule: what the daily check does (permissions.md
    /// §5.4).
    pub fn turned_off<'a>(&'a self, installed: &[(String, String, Option<String>)]) -> Vec<(String, &'a Block)> {
        installed.iter().filter_map(|(id, v, h)| self.find(id, v, h.as_deref()).map(|b| (id.clone(), b))).collect()
    }
}

/// What a maintainer signs to make version `version` of `id`, whose
/// package has SHA-256 `sha256`, Verified: one line, so it can't be read
/// two ways.
pub fn review_statement(id: &str, version: &str, sha256: &str) -> String {
    format!("pitwall-verified {id} {version} sha256:{}\n", sha256.to_ascii_lowercase())
}

/// Whether `sig` (minisign) is by one of `keys` over `bytes`, the keys
/// only: unlike a package's signature, the key in its comment is never
/// used. The key that signed it, or why not.
pub fn verify_by(bytes: &[u8], sig: &str, keys: &[Signer]) -> Result<Signer, String> {
    if sig.len() as u64 > MAX_SIGNATURE_BYTES {
        return Err("Its signature isn't a minisign signature.".into());
    }
    let parsed = minisign_verify::Signature::decode(sig).map_err(|_| "Its signature isn't a minisign signature.".to_string())?;
    for k in keys {
        let Ok(pk) = minisign_verify::PublicKey::from_base64(&k.public_key) else { continue };
        if pk.verify(bytes, &parsed, false).is_ok() {
            return Ok(k.clone());
        }
    }
    if keys.is_empty() {
        return Err("No key to check it with is set.".into());
    }
    Err("Its signature isn't by a key Pitwall trusts for it.".into())
}

/// How one version stands for Browse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// A maintainer's review, signed by a review key this Pitwall trusts.
    Verified,
    /// Newer than a Verified version and not reviewed yet.
    NotYetReviewed,
    /// Never reviewed (no version of it is Verified, or it is older).
    NotVerified,
    /// Withdrawn: not offered.
    Yanked,
    /// On the block list, with its reason: refused.
    Blocked(String),
}

impl Standing {
    /// The badge or line Browse shows for it.
    pub fn words(&self) -> String {
        match self {
            Standing::Verified => "Verified".into(),
            Standing::NotYetReviewed => "Not yet reviewed".into(),
            Standing::NotVerified => "Not verified".into(),
            Standing::Yanked => "Withdrawn".into(),
            Standing::Blocked(why) => format!("Blocked: {why}"),
        }
    }
}

/// What Pitwall needs to judge a listing: the review keys it trusts, the
/// block list, its own version and the tile worlds it serves.
#[derive(Debug, Clone, Copy)]
pub struct Judge<'a> {
    pub review_keys: &'a [Signer],
    pub blocked: &'a Blocked,
    /// This Pitwall's version.
    pub pitwall: &'a str,
    /// The worlds it serves (`pitwall:tile@0.1.0`).
    pub worlds: &'a [&'a str],
}

impl Listing {
    /// Whether `r`'s review is signed by one of `keys` over its statement.
    pub fn is_verified(&self, r: &Release, keys: &[Signer]) -> bool {
        r.review.as_ref().is_some_and(|rev| rev.reproducible && verify_by(review_statement(&self.id, &r.version, &r.sha256).as_bytes(), &rev.signature, keys).is_ok())
    }

    /// How version `r` stands.
    pub fn standing(&self, r: &Release, j: &Judge) -> Standing {
        if let Some(b) = j.blocked.find(&self.id, &r.version, Some(&r.sha256)) {
            return Standing::Blocked(b.reason.clone());
        }
        if r.yanked {
            return Standing::Yanked;
        }
        if self.is_verified(r, j.review_keys) {
            return Standing::Verified;
        }
        let older_verified = self.versions.iter().any(|o| super::install::compare_versions(&o.version, &r.version) == Ordering::Less && !o.yanked && self.is_verified(o, j.review_keys));
        if older_verified {
            Standing::NotYetReviewed
        } else {
            Standing::NotVerified
        }
    }

    /// Whether `r` runs on this Pitwall: its world served, its Pitwall
    /// requirement met.
    pub fn works_here(r: &Release, j: &Judge) -> bool {
        let pitwall_ok = match r.pitwall.as_deref().and_then(|p| p.strip_prefix(">=")) {
            Some(min) => super::install::compare_versions(j.pitwall, min) != Ordering::Less,
            None => true,
        };
        pitwall_ok && j.worlds.contains(&r.world.as_str())
    }

    /// The versions that may be installed: not yanked, not blocked, that
    /// run here; newest first.
    pub fn offered<'a>(&'a self, j: &Judge) -> Vec<&'a Release> {
        let mut v: Vec<&Release> = self.versions.iter().filter(|r| !r.yanked && j.blocked.find(&self.id, &r.version, Some(&r.sha256)).is_none() && Self::works_here(r, j)).collect();
        v.sort_by(|a, b| super::install::compare_versions(&b.version, &a.version));
        v
    }

    /// The newest version that may be installed.
    pub fn newest_release<'a>(&'a self, j: &Judge) -> Option<&'a Release> {
        self.offered(j).into_iter().next()
    }

    /// The newest Verified version that may be installed.
    pub fn verified_release<'a>(&'a self, j: &Judge) -> Option<&'a Release> {
        self.offered(j).into_iter().find(|r| self.is_verified(r, j.review_keys))
    }

    /// What Install installs by default: the newest Verified version
    /// when there is one (a newer one not yet reviewed waits, §7.3), else
    /// the newest.
    pub fn default_release<'a>(&'a self, j: &Judge) -> Option<&'a Release> {
        self.verified_release(j).or_else(|| self.newest_release(j))
    }

    /// Version `version`, if listed.
    pub fn release(&self, version: &str) -> Option<&Release> {
        self.versions.iter().find(|r| r.version == version)
    }
}

impl Index {
    /// Extension `id`, if listed.
    pub fn listing(&self, id: &str) -> Option<&Listing> {
        self.extensions.iter().find(|l| l.id == id)
    }

    /// Why `id` was delisted, if it was.
    pub fn delisted_why(&self, id: &str) -> Option<&str> {
        self.delisted.iter().find(|d| d.id == id).map(|d| d.reason.as_str())
    }
}

/// Why installing or updating to version `version` of `id` from the
/// index is refused (`sha256`: the package's, once downloaded); `None`
/// when it may go ahead. Blocked, withdrawn, unlisted, delisted, or not
/// the file listed: each in words the user is shown.
pub fn refusal(index: &Index, blocked: &Blocked, id: &str, version: &str, sha256: Option<&str>) -> Option<String> {
    if let Some(b) = blocked.find(id, version, sha256) {
        return Some(format!("{id} {version} is blocked by the extensions index: {}. Pitwall won't install it.", b.reason));
    }
    let Some(l) = index.listing(id) else {
        return Some(match index.delisted_why(id) {
            Some(why) => format!("{id} is no longer listed in the extensions index ({why}). An installed copy keeps working."),
            None => format!("The extensions index has no extension “{id}”."),
        });
    };
    let Some(r) = l.release(version) else {
        return Some(format!("The extensions index has no version {version} of {}.", l.name));
    };
    if r.yanked {
        return Some(format!("{} {version} was withdrawn by its author: it isn't offered for new installs or updates. An installed copy keeps working.", l.name));
    }
    if sha256.is_some_and(|h| !h.eq_ignore_ascii_case(&r.sha256)) {
        return Some("The file isn't the one listed: its SHA-256 doesn't match the index. Not installed.".into());
    }
    None
}

/// What an installed copy that came from the index says about itself,
/// when the index changed under it: blocked, withdrawn or no longer
/// listed. `None`: nothing to say.
pub fn installed_note(index: &Index, blocked: &Blocked, id: &str, version: &str, sha256: Option<&str>) -> Option<String> {
    if let Some(b) = blocked.find(id, version, sha256) {
        return Some(format!("Turned off: the extensions index blocked {version} ({}). Remove it, or install a version that isn't blocked.", b.reason));
    }
    match index.listing(id) {
        None => Some(match index.delisted_why(id) {
            Some(why) => format!("No longer listed ({why}). It keeps working, but gets no updates."),
            None => "No longer listed. It keeps working, but gets no updates.".into(),
        }),
        Some(l) => l.release(version).filter(|r| r.yanked).map(|_| format!("{version} was withdrawn by its author.")),
    }
}

/// What a problem report is about (marketplace.md §7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Malware,
    Misleading,
    Broken,
    Other,
}

impl ReportKind {
    pub const ALL: [ReportKind; 4] = [ReportKind::Malware, ReportKind::Misleading, ReportKind::Broken, ReportKind::Other];

    /// Its value in the issue form.
    pub fn id(self) -> &'static str {
        match self {
            ReportKind::Malware => "malware",
            ReportKind::Misleading => "misleading",
            ReportKind::Broken => "broken",
            ReportKind::Other => "other",
        }
    }

    /// As the report menu says it.
    pub fn words(self) -> &'static str {
        match self {
            ReportKind::Malware => "It does something harmful",
            ReportKind::Misleading => "It isn't what it says it is",
            ReportKind::Broken => "It doesn't work",
            ReportKind::Other => "Something else",
        }
    }
}

/// The address of a new issue on index repository `repo` (`owner/name`)
/// reporting version `version` of `id` (package `sha256`) as `kind`,
/// prefilled through the repository's `report.yml` issue form; opened in
/// the browser, nothing is sent from Pitwall.
pub fn report_url(repo: &str, id: &str, version: &str, sha256: &str, kind: ReportKind) -> String {
    let title = format!("Report: {id} {version} ({})", kind.id());
    let q = [("template", "report.yml"), ("title", title.as_str()), ("labels", "report"), ("extension", id), ("version", version), ("sha256", sha256), ("category", kind.id())];
    format!("https://github.com/{repo}/issues/new?{}", query(&q))
}

/// `k=v&…`, each percent-encoded.
pub fn query(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

/// `s` percent-encoded for a URL's query (everything but unreserved
/// characters).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Seconds since 1970 of `YYYY-MM-DDTHH:MM:SSZ` (the only form the index
/// writes), or of `YYYY-MM-DD` at midnight.
pub fn parse_time(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<u64> { s.get(r).filter(|t| t.bytes().all(|c| c.is_ascii_digit())).and_then(|t| t.parse().ok()) };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') || !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1970 {
        return None;
    }
    let (hh, mm, ss) = match s.len() {
        10 => (0, 0, 0),
        20 if b[10] == b'T' && b[13] == b':' && b[16] == b':' && b[19] == b'Z' => (num(11..13)?, num(14..16)?, num(17..19)?),
        _ => return None,
    };
    if hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// `YYYY-MM-DDTHH:MM:SSZ` of `secs` since 1970.
pub fn format_time(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Whether an index generated at `generated` is stale at `now` (seconds
/// since 1970): older than [`STALE_AFTER_DAYS`].
pub fn is_stale(generated: &str, now: u64) -> bool {
    parse_time(generated).is_none_or(|g| now.saturating_sub(g) > STALE_AFTER_DAYS * 86_400)
}

fn text_ok(what: &str, s: &str, max: usize) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err(format!("{what} is empty"));
    }
    if s.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    if s.chars().any(|c| c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200B}'..='\u{200F}')) {
        return Err(format!("{what} has control or direction characters"));
    }
    Ok(())
}

/// Whether `s` is a lower-case hex SHA-256.
pub fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `s` is a git commit id (40 or 64 lower-case hex digits).
pub fn is_commit(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Why `url` can't be a GitHub repository address
/// (`https://github.com/<owner>/<name>`).
pub fn check_repository(url: &str) -> Result<(), String> {
    let rest = url.strip_prefix("https://github.com/").ok_or_else(|| format!("repository {url} isn't an https://github.com/ address"))?;
    let parts: Vec<&str> = rest.split('/').collect();
    let ok = |p: &str| !p.is_empty() && p.len() <= 100 && !p.starts_with('.') && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    if parts.len() != 2 || !parts.iter().all(|p| ok(p)) {
        return Err(format!("repository {url} isn't https://github.com/<owner>/<name>"));
    }
    Ok(())
}

/// Why `url` can't be a package address of `repository`: one of its
/// GitHub releases' assets, a `.pwtile`.
pub fn check_release_url(url: &str, repository: &str) -> Result<(), String> {
    let rest = url.strip_prefix(&format!("{repository}/releases/download/")).ok_or_else(|| format!("{url} isn't a release asset of {repository} ({repository}/releases/download/…)"))?;
    let ok = rest.split('/').count() == 2 && rest.ends_with(".pwtile") && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./+".contains(&b)) && !rest.contains("..");
    if !ok || url.len() > MAX_TEXT {
        return Err(format!("{url} isn't a .pwtile release asset address"));
    }
    Ok(())
}

/// Why `v` can't be a listed version: dot-separated numbers, maybe a
/// `-` pre-release, as a manifest's.
pub fn check_version(v: &str) -> Result<(), String> {
    let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p)));
    let nums = core.split('.').collect::<Vec<_>>();
    let ok = nums.len() == 3 && nums.iter().all(|n| !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit())) && pre.is_none_or(|p| !p.is_empty() && p.len() <= 32 && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.'));
    if !ok {
        return Err(format!("version “{}” isn't like 1.2.3 or 1.2.3-beta.1", v.escape_debug()));
    }
    Ok(())
}

fn check_listing(l: &Listing) -> Result<(), String> {
    let at = |e: String| format!("“{}”: {e}", l.id.escape_debug());
    super::check_id(&l.id).map_err(&at)?;
    text_ok("its name", &l.name, MAX_NAME).map_err(&at)?;
    text_ok("its author", &l.author, 64).map_err(&at)?;
    text_ok("its GitHub login", &l.github, 64).map_err(&at)?;
    text_ok("its description", &l.description, MAX_DESCRIPTION).map_err(&at)?;
    text_ok("its licence", &l.license, 100).map_err(&at)?;
    if let Some(i) = &l.icon {
        text_ok("its icon", i, 64).map_err(&at)?;
    }
    check_repository(&l.repository).map_err(&at)?;
    if l.categories.is_empty() || l.categories.len() > MAX_CATEGORIES {
        return Err(at(format!("it must be in 1 to {MAX_CATEGORIES} categories")));
    }
    if let Some(c) = l.categories.iter().find(|c| category_name(c).is_none()) {
        return Err(at(format!("“{}” isn't a category", c.escape_debug())));
    }
    if l.permissions.len() > MAX_PERMISSIONS {
        return Err(at("it lists too many permissions".into()));
    }
    for p in &l.permissions {
        text_ok("a permission's name", &p.name, 64).map_err(&at)?;
        text_ok("a permission's reason", &p.reason, MAX_TEXT).map_err(&at)?;
        if p.hosts.len() > MAX_HOSTS || p.hosts.iter().any(|h| h.len() > 255 || h.chars().any(|c| c.is_control() || c.is_whitespace())) {
            return Err(at(format!("permission {} lists hosts it can't", p.name)));
        }
    }
    if l.screens.len() > MAX_SCREENS {
        return Err(at("it has too many screenshots".into()));
    }
    for s in &l.screens {
        let ok = s.strip_prefix(&format!("screens/{}/", l.id)).and_then(|f| f.split_once('.')).is_some_and(|(h, e)| is_sha256(h) && matches!(e, "png" | "webp"));
        if !ok {
            return Err(at(format!("screenshot {} isn't screens/{}/<sha256>.png or .webp", s.escape_debug(), l.id)));
        }
    }
    if l.keys.is_empty() || l.keys.len() > MAX_KEYS {
        return Err(at(format!("its author has 1 to {MAX_KEYS} keys")));
    }
    for k in &l.keys {
        Signer::from_public_key(k).map_err(|e| at(format!("a key: {e}")))?;
    }
    if l.versions.is_empty() || l.versions.len() > MAX_VERSIONS {
        return Err(at(format!("it lists 1 to {MAX_VERSIONS} versions")));
    }
    let mut seen = std::collections::HashSet::new();
    for r in &l.versions {
        check_version(&r.version).map_err(&at)?;
        if !seen.insert(r.version.as_str()) {
            return Err(at(format!("version {} is listed twice", r.version)));
        }
        check_release_url(&r.url, &l.repository).map_err(&at)?;
        if !is_sha256(&r.sha256) || !is_commit(&r.commit) {
            return Err(at(format!("version {}'s sha256 or commit isn't lower-case hex", r.version)));
        }
        if r.size == 0 || r.size > super::package::MAX_BYTES {
            return Err(at(format!("version {}'s size is out of range", r.version)));
        }
        if parse_time(&r.date).is_none() {
            return Err(at(format!("version {}'s date isn't YYYY-MM-DD", r.version)));
        }
        text_ok("a version's world", &r.world, 64).map_err(&at)?;
        if let Some(p) = &r.pitwall {
            if !p.strip_prefix(">=").is_some_and(|v| check_version(v).is_ok()) {
                return Err(at(format!("version {}'s pitwall isn't like >=0.14.0", r.version)));
            }
        }
        if r.changelog.as_ref().is_some_and(|c| c.len() > MAX_CHANGELOG) {
            return Err(at(format!("version {}'s changelog is longer than {MAX_CHANGELOG} bytes", r.version)));
        }
        if let Some(rev) = &r.review {
            text_ok("a review's maintainer", &rev.by, 64).map_err(&at)?;
            if parse_time(&rev.date).is_none() || rev.signature.len() as u64 > MAX_SIGNATURE_BYTES {
                return Err(at(format!("version {}'s review can't be read", r.version)));
            }
        }
    }
    Ok(())
}

/// Read `index.json` text `bytes`, which was checked against its
/// signature first: its format, at most [`MAX_INDEX_BYTES`] and
/// [`MAX_EXTENSIONS`], every listing within its limits (ids, names and
/// descriptions of plain text, https GitHub addresses, hex hashes, known
/// categories, valid keys), no id twice. Why not, in words.
pub fn parse_index(bytes: &[u8]) -> Result<Index, String> {
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err(format!("The index is larger than {}.", super::package::size_words(MAX_INDEX_BYTES)));
    }
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| format!("The index isn't JSON: {e}."))?;
    let format = v.get("format").and_then(|f| f.as_u64()).ok_or("The index has no format.")?;
    if format != FORMAT {
        return Err(format!("The index is in format {format}, which this Pitwall doesn't read: update Pitwall."));
    }
    if v.get("extensions").and_then(|e| e.as_array()).is_some_and(|a| a.len() > MAX_EXTENSIONS) {
        return Err(format!("The index lists more than {MAX_EXTENSIONS} extensions."));
    }
    let index: Index = serde_json::from_value(v).map_err(|e| format!("The index can't be read: {e}."))?;
    if parse_time(&index.generated).is_none_or(|_| index.generated.len() != 20) {
        return Err("The index's date can't be read.".into());
    }
    let mut ids = std::collections::HashSet::new();
    for l in &index.extensions {
        check_listing(l).map_err(|e| format!("The index can't be used: {e}."))?;
        if !ids.insert(l.id.as_str()) {
            return Err(format!("The index lists “{}” twice.", l.id));
        }
    }
    for d in &index.delisted {
        super::check_id(&d.id).and_then(|()| text_ok("a delisting's reason", &d.reason, MAX_TEXT)).map_err(|e| format!("The index can't be used: {e}."))?;
    }
    Ok(index)
}

/// Read `blocked.json` text `bytes`, checked against its signature
/// first, as [`parse_index`] does.
pub fn parse_blocked(bytes: &[u8]) -> Result<Blocked, String> {
    if bytes.len() as u64 > MAX_BLOCKED_BYTES {
        return Err(format!("The block list is larger than {}.", super::package::size_words(MAX_BLOCKED_BYTES)));
    }
    let b: Blocked = serde_json::from_slice(bytes).map_err(|e| format!("The block list can't be read: {e}."))?;
    if b.format != FORMAT {
        return Err(format!("The block list is in format {}, which this Pitwall doesn't read: update Pitwall.", b.format));
    }
    if parse_time(&b.generated).is_none_or(|_| b.generated.len() != 20) {
        return Err("The block list's date can't be read.".into());
    }
    if b.blocked.len() > MAX_BLOCKS {
        return Err(format!("The block list has more than {MAX_BLOCKS} rules."));
    }
    for r in &b.blocked {
        let ok = super::check_id(&r.id).and_then(|()| text_ok("a block's reason", &r.reason, MAX_TEXT));
        ok.map_err(|e| format!("The block list can't be used: {e}."))?;
        if r.versions.len() > MAX_VERSIONS || r.sha256.len() > MAX_VERSIONS || r.versions.iter().any(|v| check_version(v).is_err()) || r.sha256.iter().any(|h| !is_sha256(h)) || parse_time(&r.date).is_none() {
            return Err(format!("The block list's rule for “{}” can't be read.", r.id));
        }
    }
    Ok(b)
}

#[cfg(test)]
mod tests;
