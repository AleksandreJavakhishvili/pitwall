//! An extension's own data, outside its installed folder so it survives
//! updates and Go back (packaging.md §6.4):
//! `<data>/extension-data/<id>/`, private to the user, holding
//! - `kv.bin`: the `storage` interface's key-value store ([`KvStore`]);
//! - `settings.json`: the values of the manifest's settings, as Settings
//!   keeps them (`{"key": "value"}`, strings as `SettingDef` stores them;
//!   secrets are never here, they are in the system's secure store).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::limits::{MAX_STORAGE_KEY, STORAGE_BYTES};
use crate::platform;

/// The folder of extension `id`'s data under Pitwall's data folder.
pub fn extension_data_dir(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join("extension-data").join(id)
}

const KV_FILE: &str = "kv.bin";
const SETTINGS_FILE: &str = "settings.json";
const KV_MAGIC: &[u8] = b"PWKV1\n";

/// An extension's data folder: where its settings and stored data live.
/// The app uses it for the Settings form (read and write values; the
/// running tile then needs [`crate::TileHandle::settings_changed`]).
#[derive(Debug, Clone)]
pub struct ExtensionData {
    dir: PathBuf,
}

impl ExtensionData {
    /// Extension `id`'s data under Pitwall's data folder `data_dir`.
    pub fn new(data_dir: &Path, id: &str) -> ExtensionData {
        ExtensionData { dir: extension_data_dir(data_dir, id) }
    }

    /// The folder (it may not exist yet).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The settings the user set, as stored strings by key. Empty when
    /// none are set (or the file can't be read).
    pub fn settings(&self) -> BTreeMap<String, String> {
        std::fs::read(self.dir.join(SETTINGS_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Set setting `key` to `value` as it is stored (`None`: back to its
    /// default). Check the value first with `SettingDef::check_value`;
    /// never pass a secret.
    pub fn set_setting(&self, key: &str, value: Option<&str>) -> std::io::Result<()> {
        let mut all = self.settings();
        match value {
            Some(v) => all.insert(key.to_string(), v.to_string()),
            None => all.remove(key),
        };
        platform::private_dir(&self.dir)?;
        let bytes = serde_json::to_vec_pretty(&all).map_err(std::io::Error::other)?;
        platform::write_private(&self.dir.join(SETTINGS_FILE), &bytes)
    }

    /// Bytes its key-value store takes on disk.
    pub fn storage_bytes(&self) -> u64 {
        std::fs::metadata(self.dir.join(KV_FILE)).map(|m| m.len()).unwrap_or(0)
    }
}

/// Why a storage call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KvError {
    BadKey(String),
    Full,
}

/// The `storage` interface's store: all of it in memory (at most 10 MiB),
/// written to `kv.bin` whole, atomically, after a call that changed it.
pub(crate) struct KvStore {
    path: PathBuf,
    map: BTreeMap<String, Vec<u8>>,
    used: u64,
    dirty: bool,
}

impl KvStore {
    /// Open the store in `dir`. A file that can't be read is set aside as
    /// `kv.bin.bad` and the store starts empty; the warning says so.
    pub fn open(dir: &Path) -> (KvStore, Option<String>) {
        let path = dir.join(KV_FILE);
        let mut warning = None;
        let map = match std::fs::read(&path) {
            Ok(bytes) => match decode(&bytes) {
                Some(map) => map,
                None => {
                    let _ = std::fs::rename(&path, dir.join("kv.bin.bad"));
                    warning = Some("its stored data couldn't be read; it was set aside as kv.bin.bad and the store starts empty".to_string());
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        let used = map.iter().map(|(k, v)| (k.len() + v.len()) as u64).sum();
        (KvStore { path, map, used, dirty: false }, warning)
    }

    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>, KvError> {
        check_key(key)?;
        Ok(self.map.get(key).cloned())
    }

    pub fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), KvError> {
        check_key(&key)?;
        let old = self.map.get(&key).map_or(0, |v| (key.len() + v.len()) as u64);
        let new = (key.len() + value.len()) as u64;
        if self.used - old + new > STORAGE_BYTES {
            return Err(KvError::Full);
        }
        self.used = self.used - old + new;
        self.map.insert(key, value);
        self.dirty = true;
        Ok(())
    }

    pub fn delete(&mut self, key: &str) -> Result<(), KvError> {
        check_key(key)?;
        if let Some(v) = self.map.remove(key) {
            self.used -= (key.len() + v.len()) as u64;
            self.dirty = true;
        }
        Ok(())
    }

    pub fn list(&self, prefix: &str) -> Vec<String> {
        self.map.range(prefix.to_string()..).take_while(|(k, _)| k.starts_with(prefix)).map(|(k, _)| k.clone()).collect()
    }

    pub fn usage(&self) -> u64 {
        self.used
    }

    /// Write the store if it changed since the last flush.
    pub fn flush(&mut self) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(dir) = self.path.parent() {
            platform::private_dir(dir)?;
        }
        platform::write_private(&self.path, &encode(&self.map))?;
        self.dirty = false;
        Ok(())
    }
}

fn check_key(key: &str) -> Result<(), KvError> {
    if key.is_empty() || key.len() > MAX_STORAGE_KEY {
        return Err(KvError::BadKey(format!("a key must be 1 to {MAX_STORAGE_KEY} bytes")));
    }
    if key.chars().any(char::is_control) {
        return Err(KvError::BadKey("a key can't have control characters".into()));
    }
    Ok(())
}

/// `PWKV1\n`, then for each entry: key length (u32 LE), key, value length
/// (u32 LE), value.
fn encode(map: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = KV_MAGIC.to_vec();
    for (k, v) in map {
        out.extend_from_slice(&(k.len() as u32).to_le_bytes());
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(&(v.len() as u32).to_le_bytes());
        out.extend_from_slice(v);
    }
    out
}

fn decode(bytes: &[u8]) -> Option<BTreeMap<String, Vec<u8>>> {
    let mut rest = bytes.strip_prefix(KV_MAGIC)?;
    let mut map = BTreeMap::new();
    let take = |rest: &mut &[u8]| -> Option<Vec<u8>> {
        let (len, tail) = rest.split_first_chunk::<4>()?;
        let len = u32::from_le_bytes(*len) as usize;
        if tail.len() < len {
            return None;
        }
        let (v, tail) = tail.split_at(len);
        *rest = tail;
        Some(v.to_vec())
    };
    while !rest.is_empty() {
        let k = String::from_utf8(take(&mut rest)?).ok()?;
        let v = take(&mut rest)?;
        map.insert(k, v);
    }
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_keeps_its_quota_and_survives_a_reopen() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("ext");
        let (mut kv, warning) = KvStore::open(&dir);
        assert!(warning.is_none());
        kv.set("notes/1".into(), b"one".to_vec()).unwrap();
        kv.set("notes/2".into(), b"two".to_vec()).unwrap();
        kv.set("other".into(), vec![]).unwrap();
        kv.delete("missing").unwrap();
        assert_eq!(kv.list("notes/"), ["notes/1", "notes/2"]);
        assert_eq!(kv.usage(), 7 + 3 + 7 + 3 + 5);
        assert!(matches!(kv.set(String::new(), vec![]), Err(KvError::BadKey(_))));
        assert!(matches!(kv.get("a\nb"), Err(KvError::BadKey(_))));
        assert_eq!(kv.set("big".into(), vec![0; STORAGE_BYTES as usize]), Err(KvError::Full));
        kv.flush().unwrap();
        let (again, _) = KvStore::open(&dir);
        assert_eq!(again.get("notes/2").unwrap().as_deref(), Some(&b"two"[..]));
        assert_eq!(again.usage(), kv.usage());
        // A damaged file is set aside, not trusted.
        std::fs::write(dir.join(KV_FILE), b"PWKV1\n\xff\xff\xff\xff").unwrap();
        let (empty, warning) = KvStore::open(&dir);
        assert!(warning.is_some() && empty.list("").is_empty());
        assert!(dir.join("kv.bin.bad").exists());
    }

    #[test]
    fn settings_are_kept_per_extension() {
        let t = tempfile::tempdir().unwrap();
        let data = ExtensionData::new(t.path(), "hello");
        assert!(data.settings().is_empty());
        data.set_setting("greeting", Some("Hi")).unwrap();
        data.set_setting("count", Some("3")).unwrap();
        data.set_setting("count", None).unwrap();
        assert_eq!(data.settings().into_iter().collect::<Vec<_>>(), [("greeting".to_string(), "Hi".to_string())]);
        assert_eq!(data.dir(), t.path().join("extension-data").join("hello"));
        assert!(ExtensionData::new(t.path(), "other").settings().is_empty());
    }
}
