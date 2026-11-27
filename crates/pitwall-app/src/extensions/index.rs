//! The extensions index in the app (docs/spec/tile-sdk/marketplace.md §7):
//! where Settings → Extensions → Browse and the daily block-list check
//! read it from, over `pitwall_core::extensions::index` (its format,
//! signatures, Verified, takedown and download).
//!
//! **Which index.** [`set_setup`] (tests, previews), else
//! `PITWALL_EXTENSION_INDEX` (an https base like `…/dist/v1/`, or a
//! `dist/v1/` folder on this computer) with the index key it is signed by
//! in `PITWALL_EXTENSION_INDEX_KEY`, else the official index. The official
//! one isn't read while this Pitwall ships no index key
//! (`OFFICIAL_INDEX_KEYS` is empty until the index opens, §7.4): Browse
//! says it isn't open yet and nothing is fetched.
//!
//! **The daily check** ([`start_daily_check`]): once at start and every
//! day after, the index and its block list fetched; an installed copy the
//! block list covers is turned off ([`Blocked::turned_off`]) and its card
//! says why ([`note`]); one installed from the index that was withdrawn
//! or delisted says so too ([`installed_note`]).
//!
//! Nothing here installs anything: Install downloads a listed version
//! ([`download`]: refused when blocked or withdrawn, its SHA-256 and its
//! author's signature checked), then it goes through Install from file…'s
//! trust screen like any package, and is recorded as from the index.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Global};
use pitwall_core::extensions::npm::Http;
use pitwall_core::extensions::package::Signer;
use pitwall_core::extensions::permissions;

pub use pitwall_core::extensions::index::{category_name, download, installed_note, is_stale, refusal, report_url, Blocked, Fetched, FixtureSource, HttpsSource, Index, IndexConfig, IndexSource, Judge, ListedPermission, Listing, Release, ReportKind, Standing, CATEGORIES};

use super::https::Https;

/// The environment variable naming the index to read.
pub const ENV: &str = "PITWALL_EXTENSION_INDEX";
/// The environment variable with that index's public key.
pub const ENV_KEY: &str = "PITWALL_EXTENSION_INDEX_KEY";

/// An index to read: where from, how it is checked, and how its files
/// (screenshots, packages) are fetched.
pub struct Setup {
    pub source: Arc<dyn IndexSource>,
    pub config: IndexConfig,
    pub http: Arc<dyn Http + Send + Sync>,
    /// Where it is, as Browse says it.
    pub place: String,
}

impl Setup {
    pub fn new(source: Arc<dyn IndexSource>, config: IndexConfig, http: Arc<dyn Http + Send + Sync>, place: impl Into<String>) -> Setup {
        Setup { source, config, http, place: place.into() }
    }

    /// The official index, cached in `cache`; `None` while this Pitwall
    /// has no index key to check it with.
    pub fn official(cache: &Path) -> Option<Setup> {
        let config = IndexConfig::official();
        if config.keys.is_empty() {
            return None;
        }
        let source = Arc::new(HttpsSource::new(Https, config.clone(), cache));
        Some(Setup::new(source, config, Arc::new(Https), "the Pitwall extensions index"))
    }

    /// The index `value` names (an https base or an absolute folder),
    /// checked with `key`.
    pub fn from_env(value: &str, key: &str, cache: &Path) -> Option<Setup> {
        let v = value.trim();
        let key = Signer::from_public_key(key.trim()).ok()?;
        let mut config = IndexConfig::official();
        config.keys = vec![key];
        if v.starts_with("https://") || (v.starts_with("http://") && pitwall_core::extensions::npm::is_loopback(v)) {
            config.base = if v.ends_with('/') { v.to_string() } else { format!("{v}/") };
            let place = v.trim_start_matches("https://").to_string();
            return Some(Setup::new(Arc::new(HttpsSource::new(Https, config.clone(), cache)), config, Arc::new(Https), place));
        }
        let dir = Path::new(v);
        if !dir.is_absolute() {
            return None;
        }
        config.base = format!("{}/", dir.display());
        let source = Arc::new(FixtureSource::dir(dir, &config.keys));
        Some(Setup::new(source, config, Arc::new(Https), pitwall_core::paths::tildify(v)))
    }

    /// File `rel` of the index (a screenshot): over https, or from the
    /// folder it is in; at most `limit` bytes.
    pub fn get(&self, rel: &str, limit: u64) -> Result<Vec<u8>, String> {
        if rel.split('/').any(|p| p == ".." || p.is_empty()) {
            return Err(format!("“{rel}” isn't a file of the index."));
        }
        let base = &self.config.base;
        if base.starts_with("https://") || base.starts_with("http://") {
            return self.http.get(&self.config.url(rel), "*/*", limit, Duration::from_secs(10), Duration::from_secs(60)).map_err(|e| match e {
                pitwall_core::extensions::npm::GetError::TooBig => "It is larger than Pitwall reads.".to_string(),
                pitwall_core::extensions::npm::GetError::Failed(why) => why,
            });
        }
        let file = Path::new(base).join(rel);
        let bytes = std::fs::read(&file).map_err(|e| format!("Couldn't read {rel}: {e}."))?;
        if bytes.len() as u64 > limit {
            return Err("It is larger than Pitwall reads.".into());
        }
        Ok(bytes)
    }

    /// How this Pitwall judges listings, with `blocked`.
    pub fn judge<'a>(&'a self, blocked: &'a Blocked) -> Judge<'a> {
        Judge { review_keys: &self.config.review_keys, blocked, pitwall: pitwall_core::extensions::PITWALL, worlds: &pitwall_tile_host::WORLDS }
    }
}

/// The index read: the setup it came from, and what it gave.
#[derive(Clone)]
pub struct Read {
    pub setup: Arc<Setup>,
    pub fetched: Arc<Fetched>,
}

impl Read {
    pub fn judge(&self) -> Judge<'_> {
        self.setup.judge(&self.fetched.blocked)
    }
    pub fn index(&self) -> &Index {
        &self.fetched.index
    }
    /// Older than [`pitwall_core::extensions::index::STALE_AFTER_DAYS`].
    pub fn stale(&self) -> bool {
        is_stale(&self.fetched.index.generated, now())
    }
    /// The copy it is when the fetch failed, and why.
    pub fn cached_because(&self) -> Option<&str> {
        match &self.fetched.origin {
            pitwall_core::extensions::index::Origin::Cache { why } => Some(why),
            _ => None,
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

/// The setup used (`None`: none).
#[derive(Clone, Default)]
struct SetupHandle(Option<Arc<Setup>>);

impl Global for SetupHandle {}

/// The index read last, and what it says of installed copies.
#[derive(Clone, Default)]
struct Checked {
    last: Option<Read>,
    notes: HashMap<String, String>,
}

impl Global for Checked {}

/// Read `setup` (tests, previews; `None`: no index).
pub fn set_setup(setup: Option<Setup>, cx: &mut App) {
    cx.set_global(SetupHandle(setup.map(Arc::new)));
    cx.set_global(Checked::default());
}

/// The index to read: the one set, else the one `PITWALL_EXTENSION_INDEX`
/// names, else the official one (once it has a key).
pub fn setup(cx: &mut App) -> Option<Arc<Setup>> {
    if let Some(h) = cx.try_global::<SetupHandle>() {
        return h.0.clone();
    }
    let cache = cache_dir(cx);
    let s = match (std::env::var(ENV), std::env::var(ENV_KEY)) {
        (Ok(v), Ok(k)) if !v.trim().is_empty() => Setup::from_env(&v, &k, &cache),
        _ => Setup::official(&cache),
    }
    .map(Arc::new);
    cx.set_global(SetupHandle(s.clone()));
    s
}

fn cache_dir(cx: &App) -> PathBuf {
    crate::extensions::dir(cx).and_then(|d| d.parent().map(|p| p.join("extension-index"))).unwrap_or_else(|| std::env::temp_dir().join("pitwall-extension-index"))
}

/// Where downloads from the index go before their trust screen.
pub fn downloads_dir(cx: &App) -> Option<PathBuf> {
    crate::extensions::dir(cx).and_then(|d| d.parent().map(|p| p.join("extension-downloads")))
}

/// The index read last (by Browse or the daily check).
pub fn last(cx: &App) -> Option<Read> {
    cx.try_global::<Checked>()?.last.clone()
}

/// What the index says of installed extension `id` (turned off as
/// blocked, withdrawn, no longer listed).
pub fn note(id: &str, cx: &App) -> Option<String> {
    cx.try_global::<Checked>()?.notes.get(id).cloned()
}

/// The copies installed: id, version and (a package's) SHA-256, and
/// whether it came from the index.
fn installed(cx: &App) -> Vec<(String, String, Option<String>, bool)> {
    let Some(dir) = crate::extensions::dir(cx) else { return vec![] };
    crate::extensions::found(cx)
        .iter()
        .filter_map(|f| {
            let m = f.manifest.as_ref().ok()?;
            let (sha, from_index) = match pitwall_core::extensions::install::origin(&dir.join(&f.folder)) {
                Some(pitwall_core::extensions::install::Origin::Package { sha256, index, .. }) => (Some(sha256), index.is_some()),
                _ => (None, false),
            };
            Some((m.id.clone(), m.version.clone().unwrap_or_default(), sha, from_index))
        })
        .collect()
}

/// The index read now: kept as the last one, installed copies it blocks
/// turned off, and its notes on the others kept for their cards. The ids
/// turned off.
pub fn apply(read: Read, cx: &mut App) -> Vec<String> {
    let copies = installed(cx);
    let list: Vec<(String, String, Option<String>)> = copies.iter().map(|(i, v, s, _)| (i.clone(), v.clone(), s.clone())).collect();
    let off: Vec<String> = read.fetched.blocked.turned_off(&list).into_iter().map(|(id, _)| id).collect();
    let mut notes = HashMap::new();
    for (id, version, sha, from_index) in &copies {
        let blocked = off.contains(id);
        if blocked || *from_index {
            if let Some(n) = installed_note(&read.fetched.index, &read.fetched.blocked, id, version, sha.as_deref()) {
                notes.insert(id.clone(), n);
            }
        }
    }
    let ui = crate::ui_state::get(cx);
    let turn: Vec<String> = off.iter().filter(|id| ui.extension_on(id)).cloned().collect();
    if !turn.is_empty() {
        crate::ui_state::update(cx, |s| {
            for id in &turn {
                *s = s.clone().set_extension(id, false);
            }
        });
    }
    cx.set_global(Checked { last: Some(read), notes });
    turn
}

/// The index fetched off the UI thread.
pub fn fetch(setup: Arc<Setup>, cx: &App) -> gpui::Task<Result<Read, String>> {
    cx.background_executor().spawn(async move {
        let fetched = setup.source.fetch()?;
        Ok(Read { setup, fetched: Arc::new(fetched) })
    })
}

/// How often the block list is checked.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// At start: the index checked now and every day (nothing while there is
/// no index to read).
pub fn start_daily_check(cx: &mut App) {
    let Some(s) = setup(cx) else { return };
    cx.spawn(async move |cx| loop {
        let task = cx.update(|cx| fetch(s.clone(), cx));
        if let Ok(read) = task.await {
            cx.update(|cx| apply(read, cx));
        }
        cx.background_executor().timer(CHECK_EVERY).await;
    })
    .detach();
}

/// A listing's permissions, `net` first, then in the permission screen's
/// order.
pub fn asks(l: &Listing) -> Vec<&ListedPermission> {
    let rank = |n: &str| if n == permissions::NET { 0 } else { 1 + permissions::ALL.iter().position(|p| p.name == n).unwrap_or(permissions::ALL.len()) };
    let mut out: Vec<&ListedPermission> = l.permissions.iter().collect();
    out.sort_by_key(|p| rank(&p.name));
    out
}

/// What it may do, in words, for its card (§7.4): "Connects to
/// api.github.com · Reads your agents · Can send text to your agents".
pub fn summary(l: &Listing) -> String {
    let words: Vec<String> = asks(l).into_iter().filter_map(phrase).collect();
    if words.is_empty() {
        "Asks for no permissions".into()
    } else {
        words.join(" · ")
    }
}

/// One permission in a few words; `None` for those that give nothing
/// worth listing (memory, sql sizes).
fn phrase(p: &ListedPermission) -> Option<String> {
    Some(match p.name.as_str() {
        permissions::NET => match p.hosts.len() {
            0 => "Connects to servers".into(),
            1..=2 => format!("Connects to {}", p.hosts.join(", ")),
            n => format!("Connects to {} and {} more", p.hosts[0], n - 1),
        },
        permissions::AGENTS_READ => "Reads your agents".into(),
        permissions::PROJECTS_READ => "Reads your projects".into(),
        permissions::WORKTREES_READ => "Reads your worktrees".into(),
        permissions::PROFILES_READ => "Reads profile names".into(),
        permissions::SHOW => "Shows you files and pages".into(),
        permissions::AGENTS_FOCUS => "Can focus an agent".into(),
        permissions::AGENTS_QUEUE => "Can send text to your agents".into(),
        permissions::AGENTS_START => "Can start agents".into(),
        permissions::NOTIFY => "Notifies you".into(),
        permissions::FILES_READ => "Reads files in your projects".into(),
        permissions::FILES_WRITE => "Changes files in your projects".into(),
        permissions::MEMORY | permissions::SQL => return None,
        other => format!("Asks for “{other}”"),
    })
}

/// Its categories' names.
pub fn category_names(l: &Listing) -> Vec<&'static str> {
    l.categories.iter().filter_map(|c| category_name(c)).collect()
}

/// Why version `r` doesn't run here (an older Pitwall, a world it doesn't
/// serve); `None`: it does.
pub fn not_here(r: &Release, j: &Judge) -> Option<String> {
    if Listing::works_here(r, j) {
        return None;
    }
    if !j.worlds.contains(&r.world.as_str()) {
        return Some(format!("Built for {}, which this Pitwall doesn't serve", r.world));
    }
    Some(match r.pitwall.as_deref().and_then(|p| p.strip_prefix(">=")) {
        Some(min) => format!("Needs Pitwall {min} or later"),
        None => "Doesn't run on this Pitwall".into(),
    })
}

/// Its versions, newest first.
pub fn newest_first(l: &Listing) -> Vec<&Release> {
    let mut v: Vec<&Release> = l.versions.iter().collect();
    v.sort_by(|a, b| pitwall_core::extensions::install::compare_versions(&b.version, &a.version));
    v
}

/// What Browse shows of the index: the search, a category, the filters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    pub text: String,
    pub category: Option<String>,
    pub verified_only: bool,
    pub works_here: bool,
}

/// The listings `q` keeps, in order: those whose name matches first, then
/// by name. "Verified only" keeps those with a Verified version that may
/// be installed; "Works with this Pitwall", those with any version that
/// runs here.
pub fn search<'a>(index: &'a Index, q: &Query, j: &Judge) -> Vec<&'a Listing> {
    let words = q.text.trim().to_lowercase();
    let text = |l: &Listing| format!("{} {} {} {} {}", l.name, l.id, l.description, l.author, category_names(l).join(" "));
    let mut out: Vec<&Listing> = index
        .extensions
        .iter()
        .filter(|l| crate::palette::matching::matches(&text(l), &words))
        .filter(|l| q.category.as_ref().is_none_or(|c| l.categories.contains(c)))
        .filter(|l| !q.verified_only || l.verified_release(j).is_some())
        .filter(|l| !q.works_here || l.versions.iter().any(|r| Listing::works_here(r, j)))
        .collect();
    let in_name = |l: &Listing| !words.is_empty() && crate::palette::matching::matches(&l.name, &words);
    out.sort_by(|a, b| in_name(b).cmp(&in_name(a)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pitwall_core::testing::index::IndexFixture;

    pub(crate) const CI_PERMS: &str = r#""net": {"reason": "Read the runs of your branches", "hosts": ["api.github.com"]}, "agents.read": {"reason": "Match runs to your agents' branches"}, "agents.queue": {"reason": "Send a failing run's log to its agent"}"#;

    /// A fixture index: CI status (1.0.0 Verified, 1.1.0 not yet
    /// reviewed), DB browser (never reviewed), Old thing (needs a newer
    /// Pitwall).
    fn fixture() -> (IndexFixture, Fetched) {
        let f = IndexFixture::new();
        f.release("ci-status", "1.0.0", CI_PERMS);
        f.release("ci-status", "1.1.0", CI_PERMS);
        f.review("ci-status", "1.0.0");
        f.release("db-browser", "0.3.0", r#""files.read": {"reason": "Open the SQLite files in your project"}"#);
        f.edit("db-browser", |e| e.categories = vec!["databases".into()]);
        f.release("old-thing", "2.0.0", "");
        f.edit("old-thing", |e| e.categories = vec!["fun".into()]);
        f.publish("2026-11-27T10:00:00Z");
        let got = needs_newer(f.source().fetch().unwrap());
        (f, got)
    }

    /// `got` with Old thing needing a Pitwall from the future (the index
    /// build checks a package's requirement against its manifest, which
    /// the fixture's packages don't set).
    pub(crate) fn needs_newer(mut got: Fetched) -> Fetched {
        if let Some(l) = got.index.extensions.iter_mut().find(|l| l.id == "old-thing") {
            l.versions[0].pitwall = Some(">=99.0.0".into());
        }
        got
    }

    #[test]
    fn the_index_says_what_each_extension_may_do_and_which_version_to_install() {
        let (f, got) = fixture();
        let config = f.config();
        let j = Judge { review_keys: &config.review_keys, blocked: &got.blocked, pitwall: pitwall_core::extensions::PITWALL, worlds: &pitwall_tile_host::WORLDS };
        let ci = got.index.listing("ci-status").unwrap();
        assert_eq!(summary(ci), "Connects to api.github.com · Reads your agents · Can send text to your agents");
        // The newest is not yet reviewed: Install offers the Verified one.
        assert_eq!(ci.default_release(&j).unwrap().version, "1.0.0");
        assert_eq!(ci.standing(ci.release("1.1.0").unwrap(), &j), Standing::NotYetReviewed);
        let old = got.index.listing("old-thing").unwrap();
        assert_eq!(not_here(&old.versions[0], &j).as_deref(), Some("Needs Pitwall 99.0.0 or later"));
        let ids = |q: &Query| search(&got.index, q, &j).iter().map(|l| l.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&Query::default()), ["ci-status", "db-browser", "old-thing"]);
        assert_eq!(ids(&Query { verified_only: true, ..Default::default() }), ["ci-status"]);
        assert_eq!(ids(&Query { works_here: true, ..Default::default() }), ["ci-status", "db-browser"]);
        assert_eq!(ids(&Query { category: Some("databases".into()), ..Default::default() }), ["db-browser"]);
        assert_eq!(ids(&Query { text: "octo ci".into(), ..Default::default() }), ["ci-status"]);
    }

    #[test]
    fn an_index_named_by_the_environment_needs_its_key() {
        let t = pitwall_core::testing::TempDir::new("index-env");
        assert!(Setup::from_env("https://x.example/dist/v1", "not a key", t.path()).is_none());
        let key = pitwall_core::testing::Signing::new().signer().public_key;
        let s = Setup::from_env("https://x.example/dist/v1", &key, t.path()).unwrap();
        assert_eq!(s.config.url("screens/a.webp"), "https://x.example/dist/v1/screens/a.webp");
        assert!(Setup::from_env("relative/dist", &key, t.path()).is_none());
        assert!(s.get("../secret", 10).is_err());
    }
}
