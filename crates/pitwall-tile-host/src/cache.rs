//! Pitwall's compile cache (runtime.md §2.6): a component compiled once
//! with Cranelift is kept as `<data>/cache/tiles/<id>/<sha256>.cwasm`, in a
//! folder only this user may read, never in the extension's folder, and
//! memory-mapped on the next start (read into memory on macOS, where the
//! hardened runtime doesn't let a mapped file run). Wasmtime's header
//! check ties each file to this engine's version and settings; a file
//! that fails to load is deleted and the wasm compiled again.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use wasmtime::component::Component;
use wasmtime::Engine;

use crate::platform;

/// The cache folder of extension `id` under Pitwall's data folder.
pub fn cache_dir(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join("cache").join("tiles").join(id)
}

/// The cache file for these wasm bytes.
pub fn cache_file(data_dir: &Path, id: &str, wasm: &[u8]) -> PathBuf {
    let hash = Sha256::digest(wasm);
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    cache_dir(data_dir, id).join(format!("{hex}.cwasm"))
}

/// The compiled component, from the cache when it has these bytes (true),
/// else compiled now and written to it (false). Slow on a miss: never call
/// it on the UI thread.
pub fn load_or_compile(engine: &Engine, data_dir: &Path, id: &str, wasm: &[u8]) -> Result<(Component, bool), String> {
    let path = cache_file(data_dir, id, wasm);
    if path.is_file() {
        // SAFETY: `deserialize_file` trusts the file's contents (runtime.md
        // §2.6). The file is written only by `write` below, from our own
        // compile, into a folder only this user may write; its name is the
        // SHA-256 of the wasm it was compiled from; and Wasmtime checks its
        // header against this engine's version and settings, refusing
        // (not running) a file from another one. Files in the cache are
        // never rewritten in place, so the mapping can't change under us.
        // Where a mapped file can't be made executable (macOS's hardened
        // runtime), its bytes are loaded instead: the same check, in
        // memory of our own.
        let loaded = if platform::RUN_MAPPED_FILES {
            unsafe { Component::deserialize_file(engine, &path) }
        } else {
            std::fs::read(&path).map_err(wasmtime::Error::from).and_then(|bytes| unsafe { Component::deserialize(engine, &bytes) })
        };
        match loaded {
            Ok(c) => return Ok((c, true)),
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let component = Component::new(engine, wasm).map_err(|e| format!("it isn't a component this Pitwall can compile: {e:#}"))?;
    // A failed write only costs a compile next time.
    if let Ok(bytes) = component.serialize() {
        let _ = write(&path, &bytes);
    }
    Ok((component, false))
}

fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    if let Some(tiles) = dir.parent() {
        platform::private_dir(tiles)?;
    }
    platform::private_dir(dir)?;
    // Keep one older entry (the previous version's, so Go back starts at
    // once, packaging.md §6.4); remove the rest.
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut older: Vec<(std::time::SystemTime, PathBuf)> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p != path && p.extension().is_some_and(|x| x == "cwasm"))
            .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
            .collect();
        older.sort_by_key(|o| std::cmp::Reverse(o.0));
        for (_, p) in older.into_iter().skip(1) {
            let _ = std::fs::remove_file(p);
        }
    }
    platform::write_private(path, bytes)
}
