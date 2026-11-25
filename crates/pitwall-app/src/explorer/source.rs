//! Where the explorer reads from: `pitwall-core`'s explorer module on the
//! hosted engine (the Tauri commands `list_files`, `list_all_files`,
//! `read_file`, `search_files`, `cancel_search` call the same functions).
//! Behind a trait so views can be tested with made-up folders. Every call
//! blocks (git, disk, agw over ssh): callers run them on the background
//! executor.

use std::sync::Arc;

use gpui::{App, Global};

use pitwall_core::explorer::{
    self as core, DirListing, FileIndex, FileView, SearchQuery, SearchResult,
};
use pitwall_core::Shared;

pub type Res<T> = Result<T, String>;

/// The explorer's backend calls (docs/spec/explorer.md "API").
pub trait Source: Send + Sync + 'static {
    /// One folder's children (`ignored`: "Show ignored files").
    fn list_files(&self, agent_id: &str, dir: &str, ignored: bool) -> Res<DirListing>;
    /// Every file, for quick open.
    fn list_all_files(&self, agent_id: &str) -> Res<FileIndex>;
    /// One file (`large`: "Load anyway", up to 10 MiB).
    fn read_file(&self, agent_id: &str, path: &str, large: bool) -> Res<FileView>;
    /// ripgrep or git grep; a newer search for the agent cancels this one.
    fn search(&self, agent_id: &str, query: &SearchQuery) -> Res<SearchResult>;
    fn cancel_search(&self, agent_id: &str);
    /// A file on this machine outside the agent's folder, by its absolute
    /// path (a ⌘-clicked link).
    fn read_outside(&self, path: &str, large: bool) -> Res<FileView> {
        core::read_local_file(path, large)
    }
    /// An image's bytes (one `read_file` said is an image).
    fn read_image(&self, agent_id: &str, path: &str) -> Res<Vec<u8>>;
    /// [`read_image`](Self::read_image) for a file outside the agent's
    /// folder, on this machine.
    fn read_outside_image(&self, path: &str) -> Res<Vec<u8>> {
        core::read_local_image(path)
    }
    /// The user saves `text` they edited over `path` (`base`: the file as
    /// it was read; it changed since: nothing written). Files of agents on
    /// this computer and `path:` folders only; agents never reach this.
    fn write_file(&self, agent_id: &str, path: &str, text: &str, base: Option<&str>) -> Res<core::Saved> {
        let _ = (agent_id, text, base);
        Err(format!("{path} can't be saved here"))
    }
}

/// What the viewer reads is named by a key: an agent's id (its folder, on
/// its machine), or `path:` and a folder on this computer (an agent asked
/// to show a folder outside every agent's: `core::explorer::local`).
pub const PATH_KEY: &str = "path:";

/// The key of folder `root` on this computer.
pub fn path_key(root: &str) -> String {
    format!("{PATH_KEY}{root}")
}

/// The folder a `path:` key names (`None`: an agent's id).
pub fn root_of_key(key: &str) -> Option<&str> {
    key.strip_prefix(PATH_KEY)
}

/// The hosted engine.
pub struct EngineSource(pub Shared);

impl Source for EngineSource {
    fn list_files(&self, agent_id: &str, dir: &str, ignored: bool) -> Res<DirListing> {
        match root_of_key(agent_id) {
            Some(root) => core::local::list_files(root, dir, ignored),
            None => core::list_files(&self.0, agent_id, dir, ignored),
        }
    }
    fn list_all_files(&self, agent_id: &str) -> Res<FileIndex> {
        match root_of_key(agent_id) {
            Some(root) => core::local::list_all_files(root),
            None => core::list_all_files(&self.0, agent_id),
        }
    }
    fn read_file(&self, agent_id: &str, path: &str, large: bool) -> Res<FileView> {
        match root_of_key(agent_id) {
            Some(root) => core::local::read_file(root, path, large),
            None => core::read_file(&self.0, agent_id, path, large),
        }
    }
    fn search(&self, agent_id: &str, query: &SearchQuery) -> Res<SearchResult> {
        match root_of_key(agent_id) {
            Some(root) => core::local::search(&self.0, agent_id, root, query),
            None => core::search(&self.0, agent_id, query),
        }
    }
    fn cancel_search(&self, agent_id: &str) {
        core::cancel_search(&self.0, agent_id)
    }
    fn read_image(&self, agent_id: &str, path: &str) -> Res<Vec<u8>> {
        match root_of_key(agent_id) {
            Some(root) => core::local::read_image(root, path),
            None => core::read_image(&self.0, agent_id, path),
        }
    }
    fn write_file(&self, agent_id: &str, path: &str, text: &str, base: Option<&str>) -> Res<core::Saved> {
        match root_of_key(agent_id) {
            Some(root) => core::local::write_file(root, path, text, base),
            None => core::write_file(&self.0, agent_id, path, text, base),
        }
    }
}

/// The app's explorer backend (set once the engine is hosted).
#[derive(Clone)]
pub struct ExplorerSource(pub Arc<dyn Source>);

impl Global for ExplorerSource {}

impl ExplorerSource {
    pub fn engine(engine: Shared) -> ExplorerSource {
        ExplorerSource(Arc::new(EngineSource(engine)))
    }

    pub fn get(cx: &App) -> Option<Arc<dyn Source>> {
        cx.try_global::<ExplorerSource>().map(|s| s.0.clone())
    }
}

/// The answer when a search was cancelled (by a newer one or by leaving).
pub const CANCELLED: &str = "cancelled";

/// Text up to this size is read without asking (core `read::MAX_TEXT`).
pub const TEXT_CAP: u64 = 2 * 1024 * 1024;
/// "Load anyway" reads up to this size (core `read::MAX_LARGE`).
pub const LARGE_CAP: u64 = 10 * 1024 * 1024;

#[cfg(test)]
pub mod fake {
    //! A made-up folder in memory, for tests.

    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use pitwall_core::explorer::{
        ContentKind, EntryKind, FileEntry, MatchRange, SearchEngine, SearchMatch,
    };

    use super::*;

    #[derive(Default)]
    pub struct FakeSource {
        /// Path → contents (folders are implied).
        pub files: Mutex<BTreeMap<String, String>>,
        /// Ignored paths (listed only with `ignored`).
        pub ignored: Mutex<Vec<String>>,
        pub lists: AtomicUsize,
        pub cancels: AtomicUsize,
        /// Saves written.
        pub writes: AtomicUsize,
        /// Saving is refused (another machine's agent, say).
        pub refuse_writes: std::sync::atomic::AtomicBool,
    }

    impl FakeSource {
        pub fn new(files: &[(&str, &str)]) -> Arc<FakeSource> {
            Arc::new(FakeSource {
                files: Mutex::new(
                    files
                        .iter()
                        .map(|(p, t)| (p.to_string(), t.to_string()))
                        .collect(),
                ),
                ..Default::default()
            })
        }
    }

    impl Source for FakeSource {
        fn list_files(&self, _: &str, dir: &str, ignored: bool) -> Res<DirListing> {
            self.lists.fetch_add(1, Ordering::SeqCst);
            let files = self.files.lock().unwrap();
            let hidden = self.ignored.lock().unwrap();
            let prefix = if dir.is_empty() {
                String::new()
            } else {
                format!("{dir}/")
            };
            if !dir.is_empty() && !files.keys().any(|p| p.starts_with(&prefix)) {
                return Err(format!("{dir}: not found"));
            }
            let mut entries: BTreeMap<String, FileEntry> = BTreeMap::new();
            for p in files.keys() {
                let Some(rest) = p.strip_prefix(&prefix) else {
                    continue;
                };
                let is_ignored = hidden.iter().any(|h| p.starts_with(h.as_str()));
                if is_ignored && !ignored {
                    continue;
                }
                let (name, kind) = match rest.split_once('/') {
                    Some((d, _)) => (d, EntryKind::Dir),
                    None => (rest, EntryKind::File),
                };
                entries.entry(name.to_string()).or_insert(FileEntry {
                    name: name.into(),
                    path: format!("{prefix}{name}"),
                    kind,
                    status: None,
                    changes: 0,
                    ignored: is_ignored,
                });
            }
            let mut entries: Vec<FileEntry> = entries.into_values().collect();
            entries.sort_by_key(|e| (e.kind != EntryKind::Dir, e.name.to_lowercase()));
            Ok(DirListing {
                dir: dir.into(),
                entries,
                truncated: false,
                git: true,
            })
        }

        fn list_all_files(&self, _: &str) -> Res<FileIndex> {
            let files = self.files.lock().unwrap().keys().cloned().collect();
            Ok(FileIndex {
                files,
                truncated: false,
                git: true,
            })
        }

        fn read_file(&self, _: &str, path: &str, large: bool) -> Res<FileView> {
            let files = self.files.lock().unwrap();
            let text = files.get(path).ok_or(format!("{path}: not found"))?;
            let size = text.len() as u64;
            let cap = if large { LARGE_CAP } else { TEXT_CAP };
            let (kind, text) = if pitwall_core::explorer::is_svg(path) {
                // Drawn, its text sent too (as the core does).
                (ContentKind::Image, Some(text.clone()))
            } else if pitwall_core::explorer::is_image(path) {
                (ContentKind::Image, None)
            } else if pitwall_core::explorer::is_pdf(path) || text.contains('\0') {
                (ContentKind::Binary, None)
            } else if size > cap {
                (ContentKind::TooLarge, None)
            } else {
                (ContentKind::Text, Some(text.clone()))
            };
            Ok(FileView {
                path: path.into(),
                size,
                kind,
                text,
                lang: None,
                lossy: false,
            })
        }

        fn search(&self, _: &str, q: &SearchQuery) -> Res<SearchResult> {
            let files = self.files.lock().unwrap();
            let mut matches = Vec::new();
            for (path, text) in files.iter() {
                for (i, line) in text.lines().enumerate() {
                    if let Some(b) = line.find(&q.query) {
                        let start = line[..b].encode_utf16().count() as u32;
                        let end = start + q.query.encode_utf16().count() as u32;
                        matches.push(SearchMatch {
                            path: path.clone(),
                            line: i as u32 + 1,
                            column: line[..b].chars().count() as u32 + 1,
                            text: line.into(),
                            text_offset: 0,
                            ranges: vec![MatchRange { start, end }],
                        });
                    }
                }
            }
            let files = group_count(&matches);
            Ok(SearchResult {
                matches,
                files,
                truncated: false,
                engine: SearchEngine::Ripgrep,
            })
        }

        fn cancel_search(&self, _: &str) {
            self.cancels.fetch_add(1, Ordering::SeqCst);
        }

        fn read_image(&self, _: &str, path: &str) -> Res<Vec<u8>> {
            let files = self.files.lock().unwrap();
            files.get(path).map(|t| t.as_bytes().to_vec()).ok_or(format!("{path}: not found"))
        }

        fn write_file(&self, _: &str, path: &str, text: &str, base: Option<&str>) -> Res<pitwall_core::explorer::Saved> {
            if self.refuse_writes.load(Ordering::SeqCst) {
                return Err("Files of agents on another machine can't be edited here.".into());
            }
            let mut files = self.files.lock().unwrap();
            let now = files.get(path).ok_or(format!("{path}: not found"))?;
            if base.is_some_and(|b| b != now) {
                return Ok(pitwall_core::explorer::Saved::ChangedOnDisk);
            }
            files.insert(path.to_string(), text.to_string());
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(pitwall_core::explorer::Saved::Written)
        }
    }

    /// Folders on this disk, read as the app reads `path:` keys
    /// (`core::local`, with their change letters); an agent's id reads its
    /// folder.
    pub struct DiskSource {
        /// Agent id → its folder.
        pub agents: Mutex<std::collections::HashMap<String, String>>,
    }

    impl DiskSource {
        pub fn new(agents: &[(&str, &str)]) -> Arc<DiskSource> {
            Arc::new(DiskSource { agents: Mutex::new(agents.iter().map(|(a, f)| (a.to_string(), f.to_string())).collect()) })
        }

        fn root(&self, key: &str) -> Res<String> {
            match root_of_key(key) {
                Some(r) => Ok(r.to_string()),
                None => self.agents.lock().unwrap().get(key).cloned().ok_or(format!("no agent {key}")),
            }
        }
    }

    impl Source for DiskSource {
        fn list_files(&self, key: &str, dir: &str, ignored: bool) -> Res<DirListing> {
            core::local::list_files(&self.root(key)?, dir, ignored)
        }
        fn list_all_files(&self, key: &str) -> Res<FileIndex> {
            core::local::list_all_files(&self.root(key)?)
        }
        fn read_file(&self, key: &str, path: &str, large: bool) -> Res<FileView> {
            core::local::read_file(&self.root(key)?, path, large)
        }
        fn search(&self, _: &str, _: &SearchQuery) -> Res<SearchResult> {
            Err("no search here".into())
        }
        fn cancel_search(&self, _: &str) {}
        fn read_image(&self, key: &str, path: &str) -> Res<Vec<u8>> {
            core::local::read_image(&self.root(key)?, path)
        }
        fn write_file(&self, key: &str, path: &str, text: &str, base: Option<&str>) -> Res<pitwall_core::explorer::Saved> {
            core::local::write_file(&self.root(key)?, path, text, base)
        }
    }

    fn group_count(m: &[SearchMatch]) -> u32 {
        let mut p: Vec<&str> = m.iter().map(|m| m.path.as_str()).collect();
        p.dedup();
        p.len() as u32
    }
}
