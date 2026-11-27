//! Where Pitwall gets the index from ([`IndexSource`]): the index over
//! https ([`HttpsSource`]: a cache, conditional GETs with ETags, the
//! cached copy when offline, signatures checked against the pinned index
//! keys before anything is kept or used, never an older index than the
//! one it has), or a fixture for tests and previews ([`FixtureSource`]).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{parse_blocked, parse_index, parse_time, verify_by, Blocked, Index, IndexConfig, MAX_BLOCKED_BYTES, MAX_INDEX_BYTES, MAX_SIGNATURE_BYTES};
use crate::extensions::npm::GetError;
use crate::extensions::package::Signer;

/// How Pitwall reads one address of the index: the host's https client.
/// One GET of `url`, sending `If-None-Match: <etag>` when given; at most
/// `limit` bytes; no credentials, cookies or proxy; redirects only to
/// https. Plain http only to a loopback address (a test's).
pub trait IndexHttp: Send + Sync {
    fn get(&self, url: &str, etag: Option<&str>, limit: u64) -> Result<Answer, GetError>;
}

impl<T: IndexHttp + ?Sized> IndexHttp for std::sync::Arc<T> {
    fn get(&self, url: &str, etag: Option<&str>, limit: u64) -> Result<Answer, GetError> {
        (**self).get(url, etag, limit)
    }
}

/// What a conditional GET gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// 304: the copy Pitwall has is current.
    NotModified,
    /// 200, with the ETag the server sent.
    Body { bytes: Vec<u8>, etag: Option<String> },
}

/// Where the index Pitwall has came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Fetched now.
    Network,
    /// The cached copy, which the server says is current.
    NotModified,
    /// The cached copy, because fetching failed: why, in words.
    Cache { why: String },
    /// A fixture's.
    Fixture,
}

/// An index and its block list, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub index: Index,
    pub blocked: Blocked,
    /// The index's; the block list's is the same or better.
    pub origin: Origin,
}

/// Where the index comes from. A fetch either gives an index whose
/// signature checked, or says why there is none; it never gives one that
/// didn't check.
pub trait IndexSource: Send + Sync {
    fn fetch(&self) -> Result<Fetched, String>;
}

/// The index over https, cached in a folder of Pitwall's.
pub struct HttpsSource<H: IndexHttp> {
    http: H,
    config: IndexConfig,
    cache: PathBuf,
    /// One fetch at a time, so two don't write the cache at once.
    lock: Mutex<()>,
}

/// The two files of the index.
const INDEX: &str = "index.json";
const BLOCKED: &str = "blocked.json";

impl<H: IndexHttp> HttpsSource<H> {
    /// The index `config` names, fetched with `http`, cached in `cache`
    /// (made when needed).
    pub fn new(http: H, config: IndexConfig, cache: impl Into<PathBuf>) -> HttpsSource<H> {
        HttpsSource { http, config, cache: cache.into(), lock: Mutex::new(()) }
    }

    pub fn config(&self) -> &IndexConfig {
        &self.config
    }

    /// The cached copy of `name`, checked again: its bytes, signature and
    /// ETag.
    fn cached(&self, name: &str, max: u64) -> Option<(Vec<u8>, String, Option<String>)> {
        let bytes = read_max(&self.cache.join(name), max)?;
        let sig = String::from_utf8(read_max(&self.cache.join(format!("{name}.minisig")), MAX_SIGNATURE_BYTES)?).ok()?;
        verify_by(&bytes, &sig, &self.config.keys).ok()?;
        let etag = read_max(&self.cache.join(format!("{name}.etag")), 1024).and_then(|e| String::from_utf8(e).ok());
        Some((bytes, sig, etag))
    }

    /// File `name` of the index, fresh when it can be, checked; `date`
    /// reads its `generated` so an older one than the cache's is refused.
    fn file(&self, name: &str, max: u64, date: &dyn Fn(&[u8]) -> Option<String>) -> Result<(Vec<u8>, Origin), String> {
        if self.config.keys.is_empty() {
            return Err("No index key is set in this Pitwall, so the index can't be checked: not fetched.".into());
        }
        let cached = self.cached(name, max);
        let fallback = |why: String| -> Result<(Vec<u8>, Origin), String> {
            match &cached {
                Some((b, _, _)) => Ok((b.clone(), Origin::Cache { why })),
                None => Err(why),
            }
        };
        let url = self.config.url(name);
        if !url.starts_with("https://") && !crate::extensions::npm::is_loopback(&url) {
            return fallback("The index's address isn't https: not fetched.".into());
        }
        let etag = cached.as_ref().and_then(|c| c.2.as_deref());
        let (bytes, new_etag) = match self.http.get(&url, etag, max) {
            Ok(Answer::NotModified) if cached.is_some() => return Ok((cached.map(|c| c.0).unwrap_or_default(), Origin::NotModified)),
            Ok(Answer::NotModified) => match self.http.get(&url, None, max) {
                Ok(Answer::Body { bytes, etag }) => (bytes, etag),
                Ok(Answer::NotModified) => return Err("The index server answered “not modified” to a plain request.".into()),
                Err(e) => return fallback(get_words(e)),
            },
            Ok(Answer::Body { bytes, etag }) => (bytes, etag),
            Err(e) => return fallback(get_words(e)),
        };
        let sig = match self.http.get(&format!("{url}.minisig"), None, MAX_SIGNATURE_BYTES) {
            Ok(Answer::Body { bytes, .. }) => String::from_utf8(bytes).unwrap_or_default(),
            Ok(Answer::NotModified) => String::new(),
            Err(e) => return fallback(get_words(e)),
        };
        if let Err(why) = verify_by(&bytes, &sig, &self.config.keys) {
            return fallback(format!("The {name} Pitwall got didn't check: {why} Kept the copy it had."));
        }
        if let (Some(new), Some(old)) = (date(&bytes), cached.as_ref().and_then(|c| date(&c.0))) {
            if parse_time(&new) < parse_time(&old) {
                return fallback(format!("The {name} Pitwall got is older than the one it has ({new}, not {old}): kept the newer one."));
            }
        }
        // Kept only once checked, the body last so a half-written cache
        // never checks.
        let _ = std::fs::create_dir_all(&self.cache);
        let kept = write_atomic(&self.cache.join(format!("{name}.minisig")), sig.as_bytes())
            .and_then(|()| write_atomic(&self.cache.join(format!("{name}.etag")), new_etag.as_deref().unwrap_or_default().as_bytes()))
            .and_then(|()| write_atomic(&self.cache.join(name), &bytes));
        if kept.is_err() {
            let _ = std::fs::remove_file(self.cache.join(format!("{name}.etag")));
        }
        Ok((bytes, Origin::Network))
    }
}

impl<H: IndexHttp> IndexSource for HttpsSource<H> {
    fn fetch(&self) -> Result<Fetched, String> {
        let _one = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let date = |b: &[u8]| serde_json::from_slice::<serde_json::Value>(b).ok().and_then(|v| v.get("generated").and_then(|g| g.as_str()).map(str::to_string));
        let (ib, origin) = self.file(INDEX, MAX_INDEX_BYTES, &date)?;
        let index = parse_index(&ib)?;
        // No block list (never fetched, nothing cached): nothing blocked
        // yet, the index alone still usable; fetch failures change
        // nothing (permissions.md §5.4).
        let blocked = match self.file(BLOCKED, MAX_BLOCKED_BYTES, &date) {
            Ok((bb, _)) => parse_blocked(&bb)?,
            Err(_) => Blocked { format: super::FORMAT, generated: index.generated.clone(), blocked: Vec::new() },
        };
        Ok(Fetched { index, blocked, origin })
    }
}

fn get_words(e: GetError) -> String {
    match e {
        GetError::TooBig => "The index is larger than Pitwall reads.".into(),
        GetError::Failed(why) => why,
    }
}

fn read_max(path: &Path, max: u64) -> Option<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > max {
        return None;
    }
    std::fs::read(path).ok()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let part = path.with_extension("part");
    std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, path))
}

/// An index given whole, or read from a folder like `dist/v1/` and
/// checked as the https one is: for tests, previews and an index on disk.
pub struct FixtureSource {
    got: Result<Fetched, String>,
}

impl FixtureSource {
    /// An index and block list as given (no signatures).
    pub fn new(index: Index, blocked: Blocked) -> FixtureSource {
        FixtureSource { got: Ok(Fetched { index, blocked, origin: Origin::Fixture }) }
    }

    /// A source whose every fetch fails with `why`.
    pub fn failing(why: &str) -> FixtureSource {
        FixtureSource { got: Err(why.into()) }
    }

    /// `index.json` and `blocked.json` with their signatures in folder
    /// `dir`, checked against `keys` and parsed.
    pub fn dir(dir: &Path, keys: &[Signer]) -> FixtureSource {
        let one = |name: &str, max: u64| -> Result<Vec<u8>, String> {
            let bytes = read_max(&dir.join(name), max).ok_or_else(|| format!("{name} isn't in {}.", dir.display()))?;
            let sig = read_max(&dir.join(format!("{name}.minisig")), MAX_SIGNATURE_BYTES).and_then(|s| String::from_utf8(s).ok()).ok_or_else(|| format!("{name} has no signature."))?;
            verify_by(&bytes, &sig, keys).map_err(|why| format!("{name} didn't check: {why}"))?;
            Ok(bytes)
        };
        let got = one(INDEX, MAX_INDEX_BYTES).and_then(|b| parse_index(&b)).and_then(|index| {
            let blocked = parse_blocked(&one(BLOCKED, MAX_BLOCKED_BYTES)?)?;
            Ok(Fetched { index, blocked, origin: Origin::Fixture })
        });
        FixtureSource { got }
    }
}

impl IndexSource for FixtureSource {
    fn fetch(&self) -> Result<Fetched, String> {
        self.got.clone()
    }
}
