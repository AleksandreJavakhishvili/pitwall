//! Tile packages (docs/spec/tile-sdk/packaging.md §6): the `.pwtile`
//! file, its minisign signature, and the rules a package is installed and
//! updated by.
//!
//! A `.pwtile` is a zip ([`zip`]) of an extension of tiles: its
//! `extension.json`, its component `tile.wasm`, a `README.md`, and
//! optionally a licence, a `CHANGELOG.md`, an agent skill
//! (`agents/SKILL.md`), the input schemas its tiles name, `assets/`
//! (svg, png, webp), `screenshots/` (png, webp), `media/` (the images its
//! README shows: png, webp, jpeg, gif) and its icon file (a png or webp
//! the manifest's `icon` names), each image checked by its contents
//! ([`super::media`]). [`pack`] makes one
//! from an author's folder, the same bytes every time for the same files.
//!
//! Its signature is a minisign signature of the whole file, beside it as
//! `<file>.minisig` ([`signature_path`]). Out of an index (there is none
//! yet) a key is trusted on first use: installing a signed package trusts
//! its key for that extension, recorded in the installed copy's marker
//! ([`super::install::Origin::Package`]); an update must be signed by the
//! same key, and another key (or none) is refused unless the user says so
//! ([`Accept`]). An unsigned package installs only when the user asks
//! ("Install unsigned"), marked as a developer preview like a folder.
//!
//! Reading a package ([`open`]) checks, in this order and before anything
//! is written: its size, its signature over the bytes, then the zip's
//! structure, every name ([`check_name`]) and size, what files it holds,
//! its manifest (tiles only: nothing that runs programs) and that its
//! component is one. Only then is it unpacked ([`stage`]) into a hidden
//! folder of Pitwall's, and installed from there as any extension is.

pub mod zip;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use sha2::Digest as _;

use super::install::{self, Local, Offer, Origin, Refused};
use super::{Manifest, MANIFEST};

/// A package's file extension.
pub const EXTENSION: &str = "pwtile";
/// Its signature's, after the package's own (`notes-0.1.0.pwtile.minisig`).
pub const SIGNATURE: &str = "minisig";
/// The most a package may be, packed and unpacked.
pub const MAX_BYTES: u64 = 50 << 20;
/// The most files it may hold.
pub const MAX_FILES: usize = super::preview::MAX_FILES;
/// The largest component.
pub const MAX_WASM_BYTES: u64 = super::preview::MAX_WASM_BYTES;
/// The largest screenshot, and how many there may be (a dark and light
/// pair counts once).
pub const MAX_SCREENSHOT_BYTES: u64 = super::media::MAX_SCREENSHOT_BYTES;
pub const MAX_SCREENSHOTS: usize = super::media::MAX_SCREENSHOTS;
/// The largest agent skill (agent-tools.md §11.11).
pub const MAX_SKILL_BYTES: u64 = 16 << 10;
/// The largest text file (README, licence, changelog, schema) or asset.
pub const MAX_TEXT_BYTES: u64 = 1 << 20;
/// The largest signature file.
pub const MAX_SIGNATURE_BYTES: u64 = 4 << 10;
/// Where a package's component is.
pub const WASM: &str = "tile.wasm";
/// Its readme, required.
pub const README: &str = "README.md";
/// Its agent skill.
pub const SKILL: &str = "agents/SKILL.md";

/// What every surface says about an unsigned package.
pub const UNSIGNED_WARNING: &str = "Unsigned: nobody vouches for this file. Only install it if you trust whoever gave it to you.";

/// The files a package holds at its top, besides the input schemas its
/// tiles name, its icon file, `agents/SKILL.md`, `assets/`,
/// `screenshots/` and `media/`.
const TOP: [&str; 8] = [MANIFEST, WASM, README, "CHANGELOG.md", "LICENSE", "LICENSE.md", "LICENSE.txt", "NOTICE"];

/// The key a package is signed with: minisign's key id (as `minisign`
/// prints it: 16 hex digits) and its public key (base64, as the second
/// line of a `minisign.pub`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    pub key_id: String,
    pub public_key: String,
}

impl Signer {
    /// The signer of public key `b64` (the second line of a
    /// `minisign.pub`); why not, in words.
    pub fn from_public_key(b64: &str) -> Result<Signer, String> {
        let b64 = b64.trim();
        minisign_verify::PublicKey::from_base64(b64).map_err(|_| "it isn't a minisign public key".to_string())?;
        let raw = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| "it isn't a minisign public key".to_string())?;
        Ok(Signer { key_id: key_id(&raw[2..10]), public_key: b64.to_string() })
    }

    /// Whether `other` is the same key.
    pub fn same(&self, other: &Signer) -> bool {
        self.key_id == other.key_id && self.public_key == other.public_key
    }
}

/// A key id as `minisign` prints it: the 8 bytes as a little-endian
/// number, in 16 upper-case hex digits.
fn key_id(bytes: &[u8]) -> String {
    let mut n = [0u8; 8];
    n.copy_from_slice(&bytes[..8]);
    format!("{:016X}", u64::from_le_bytes(n))
}

/// Why file name `name` can't be in a package; `Ok` when it can. Names
/// are relative, `/`-separated, plain ASCII (letters, digits, `.`, `-`,
/// `_`: the same on every system and already in NFC), with no empty, `.`,
/// `..` or hidden parts, no part ending in `.`, no name Windows reserves
/// (`CON`, `NUL`, `COM1`…), at most 8 deep and 200 bytes long.
pub fn check_name(name: &str) -> Result<(), String> {
    let shown = name.escape_debug().to_string();
    let bad = |why: &str| Err(format!("“{shown}” can't be in a package: {why}."));
    if name.is_empty() || name.len() > 200 {
        return bad("its name is empty or longer than 200 characters");
    }
    if name.starts_with('/') {
        return bad("its path is absolute");
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b)) {
        return bad("names in a package use only letters, digits, “.”, “-” and “_”");
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() > 8 {
        return bad("it is more than 8 folders deep");
    }
    for p in parts {
        match p {
            "" => return bad("its path has an empty part"),
            "." | ".." => return bad("its path has a “.” or “..” part"),
            _ if p.starts_with('.') => return bad("hidden files aren't packaged"),
            _ if p.ends_with('.') => return bad("a part of its path ends in “.”"),
            _ => {}
        }
        let stem = p.split('.').next().unwrap_or_default().to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
        if reserved {
            return bad("its name is one Windows reserves");
        }
    }
    Ok(())
}

/// Whether manifest `m`'s files (names and sizes) are what a package may
/// hold: the required ones there, each other one of a kind a package
/// holds, each within its size.
fn check_layout(files: &[(&str, u64)], m: &Manifest) -> Result<(), String> {
    for t in &m.contributes.tiles {
        if t.wasm != WASM {
            return Err(format!("In a package the component is {WASM}, but tile “{}” names {}.", t.id, t.wasm));
        }
    }
    if m.version.is_none() {
        return Err(format!("A package needs a version: add \"version\" to its {MANIFEST}."));
    }
    for need in [MANIFEST, WASM, README] {
        if !files.iter().any(|(n, _)| *n == need) {
            return Err(format!("It has no {need}, which every package holds."));
        }
    }
    let schemas: HashSet<&str> = m.contributes.tiles.iter().filter_map(|t| t.input.as_deref()).collect();
    for s in &schemas {
        if !files.iter().any(|(n, _)| n == s) {
            return Err(format!("Its tiles name {s}, which isn't in it."));
        }
    }
    let mut shots: Vec<String> = Vec::new();
    for (name, size) in files {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        let max = match *name {
            n if Some(n) == m.icon_file.as_deref() => super::media::MAX_ICON_BYTES,
            WASM => MAX_WASM_BYTES,
            n if TOP.contains(&n) || schemas.contains(n) => MAX_TEXT_BYTES,
            SKILL => MAX_SKILL_BYTES,
            n if n.starts_with("assets/") => match ext.as_str() {
                "svg" | "png" | "webp" => MAX_TEXT_BYTES,
                "ttf" | "otf" | "woff" | "woff2" => return Err(format!("{n} is a font: a package holds no fonts (tiles draw text with Pitwall's).")),
                _ => return Err(format!("{n}: a package's assets are svg, png or webp images.")),
            },
            n if n.starts_with("screenshots/") && n.matches('/').count() == 1 => {
                shots.push(n.to_string());
                if !matches!(ext.as_str(), "png" | "webp") {
                    return Err(format!("{n}: screenshots are png or webp."));
                }
                MAX_SCREENSHOT_BYTES
            }
            n if n.starts_with("media/") => match ext.as_str() {
                "png" | "webp" | "jpg" | "jpeg" | "gif" => super::media::MAX_README_IMAGE_BYTES,
                _ => return Err(format!("{n}: media/ holds the images its README shows: png, webp, jpeg or gif.")),
            },
            n => return Err(format!("{n} isn't a file a package holds: {MANIFEST}, {WASM}, {README}, a licence, CHANGELOG.md, {SKILL}, the input schemas its tiles name, its icon, assets/, screenshots/ and media/.")),
        };
        if *size > max {
            return Err(format!("{name} is larger than {}.", size_words(max)));
        }
    }
    super::media::check_shot_count(&shots)?;
    if let Some(icon) = &m.icon_file {
        if !files.iter().any(|(n, _)| n == icon) {
            return Err(format!("Its icon {icon} isn't in it."));
        }
    }
    Ok(())
}

/// `s` as a sentence: “it…” capitalised (a file name first stays as it
/// is).
fn capital(s: &str) -> String {
    match s.strip_prefix("it") {
        Some(rest) if rest.starts_with(' ') || rest.starts_with("s ") || rest.starts_with('’') || rest.starts_with('\'') => format!("It{rest}"),
        _ => s.to_string(),
    }
}

/// A size in words: "16 KiB", "2 MiB".
pub fn size_words(n: u64) -> String {
    match n {
        n if n >= 1 << 20 => format!("{} MiB", ((n as f64) / (1u64 << 20) as f64 * 10.).round() / 10.),
        n if n >= 1 << 10 => format!("{} KiB", n.div_ceil(1 << 10)),
        n => format!("{n} bytes"),
    }
}

/// A package made by [`pack`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    pub bytes: Vec<u8>,
    pub manifest: Manifest,
    /// Its files with their sizes, by name.
    pub files: Vec<(String, u64)>,
    /// The SHA-256 of `bytes`, in hex.
    pub sha256: String,
}

/// The file name a package of `m` gets: `<id>-<version>.pwtile`.
pub fn file_name(m: &Manifest) -> String {
    format!("{}-{}.{EXTENSION}", m.id, m.version.as_deref().unwrap_or("0.0.0"))
}

/// Where the signature of package file `package` is: beside it, named
/// after it plus `.minisig`.
pub fn signature_path(package: &Path) -> PathBuf {
    let mut name = package.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{SIGNATURE}"));
    package.with_file_name(name)
}

/// The SHA-256 of `bytes`, in hex.
pub fn sha256(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Package the extension of tiles in folder `dir`: its manifest and
/// component (checked as a preview folder is: tiles only, a component),
/// its readme, and what else a package may hold of what is there
/// (licence, changelog, agent skill, input schemas, its icon file,
/// `assets/`, `screenshots/`, `media/`), each image checked
/// ([`super::media::files`]: a README image it can't show, or one outside
/// those folders, is refused). Everything else in the folder (sources,
/// `target/`) stays out. The same files give the same bytes. Read back before it
/// is returned, so what it makes is what Pitwall installs.
pub fn pack(dir: &Path) -> Result<Packed, String> {
    if !dir.is_dir() {
        return Err(format!("{} isn't a folder.", dir.display()));
    }
    let m = super::read_in_place(dir).map_err(|why| format!("Its manifest can't be used: {why}."))?;
    super::preview::tiles_only(&m, "a package")?;
    let mut names: Vec<String> = Vec::new();
    for top in TOP.iter().map(|s| s.to_string()).chain(m.contributes.tiles.iter().filter_map(|t| t.input.clone())).chain([SKILL.to_string()]) {
        if !names.contains(&top) && std::fs::symlink_metadata(dir.join(&top)).is_ok() {
            names.push(top);
        }
    }
    for sub in ["assets", "screenshots", "media"] {
        if dir.join(sub).is_dir() {
            walk(dir, sub, &mut names)?;
        }
    }
    for f in super::media::files(dir, &m).map_err(|why| format!("{}.", capital(&why)))? {
        if !names.contains(&f) {
            names.push(f);
        }
    }
    let mut files = Vec::new();
    for name in &names {
        check_name(name)?;
        let path = dir.join(name);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{name} can't be read: {e}"))?;
        if !meta.is_file() {
            return Err(format!("{name} is a link or isn't a plain file: a package holds plain files only."));
        }
        if meta.len() > MAX_BYTES {
            return Err(format!("{name} is larger than {}.", size_words(MAX_BYTES)));
        }
        files.push((name.clone(), std::fs::read(&path).map_err(|e| format!("{name} can't be read: {e}"))?));
    }
    let sizes: Vec<(&str, u64)> = files.iter().map(|(n, b)| (n.as_str(), b.len() as u64)).collect();
    check_layout(&sizes, &m)?;
    let bytes = zip::write(&files);
    let back = open_bytes(&bytes)?;
    let mut listed: Vec<(String, u64)> = back.1.iter().map(|e| (e.name.clone(), e.bytes.len() as u64)).collect();
    listed.sort();
    Ok(Packed { sha256: sha256(&bytes), bytes, manifest: back.0, files: listed })
}

/// The plain files under `dir/rel` (hidden ones left out), as package
/// names.
fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir.join(rel)).map_err(|e| format!("{rel} can't be read: {e}"))?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = format!("{rel}/{name}");
        let kind = std::fs::symlink_metadata(e.path()).map_err(|e| format!("{path} can't be read: {e}"))?;
        if kind.is_dir() {
            walk(dir, &path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// A package read and checked by [`open`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub manifest: Manifest,
    pub files: Vec<zip::Entry>,
    /// The SHA-256 of the file, in hex.
    pub sha256: String,
    /// The key that signed it (checked); `None`: it has no signature.
    pub signer: Option<Signer>,
    /// Its size in bytes.
    pub size: u64,
}

/// Read package file `path` and check it: at most [`MAX_BYTES`], its
/// signature beside it (when there is one) verified over its bytes, then
/// the zip and what it holds ([`open_bytes`]). `trusted`: the key the
/// installed copy was signed with, used when the signature names it.
/// Why not, in words; nothing is written.
pub fn open(path: &Path, trusted: Option<&Signer>) -> Result<Opened, String> {
    let meta = std::fs::metadata(path).map_err(|_| format!("{} isn't there.", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} isn't a file.", path.display()));
    }
    if meta.len() > MAX_BYTES {
        return Err(format!("It is larger than {}.", size_words(MAX_BYTES)));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    std::io::Read::read_to_end(&mut std::io::Read::take(std::fs::File::open(path).map_err(|e| format!("It can't be read: {e}."))?, MAX_BYTES + 1), &mut bytes).map_err(|e| format!("It can't be read: {e}."))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("It is larger than {}.", size_words(MAX_BYTES)));
    }
    let sig = signature_path(path);
    let signer = match std::fs::symlink_metadata(&sig) {
        Ok(m) if !m.is_file() || m.len() > MAX_SIGNATURE_BYTES => return Err(format!("Its signature {} isn't a minisign signature.", file_shown(&sig))),
        Ok(_) => {
            let text = std::fs::read_to_string(&sig).map_err(|_| format!("Its signature {} isn't a minisign signature.", file_shown(&sig)))?;
            Some(verify(&bytes, &text, trusted)?)
        }
        Err(_) => None,
    };
    let (manifest, files) = open_bytes(&bytes)?;
    Ok(Opened { manifest, files, sha256: sha256(&bytes), signer, size: bytes.len() as u64 })
}

/// The untrusted comment `pitwall extension sign` writes in a signature:
/// it names the public key, so the file can be checked without it.
pub fn signature_comment(signer: &Signer) -> String {
    format!("pitwall package signature, public key {}", signer.public_key)
}

fn file_shown(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// The key that made minisign signature `sig` (the text of a `.minisig`)
/// of `bytes`, checked. Its public key is `trusted` when that is the key
/// the signature names; otherwise the one in the signature's untrusted
/// comment (`pitwall extension sign` writes it there; with `minisign`,
/// `-c "public key <key>"`), which must have that key id.
pub fn verify(bytes: &[u8], sig: &str, trusted: Option<&Signer>) -> Result<Signer, String> {
    let not = || "Its signature file isn't a minisign signature.".to_string();
    let parsed = minisign_verify::Signature::decode(sig).map_err(|_| not())?;
    let raw = sig.lines().nth(1).and_then(|l| base64::engine::general_purpose::STANDARD.decode(l.trim()).ok()).filter(|r| r.len() == 74).ok_or_else(not)?;
    if raw[..2] == *b"Ed" {
        return Err("It is signed in minisign's old format: sign it again with `pitwall extension sign` or minisign 0.8 or later.".into());
    }
    let id = key_id(&raw[2..10]);
    let signer = match trusted.filter(|t| t.key_id == id) {
        Some(t) => t.clone(),
        None => parsed
            .untrusted_comment()
            .split_whitespace()
            .filter_map(|w| Signer::from_public_key(w.trim_matches(|c: char| c == '"' || c == ',' || c == '.')).ok())
            .find(|s| s.key_id == id)
            .ok_or_else(|| format!("It is signed by key {id}, but its public key isn't in the signature file: sign it with `pitwall extension sign`, or with minisign add -c \"public key <your public key>\"."))?,
    };
    let key = minisign_verify::PublicKey::from_base64(&signer.public_key).map_err(|_| not())?;
    key.verify(bytes, &parsed, false).map_err(|_| "Its signature doesn't match the file: it was changed after it was signed, or the signature is another file's.".to_string())?;
    Ok(signer)
}

/// Package bytes `bytes`, read and checked: the zip ([`zip::list`], every
/// name by [`check_name`], at most [`MAX_FILES`] and [`MAX_BYTES`]
/// unpacked), its manifest (only tiles: nothing that runs programs), the
/// files it holds and their sizes, and its component's header.
pub fn open_bytes(bytes: &[u8]) -> Result<(Manifest, Vec<zip::Entry>), String> {
    let limits = zip::Limits { packed: MAX_BYTES, unpacked: MAX_BYTES, files: MAX_FILES, entry: MAX_WASM_BYTES };
    let listed = zip::list(bytes, limits, &check_name)?;
    let manifest = listed.iter().find(|l| l.name == MANIFEST).ok_or_else(|| format!("It has no {MANIFEST}, which every package holds."))?;
    if manifest.size > 64 << 10 {
        return Err(format!("Its {MANIFEST} is larger than 64 KiB."));
    }
    let text = String::from_utf8(zip::read(bytes, manifest)?).map_err(|_| format!("Its {MANIFEST} isn't UTF-8 text."))?;
    let m = super::parse(&text).map_err(|why| format!("Its manifest can't be used: {why}."))?;
    super::preview::tiles_only(&m, "a package")?;
    let sizes: Vec<(&str, u64)> = listed.iter().map(|l| (l.name.as_str(), l.size)).collect();
    check_layout(&sizes, &m)?;
    let files: Vec<zip::Entry> = listed.iter().map(|l| Ok(zip::Entry { name: l.name.clone(), bytes: zip::read(bytes, l)? })).collect::<Result<_, String>>()?;
    let contents: Vec<(&str, &[u8])> = files.iter().map(|f| (f.name.as_str(), f.bytes.as_slice())).collect();
    super::media::check_files(&contents, &m).map_err(|why| format!("{}.", capital(why.trim_end_matches('.'))))?;
    let wasm = files.iter().find(|f| f.name == WASM).map(|f| f.bytes.as_slice()).unwrap_or_default();
    super::preview::component_head(wasm, WASM)?;
    Ok((m, files))
}

/// Write `files` into folder `into`, which must not exist yet (made
/// here). Their names were checked ([`check_name`]): each lands inside.
pub fn unpack(files: &[zip::Entry], into: &Path) -> Result<(), String> {
    std::fs::create_dir(into).map_err(|e| format!("Can't make {}: {e}", into.display()))?;
    for f in files {
        check_name(&f.name)?;
        let path = into.join(&f.name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Can't make {}: {e}", parent.display()))?;
        }
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("Can't write {}: {e}", f.name))?;
        std::io::Write::write_all(&mut out, &f.bytes).map_err(|e| format!("Can't write {}: {e}", f.name))?;
    }
    Ok(())
}

/// Package file `path`, read and checked ([`open`]; `extensions` the
/// extensions folder, whose installed copy's key is the trusted one),
/// then unpacked into a hidden folder of `extensions`
/// (`.fetched-package-…`, cleared at start like a fetch's) and checked
/// again there as a preview folder is: the offer to install, its folder
/// Pitwall's to delete.
pub fn stage(path: &Path, extensions: &Path) -> Result<(Offer, Opened), String> {
    let path = path.canonicalize().map_err(|_| format!("{} isn't there.", path.display()))?;
    // The id isn't known before the file is read: the key of the copy
    // installed under the id it names, if signed, is checked after.
    let first = open(&path, None).or_else(|why| {
        // Signed by the installed copy's key but without the key in its
        // signature file: read again trusting each installed signed copy
        // whose key the signature names.
        trusted_keys(extensions).iter().find_map(|k| open(&path, Some(k)).ok()).ok_or(why)
    })?;
    let opened = match (installed_signer(extensions, &first.manifest.id), &first.signer) {
        // Its key as recorded at install, not as the signature file says.
        (Some(k), Some(s)) if k.key_id == s.key_id => open(&path, Some(&k))?,
        _ => first,
    };
    std::fs::create_dir_all(extensions).map_err(|e| format!("Can't make {}: {e}", extensions.display()))?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or_default();
    let into = extensions.join(format!(".fetched-package-{}-{}-{nanos}", opened.manifest.id, std::process::id()));
    let checked = unpack(&opened.files, &into).and_then(|()| super::preview::check(&into));
    match checked {
        Ok(m) if m == opened.manifest => {
            let origin = Origin::Package { path: path.to_string_lossy().into_owned(), sha256: opened.sha256.clone(), signer: opened.signer.clone(), index: None };
            Ok((Offer { manifest: m, folder: into, origin }, opened))
        }
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&into);
            Err("It unpacked differently from how it read: not installed.".into())
        }
        Err(why) => {
            let _ = std::fs::remove_dir_all(&into);
            Err(why)
        }
    }
}

/// The key installed extension `id` was signed with, when it came from a
/// signed package.
pub fn installed_signer(extensions: &Path, id: &str) -> Option<Signer> {
    match install::origin(&extensions.join(id)) {
        Some(Origin::Package { signer, .. }) => signer,
        _ => None,
    }
}

/// The keys of every installed signed package.
fn trusted_keys(extensions: &Path) -> Vec<Signer> {
    let Ok(entries) = std::fs::read_dir(extensions) else { return Vec::new() };
    entries.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).filter_map(|e| installed_signer(extensions, &e.file_name().to_string_lossy())).collect()
}

/// How a package's signature stands against the installed copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// Signed, and nothing installed was signed: installing trusts this
    /// key for the extension's updates.
    New(Signer),
    /// Signed by the key the installed copy was.
    Same(Signer),
    /// Signed by another key than the installed copy, or not signed
    /// while it was (`now: None`).
    Changed { now: Option<Signer>, was: Signer },
    /// Not signed (nor was the installed copy).
    Unsigned,
}

/// How signer `now` stands against the installed copy's origin
/// `installed`.
pub fn key(installed: Option<&Origin>, now: Option<&Signer>) -> Key {
    let was = match installed {
        Some(Origin::Package { signer: Some(s), .. }) => Some(s),
        _ => None,
    };
    match (was, now) {
        (None, None) => Key::Unsigned,
        (None, Some(s)) => Key::New(s.clone()),
        (Some(w), Some(s)) if w.same(s) => Key::Same(s.clone()),
        (Some(w), now) => Key::Changed { now: now.cloned(), was: w.clone() },
    }
}

impl Key {
    /// In a line, for the review and the CLI.
    pub fn line(&self, name: &str) -> String {
        match self {
            Key::New(s) => format!("Signed by key {}. Installing trusts this key for {name}: its updates must be signed by it too.", s.key_id),
            Key::Same(s) => format!("Signed by key {}, the same key as the installed version.", s.key_id),
            Key::Changed { now: Some(s), was } => format!("Signed by another key: {} (the installed version was signed by {}). Only trust it if its author told you the key changed.", s.key_id, was.key_id),
            Key::Changed { now: None, was } => format!("Not signed, but the installed version is (key {}). Only install it if its author told you why.", was.key_id),
            Key::Unsigned => UNSIGNED_WARNING.into(),
        }
    }
}

/// What the user agreed to beyond a package signed by the key already
/// trusted: installing one without a signature, or one signed by another
/// key than the installed copy (or none, while it was signed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Accept {
    pub unsigned: bool,
    pub new_key: bool,
}

/// Why `offer` can't replace or be installed beside `installed` (the
/// installed copy, if any) with what the user agreed to (`accept`);
/// `None` when it can. A package must be newer than the installed copy
/// (Go back is how to return to an older one), or the same file; an
/// unsigned one needs `accept.unsigned`, a key change `accept.new_key`.
/// A folder (always unsigned) can't replace a signed package.
pub fn refusal(installed: Option<&Local>, offer: &Offer, accept: Accept) -> Option<String> {
    let was = installed.and_then(|l| l.origin.as_ref());
    let name = &offer.manifest.name;
    let now = match &offer.origin {
        Origin::Package { signer, .. } => signer.as_ref(),
        Origin::Folder { .. } => {
            return match key(was, None) {
                Key::Changed { was, .. } => Some(format!("{name} is installed from a package signed by key {}: a folder can't replace it. Remove it first, or install a newer signed package.", was.key_id)),
                _ => None,
            };
        }
        _ => return None,
    };
    if let (Some(local), Some(v)) = (installed, offer.manifest.version.as_deref()) {
        let have = local.version.as_deref().unwrap_or_default();
        match install::compare_versions(v, have) {
            std::cmp::Ordering::Less => return Some(format!("This file is {name} {v}, older than the installed {have}. To return to the version before the last update, use Go back.")),
            std::cmp::Ordering::Equal => {
                if let (Some(Origin::Package { sha256: a, .. }), Origin::Package { sha256: b, .. }) = (was, &offer.origin) {
                    return Some(if a == b { format!("{name} {v} is installed from this file already.") } else { format!("This file is {name} {v}, the installed version, but its contents differ from the installed copy's. A new release needs a new version.") });
                }
            }
            std::cmp::Ordering::Greater => {}
        }
    }
    match key(was, now) {
        Key::Unsigned if !accept.unsigned => Some(format!("{name}'s file isn't signed. Install it only if you trust whoever gave it to you: choose Install unsigned (`--unsigned`).")),
        Key::Changed { now, was } if !accept.new_key => Some(match now {
            Some(s) => format!("{name} is signed by another key ({}) than the installed version ({}). A new key is trusted only when you say so (`--new-key`).", s.key_id, was.key_id),
            None => format!("{name}'s file isn't signed, but the installed version is (key {}). Installing it anyway needs your say-so (`--new-key`).", was.key_id),
        }),
        _ => None,
    }
}

/// Install package `offer` (from [`stage`]) as [`install::install`]
/// does, once [`refusal`] has nothing against it.
pub fn install(offer: &Offer, extensions: &Path, accept: Accept) -> Result<(), String> {
    let target = extensions.join(&offer.manifest.id);
    let local = target.is_dir().then(|| install::local(&target));
    if let Some(why) = refusal(local.as_ref(), offer, accept) {
        return Err(why);
    }
    install::install(offer, extensions)
}

/// Replace the installed copy with package `offer` as [`install::update`]
/// does (the installed copy kept for Go back), once [`refusal`] has
/// nothing against it (checked against the copy installed now, not the
/// one the user looked at).
pub fn update(offer: &Offer, extensions: &Path, replace_changed: bool, accept: Accept) -> Result<(), Refused> {
    let local = install::local(&extensions.join(&offer.manifest.id));
    if let Some(why) = refusal(Some(&local), offer, accept) {
        return Err(Refused::Failed(why));
    }
    install::update(offer, extensions, replace_changed)
}

/// One change of permissions an update makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionChange {
    /// It asks for a permission it didn't (not granted until allowed):
    /// its name and reason.
    Added { name: String, reason: String },
    /// It asks for more of one it had: `net`'s new hosts (asked again),
    /// a larger `memory` or `sql` size (from, to).
    Widened { name: String, lines: Vec<String> },
    /// It no longer asks for one (its grant is forgotten).
    Removed { name: String },
}

impl PermissionChange {
    /// Its permission's name.
    pub fn name(&self) -> &str {
        match self {
            PermissionChange::Added { name, .. } | PermissionChange::Widened { name, .. } | PermissionChange::Removed { name } => name,
        }
    }
}

/// What updating `old` to `new` changes in the permissions it asks for:
/// added ones first, then widened, then removed.
pub fn permission_changes(old: &Manifest, new: &Manifest) -> Vec<PermissionChange> {
    use super::permissions::{amount, MEMORY, NET, SQL};
    let had = |n: &str| old.permissions.iter().any(|p| p.name == n);
    let mut out: Vec<PermissionChange> = new.permissions.iter().filter(|p| !had(&p.name)).map(|p| PermissionChange::Added { name: p.name.clone(), reason: p.reason.clone() }).collect();
    for p in new.permissions.iter().filter(|p| had(&p.name)) {
        let lines = match p.name.as_str() {
            NET => {
                let was: Vec<String> = old.net_hosts.iter().map(|h| h.written()).collect();
                let from_settings = new.net_setting_hosts.iter().filter(|k| !old.net_setting_hosts.contains(k)).map(|k| super::permissions::setting_host_words(new, k));
                new.net_hosts.iter().map(|h| h.written()).filter(|h| !was.contains(h)).chain(from_settings).map(|h| format!("+ {h}")).collect()
            }
            MEMORY | SQL if mib(new, &p.name) > mib(old, &p.name) => vec![format!("{} → {}", amount(old, &p.name).unwrap_or_default(), amount(new, &p.name).unwrap_or_default())],
            _ => vec![],
        };
        if !lines.is_empty() {
            out.push(PermissionChange::Widened { name: p.name.clone(), lines });
        }
    }
    out.extend(old.permissions.iter().filter(|o| !new.permissions.iter().any(|p| p.name == o.name)).map(|o| PermissionChange::Removed { name: o.name.clone() }));
    out
}

fn mib(m: &Manifest, name: &str) -> u32 {
    match name {
        super::permissions::MEMORY => m.memory_mib.unwrap_or(super::TILE_MEMORY_MIB),
        _ => m.sql_mib.unwrap_or(super::TILE_SQL_MIB),
    }
}

/// What the review and the CLI list for package `m` against the installed
/// manifest `old` (`None`: a new install): how its signature stands, the
/// tiles it adds and what it asks for (a new install), or what changes in
/// its permissions (an update).
pub fn review_lines(m: &Manifest, key: &Key, old: Option<&Manifest>) -> Vec<String> {
    let mut out = vec![key.line(&m.name)];
    match old {
        None => out.extend(super::preview::review_lines(m).into_iter().filter(|l| l != super::preview::WARNING)),
        Some(old) => {
            let changes = permission_changes(old, m);
            if changes.is_empty() {
                out.push("Its permissions don't change.".into());
            }
            for c in changes {
                let title = super::permissions::find(c.name()).map_or(c.name().to_string(), |k| format!("{} ({})", k.title, c.name()));
                match c {
                    PermissionChange::Added { reason, .. } => out.push(format!("New: {title}: “{reason}”. Not granted until you allow it.")),
                    PermissionChange::Widened { lines, .. } => out.push(format!("More: {title}: {}. Asked again before it is used.", lines.join(", "))),
                    PermissionChange::Removed { .. } => out.push(format!("No longer asks for {title}.")),
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Signing, TempDir};

    const COMPONENT: [u8; 8] = [0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00];

    fn manifest(version: &str, perms: &str) -> String {
        format!(r#"{{"v": 2, "id": "hello", "name": "Hello", "version": "{version}", "permissions": {{{perms}}},
            "contributes": {{"tiles": [{{"id": "main", "title": "Hello", "wasm": "tile.wasm", "input": "input.schema.json"}}]}}}}"#)
    }

    /// An author's folder: what a package holds, and what it doesn't
    /// (sources, the build folder, a hidden file).
    fn project(t: &TempDir, version: &str, perms: &str) -> PathBuf {
        let d = t.path().join(format!("hello-{version}"));
        for sub in ["src", "target/release", "assets/icons", "screenshots"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        std::fs::write(d.join(MANIFEST), manifest(version, perms)).unwrap();
        let mut wasm = COMPONENT.to_vec();
        wasm.extend(std::iter::repeat_n(7u8, 3000));
        std::fs::write(d.join(WASM), wasm).unwrap();
        std::fs::write(d.join(README), "# Hello\n").unwrap();
        std::fs::write(d.join("LICENSE"), "MIT").unwrap();
        std::fs::write(d.join("input.schema.json"), "{}").unwrap();
        std::fs::write(d.join("assets/icons/wave.svg"), "<svg/>").unwrap();
        std::fs::write(d.join("assets/.DS_Store"), "junk").unwrap();
        std::fs::write(d.join("screenshots/main.png"), super::super::media::png_header(1280, 800)).unwrap();
        std::fs::write(d.join("src/lib.rs"), "fn main() {}").unwrap();
        std::fs::write(d.join("target/release/x.wasm"), "x").unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]").unwrap();
        d
    }

    fn write_package(t: &TempDir, p: &Packed, signing: Option<&Signing>) -> PathBuf {
        let path = t.path().join(file_name(&p.manifest));
        std::fs::write(&path, &p.bytes).unwrap();
        if let Some(s) = signing {
            std::fs::write(signature_path(&path), s.sign(&p.bytes)).unwrap();
        }
        path
    }

    /// Its icon file, screenshots and the images its README shows from
    /// its folder are packed, each checked; a README image outside the
    /// folder (or not an image) is refused, one from the network isn't
    /// packed.
    #[test]
    fn listing_media_is_packed_and_checked() {
        use super::super::media::png_header;
        let t = TempDir::new("pack-media");
        let d = project(&t, "1.0.0", "");
        let m = std::fs::read_to_string(d.join(MANIFEST)).unwrap().replacen(r#""name": "Hello","#, r#""name": "Hello", "icon": "icon.png", "tagline": "Says hello", "categories": ["fun"], "screenshots": [{"path": "screenshots/main.png", "caption": "The tile"}],"#, 1);
        std::fs::write(d.join(MANIFEST), m).unwrap();
        std::fs::write(d.join("icon.png"), png_header(256, 256)).unwrap();
        std::fs::create_dir_all(d.join("media")).unwrap();
        std::fs::write(d.join("media/flow.png"), png_header(900, 500)).unwrap();
        std::fs::write(d.join("docs.png"), png_header(900, 500)).unwrap();
        std::fs::write(d.join(README), "# Hello\n\n![Flow](media/flow.png)\n\n![Badge](https://img.shields.io/x.svg)\n").unwrap();
        let p = pack(&d).unwrap();
        let names: Vec<&str> = p.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["LICENSE", "README.md", "assets/icons/wave.svg", "extension.json", "icon.png", "input.schema.json", "media/flow.png", "screenshots/main.png", "tile.wasm"]);
        assert_eq!((p.manifest.icon_file.as_deref(), p.manifest.tagline.as_deref()), (Some("icon.png"), Some("Says hello")));
        // `..` and an image outside the image folders are refused.
        std::fs::write(d.join(README), "# Hello\n\n![Up](../secret.png)\n").unwrap();
        assert!(pack(&d).unwrap_err().contains("goes out of the extension's folder"));
        std::fs::write(d.join(README), "# Hello\n\n![Here](docs.png)\n").unwrap();
        assert!(pack(&d).unwrap_err().contains("put the images it shows in media/"));
        std::fs::write(d.join(README), "# Hello\n\n![Missing](media/none.png)\n").unwrap();
        assert!(pack(&d).unwrap_err().contains("none.png isn't in its folder"));
        std::fs::write(d.join(README), "# Hello\n").unwrap();
        // An icon that isn't square, or an SVG in disguise.
        std::fs::write(d.join("icon.png"), png_header(256, 128)).unwrap();
        assert!(pack(&d).unwrap_err().contains("square"));
        std::fs::write(d.join("icon.png"), "<svg/>").unwrap();
        assert!(pack(&d).unwrap_err().contains("isn't a PNG or WebP"));
    }

    /// Packing takes what a package holds and nothing else, gives the same
    /// bytes again, and reads back.
    #[test]
    fn a_folder_packs_the_same_every_time() {
        let t = TempDir::new("pack");
        let d = project(&t, "1.0.0", "");
        let p = pack(&d).unwrap();
        let names: Vec<&str> = p.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["LICENSE", "README.md", "assets/icons/wave.svg", "extension.json", "input.schema.json", "screenshots/main.png", "tile.wasm"]);
        assert_eq!(file_name(&p.manifest), "hello-1.0.0.pwtile");
        // Touched files, another folder: the same bytes.
        let other = TempDir::new("pack-again");
        let d2 = project(&other, "1.0.0", "");
        assert_eq!(pack(&d2).unwrap().bytes, p.bytes);
        assert_eq!(p.sha256, sha256(&p.bytes));
        // No readme: refused, saying what's missing.
        std::fs::remove_file(d.join(README)).unwrap();
        assert!(pack(&d).unwrap_err().contains("no README.md"));
        // A font in assets: refused.
        std::fs::write(d.join(README), "# Hello").unwrap();
        std::fs::write(d.join("assets/x.ttf"), "font").unwrap();
        assert!(pack(&d).unwrap_err().contains("is a font"));
    }

    /// Names that could escape the folder or clash on some system are
    /// refused.
    #[test]
    fn names_are_checked() {
        for ok in ["tile.wasm", "assets/a-b_c.svg", "screenshots/1.png"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        for bad in ["/etc/passwd", "../x", "a/../b", "a//b", "./a", ".hidden", "a/.git/x", "C:/x", "a\\b", "nul.txt", "assets/COM1.png", "café.md", "a.", "", "a b"] {
            assert!(check_name(bad).is_err(), "{bad:?} passed");
        }
    }

    /// A package with a file it shouldn't hold, with a path that climbs
    /// out, or with a module instead of a component, is refused before
    /// anything is unpacked.
    #[test]
    fn a_crafted_package_is_refused() {
        let base = |extra: (&str, &[u8])| {
            let mut f: Vec<(String, Vec<u8>)> = vec![(MANIFEST.into(), manifest("1.0.0", "").into_bytes()), (WASM.into(), COMPONENT.to_vec()), (README.into(), b"# Hi".to_vec()), ("input.schema.json".into(), b"{}".to_vec())];
            f.push((extra.0.into(), extra.1.to_vec()));
            zip::write(&f)
        };
        assert!(open_bytes(&base(("LICENSE", b"MIT"))).is_ok());
        assert!(open_bytes(&base(("../evil.sh", b"rm -rf"))).unwrap_err().contains("“.” or “..”"));
        assert!(open_bytes(&base(("/abs", b"x"))).unwrap_err().contains("absolute"));
        assert!(open_bytes(&base(("run.sh", b"x"))).unwrap_err().contains("isn't a file a package holds"));
        assert!(open_bytes(&base(("README.MD", b"x"))).unwrap_err().contains("Two files"));
        let module = zip::write(&[(MANIFEST.into(), manifest("1.0.0", "").into_bytes()), (WASM.into(), vec![0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]), (README.into(), b"#".to_vec()), ("input.schema.json".into(), b"{}".to_vec())]);
        assert!(open_bytes(&module).unwrap_err().contains("module, not a component"));
        let adapter = manifest("1.0.0", "").replace(r#""version": "1.0.0","#, r#""version": "1.0.0", "adapter": {"command": ["node", "a.mjs"]},"#);
        let runs = zip::write(&[(MANIFEST.into(), adapter.into_bytes()), (WASM.into(), COMPONENT.to_vec()), (README.into(), b"#".to_vec()), ("input.schema.json".into(), b"{}".to_vec())]);
        assert!(open_bytes(&runs).unwrap_err().contains("runs programs"));
        let png = super::super::media::png_header(640, 400);
        let screenshots: Vec<(String, Vec<u8>)> = (0..9).map(|i| (format!("screenshots/{i}.png"), png.clone())).collect();
        let mut many = vec![(MANIFEST.into(), manifest("1.0.0", "").into_bytes()), (WASM.into(), COMPONENT.to_vec()), (README.into(), b"#".to_vec()), ("input.schema.json".into(), b"{}".to_vec())];
        many.extend(screenshots);
        assert!(open_bytes(&zip::write(&many)).unwrap_err().contains("more than 8 screenshots"));
        // A dark and light pair counts once: 8 pairs are fine.
        let pairs: Vec<(String, Vec<u8>)> = (0..8).flat_map(|i| [(format!("screenshots/{i}-dark.png"), png.clone()), (format!("screenshots/{i}-light.png"), png.clone())]).collect();
        let mut ok = vec![(MANIFEST.into(), manifest("1.0.0", "").into_bytes()), (WASM.into(), COMPONENT.to_vec()), (README.into(), b"#".to_vec()), ("input.schema.json".into(), b"{}".to_vec())];
        ok.extend(pairs);
        assert!(open_bytes(&zip::write(&ok)).is_ok());
        // Images are checked by what they hold, not their names.
        assert!(open_bytes(&base(("screenshots/x.png", b"<svg onload=alert(1)/>"))).unwrap_err().contains("isn't a PNG or WebP"));
        assert!(open_bytes(&base(("media/x.gif", &png))).unwrap_err().contains("aren't what its name says"));
        assert!(open_bytes(&base(("media/x.svg", b"<svg/>"))).unwrap_err().contains("media/ holds"));
        assert!(open_bytes(&base(("media/x.png", &png))).is_ok());
    }

    /// A signature by the key in its file passes and names the key; one
    /// over other bytes, a tampered package and one without its public
    /// key fail; an installed copy's key is used when the signature names
    /// it.
    #[test]
    fn signatures_are_checked_over_the_bytes() {
        let t = TempDir::new("sign");
        let p = pack(&project(&t, "1.0.0", "")).unwrap();
        let key = Signing::new();
        let path = write_package(&t, &p, Some(&key));
        let o = open(&path, None).unwrap();
        assert_eq!(o.signer.as_ref(), Some(&key.signer()));
        assert_eq!(o.signer.as_ref().unwrap().key_id.len(), 16);
        // Tampered after signing.
        let mut changed = p.bytes.clone();
        let n = changed.len();
        changed[n / 2] ^= 1;
        std::fs::write(&path, &changed).unwrap();
        assert!(open(&path, None).unwrap_err().contains("doesn't match the file"));
        std::fs::write(&path, &p.bytes).unwrap();
        // Signed with minisign without the key in its comment: refused
        // unless the key is trusted already.
        std::fs::write(signature_path(&path), key.sign_bare(&p.bytes)).unwrap();
        assert!(open(&path, None).unwrap_err().contains("public key isn't in the signature file"));
        assert_eq!(open(&path, Some(&key.signer())).unwrap().signer, Some(key.signer()));
        // Another key's signature of the same bytes, claiming the first
        // key's public key in its comment: refused.
        let other = Signing::new();
        let forged = other.sign(&p.bytes).replace(&other.signer().public_key, &key.signer().public_key);
        std::fs::write(signature_path(&path), forged).unwrap();
        assert!(open(&path, None).is_err());
        // Garbage.
        std::fs::write(signature_path(&path), "hello").unwrap();
        assert!(open(&path, None).unwrap_err().contains("isn't a minisign signature"));
        // No signature: unsigned.
        std::fs::remove_file(signature_path(&path)).unwrap();
        assert_eq!(open(&path, None).unwrap().signer, None);
    }

    /// Installed from a signed package; an update by the same key passes,
    /// by another key or unsigned only when agreed; older or the same
    /// version with other bytes is refused; Go back returns the copy kept.
    #[test]
    fn updates_need_the_same_key() {
        let t = TempDir::new("pkg-update");
        let exts = t.path().join("extensions");
        let key = Signing::new();
        let v1 = write_package(&t, &pack(&project(&t, "1.0.0", r#""memory": {"reason": "Pictures", "mib": 128}"#)).unwrap(), Some(&key));
        let (offer, opened) = stage(&v1, &exts).unwrap();
        assert!(offer.folder.starts_with(&exts) && offer.folder.file_name().unwrap().to_string_lossy().starts_with(".fetched-package-hello-"));
        assert_eq!(key_of(&exts, &offer), Key::New(key.signer()));
        assert!(refusal(None, &offer, Accept::default()).is_none());
        install(&offer, &exts, Accept::default()).unwrap();
        let installed = install::local(&exts.join("hello"));
        assert!(installed.as_installed);
        assert!(matches!(&installed.origin, Some(Origin::Package { signer: Some(s), sha256, .. }) if s.same(&key.signer()) && *sha256 == opened.sha256), "{installed:?}");
        assert!(!installed.origin.as_ref().unwrap().is_preview(), "signed: not a preview");
        // The same file again: nothing to do.
        let (again, _) = stage(&v1, &exts).unwrap();
        assert!(refusal(Some(&installed), &again, Accept::default()).unwrap().contains("from this file already"));

        // 1.1.0 by the same key, asking for more.
        let t2 = TempDir::new("pkg-update-2");
        let v2 = write_package(&t2, &pack(&project(&t2, "1.1.0", r#""memory": {"reason": "Pictures", "mib": 256}, "notify": "When it's done""#)).unwrap(), Some(&key));
        let (newer, _) = stage(&v2, &exts).unwrap();
        assert_eq!(key_of(&exts, &newer), Key::Same(key.signer()));
        let old = super::super::read(&exts.join("hello"), "hello").unwrap();
        let lines = review_lines(&newer.manifest, &key_of(&exts, &newer), Some(&old));
        assert!(lines[0].contains("same key"), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("New: ") && l.contains("notify") && l.contains("Not granted until you allow it")), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("More: ") && l.contains("128 MiB → 256 MiB")), "{lines:?}");
        update(&newer, &exts, false, Accept::default()).unwrap();
        assert_eq!(install::local(&exts.join("hello")).version.as_deref(), Some("1.1.0"));
        assert_eq!(install::previous(&exts, "hello").and_then(|p| p.version).as_deref(), Some("1.0.0"), "kept for Go back");

        // 1.2.0 by another key: refused unless agreed.
        let t3 = TempDir::new("pkg-update-3");
        let thief = Signing::new();
        let v3 = write_package(&t3, &pack(&project(&t3, "1.2.0", "")).unwrap(), Some(&thief));
        let (stolen, _) = stage(&v3, &exts).unwrap();
        assert!(matches!(key_of(&exts, &stolen), Key::Changed { now: Some(_), .. }));
        let Err(Refused::Failed(why)) = update(&stolen, &exts, false, Accept::default()) else { panic!("another key updated it") };
        assert!(why.contains("another key") && why.contains(&key.signer().key_id), "{why}");
        // Unsigned: refused too, even with "unsigned" agreed.
        std::fs::remove_file(signature_path(&v3)).unwrap();
        let (bare, _) = stage(&v3, &exts).unwrap();
        assert!(matches!(key_of(&exts, &bare), Key::Changed { now: None, .. }));
        assert!(update(&bare, &exts, false, Accept { unsigned: true, new_key: false }).is_err());
        // A folder can't replace it either.
        let folder = super::super::preview::offer(&project(&t3, "2.0.0", "")).unwrap();
        let installed = install::local(&exts.join("hello"));
        assert!(refusal(Some(&installed), &folder, Accept::default()).unwrap().contains("a folder can't replace it"));
        // Older: refused, pointing at Go back.
        let (older, _) = stage(&v1, &exts).unwrap();
        assert!(refusal(Some(&installed), &older, Accept::default()).unwrap().contains("Go back"));
        // Agreed to the new key: updated, and it is trusted from now on.
        update(&stolen, &exts, false, Accept { unsigned: false, new_key: true }).unwrap();
        assert_eq!(installed_signer(&exts, "hello"), Some(thief.signer()));
        // Go back: 1.1.0 with the first key again.
        install::rollback(&exts, "hello", false).unwrap();
        assert_eq!(install::local(&exts.join("hello")).version.as_deref(), Some("1.1.0"));
        assert_eq!(installed_signer(&exts, "hello"), Some(key.signer()));
        for o in [&again, &bare, &older] {
            let _ = std::fs::remove_dir_all(&o.folder);
        }
    }

    /// An unsigned package installs only when agreed, as a preview.
    #[test]
    fn an_unsigned_package_needs_a_yes() {
        let t = TempDir::new("pkg-unsigned");
        let exts = t.path().join("extensions");
        let path = write_package(&t, &pack(&project(&t, "1.0.0", "")).unwrap(), None);
        let (offer, opened) = stage(&path, &exts).unwrap();
        assert_eq!(opened.signer, None);
        assert_eq!(key_of(&exts, &offer), Key::Unsigned);
        assert!(install(&offer, &exts, Accept::default()).unwrap_err().contains("isn't signed"));
        install(&offer, &exts, Accept { unsigned: true, new_key: false }).unwrap();
        let origin = install::origin(&exts.join("hello")).unwrap();
        assert!(origin.is_preview() && origin.describe().starts_with("Unsigned package"), "{origin:?}");
    }

    /// Packages whose signature names the installed copy's key are read
    /// with that key, even without the key in the signature file.
    #[test]
    fn an_update_signed_with_minisign_uses_the_trusted_key() {
        let t = TempDir::new("pkg-bare");
        let exts = t.path().join("extensions");
        let key = Signing::new();
        let v1 = write_package(&t, &pack(&project(&t, "1.0.0", "")).unwrap(), Some(&key));
        let (offer, _) = stage(&v1, &exts).unwrap();
        install(&offer, &exts, Accept::default()).unwrap();
        let t2 = TempDir::new("pkg-bare-2");
        let p2 = pack(&project(&t2, "1.0.1", "")).unwrap();
        let v2 = t2.path().join(file_name(&p2.manifest));
        std::fs::write(&v2, &p2.bytes).unwrap();
        std::fs::write(signature_path(&v2), key.sign_bare(&p2.bytes)).unwrap();
        let (newer, _) = stage(&v2, &exts).unwrap();
        assert_eq!(key_of(&exts, &newer), Key::Same(key.signer()));
        let _ = std::fs::remove_dir_all(&newer.folder);
    }

    /// A key id is only a label its owner picks: another key claiming the
    /// installed copy's id (in the signature and the key it names) doesn't
    /// pass for it.
    #[test]
    fn a_borrowed_key_id_doesnt_pass_for_the_key() {
        let b64 = base64::engine::general_purpose::STANDARD;
        let t = TempDir::new("pkg-forge");
        let exts = t.path().join("extensions");
        let key = Signing::new();
        let v1 = write_package(&t, &pack(&project(&t, "1.0.0", "")).unwrap(), Some(&key));
        let (offer, _) = stage(&v1, &exts).unwrap();
        install(&offer, &exts, Accept::default()).unwrap();
        let victim_id = b64.decode(&key.signer().public_key).unwrap()[2..10].to_vec();
        let t2 = TempDir::new("pkg-forge-2");
        let p2 = pack(&project(&t2, "1.0.1", "")).unwrap();
        let v2 = t2.path().join(file_name(&p2.manifest));
        std::fs::write(&v2, &p2.bytes).unwrap();
        let thief = Signing::new();
        let relabel = |text: &str, at: usize| {
            let mut raw = b64.decode(text).unwrap();
            raw[at..at + 8].copy_from_slice(&victim_id);
            b64.encode(raw)
        };
        let theirs = thief.signer().public_key;
        let sig = thief.sign(&p2.bytes);
        let lines: Vec<&str> = sig.lines().collect();
        let forged = format!("{}\n{}\n{}\n{}\n", lines[0].replace(&theirs, &relabel(&theirs, 2)), relabel(lines[1], 2), lines[2], lines[3]);
        std::fs::write(signature_path(&v2), &forged).unwrap();
        // Alone it reads as that id, but with the thief's public key.
        let alone = open(&v2, None).unwrap().signer.unwrap();
        assert_eq!(alone.key_id, key.signer().key_id);
        assert!(!alone.same(&key.signer()));
        // Against the installed copy: refused.
        assert!(stage(&v2, &exts).unwrap_err().contains("doesn't match"));
    }

    fn key_of(exts: &Path, offer: &Offer) -> Key {
        let installed = install::origin(&exts.join(&offer.manifest.id));
        let Origin::Package { signer, .. } = &offer.origin else { panic!("not a package") };
        key(installed.as_ref(), signer.as_ref())
    }
}
