//! A local extensions index for tests (marketplace.md §7): an index
//! repository in a temporary folder, tile packages released at made-up
//! GitHub addresses served from memory ([`MapHttp`]), throwaway author,
//! index and review keys, and the built `dist/v1/` signed. Nothing
//! reaches the network.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use super::{Signing, TempDir};
use crate::extensions::index::repo::{self, Author, Built, Entry, EntryReview, EntryVersion, Maintainer, Maintainers};
use crate::extensions::index::{review_statement, Answer, Block, FixtureSource, Index, IndexConfig, IndexHttp};
use crate::extensions::npm::{GetError, Http, OFFLINE};
use crate::extensions::package::{self, zip};

/// Addresses served from memory, as both the package downloads
/// ([`Http`]) and the index fetches ([`IndexHttp`], with an ETag that is
/// the body's SHA-256) see them. Anything else is a 404.
#[derive(Default)]
pub struct MapHttp {
    files: Mutex<HashMap<String, Vec<u8>>>,
    offline: Mutex<bool>,
    requests: Mutex<Vec<(String, Option<String>)>>,
}

impl MapHttp {
    pub fn new() -> MapHttp {
        MapHttp::default()
    }

    /// Serve `bytes` at `url`.
    pub fn put(&self, url: &str, bytes: impl Into<Vec<u8>>) {
        self.files.lock().unwrap().insert(url.to_string(), bytes.into());
    }

    /// Stop serving `url`.
    pub fn remove(&self, url: &str) {
        self.files.lock().unwrap().remove(url);
    }

    /// Every request fails as if offline (or works again).
    pub fn set_offline(&self, offline: bool) {
        *self.offline.lock().unwrap() = offline;
    }

    /// The requests made, with the ETag each sent.
    pub fn requests(&self) -> Vec<(String, Option<String>)> {
        self.requests.lock().unwrap().clone()
    }

    fn body(&self, url: &str, etag: Option<&str>, limit: u64) -> Result<Vec<u8>, GetError> {
        self.requests.lock().unwrap().push((url.to_string(), etag.map(str::to_string)));
        if *self.offline.lock().unwrap() {
            return Err(GetError::Failed(OFFLINE.into()));
        }
        let b = self.files.lock().unwrap().get(url).cloned().ok_or_else(|| GetError::Failed("The server answered 404.".into()))?;
        if b.len() as u64 > limit {
            return Err(GetError::TooBig);
        }
        Ok(b)
    }
}

impl Http for MapHttp {
    fn get(&self, url: &str, _accept: &str, limit: u64, _connect: Duration, _timeout: Duration) -> Result<Vec<u8>, GetError> {
        self.body(url, None, limit)
    }
}

impl IndexHttp for MapHttp {
    fn get(&self, url: &str, etag: Option<&str>, limit: u64) -> Result<Answer, GetError> {
        let b = self.body(url, etag, limit)?;
        let tag = format!("\"{}\"", package::sha256(&b));
        if etag == Some(tag.as_str()) {
            return Ok(Answer::NotModified);
        }
        Ok(Answer::Body { bytes: b, etag: Some(tag) })
    }
}

/// The smallest component header: enough for a package's checks.
pub const COMPONENT: [u8; 8] = [0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00];

/// A tile package of `id` at `version` asking for `permissions` (the
/// inside of a manifest's `"permissions": {…}`), holding `wasm` as its
/// component, a readme, a licence, a changelog and a screenshot.
pub fn tile_package(id: &str, version: &str, permissions: &str, wasm: &[u8]) -> Vec<u8> {
    let manifest = format!(
        r#"{{"v": 2, "id": "{id}", "name": "{id} tile", "version": "{version}", "description": "A tile for tests", "icon": "check", "permissions": {{{permissions}}}, "contributes": {{"tiles": [{{"id": "main", "title": "Main", "wasm": "tile.wasm"}}]}}}}"#
    );
    let files = vec![
        ("CHANGELOG.md".to_string(), format!("# Changelog\n\n## {version}\n\n- Made for a test.\n\n## 0.0.1\n\n- Older.\n").into_bytes()),
        ("LICENSE".to_string(), b"MIT".to_vec()),
        ("README.md".to_string(), format!("# {id}\n").into_bytes()),
        ("extension.json".to_string(), manifest.into_bytes()),
        ("screenshots/main.png".to_string(), vec![137u8, 80, 78, 71, version.len() as u8]),
        ("tile.wasm".to_string(), wasm.to_vec()),
    ];
    zip::write(&files)
}

/// A local index repository with one author, one maintainer and the keys
/// to sign it all.
pub struct IndexFixture {
    pub dir: TempDir,
    pub http: std::sync::Arc<MapHttp>,
    /// The author `octo`'s key (GitHub login `octo`).
    pub author: Signing,
    /// The index key.
    pub index_key: Signing,
    /// The maintainer `max`'s review key.
    pub reviewer: Signing,
}

impl Default for IndexFixture {
    fn default() -> IndexFixture {
        IndexFixture::new()
    }
}

/// Where the fixture's built index is served.
pub const BASE: &str = "https://index.test/dist/v1/";

impl IndexFixture {
    pub fn new() -> IndexFixture {
        let f = IndexFixture { dir: TempDir::new("index"), http: std::sync::Arc::new(MapHttp::new()), author: Signing::new(), index_key: Signing::new(), reviewer: Signing::new() };
        let root = f.root();
        std::fs::create_dir_all(root.join("extensions")).unwrap();
        std::fs::create_dir_all(root.join("authors")).unwrap();
        std::fs::write(root.join("categories.toml"), repo::categories_toml()).unwrap();
        let m = Maintainers { maintainers: vec![Maintainer { handle: "max".into(), github: "max".into(), review_key: f.reviewer.signer().public_key }] };
        std::fs::write(root.join("maintainers.toml"), toml::to_string(&m).unwrap()).unwrap();
        f.write_author("octo", "octo", &[f.author.signer().public_key]);
        f
    }

    /// The repository's folder.
    pub fn root(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    /// `authors/<handle>.toml`.
    pub fn write_author(&self, handle: &str, github: &str, keys: &[String]) {
        let a = Author { handle: handle.into(), github: github.into(), keys: keys.to_vec(), contact: None };
        std::fs::write(self.root().join(format!("authors/{handle}.toml")), repo::author_toml(&a)).unwrap();
    }

    /// The repository of extension `id`.
    pub fn repository(id: &str) -> String {
        format!("https://github.com/octo/pitwall-{id}")
    }

    /// Release version `version` of `id`, signed by `signer`, with
    /// `permissions`, and list it in `extensions/<id>.toml` (made the
    /// first time). The entry version written.
    pub fn release_signed(&self, id: &str, version: &str, permissions: &str, signer: &Signing) -> EntryVersion {
        self.release_with(id, version, permissions, &COMPONENT, signer)
    }

    /// [`Self::release_signed`] with `wasm` as its component (a real one,
    /// for a headless run).
    pub fn release_with(&self, id: &str, version: &str, permissions: &str, wasm: &[u8], signer: &Signing) -> EntryVersion {
        let bytes = tile_package(id, version, permissions, wasm);
        let url = format!("{}/releases/download/v{version}/{id}-{version}.pwtile", Self::repository(id));
        self.http.put(&url, bytes.clone());
        self.http.put(&format!("{url}.minisig"), signer.sign(&bytes));
        let v = EntryVersion { version: version.into(), url, sha256: package::sha256(&bytes), commit: "4f1c".repeat(10), pitwall: None, world: repo::DEFAULT_WORLD.into(), yanked: false, review: None };
        let path = self.root().join(format!("extensions/{id}.toml"));
        let e = match std::fs::read_to_string(&path) {
            Ok(text) => repo::add_version(&text, v.clone()).unwrap(),
            Err(_) => Entry { id: id.into(), author: "octo".into(), repository: Self::repository(id), categories: vec!["ci".into()], license: "MIT".into(), delisted: None, versions: vec![v.clone()] },
        };
        std::fs::write(path, repo::entry_toml(&e)).unwrap();
        v
    }

    /// [`Self::release_signed`] by the author's key.
    pub fn release(&self, id: &str, version: &str, permissions: &str) -> EntryVersion {
        self.release_signed(id, version, permissions, &self.author)
    }

    /// Change entry `id` with `f`.
    pub fn edit(&self, id: &str, f: impl FnOnce(&mut Entry)) {
        let path = self.root().join(format!("extensions/{id}.toml"));
        let mut e: Entry = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        f(&mut e);
        std::fs::write(path, repo::entry_toml(&e)).unwrap();
    }

    /// Mark version `version` of `id` Verified by maintainer `max`.
    pub fn review(&self, id: &str, version: &str) {
        let signature = self.reviewer.sign_bare(review_statement(id, version, &self.sha256(id, version)).as_bytes());
        self.edit(id, |e| {
            let v = e.versions.iter_mut().find(|v| v.version == version).unwrap();
            v.review = Some(EntryReview { by: "max".into(), date: "2026-11-27".into(), reproducible: true, signature });
        });
    }

    /// The listed SHA-256 of `id` `version`.
    pub fn sha256(&self, id: &str, version: &str) -> String {
        let e: Entry = toml::from_str(&std::fs::read_to_string(self.root().join(format!("extensions/{id}.toml"))).unwrap()).unwrap();
        e.versions.into_iter().find(|v| v.version == version).unwrap().sha256
    }

    /// Yank `id` `version`.
    pub fn yank(&self, id: &str, version: &str) {
        self.edit(id, |e| e.versions.iter_mut().filter(|v| v.version == version).for_each(|v| v.yanked = true));
    }

    /// Add `block` to `blocked.toml`.
    pub fn block(&self, block: Block) {
        let path = self.root().join("blocked.toml");
        let mut b: repo::BlockedFile = std::fs::read_to_string(&path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
        b.blocked.push(block);
        std::fs::write(path, toml::to_string(&b).unwrap()).unwrap();
    }

    /// Load and build the repository at `now`.
    pub fn build(&self, now: &str, previous: Option<&Index>) -> Result<Built, Vec<String>> {
        let r = repo::load(&self.root())?;
        repo::build(&r, previous, self.http.as_ref(), now)
    }

    /// Build at `now`, sign with the index key, write it to `dist/v1/`
    /// and serve it at [`BASE`]. The build.
    pub fn publish(&self, now: &str) -> Built {
        let b = self.build(now, None).unwrap_or_else(|p| panic!("the fixture index builds: {p:?}"));
        let dist = self.dist();
        std::fs::create_dir_all(&dist).unwrap();
        for (name, bytes) in [("index.json", &b.index_json), ("blocked.json", &b.blocked_json)] {
            let sig = self.index_key.sign_bare(bytes);
            std::fs::write(dist.join(name), bytes).unwrap();
            std::fs::write(dist.join(format!("{name}.minisig")), &sig).unwrap();
            self.http.put(&format!("{BASE}{name}"), bytes.clone());
            self.http.put(&format!("{BASE}{name}.minisig"), sig);
        }
        b
    }

    /// The built `dist/v1/`.
    pub fn dist(&self) -> PathBuf {
        self.root().join("dist/v1")
    }

    /// The index config for it: served at [`BASE`], its index and review
    /// keys trusted.
    pub fn config(&self) -> IndexConfig {
        IndexConfig { base: BASE.into(), repo: "pitwalls/pitwall-extensions".into(), keys: vec![self.index_key.signer()], review_keys: vec![self.reviewer.signer()] }
    }

    /// A source reading the built `dist/v1/` from disk.
    pub fn source(&self) -> FixtureSource {
        FixtureSource::dir(&self.dist(), &[self.index_key.signer()])
    }
}
