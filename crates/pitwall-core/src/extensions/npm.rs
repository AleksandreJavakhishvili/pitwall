//! Updates from an npm package (docs/spec/extensions.md, "Updates from an
//! npm package"): a manifest's `source` names a package (and optionally a
//! registry). [`releases`] reads the registry's metadata document of the
//! package (`<registry>/<package>`, abbreviated), each version giving its
//! tarball and the tarball's integrity; [`fetch`] downloads one version's
//! tarball, checks it against that integrity (sha512) and unpacks it into a
//! hidden folder, within limits, refusing anything but plain files and
//! folders under `package/`.
//!
//! Pitwall does all of it itself: it never runs npm, node or a package's
//! scripts, and never installs dependencies (a package must hold what it
//! runs). Requests go over https with no credentials (no `.npmrc`, no
//! token, no cookie), no prompt, timeouts and size limits, through the
//! host's client ([`Http`]: the app's uses ureq), so this crate holds no
//! TLS of its own.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use serde_json::Value;

use super::install::{compare_versions, MARKER, SKIPPED};
use super::source::{fetched_manifest, hidden, is_pre, MAX_BYTES, MAX_FILES};
use super::Manifest;

/// The most the registry's metadata of a package may be, in bytes.
pub const MAX_METADATA: u64 = 5 * 1024 * 1024;

/// The most a tarball may hold uncompressed (its files' bytes are held to
/// [`MAX_BYTES`]; this bounds headers and padding too).
const MAX_TAR: u64 = MAX_BYTES + 16 * 1024 * 1024;
/// The most entries (files, folders, headers) a tarball may have.
const MAX_ENTRIES: usize = 3 * MAX_FILES;

/// How long connecting may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long reading the metadata may take.
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
/// How long downloading a tarball may take.
const TARBALL_TIMEOUT: Duration = Duration::from_secs(180);

/// What the registry's abbreviated metadata is asked for with (npm's own
/// install format, with plain JSON as a fallback).
const ACCEPT: &str = "application/vnd.npm.install-v1+json; q=1.0, application/json; q=0.8";

/// A version of a package, as the registry's metadata lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// Its tarball's sha512 integrity (`sha512-<base64>`); `None` when the
    /// registry gives none (such a version is refused).
    pub integrity: Option<String>,
    /// Where its tarball is.
    pub tarball: String,
    /// Its author marked it deprecated.
    pub deprecated: bool,
}

/// Integrity `i` short, for reading: "sha512 1a2b3c4d5e6f…".
pub fn short(i: &str) -> String {
    let b64 = i.strip_prefix("sha512-").unwrap_or(i);
    let hex: String = base64::engine::general_purpose::STANDARD.decode(b64).map(|b| b.iter().take(6).map(|x| format!("{x:02x}")).collect()).unwrap_or_else(|_| b64.chars().take(12).collect());
    format!("sha512 {hex}…")
}

/// Url `url` where a test fetches it from: the first of `via`'s (from,
/// to) prefixes it starts with, replaced. Empty outside tests.
pub fn routed(url: &str, via: &[(String, String)]) -> String {
    for (from, to) in via {
        if let Some(rest) = url.strip_prefix(from.as_str()) {
            return format!("{to}{rest}");
        }
    }
    url.to_string()
}

/// The address of package `package`'s metadata in `registry` (a scoped
/// name's `/` escaped, as npm asks it).
pub fn metadata_url(registry: &str, package: &str) -> String {
    format!("{}/{}", registry.trim_end_matches('/'), package.replace('/', "%2f"))
}

/// Whether `v` reads as a version (as a tag's version: a digit first, then
/// letters, digits and `.-+`, at most 32).
fn version_ok(v: &str) -> bool {
    !v.is_empty() && v.len() <= 32 && v.starts_with(|c: char| c.is_ascii_digit()) && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))
}

/// The sha512 entry of SRI string `sri` (`sha512-<base64 of 64 bytes>`;
/// several entries are separated by spaces), `None` when it has none.
pub fn sha512_of(sri: &str) -> Option<String> {
    sri.split_whitespace().find_map(|e| {
        let b64 = e.strip_prefix("sha512-")?;
        // An SRI entry may carry options after `?`.
        let b64 = b64.split('?').next().unwrap_or_default();
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
        (bytes.len() == 64).then(|| format!("sha512-{b64}"))
    })
}

/// The integrity of `bytes`: `sha512-<base64>`.
pub fn integrity_of(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let d = sha2::Sha512::digest(bytes);
    format!("sha512-{}", base64::engine::general_purpose::STANDARD.encode(d))
}

/// The versions in metadata document `text` of package `package`, newest
/// first. Versions that don't read as versions, or have no tarball, are
/// left out.
pub fn parse_releases(text: &[u8], package: &str) -> Result<Vec<Release>, String> {
    let doc: Value = serde_json::from_slice(text).map_err(|_| "The registry's answer isn't package metadata.".to_string())?;
    if doc["name"].as_str() != Some(package) {
        return Err(format!("The registry answered about another package than {package}."));
    }
    let Some(versions) = doc["versions"].as_object() else {
        return Err(format!("{package} has no versions on the registry."));
    };
    let mut out: Vec<Release> = Vec::new();
    for (v, info) in versions {
        if !version_ok(v) || info["version"].as_str().is_some_and(|x| x != v) {
            continue;
        }
        let Some(tarball) = info["dist"]["tarball"].as_str() else { continue };
        let integrity = info["dist"]["integrity"].as_str().and_then(sha512_of);
        let deprecated = match &info["deprecated"] {
            Value::String(s) => !s.is_empty(),
            Value::Bool(b) => *b,
            _ => false,
        };
        out.push(Release { version: v.clone(), integrity, tarball: tarball.to_string(), deprecated });
    }
    out.sort_by(|a, b| compare_versions(&b.version, &a.version).then_with(|| a.version.cmp(&b.version)));
    Ok(out)
}

/// The version to offer of `releases` (newest first, as
/// [`parse_releases`] gives them): the newest that isn't a pre-release
/// (unless `pre`: the installed version is one), and that isn't
/// deprecated when one that isn't exists.
pub fn newest(releases: &[Release], pre: bool) -> Option<&Release> {
    let mut candidates = releases.iter().filter(|r| pre || !is_pre(&r.version));
    let first = candidates.clone().next();
    candidates.find(|r| !r.deprecated).or(first)
}

/// What refusing version `version` of `package` says, when its integrity
/// is `now` where it was `was`.
pub fn changed_text(package: &str, version: &str, was: &str, now: &str) -> String {
    format!("Version {version} of {package} changed on the registry: its integrity was {} and is now {}. Not installed.", short(was), short(now))
}

/// Why offering release `r` of `package` is refused: it has no sha512
/// integrity, or its integrity isn't the one seen before for the same
/// version (`installed`: the version and integrity the installed copy came
/// from, same package; `seen`: the integrity an earlier check saw).
pub fn refusal(package: &str, r: &Release, installed: Option<(&str, &str)>, seen: Option<&str>) -> Option<String> {
    let Some(now) = &r.integrity else {
        return Some(format!("Version {} of {package} has no sha512 integrity on the registry, so Pitwall can't check its download. Not installed.", r.version));
    };
    let before = installed.filter(|(v, _)| *v == r.version).map(|(_, i)| i).or(seen)?;
    (before != now).then(|| changed_text(package, &r.version, before, now))
}

/// Why `url` (a tarball's, from the metadata) can't be fetched: https only,
/// no user or password.
fn check_tarball_url(url: &str) -> Result<(), String> {
    super::check_npm_registry(url).map_err(|e| format!("Its download address can't be used: {e}."))
}

/// Why a request didn't give a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetError {
    /// More than the limit (by its Content-Length or as read).
    TooBig,
    /// Why, in plain words ([`status_text`], [`OFFLINE`]).
    Failed(String),
}

/// How Pitwall reads an address: the host's https client (the app's), or
/// a test's. One GET of `url` with Accept header `accept`: at most `limit`
/// bytes, within `connect` to connect and `timeout` in all; no
/// credentials, no cookies, no proxy, nothing asked; redirects only to
/// https. An `http://` address only for a loopback one (a test's stand-in
/// for a registry, [`routed`]): every address Pitwall itself makes is
/// checked https first.
pub trait Http: Sync {
    fn get(&self, url: &str, accept: &str, limit: u64, connect: Duration, timeout: Duration) -> Result<Vec<u8>, GetError>;
}

/// What a request failed with, offline or the server unreachable.
pub const OFFLINE: &str = "Couldn't reach it: this computer seems to be offline, or the registry is down.";

/// What answer `code` (not 2xx) means, in plain words.
pub fn status_text(code: u16) -> String {
    match code {
        404 => "The registry has no such package or version (a private one needs a sign-in, and Pitwall sends none).".into(),
        401 | 403 => "The registry refused: it is private, and Pitwall sends no credentials.".into(),
        _ => format!("The registry answered {code}."),
    }
}

/// Whether `url` is a loopback http address (a test's stand-in).
pub fn is_loopback(url: &str) -> bool {
    ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().any(|p| url.starts_with(p))
}

/// `http.get`, with `too_big` said for a body over `limit`.
fn get(http: &dyn Http, url: &str, accept: &str, limit: u64, timeout: Duration, too_big: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") && !is_loopback(url) {
        return Err("Pitwall fetches only https addresses. Not fetched.".into());
    }
    match http.get(url, accept, limit, CONNECT_TIMEOUT, timeout) {
        Ok(b) if b.len() as u64 > limit => Err(too_big.to_string()),
        Ok(b) => Ok(b),
        Err(GetError::TooBig) => Err(too_big.to_string()),
        Err(GetError::Failed(why)) => Err(why),
    }
}

/// The versions of package `package` in `registry`, newest first
/// (blocking: call it off the UI thread; nothing is written). `via`:
/// [`routed`].
pub fn releases(registry: &str, package: &str, via: &[(String, String)], http: &dyn Http) -> Result<Vec<Release>, String> {
    super::check_npm_package(package).map_err(|e| format!("{e}."))?;
    super::check_npm_registry(registry).map_err(|e| format!("That registry can't be used: {e}."))?;
    let url = routed(&metadata_url(registry, package), via);
    let text = get(http, &url, ACCEPT, MAX_METADATA, METADATA_TIMEOUT, &format!("The registry's metadata of {package} is bigger than {} MiB. Not installed.", MAX_METADATA / 1024 / 1024))?;
    parse_releases(&text, package)
}

/// A version fetched: the extension's folder (hidden, beside the
/// installed ones), its manifest, and the version's release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub folder: PathBuf,
    pub manifest: Manifest,
    pub release: Release,
}

/// Fetch version `version` of package `package` from `registry`: its
/// metadata read again, its integrity checked to be `integrity` (the one
/// offered, when known), its tarball downloaded and checked against it,
/// then unpacked into a hidden folder in `work` (which the caller installs
/// from, then deletes). `id`: the extension it must be (an update), or any
/// (Add from npm). Refused, leaving nothing: a version that isn't listed,
/// has no sha512 integrity or another than offered; a tarball that doesn't
/// match it, is bigger than [`MAX_BYTES`] or holds more (or more than
/// [`MAX_FILES`] files) unpacked, or holds anything but files and folders
/// under `package/` ([`unpack`]); no `extension.json` at the package's
/// root, one that can't be used, of another extension or another version.
#[allow(clippy::too_many_arguments)]
pub fn fetch(registry: &str, package: &str, version: &str, integrity: Option<&str>, id: Option<&str>, work: &Path, via: &[(String, String)], http: &dyn Http) -> Result<Fetched, String> {
    let list = releases(registry, package, via, http)?;
    let r = list.into_iter().find(|r| r.version == version).ok_or_else(|| format!("Version {version} of {package} isn't on the registry."))?;
    let Some(sri) = r.integrity.clone() else {
        return Err(refusal(package, &r, None, None).unwrap_or_default());
    };
    if let Some(was) = integrity.filter(|i| *i != sri) {
        return Err(changed_text(package, version, was, &sri));
    }
    check_tarball_url(&r.tarball)?;
    let bytes = get(http, &routed(&r.tarball, via), "application/octet-stream", MAX_BYTES, TARBALL_TIMEOUT, &format!("It is bigger than {} MiB. Not installed.", MAX_BYTES / 1024 / 1024))?;
    if integrity_of(&bytes) != sri {
        return Err(format!("The download of {package} {version} doesn't match the integrity the registry gives for it ({}). Not installed.", short(&sri)));
    }
    std::fs::create_dir_all(work).map_err(|e| format!("Can't make {}: {e}", work.display()))?;
    let staged = hidden(work, "fetched");
    let what = format!("{package} {version}");
    let done = unpack(&bytes, &staged).and_then(|()| fetched_manifest(&staged, &what, "the package's root", id, version));
    match done {
        Ok(manifest) => Ok(Fetched { folder: staged, manifest, release: r }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staged);
            Err(e)
        }
    }
}

/// A tar header's number field (octal, ending with NUL or space).
fn octal(field: &[u8]) -> Option<u64> {
    if field.first().is_some_and(|b| b & 0x80 != 0) {
        return None; // base-256: far beyond any limit here
    }
    let s: String = field.iter().take_while(|b| **b != 0).map(|b| *b as char).collect();
    let s = s.trim();
    if s.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(s, 8).ok()
}

/// A NUL-terminated header text.
fn text(field: &[u8]) -> Result<String, String> {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    String::from_utf8(field[..end].to_vec()).map_err(|_| "It has a file name that isn't text. Not installed.".to_string())
}

/// Where entry `name` goes, inside the package: its parts after
/// `package/` (empty for `package/` itself). Refused: an absolute path, a
/// `..` part, a name outside `package/`, or one with `\`, `:` or control
/// characters (meant for another system).
fn inner(name: &str) -> Result<Vec<String>, String> {
    if name.starts_with('/') || Path::new(name).components().any(|c| matches!(c, Component::RootDir | Component::Prefix(_))) {
        return Err(format!("It has an absolute path ({name}). Not installed."));
    }
    let mut parts = name.split('/').filter(|p| !p.is_empty() && *p != ".");
    if parts.clone().any(|p| p == "..") {
        return Err(format!("It has a path that leaves its folder ({name}). Not installed."));
    }
    if parts.next() != Some("package") {
        return Err(format!("It has a file outside package/ ({name}). Not installed."));
    }
    let rest: Vec<String> = parts.map(str::to_string).collect();
    if rest.iter().any(|p| p.contains(['\\', ':']) || p.chars().any(char::is_control)) {
        return Err(format!("It has a file name Pitwall doesn't unpack ({name}). Not installed."));
    }
    Ok(rest)
}

/// Read `n` bytes of `r` (`into`: where they go, or dropped).
fn copy_n(r: &mut impl Read, n: u64, into: Option<&mut std::fs::File>) -> Result<(), String> {
    let mut left = r.take(n);
    let res = match into {
        Some(f) => std::io::copy(&mut left, f),
        None => std::io::copy(&mut left, &mut std::io::sink()),
    };
    match res {
        Ok(got) if got == n => Ok(()),
        _ => Err("It isn't a complete package (the download is cut short or too big unpacked). Not installed.".into()),
    }
}

/// The 512-byte padding after `n` bytes of an entry.
fn pad(n: u64) -> u64 {
    (512 - n % 512) % 512
}

/// Unpack gzipped tarball `tgz` (an npm package) into new folder `dest`:
/// its files and folders under `package/`, at `dest`'s top. Files get
/// plain permissions, executable when the tarball says so (as git checks
/// one out), nothing else of its modes, owners or times. Left out: what
/// [`SKIPPED`] names (`node_modules`, `.git`, `.DS_Store`) and a marker of
/// Pitwall's at the top. Refused: a symbolic or hard link, a device, a
/// pipe or any other kind of entry; an absolute path, a `..` part, a file
/// outside `package/` ([`inner`]); the same file twice; more than
/// [`MAX_FILES`] files or [`MAX_BYTES`] bytes. Nothing is written outside
/// `dest`: every path is made of plain parts below it, and no link is ever
/// made. The caller deletes `dest` when this fails.
pub fn unpack(tgz: &[u8], dest: &Path) -> Result<(), String> {
    std::fs::create_dir(dest).map_err(|e| format!("Can't make {}: {e}", dest.display()))?;
    let mut r = flate2::read::GzDecoder::new(tgz).take(MAX_TAR);
    let (mut files, mut bytes, mut entries) = (0usize, 0u64, 0usize);
    let mut long_name: Option<String> = None;
    let mut pax_path: Option<String> = None;
    loop {
        let mut h = [0u8; 512];
        if let Err(e) = r.read_exact(&mut h) {
            return Err(if r.limit() == 0 {
                format!("It is bigger than {} MiB unpacked. Not installed.", MAX_BYTES / 1024 / 1024)
            } else if e.kind() == std::io::ErrorKind::UnexpectedEof {
                "It isn't a complete package (.tgz). Not installed.".into()
            } else {
                format!("It isn't a package Pitwall can unpack (.tgz): {e}. Not installed.")
            });
        }
        if h.iter().all(|b| *b == 0) {
            break;
        }
        entries += 1;
        if entries > MAX_ENTRIES {
            return Err(format!("It has more than {MAX_FILES} files. Not installed."));
        }
        let sum: u64 = h.iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { *b as u64 }).sum();
        if octal(&h[148..156]) != Some(sum) {
            return Err("It isn't a package Pitwall can unpack (a damaged header). Not installed.".into());
        }
        let size = octal(&h[124..136]).ok_or("It isn't a package Pitwall can unpack (a damaged header). Not installed.")?;
        let kind = h[156];
        // Headers that carry the next entry's name.
        match kind {
            b'x' | b'g' | b'L' => {
                if size > 64 * 1024 {
                    return Err("It has a header bigger than Pitwall reads. Not installed.".into());
                }
                let mut data = vec![0u8; size as usize];
                r.read_exact(&mut data).map_err(|_| "It isn't a complete package (.tgz). Not installed.".to_string())?;
                copy_n(&mut r, pad(size), None)?;
                match kind {
                    b'L' => long_name = Some(text(&data)?),
                    b'x' => {
                        for (k, v) in pax_records(&data)? {
                            match k.as_str() {
                                "path" => pax_path = Some(v),
                                // A size or link beyond the header's: not
                                // what an npm package needs.
                                "size" | "linkpath" => return Err(format!("It has an entry Pitwall doesn't unpack (a pax “{k}”). Not installed.")),
                                _ => {}
                            }
                        }
                    }
                    // Global headers apply to no entry Pitwall writes.
                    _ => {}
                }
                continue;
            }
            _ => {}
        }
        let name = match (pax_path.take(), long_name.take()) {
            (Some(p), _) | (None, Some(p)) => p,
            (None, None) => {
                let base = text(&h[0..100])?;
                if &h[257..262] == b"ustar" {
                    let prefix = text(&h[345..500])?;
                    if prefix.is_empty() { base } else { format!("{prefix}/{base}") }
                } else {
                    base
                }
            }
        };
        let what = match kind {
            b'0' | 0 | b'7' => "file",
            b'5' => "folder",
            b'1' => return Err(format!("It has a hard link ({name}), which Pitwall doesn't unpack. Not installed.")),
            b'2' => return Err(format!("It has a symbolic link ({name}), which Pitwall doesn't unpack. Not installed.")),
            b'K' => return Err(format!("It has a link ({name}), which Pitwall doesn't unpack. Not installed.")),
            b'3' | b'4' => return Err(format!("It has a device ({name}), which Pitwall doesn't unpack. Not installed.")),
            b'6' => return Err(format!("It has a pipe ({name}), which Pitwall doesn't unpack. Not installed.")),
            other => return Err(format!("It has an entry of a kind Pitwall doesn't unpack ({name}, type {}). Not installed.", other as char)),
        };
        let parts = inner(&name)?;
        let skipped = parts.iter().any(|p| SKIPPED.contains(&p.as_str())) || (parts.len() == 1 && parts[0] == MARKER);
        if what == "folder" {
            if size != 0 {
                return Err("It isn't a package Pitwall can unpack (a folder with contents). Not installed.".into());
            }
            if !skipped && !parts.is_empty() {
                let dir = parts.iter().fold(dest.to_path_buf(), |p, c| p.join(c));
                std::fs::create_dir_all(&dir).map_err(|e| format!("Can't unpack {name}: {e}"))?;
            }
            continue;
        }
        if parts.is_empty() {
            return Err(format!("It has a file where its folder should be ({name}). Not installed."));
        }
        if skipped {
            copy_n(&mut r, size + pad(size), None)?;
            continue;
        }
        files += 1;
        bytes += size;
        if files > MAX_FILES {
            return Err(format!("It has more than {MAX_FILES} files. Not installed."));
        }
        if bytes > MAX_BYTES {
            return Err(format!("It is bigger than {} MiB unpacked. Not installed.", MAX_BYTES / 1024 / 1024));
        }
        let path = parts.iter().fold(dest.to_path_buf(), |p, c| p.join(c));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Can't unpack {name}: {e}"))?;
        }
        // A new file only: the same name twice is refused, and nothing
        // that is there already is opened.
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                format!("It has {name} twice. Not installed.")
            } else {
                format!("Can't unpack {name}: {e}")
            }
        })?;
        copy_n(&mut r, size, Some(&mut f))?;
        drop(f);
        copy_n(&mut r, pad(size), None)?;
        let mode = octal(&h[100..108]).unwrap_or(0);
        if mode & 0o111 != 0 {
            crate::platform::make_executable(&path).map_err(|e| format!("Can't unpack {name}: {e}"))?;
        }
    }
    Ok(())
}

/// The records of a pax header (`<len> <key>=<value>\n` each).
fn pax_records(data: &[u8]) -> Result<Vec<(String, String)>, String> {
    let bad = || "It isn't a package Pitwall can unpack (a damaged pax header). Not installed.".to_string();
    let mut out = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let space = rest.iter().position(|b| *b == b' ').ok_or_else(bad)?;
        let len: usize = std::str::from_utf8(&rest[..space]).ok().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
        if len <= space + 1 || len > rest.len() || rest[len - 1] != b'\n' {
            return Err(bad());
        }
        let record = std::str::from_utf8(&rest[space + 1..len - 1]).map_err(|_| bad())?;
        let (k, v) = record.split_once('=').ok_or_else(bad)?;
        out.push((k.to_string(), v.to_string()));
        rest = &rest[len..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
