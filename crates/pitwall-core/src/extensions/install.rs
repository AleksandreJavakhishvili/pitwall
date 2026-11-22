//! Installing the extensions Pitwall ships (docs/spec/extensions.md,
//! "Installing"): the app carries copies of some extension folders (its
//! bundle); Settings lists those not yet in `<data>/extensions/` and
//! copies one there on Install, updates it when the bundled copy is newer,
//! and removes it. Nothing here runs an extension: an installed one is off
//! until the user turns it on.
//!
//! A folder is always written next to its place and renamed into it, so a
//! failed copy leaves nothing half-installed. Pitwall records what it
//! installed in the folder ([`MARKER`]: the version and a digest of the
//! files), so it can tell a folder the user changed by hand and never
//! replaces one silently.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{read, Manifest};
#[cfg(test)]
use super::MANIFEST;

/// What Pitwall installed, in the installed folder (not part of the digest).
pub const MARKER: &str = ".pitwall-install.json";

/// Never copied, never part of the digest: what a package manager or the
/// system adds.
const SKIPPED: [&str; 3] = ["node_modules", ".git", ".DS_Store"];

/// A bundled extension, ready to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub manifest: Manifest,
    /// Its folder in the bundle.
    pub folder: PathBuf,
}

/// The usable extensions in bundle folder `bundle`: by name, demos last.
pub fn bundled(bundle: &Path) -> Vec<Offer> {
    let mut out: Vec<Offer> = super::discover(bundle)
        .into_iter()
        .filter_map(|f| {
            let m = f.manifest.ok()?;
            Some(Offer { folder: bundle.join(&f.folder), manifest: m })
        })
        .collect();
    out.sort_by(|a, b| a.manifest.demo.cmp(&b.manifest.demo).then_with(|| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase())));
    out
}

/// A bundled extension's place in the data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Not installed: it can be.
    Available,
    /// Installed and as new as the bundled one (or newer).
    Current,
    /// The bundled copy is newer: this version.
    Newer(String),
}

/// What Pitwall can say about installed folder `dir`: whether it is as
/// Pitwall installed it (false for one changed by hand, or put there by
/// hand), and the version it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub as_installed: bool,
    pub version: Option<String>,
}

/// Bundled extension `offer` against data folder `extensions`.
pub fn status(offer: &Offer, extensions: &Path) -> Status {
    let dir = extensions.join(&offer.manifest.id);
    if !dir.is_dir() {
        return Status::Available;
    }
    let local = local(&dir);
    match (&offer.manifest.version, &local.version) {
        (Some(b), Some(l)) if compare_versions(b, l) == Ordering::Greater => Status::Newer(b.clone()),
        // One with no version (put there by hand from an older copy) is
        // older than any bundled version.
        (Some(b), None) => Status::Newer(b.clone()),
        _ => Status::Current,
    }
}

/// What installed folder `dir` is.
pub fn local(dir: &Path) -> Local {
    let version = read(dir, &dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).ok().and_then(|m| m.version);
    let marker: Option<Value> = std::fs::read_to_string(dir.join(MARKER)).ok().and_then(|t| serde_json::from_str(&t).ok());
    let as_installed = marker.as_ref().and_then(|m| m["digest"].as_str()).is_some_and(|d| digest(dir).as_deref() == Ok(d));
    Local { as_installed, version }
}

/// Order of versions `a` and `b` ("1.2.10" > "1.2.9"; "1.0.0-beta" <
/// "1.0.0"): dot-separated parts, numbers by value, others as text.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let split = |v: &str| -> (String, Option<String>) {
        let v = v.trim().trim_start_matches('v');
        let v = v.split('+').next().unwrap_or_default();
        match v.split_once('-') {
            Some((core, pre)) => (core.to_string(), Some(pre.to_string())),
            None => (v.to_string(), None),
        }
    };
    let parts = |s: &str| -> Vec<String> { s.split('.').map(str::to_string).collect() };
    let cmp_parts = |x: &[String], y: &[String]| -> Ordering {
        for i in 0..x.len().max(y.len()) {
            let (p, q) = (x.get(i).map_or("0", String::as_str), y.get(i).map_or("0", String::as_str));
            let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                (Ok(m), Ok(n)) => m.cmp(&n),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                _ => p.cmp(q),
            };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    };
    let ((ca, pa), (cb, pb)) = (split(a), split(b));
    cmp_parts(&parts(&ca), &parts(&cb)).then_with(|| match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => cmp_parts(&parts(&x), &parts(&y)),
    })
}

/// Why an update wasn't made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The installed folder isn't as Pitwall installed it: replacing it
    /// would lose the user's changes (asked first).
    ChangedLocally,
    Failed(String),
}

/// Copy bundled `offer` into `extensions/<id>/`. It must not be there.
pub fn install(offer: &Offer, extensions: &Path) -> Result<(), String> {
    let id = &offer.manifest.id;
    let target = extensions.join(id);
    if target.exists() {
        return Err(format!("{} is already installed", offer.manifest.name));
    }
    std::fs::create_dir_all(extensions).map_err(|e| format!("Can't make {}: {e}", extensions.display()))?;
    let staged = stage(offer, extensions)?;
    std::fs::rename(&staged, &target).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staged);
        format!("Can't put it in place: {e}")
    })
}

/// Replace installed `extensions/<id>/` with bundled `offer` (its settings
/// and secrets aren't in the folder: they stay). A folder that isn't as
/// Pitwall installed it is replaced only when `replace_changed`.
pub fn update(offer: &Offer, extensions: &Path, replace_changed: bool) -> Result<(), Refused> {
    let target = extensions.join(&offer.manifest.id);
    if !target.is_dir() {
        return Err(Refused::Failed(format!("{} isn't installed", offer.manifest.name)));
    }
    if !replace_changed && !local(&target).as_installed {
        return Err(Refused::ChangedLocally);
    }
    let staged = stage(offer, extensions).map_err(Refused::Failed)?;
    let old = sibling(extensions, &offer.manifest.id, "old");
    if let Err(e) = rename_retrying(&target, &old) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(Refused::Failed(format!("Can't move the installed copy aside: {e}")));
    }
    if let Err(e) = rename_retrying(&staged, &target) {
        let _ = std::fs::rename(&old, &target);
        let _ = std::fs::remove_dir_all(&staged);
        return Err(Refused::Failed(format!("Can't put it in place: {e}")));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Delete installed `extensions/<id>/`: moved aside first (so it is gone
/// from the list at once, even if deleting takes a moment), then deleted.
pub fn remove(extensions: &Path, id: &str) -> Result<(), String> {
    let target = extensions.join(id);
    if !target.is_dir() {
        return Ok(());
    }
    let gone = sibling(extensions, id, "removing");
    rename_retrying(&target, &gone).map_err(|e| format!("Can't remove it: {e}"))?;
    std::fs::remove_dir_all(&gone).map_err(|e| format!("Can't delete {}: {e}", gone.display()))
}

/// A hidden sibling of `extensions/<id>` (hidden folders aren't listed).
fn sibling(extensions: &Path, id: &str, what: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or_default();
    extensions.join(format!(".{id}.{what}-{}-{nanos}", std::process::id()))
}

/// A copy of `offer` with its marker, in a hidden sibling of its place.
fn stage(offer: &Offer, extensions: &Path) -> Result<PathBuf, String> {
    let id = &offer.manifest.id;
    // The bundled copy must be usable as installed.
    read(&offer.folder, id).map_err(|e| format!("The bundled copy can't be used: {e}"))?;
    let staged = sibling(extensions, id, "installing");
    let done = copy_tree(&offer.folder, &staged).and_then(|()| {
        let d = digest(&staged)?;
        let marker = json!({ "version": offer.manifest.version, "digest": d });
        std::fs::write(staged.join(MARKER), format!("{marker:#}\n")).map_err(|e| e.to_string())
    });
    match done {
        Ok(()) => Ok(staged),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staged);
            Err(format!("Can't copy it: {e}"))
        }
    }
}

/// A folder renamed; on Windows a moment later again while something
/// (an adapter just stopped, a virus scanner) still holds it.
fn rename_retrying(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if tries < 20 && cfg!(windows) => {
                let _ = e;
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name();
        if SKIPPED.iter().any(|s| name == *s) || name == MARKER {
            continue;
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let dest = to.join(&name);
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &dest).map_err(|e| format!("{}: {e}", entry.path().display()))?;
        }
        // Links aren't copied: an extension is plain files.
    }
    Ok(())
}

/// A digest of folder `dir`'s files (names and contents; not the marker,
/// not what [`SKIPPED`] names): FNV-1a, enough to notice a change.
pub fn digest(dir: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    list(dir, Path::new(""), &mut files)?;
    files.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for rel in files {
        let bytes = std::fs::read(dir.join(&rel)).map_err(|e| e.to_string())?;
        eat(rel.as_bytes());
        eat(&[0]);
        eat(&(bytes.len() as u64).to_le_bytes());
        eat(&bytes);
    }
    Ok(format!("{h:016x}"))
}

fn list(root: &Path, rel: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(root.join(rel)).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name();
        if SKIPPED.iter().any(|s| name == *s) || (rel.as_os_str().is_empty() && name == MARKER) {
            continue;
        }
        let path = rel.join(&name);
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            list(root, &path, out)?;
        } else if kind.is_file() {
            // `/` on every system, so a digest doesn't depend on it.
            out.push(path.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn ext(root: &Path, id: &str, version: Option<&str>, demo: bool) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(dir.join("adapter/node_modules/x")).unwrap();
        let v = version.map(|v| format!(r#", "version": "{v}""#)).unwrap_or_default();
        std::fs::write(dir.join(MANIFEST), format!(r#"{{"id": "{id}", "name": "{}", "demo": {demo}{v}}}"#, id.to_uppercase())).unwrap();
        std::fs::write(dir.join("adapter/main.mjs"), "// main").unwrap();
        std::fs::write(dir.join("adapter/node_modules/x/big.js"), "// not copied").unwrap();
        dir
    }

    #[test]
    fn versions_are_ordered_by_their_parts() {
        assert_eq!(compare_versions("1.2.10", "1.2.9"), Ordering::Greater);
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0.0-beta", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("v0.5.0", "0.4.9"), Ordering::Greater);
        assert_eq!(compare_versions("0.4.0-beta.2", "0.4.0-beta.10"), Ordering::Less);
    }

    /// The bundle lists its usable extensions by name, demos last.
    #[test]
    fn the_bundle_lists_its_extensions_demos_last() {
        let t = TempDir::new("ext-bundle");
        ext(t.path(), "zeta", None, false);
        ext(t.path(), "alpha", None, true);
        ext(t.path(), "beta", None, false);
        std::fs::create_dir_all(t.path().join("broken")).unwrap();
        let ids: Vec<String> = bundled(t.path()).into_iter().map(|o| o.manifest.id).collect();
        assert_eq!(ids, ["beta", "zeta", "alpha"]);
    }

    /// Install copies the folder in (without node_modules), leaves no
    /// staging folder behind and refuses to install twice; the copy is as
    /// installed until a file changes.
    #[test]
    fn install_update_and_remove() {
        let t = TempDir::new("ext-install");
        let (bundle, data) = (t.path().join("bundle"), t.path().join("data/extensions"));
        ext(&bundle, "inbox", Some("1.0.0"), false);
        let offer = bundled(&bundle).remove(0);
        assert_eq!(status(&offer, &data), Status::Available);
        install(&offer, &data).unwrap();
        let dir = data.join("inbox");
        assert!(dir.join("adapter/main.mjs").is_file());
        assert!(!dir.join("adapter/node_modules").exists(), "node_modules isn't copied");
        let names: Vec<String> = std::fs::read_dir(&data).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["inbox"], "nothing staged is left");
        assert_eq!(status(&offer, &data), Status::Current);
        assert_eq!(local(&dir), Local { as_installed: true, version: Some("1.0.0".into()) });
        assert!(install(&offer, &data).unwrap_err().contains("already installed"));
        // What npm adds doesn't count as a change.
        std::fs::create_dir_all(dir.join("adapter/node_modules/y")).unwrap();
        std::fs::write(dir.join("adapter/node_modules/y/a.js"), "").unwrap();
        assert!(local(&dir).as_installed);

        // A newer bundled copy: an update, which keeps nothing of the old.
        ext(&bundle, "inbox", Some("1.1.0"), false);
        std::fs::write(bundle.join("inbox/adapter/main.mjs"), "// main 1.1").unwrap();
        let offer = bundled(&bundle).remove(0);
        assert_eq!(status(&offer, &data), Status::Newer("1.1.0".into()));
        update(&offer, &data, false).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("adapter/main.mjs")).unwrap(), "// main 1.1");
        assert_eq!(status(&offer, &data), Status::Current);
        let names: Vec<String> = std::fs::read_dir(&data).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["inbox"], "the old copy is gone");

        // Changed by hand: an update is refused until the user says so.
        std::fs::write(dir.join("adapter/main.mjs"), "// my fix").unwrap();
        assert!(!local(&dir).as_installed);
        ext(&bundle, "inbox", Some("1.2.0"), false);
        let offer = bundled(&bundle).remove(0);
        assert_eq!(update(&offer, &data, false), Err(Refused::ChangedLocally));
        assert_eq!(std::fs::read_to_string(dir.join("adapter/main.mjs")).unwrap(), "// my fix", "left as it was");
        update(&offer, &data, true).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("adapter/main.mjs")).unwrap(), "// main");

        remove(&data, "inbox").unwrap();
        assert_eq!(std::fs::read_dir(&data).unwrap().count(), 0);
        assert_eq!(status(&offer, &data), Status::Available);
    }

    /// A folder put there by hand: not as installed, and older than any
    /// bundled version when it has none.
    #[test]
    fn a_folder_put_in_by_hand_is_never_replaced_silently() {
        let t = TempDir::new("ext-by-hand");
        let (bundle, data) = (t.path().join("bundle"), t.path().join("data"));
        ext(&bundle, "inbox", Some("0.2.0"), false);
        ext(&data, "inbox", None, false);
        let offer = bundled(&bundle).remove(0);
        assert_eq!(status(&offer, &data), Status::Newer("0.2.0".into()));
        assert!(!local(&data.join("inbox")).as_installed);
        assert_eq!(update(&offer, &data, false), Err(Refused::ChangedLocally));
    }

    /// A failed copy leaves neither a half folder nor a staging folder.
    #[test]
    fn a_failed_install_leaves_nothing() {
        let t = TempDir::new("ext-install-fail");
        let (bundle, data) = (t.path().join("bundle"), t.path().join("data"));
        ext(&bundle, "inbox", None, false);
        let offer = bundled(&bundle).remove(0);
        // The bundled copy goes wrong after it was listed.
        std::fs::write(bundle.join("inbox").join(MANIFEST), "{").unwrap();
        assert!(install(&offer, &data).unwrap_err().contains("can't be used"));
        assert_eq!(std::fs::read_dir(&data).map(|d| d.count()).unwrap_or(0), 0);
    }
}
