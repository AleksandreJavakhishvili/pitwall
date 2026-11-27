//! The zip inside a `.pwtile` (docs/spec/tile-sdk/packaging.md §6.1),
//! written and read here rather than with a general zip library: only
//! what a package needs, and every rule checked before a byte is
//! inflated.
//!
//! Written: files only (no folder entries), sorted by name, each stored
//! or deflated (whichever is smaller), with a fixed date (1980-01-01) and
//! mode (0644), so the same files always give the same bytes.
//!
//! Read: stored or deflated entries only; no encryption, no zip64, one
//! disk, no comment past the end record; each entry's local header must
//! agree with the central directory, entries may not overlap, and every
//! size is checked against [`Limits`] before anything is inflated. Links
//! and other special files (by their Unix mode) are refused, and each
//! inflated entry must have exactly its declared size and CRC-32.

use std::io::{Read, Write};

/// The most a package may hold: bytes packed and unpacked, files, and
/// the largest single entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub packed: u64,
    pub unpacked: u64,
    pub files: usize,
    pub entry: u64,
}

/// One file read from a package: its name and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// One file named in a package's central directory, not yet inflated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub size: u64,
    method: u16,
    crc: u32,
    start: usize,
    packed: usize,
}

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
/// 1980-01-01, the earliest date a zip can hold.
const DATE: u16 = (1 << 5) | 1;
/// Made by: Unix (3), zip 2.0.
const MADE_BY: u16 = (3 << 8) | 20;
/// A plain file, rw-r--r--.
const MODE: u32 = 0o100_644;
/// Names are UTF-8 (general purpose flag bit 11).
const UTF8: u16 = 1 << 11;

/// The zip of `files` (names and bytes; names already checked), sorted by
/// name: the same files give the same bytes.
pub fn write(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut sorted: Vec<&(String, Vec<u8>)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, bytes) in sorted {
        let mut crc = flate2::Crc::new();
        crc.update(bytes);
        let crc = crc.sum();
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        let deflated = z.write_all(bytes).and_then(|()| z.finish()).ok().filter(|d| d.len() < bytes.len());
        let (method, data): (u16, &[u8]) = match &deflated {
            Some(d) => (8, d),
            None => (0, bytes),
        };
        let offset = out.len() as u32;
        let head = |buf: &mut Vec<u8>| {
            put16(buf, 20);
            put16(buf, UTF8);
            put16(buf, method);
            put16(buf, 0);
            put16(buf, DATE);
            put32(buf, crc);
            put32(buf, data.len() as u32);
            put32(buf, bytes.len() as u32);
            put16(buf, name.len() as u16);
            put16(buf, 0);
        };
        put32(&mut out, LOCAL);
        head(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        put32(&mut central, CENTRAL);
        put16(&mut central, MADE_BY);
        head(&mut central);
        put16(&mut central, 0); // comment
        put16(&mut central, 0); // disk
        put16(&mut central, 0); // internal attributes
        put32(&mut central, MODE << 16);
        put32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let (at, size, n) = (out.len() as u32, central.len() as u32, files.len() as u16);
    out.extend_from_slice(&central);
    put32(&mut out, END);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, n);
    put16(&mut out, n);
    put32(&mut out, size);
    put32(&mut out, at);
    put16(&mut out, 0);
    out
}

/// The files `zip` names, checked against `limits` and with each name
/// passed to `name_ok` (why not, in words), without inflating anything.
/// Folder entries (a name ending in `/`, empty) are left out.
pub fn list(zip: &[u8], limits: Limits, name_ok: &dyn Fn(&str) -> Result<(), String>) -> Result<Vec<Listed>, String> {
    let bad = |why: &str| format!("It isn't a package Pitwall can read: {why}.");
    if zip.len() as u64 > limits.packed {
        return Err(format!("It is larger than {} MiB.", limits.packed >> 20));
    }
    if zip.len() < 22 {
        return Err(bad("it is too short to be a zip"));
    }
    // The end record: the last 22 bytes (a package has no zip comment).
    let end = zip.len() - 22;
    if get32(zip, end) != Some(END) {
        return Err(bad("it doesn't end like a zip (or has a comment)"));
    }
    let field = |at: usize| get16(zip, end + at).unwrap_or(u16::MAX);
    let (disk, cd_disk, here, total) = (field(4), field(6), field(8), field(10));
    let cd_size = get32(zip, end + 12).unwrap_or(u32::MAX) as usize;
    let cd_at = get32(zip, end + 16).unwrap_or(u32::MAX) as usize;
    if disk != 0 || cd_disk != 0 || here != total {
        return Err(bad("it spans several disks"));
    }
    if here == u16::MAX || cd_size == u32::MAX as usize || cd_at == u32::MAX as usize {
        return Err(bad("it is a zip64 file"));
    }
    if total as usize > limits.files {
        return Err(format!("It holds more than {} files.", limits.files));
    }
    if cd_at.checked_add(cd_size) != Some(end) {
        return Err(bad("its directory isn't where its end record says"));
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let (mut at, mut unpacked) = (cd_at, 0u64);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for _ in 0..total {
        if get32(zip, at) != Some(CENTRAL) {
            return Err(bad("its directory is broken"));
        }
        let f16 = |o: usize| get16(zip, at + o).ok_or_else(|| bad("its directory is cut short"));
        let f32 = |o: usize| get32(zip, at + o).ok_or_else(|| bad("its directory is cut short"));
        let made_by = f16(4)?;
        let flags = f16(8)?;
        let method = f16(10)?;
        let crc = f32(16)?;
        let packed = f32(20)?;
        let size = f32(24)?;
        let (name_len, extra_len, comment_len) = (f16(28)? as usize, f16(30)? as usize, f16(32)? as usize);
        let disk_start = f16(34)?;
        let external = f32(38)?;
        let local = f32(42)? as usize;
        let name_bytes = zip.get(at + 46..at + 46 + name_len).ok_or_else(|| bad("its directory is cut short"))?;
        let name = std::str::from_utf8(name_bytes).map_err(|_| bad("a file name isn't UTF-8"))?.to_string();
        at += 46 + name_len + extra_len + comment_len;
        if at > end {
            return Err(bad("its directory is cut short"));
        }
        if flags & 0x0001 != 0 || flags & 0x0040 != 0 || flags & 0x2000 != 0 {
            return Err(format!("{name} is encrypted: a package holds plain files."));
        }
        if method != 0 && method != 8 {
            return Err(bad(&format!("{name} is compressed in a way other than deflate")));
        }
        if packed == u32::MAX || size == u32::MAX || local == u32::MAX as usize || disk_start != 0 {
            return Err(bad("it is a zip64 file"));
        }
        // The file's type from its Unix mode, when it was made on Unix
        // (or macOS); on DOS and Windows, the folder attribute.
        let host = made_by >> 8;
        let mode = external >> 16;
        let is_dir = match host {
            3 | 19 if mode != 0 => match mode & 0o170_000 {
                0o100_000 => false,
                0o040_000 => true,
                0o120_000 => return Err(format!("{name} is a link: a package holds plain files only.")),
                _ => return Err(format!("{name} isn't a plain file.")),
            },
            _ => external & 0x10 != 0,
        };
        if is_dir || name.ends_with('/') {
            if !(is_dir && name.ends_with('/')) || size != 0 {
                return Err(bad(&format!("{name} is both a folder and a file")));
            }
            name_ok(name.trim_end_matches('/'))?;
            continue;
        }
        name_ok(&name)?;
        if !seen.insert(name.to_lowercase()) {
            return Err(format!("Two files are named {name}, apart from upper and lower case."));
        }
        if size as u64 > limits.entry {
            return Err(format!("{name} is larger than {} MiB.", limits.entry >> 20));
        }
        unpacked += size as u64;
        if unpacked > limits.unpacked {
            return Err(format!("Unpacked, it holds more than {} MiB.", limits.unpacked >> 20));
        }
        if method == 0 && packed != size {
            return Err(bad(&format!("{name}'s sizes disagree")));
        }
        // Its local header: the same name and method, then its data.
        if get32(zip, local) != Some(LOCAL) {
            return Err(bad(&format!("{name}'s header isn't where the directory says")));
        }
        let l16 = |o: usize| get16(zip, local + o).ok_or_else(|| bad("a header is cut short"));
        let (l_method, l_name, l_extra) = (l16(8)?, l16(26)? as usize, l16(28)? as usize);
        if l_method != method || zip.get(local + 30..local + 30 + l_name) != Some(name_bytes) {
            return Err(bad(&format!("{name}'s header disagrees with the directory")));
        }
        let start = local + 30 + l_name + l_extra;
        let stop = start.checked_add(packed as usize).filter(|s| *s <= cd_at).ok_or_else(|| bad(&format!("{name} runs past its end")))?;
        ranges.push((local, stop));
        out.push(Listed { name, size: size as u64, method, crc, start, packed: packed as usize });
    }
    if at != end {
        return Err(bad("its directory has more than its end record counts"));
    }
    ranges.sort();
    if ranges.windows(2).any(|w| w[1].0 < w[0].1) {
        return Err(bad("two of its files overlap"));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The bytes of `listed`, inflated from `zip` (never more than its
/// declared size) and checked against its CRC-32.
pub fn read(zip: &[u8], listed: &Listed) -> Result<Vec<u8>, String> {
    let data = zip.get(listed.start..listed.start + listed.packed).ok_or_else(|| format!("{} runs past its end.", listed.name))?;
    let bytes = if listed.method == 0 {
        data.to_vec()
    } else {
        let mut out = Vec::with_capacity(listed.size as usize);
        flate2::read::DeflateDecoder::new(data).take(listed.size + 1).read_to_end(&mut out).map_err(|e| format!("{} can't be unpacked: {e}.", listed.name))?;
        out
    };
    if bytes.len() as u64 != listed.size {
        return Err(format!("{} isn't the size the package says.", listed.name));
    }
    let mut crc = flate2::Crc::new();
    crc.update(&bytes);
    if crc.sum() != listed.crc {
        return Err(format!("{} is damaged (its checksum doesn't match).", listed.name));
    }
    Ok(bytes)
}

/// Every file of `zip`, listed ([`list`]) then read ([`read`]).
pub fn read_all(zip: &[u8], limits: Limits, name_ok: &dyn Fn(&str) -> Result<(), String>) -> Result<Vec<Entry>, String> {
    list(zip, limits, name_ok)?.iter().map(|l| Ok(Entry { name: l.name.clone(), bytes: read(zip, l)? })).collect()
}

fn put16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn get16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn get32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits { packed: 1 << 20, unpacked: 1 << 20, files: 10, entry: 1 << 19 };

    fn any(_: &str) -> Result<(), String> {
        Ok(())
    }

    fn files() -> Vec<(String, Vec<u8>)> {
        vec![("b.txt".into(), b"bee".to_vec()), ("a/long.txt".into(), "x".repeat(4000).into_bytes())]
    }

    /// Written sorted and the same every time; read back exactly, the
    /// compressible one deflated and the short one stored.
    #[test]
    fn written_the_same_and_read_back() {
        let z = write(&files());
        assert_eq!(z, write(&files().into_iter().rev().collect::<Vec<_>>()), "order doesn't matter");
        let l = list(&z, LIMITS, &any).unwrap();
        assert_eq!(l.iter().map(|l| (l.name.as_str(), l.method)).collect::<Vec<_>>(), [("a/long.txt", 8), ("b.txt", 0)]);
        let all = read_all(&z, LIMITS, &any).unwrap();
        assert_eq!(all[0].bytes.len(), 4000);
        assert_eq!(all[1].bytes, b"bee");
    }

    /// Sizes over the limits, too many files and names the caller refuses
    /// stop it before anything is inflated.
    #[test]
    fn limits_and_names_are_checked_first() {
        let z = write(&files());
        let small = Limits { entry: 100, ..LIMITS };
        assert!(list(&z, small, &any).unwrap_err().contains("a/long.txt is larger"));
        assert!(list(&z, Limits { unpacked: 3000, ..LIMITS }, &any).unwrap_err().contains("Unpacked"));
        assert!(list(&z, Limits { files: 1, ..LIMITS }, &any).unwrap_err().contains("more than 1 files"));
        assert!(list(&z, Limits { packed: 10, ..LIMITS }, &any).unwrap_err().contains("larger than"));
        let no_a = |n: &str| if n.starts_with("a/") { Err(format!("{n}: no")) } else { Ok(()) };
        assert_eq!(list(&z, LIMITS, &no_a).unwrap_err(), "a/long.txt: no");
    }

    /// A zip bomb's entry that claims a small size but inflates to more is
    /// cut at the claim and refused; a flipped byte fails its checksum.
    #[test]
    fn a_lying_entry_is_refused() {
        let mut z = write(&files());
        // The central directory's uncompressed size of the first entry.
        let cd = u32::from_le_bytes(z[z.len() - 6..z.len() - 2].try_into().unwrap()) as usize;
        z[cd + 24..cd + 28].copy_from_slice(&100u32.to_le_bytes());
        let l = list(&z, LIMITS, &any).unwrap();
        assert!(read(&z, &l[0]).unwrap_err().contains("isn't the size"));
        let mut z = write(&[("b.txt".into(), b"bee".to_vec())]);
        z[30 + 5] ^= 1;
        let l = list(&z, LIMITS, &any).unwrap();
        assert!(read(&z, &l[0]).unwrap_err().contains("checksum"));
    }

    /// Links, duplicate names in another case, a comment, encryption and
    /// garbage are refused in words.
    #[test]
    fn special_entries_are_refused() {
        let z = write(&files());
        let cd = u32::from_le_bytes(z[z.len() - 6..z.len() - 2].try_into().unwrap()) as usize;
        let mut link = z.clone();
        link[cd + 38..cd + 42].copy_from_slice(&((0o120_777u32) << 16).to_le_bytes());
        assert!(list(&link, LIMITS, &any).unwrap_err().contains("is a link"));
        let mut enc = z.clone();
        enc[cd + 8] |= 1;
        assert!(list(&enc, LIMITS, &any).unwrap_err().contains("encrypted"));
        let twice = write(&[("A.txt".into(), b"1".to_vec()), ("a.txt".into(), b"2".to_vec())]);
        assert!(list(&twice, LIMITS, &any).unwrap_err().contains("Two files"));
        let mut commented = z.clone();
        commented.extend_from_slice(b"hi");
        assert!(list(&commented, LIMITS, &any).is_err());
        assert!(list(b"not a zip at all, just text", LIMITS, &any).is_err());
        assert!(list(&[], LIMITS, &any).is_err());
    }
}
