//! Installing a tile extension from a folder before packages exist
//! (docs/spec/tile-sdk/packaging.md §6.3, "Before packages: a preview
//! folder"): an unsigned folder holding `extension.json`, its component
//! and the files its manifest names, checked here with the package rules
//! (§6.1) before anything is copied, then installed through
//! [`super::install`] with [`Origin::Folder`]. It is marked as a developer
//! preview everywhere and never updated: installing a newer folder
//! replaces it.
//!
//! Only tile extensions: a folder that runs programs (an adapter, a
//! service, setup actions, variables on agents) is refused, since nothing
//! checks what those programs do.

use std::collections::HashSet;
use std::path::Path;

use super::install::{Offer, Origin};
use super::{read_in_place, Manifest};

/// The most files a preview folder may hold (as a package).
pub const MAX_FILES: usize = 5_000;
/// The most bytes it may hold, all files together (as a package).
pub const MAX_BYTES: u64 = 50 << 20;
/// The largest component file.
pub const MAX_WASM_BYTES: u64 = 20 << 20;

/// What the permission screen and the CLI say about every preview folder.
pub const WARNING: &str = "Unsigned, preview: only install tiles from people you trust.";

/// The start of a WebAssembly component (a core module has version 1 in
/// the same place).
const COMPONENT_HEADER: [u8; 8] = [0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00];

/// Check folder `dir` as a preview tile extension: its manifest, that it
/// only adds tiles, its files (no links or special files, at most
/// [`MAX_FILES`] and [`MAX_BYTES`], names unique ignoring case) and its
/// component (present, at most [`MAX_WASM_BYTES`], a component and not a
/// core module). Why not, in plain words. Reads only; nothing runs.
pub fn check(dir: &Path) -> Result<Manifest, String> {
    let meta = std::fs::metadata(dir).map_err(|_| format!("{} isn't there", dir.display()))?;
    if !meta.is_dir() {
        return Err(format!("{} isn't a folder", dir.display()));
    }
    let m = read_in_place(dir).map_err(|why| format!("Its manifest can't be used: {why}."))?;
    if m.contributes.tiles.is_empty() {
        return Err(format!("{} adds no tiles (\"contributes.tiles\"): only tile extensions install from a folder.", m.name));
    }
    let runs: Vec<&str> = [(m.adapter.is_some(), "an adapter"), (m.service.is_some(), "a service"), (!m.actions.is_empty(), "setup actions"), (!m.agent_env.is_empty(), "variables on agents")].into_iter().filter(|(on, _)| *on).map(|(_, w)| w).collect();
    if !runs.is_empty() {
        return Err(format!("{} runs programs on this computer ({}): only tile extensions, which run in Pitwall's sandbox, install from a folder.", m.name, runs.join(", ")));
    }
    let mut seen = HashSet::new();
    let (mut files, mut bytes) = (0usize, 0u64);
    walk(dir, "", &mut seen, &mut files, &mut bytes)?;
    let wasm = &m.contributes.tiles[0].wasm;
    for t in &m.contributes.tiles {
        if t.wasm != *wasm {
            return Err(format!("Its tiles name different .wasm files ({} and {}): an extension has one component.", wasm, t.wasm));
        }
        if let Some(schema) = &t.input {
            if !dir.join(schema).is_file() {
                return Err(format!("Tile “{}” names {schema}, which isn't in the folder.", t.id));
            }
        }
    }
    let path = dir.join(wasm);
    let wmeta = std::fs::symlink_metadata(&path).map_err(|_| format!("Its component {wasm} isn't in the folder."))?;
    if !wmeta.is_file() {
        return Err(format!("Its component {wasm} must be a plain file."));
    }
    if wmeta.len() > MAX_WASM_BYTES {
        return Err(format!("Its component {wasm} is larger than {} MiB.", MAX_WASM_BYTES >> 20));
    }
    let mut head = [0u8; 8];
    let read = std::fs::File::open(&path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head));
    if read.is_err() || head[..4] != COMPONENT_HEADER[..4] {
        return Err(format!("Its component {wasm} isn't a WebAssembly file."));
    }
    if head != COMPONENT_HEADER {
        return Err(format!("{wasm} is a WebAssembly module, not a component: build it for wasm32-wasip2."));
    }
    Ok(m)
}

/// Folder `dir`, checked ([`check`]), as something to install: copied
/// from where it is, marked as an unsigned preview.
pub fn offer(dir: &Path) -> Result<Offer, String> {
    let dir = dir.canonicalize().map_err(|_| format!("{} isn't there", dir.display()))?;
    let manifest = check(&dir)?;
    Ok(Offer { manifest, origin: Origin::Folder { path: dir.to_string_lossy().into_owned() }, folder: dir })
}

/// Folder `dir` checked, then copied into a hidden folder of
/// `extensions` (`.fetched-preview-…`, which Pitwall clears at start like
/// a fetch's) and checked again there: what the user reviews is exactly
/// what installs, whatever happens to `dir` meanwhile. The offer's folder
/// is that copy, Pitwall's to delete; its origin names `dir`.
pub fn stage(dir: &Path, extensions: &Path) -> Result<Offer, String> {
    let from = offer(dir)?;
    std::fs::create_dir_all(extensions).map_err(|e| format!("Can't make {}: {e}", extensions.display()))?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or_default();
    let copy = extensions.join(format!(".fetched-preview-{}-{}-{nanos}", from.manifest.id, std::process::id()));
    let checked = super::install::copy_tree(&from.folder, &copy).and_then(|()| check(&copy).map_err(|why| format!("The folder changed while it was copied: {why}")));
    match checked {
        Ok(manifest) if manifest == from.manifest => Ok(Offer { manifest, folder: copy, origin: from.origin }),
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&copy);
            Err("The folder changed while it was copied: try again.".into())
        }
        Err(why) => {
            let _ = std::fs::remove_dir_all(&copy);
            Err(why)
        }
    }
}

/// What it asks for, line by line, for the CLI and the approval: each
/// permission with its reason (memory with its size), then the warning.
pub fn review_lines(m: &Manifest) -> Vec<String> {
    let mut out = Vec::new();
    for t in &m.contributes.tiles {
        out.push(format!("Adds the tile “{}”", t.title));
    }
    if m.permissions.is_empty() {
        out.push("It asks for no permissions.".into());
    }
    for p in &m.permissions {
        let title = super::permissions::find(&p.name).map_or(p.name.as_str(), |k| k.title);
        match super::permissions::amount(m, &p.name) {
            Some(a) => out.push(format!("{title} ({}, {a}): “{}”", p.name, p.reason)),
            None => out.push(format!("{title} ({}): “{}”", p.name, p.reason)),
        }
    }
    out.push(WARNING.into());
    out
}

fn walk(root: &Path, rel: &str, seen: &mut HashSet<String>, files: &mut usize, bytes: &mut u64) -> Result<(), String> {
    let here = if rel.is_empty() { root.to_path_buf() } else { root.join(rel) };
    let entries = std::fs::read_dir(&here).map_err(|e| format!("{} can't be read: {e}", here.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if super::install::SKIPPED.contains(&name.as_str()) || (rel.is_empty() && name == super::install::MARKER) {
            continue;
        }
        let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        if !seen.insert(path.to_lowercase()) {
            return Err(format!("Two files are named {path}, apart from upper and lower case."));
        }
        let kind = std::fs::symlink_metadata(entry.path()).map_err(|e| format!("{path} can't be read: {e}"))?;
        if kind.file_type().is_symlink() {
            return Err(format!("{path} is a link: a preview folder holds plain files only."));
        }
        if kind.is_dir() {
            walk(root, &path, seen, files, bytes)?;
        } else if kind.is_file() {
            *files += 1;
            *bytes += kind.len();
            if *files > MAX_FILES {
                return Err(format!("It holds more than {MAX_FILES} files."));
            }
            if *bytes > MAX_BYTES {
                return Err(format!("It holds more than {} MiB.", MAX_BYTES >> 20));
            }
        } else {
            return Err(format!("{path} isn't a plain file."));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    const TILE: &str = r#"{"v": 2, "id": "hello", "name": "Hello", "version": "0.1.0",
        "permissions": {"memory": {"reason": "Big pictures", "mib": 128}},
        "contributes": {"tiles": [{"id": "main", "title": "Hello", "wasm": "tile.wasm"}]}}"#;

    fn folder(t: &TempDir, manifest: &str, wasm: &[u8]) -> std::path::PathBuf {
        let d = t.path().join("hello");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("extension.json"), manifest).unwrap();
        std::fs::write(d.join("tile.wasm"), wasm).unwrap();
        d
    }

    /// A tile folder passes, with what the review lists; one that runs a
    /// program, a module instead of a component, a missing component and a
    /// bad manifest are each refused in words.
    #[test]
    fn a_preview_folder_is_checked_before_it_is_copied() {
        let t = TempDir::new("preview");
        let d = folder(&t, TILE, &COMPONENT_HEADER);
        let m = check(&d).unwrap();
        let lines = review_lines(&m);
        assert!(lines.iter().any(|l| l.contains("memory") && l.contains("128 MiB") && l.contains("Big pictures")), "{lines:?}");
        assert_eq!(lines.last().map(String::as_str), Some(WARNING));
        let o = offer(&d).unwrap();
        assert!(o.origin.is_preview());

        std::fs::write(d.join("tile.wasm"), [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]).unwrap();
        assert!(check(&d).unwrap_err().contains("module, not a component"));
        std::fs::remove_file(d.join("tile.wasm")).unwrap();
        assert!(check(&d).unwrap_err().contains("isn't in the folder"));

        let adapter = TILE.replace(r#""version": "0.1.0","#, r#""version": "0.1.0", "adapter": {"command": ["node", "a.mjs"]},"#);
        let d = folder(&t, &adapter, &COMPONENT_HEADER);
        assert!(check(&d).unwrap_err().contains("runs programs"));
        let d = folder(&t, "{ not json", &COMPONENT_HEADER);
        assert!(check(&d).unwrap_err().starts_with("Its manifest can't be used"));
        assert!(check(&t.path().join("nope")).unwrap_err().contains("isn't there"));
    }

    /// Installed from a folder: copied with a marker that says so, and
    /// read back as a preview.
    #[test]
    fn a_preview_folder_installs_marked_as_one() {
        let t = TempDir::new("preview-install");
        let d = folder(&t, TILE, &COMPONENT_HEADER);
        let exts = t.path().join("extensions");
        let staged = stage(&d, &exts).unwrap();
        assert!(staged.folder.starts_with(&exts) && staged.folder != d, "a copy of Pitwall's own");
        super::super::install::install(&staged, &exts).unwrap();
        assert!(exts.join("hello/tile.wasm").is_file());
        let origin = super::super::install::origin(&exts.join("hello")).unwrap();
        assert!(origin.is_preview(), "{origin:?}");
        assert!(super::super::install::local(&exts.join("hello")).as_installed);
    }
}
