//! An extension's listing media (docs/spec/tile-sdk/packaging.md §6.7):
//! its own icon, its screenshots and the images its README shows. All of
//! them are files in its folder, checked here the same way wherever they
//! are read (`pitwall extension check`, `pack`, a package opened for
//! install, the index's build, the app):
//!
//! - **Icon.** The manifest's `icon` may be a path in the package to a PNG
//!   or WebP (`assets/icon.png`) instead of one of Pitwall's icon names. Its
//!   type is told by its first bytes (never its name), it is at most
//!   [`MAX_ICON_BYTES`], square, and [`MIN_ICON_PX`] to [`MAX_ICON_PX`] wide.
//!   SVG isn't an icon file (it could hold anything a browser runs).
//! - **Screenshots.** PNG or WebP files in `screenshots/`, at most
//!   [`MAX_SCREENSHOTS`] shown, each at most [`MAX_SCREENSHOT_BYTES`]. A
//!   pair `<name>-dark.png` and `<name>-light.png` is one screenshot,
//!   shown as the user's theme is. The manifest's optional `screenshots`
//!   list gives their order and captions ([`ShotDef`]).
//! - **README images.** Relative paths inside the extension's folder only
//!   ([`local_ref`]): never an address, never `..`, never absolute. They
//!   must be in `assets/`, `media/` or `screenshots/`, PNG, WebP, JPEG or
//!   GIF by their first bytes, each at most [`MAX_README_IMAGE_BYTES`], at
//!   most [`MAX_README_IMAGES`]. An image from the network is never
//!   fetched: the README shows it as a plain link.
//!
//! There is no upload service: an author drops the files in the folder
//! and links them relatively.

use std::path::Path;

use super::package;

/// The largest icon file.
pub const MAX_ICON_BYTES: u64 = 256 << 10;
/// An icon file's smallest and largest side, in pixels.
pub const MIN_ICON_PX: u32 = 64;
pub const MAX_ICON_PX: u32 = 1024;
/// The most screenshots shown (a dark and light pair counts once), and the
/// largest file.
pub const MAX_SCREENSHOTS: usize = 8;
pub const MAX_SCREENSHOT_BYTES: u64 = 2 << 20;
/// The most images a README shows from its folder, and the largest.
pub const MAX_README_IMAGES: usize = 24;
pub const MAX_README_IMAGE_BYTES: u64 = 2 << 20;
/// The longest screenshot caption.
pub const MAX_CAPTION: usize = 120;
/// The folder screenshots are in.
pub const SCREENSHOTS: &str = "screenshots";
/// The folders a README's images may be in.
pub const IMAGE_FOLDERS: [&str; 3] = ["assets", "media", "screenshots"];

/// An image's type, told by its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Webp,
    Jpeg,
    Gif,
}

impl Kind {
    /// Its usual file extension.
    pub fn ext(self) -> &'static str {
        match self {
            Kind::Png => "png",
            Kind::Webp => "webp",
            Kind::Jpeg => "jpg",
            Kind::Gif => "gif",
        }
    }

    /// The kind a file name says (its extension), if an image's.
    pub fn of_name(name: &str) -> Option<Kind> {
        match name.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
            "png" => Some(Kind::Png),
            "webp" => Some(Kind::Webp),
            "jpg" | "jpeg" => Some(Kind::Jpeg),
            "gif" => Some(Kind::Gif),
            _ => None,
        }
    }
}

/// The type of image `bytes` by their first bytes; `None`: none of
/// [`Kind`] (an SVG, text, anything else).
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(Kind::Png)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(Kind::Webp)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Kind::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(Kind::Gif)
    } else {
        None
    }
}

/// The width and height of a PNG, WebP or GIF, read from its header.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let be = |i: usize| -> Option<u32> { Some(u32::from_be_bytes(bytes.get(i..i + 4)?.try_into().ok()?)) };
    let le16 = |i: usize| -> Option<u32> { Some(u16::from_le_bytes(bytes.get(i..i + 2)?.try_into().ok()?) as u32) };
    let le24 = |i: usize| -> Option<u32> {
        let b = bytes.get(i..i + 3)?;
        Some(b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16)
    };
    match sniff(bytes)? {
        // The IHDR chunk comes first.
        Kind::Png if bytes.get(12..16) == Some(b"IHDR") => Some((be(16)?, be(20)?)),
        Kind::Webp => match bytes.get(12..16)? {
            b"VP8 " => {
                // A key frame: start code 9d 01 2a, then 14-bit sizes.
                (bytes.get(23..26)? == [0x9d, 0x01, 0x2a]).then_some(())?;
                Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff))
            }
            b"VP8L" => {
                (bytes.get(20) == Some(&0x2f)).then_some(())?;
                let b = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
                Some(((b & 0x3fff) + 1, ((b >> 14) & 0x3fff) + 1))
            }
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        },
        Kind::Gif => Some((le16(6)?, le16(8)?)),
        _ => None,
    }
}

/// Whether manifest `icon` names a file in the package rather than one of
/// Pitwall's icons: it has a `/` or a file extension.
pub fn is_icon_file(icon: &str) -> bool {
    icon.contains('/') || icon.contains('.')
}

/// Why `path` can't be a manifest's icon file (by its name only: the file
/// is checked by [`check_icon`]).
pub fn check_icon_path(path: &str) -> Result<(), String> {
    package::check_name(path).map_err(|why| format!("\"icon\": {why}"))?;
    match Kind::of_name(path) {
        Some(Kind::Png | Kind::Webp) => Ok(()),
        _ if path.to_ascii_lowercase().ends_with(".svg") => Err(format!("\"icon\" “{path}” is an SVG: an icon file is a PNG or WebP (or name one of Pitwall's icons)")),
        _ => Err(format!("\"icon\" “{path}” must be a PNG or WebP file in the extension's folder, or one of Pitwall's icon names")),
    }
}

/// Why icon file `bytes` (`name`) can't be shown; its size in pixels when
/// it can.
pub fn check_icon(name: &str, bytes: &[u8]) -> Result<u32, String> {
    if bytes.len() as u64 > MAX_ICON_BYTES {
        return Err(format!("{name} is larger than {}: an icon is at most that", package::size_words(MAX_ICON_BYTES)));
    }
    match sniff(bytes) {
        Some(Kind::Png | Kind::Webp) => {}
        _ => return Err(format!("{name} isn't a PNG or WebP image (by its contents): an icon is one of those")),
    }
    let (w, h) = dimensions(bytes).ok_or_else(|| format!("{name}'s size can't be read"))?;
    if w != h {
        return Err(format!("{name} is {w}×{h}: an icon is square"));
    }
    if !(MIN_ICON_PX..=MAX_ICON_PX).contains(&w) {
        return Err(format!("{name} is {w}×{h}: an icon is {MIN_ICON_PX} to {MAX_ICON_PX} pixels wide (256 or 512 looks sharp everywhere)"));
    }
    Ok(w)
}

/// Why screenshot `bytes` (`name`) can't be shown.
pub fn check_screenshot(name: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_SCREENSHOT_BYTES {
        return Err(format!("{name} is larger than {}", package::size_words(MAX_SCREENSHOT_BYTES)));
    }
    match sniff(bytes) {
        Some(k @ (Kind::Png | Kind::Webp)) if Kind::of_name(name) == Some(k) => Ok(()),
        Some(Kind::Png | Kind::Webp) => Err(format!("{name}'s contents aren't what its name says")),
        _ => Err(format!("{name} isn't a PNG or WebP image (by its contents): screenshots are one of those")),
    }
}

/// Why README image `bytes` (`name`) can't be shown.
pub fn check_readme_image(name: &str, bytes: &[u8]) -> Result<Kind, String> {
    if bytes.len() as u64 > MAX_README_IMAGE_BYTES {
        return Err(format!("{name} is larger than {}", package::size_words(MAX_README_IMAGE_BYTES)));
    }
    match sniff(bytes) {
        Some(k) if Kind::of_name(name) == Some(k) => Ok(k),
        Some(_) => Err(format!("{name}'s contents aren't what its name says")),
        None => Err(format!("{name} isn't a PNG, WebP, JPEG or GIF image (by its contents)")),
    }
}

/// Where an image a README names is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    /// A file in the extension's folder: its package path
    /// (`media/editor.png`).
    Local(String),
    /// An address (`https://…`): never fetched, shown as a plain link.
    Remote(String),
    /// Neither, and why it isn't shown (`..`, an absolute path, `data:`).
    Refused(String),
}

/// Where image `src` (as the README writes it) is: a package path for a
/// relative one inside the folder (`./`, `%20` and a `?…`/`#…` suffix
/// dropped), an address, or refused.
pub fn local_ref(src: &str) -> Ref {
    let s = src.trim().trim_start_matches('<').trim_end_matches('>');
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ref::Remote(s.to_string());
    }
    if s.is_empty() {
        return Ref::Refused("it names no file".into());
    }
    if s.starts_with('/') || s.starts_with('\\') || s.contains("://") || lower.starts_with("data:") || lower.starts_with("file:") || (s.len() > 1 && s.as_bytes()[1] == b':') {
        return Ref::Refused(format!("“{s}” isn't a file in the extension's folder"));
    }
    let path = s.split(['?', '#']).next().unwrap_or(s).replace("%20", " ");
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => return Ref::Refused(format!("“{s}” goes out of the extension's folder (..)")),
            p => parts.push(p),
        }
    }
    let rel = parts.join("/");
    match package::check_name(&rel) {
        Ok(()) => Ref::Local(rel),
        Err(why) => Ref::Refused(why),
    }
}

/// The images Markdown `text` shows (`![alt](src)` and `<img src="…">`),
/// as written, in order, each once; code (fenced and inline) left out.
pub fn readme_refs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    };
    let mut fence: Option<&str> = None;
    for raw in text.lines() {
        let t = raw.trim_start();
        if let Some(f) = fence {
            if t.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some(&t[..3]);
            continue;
        }
        // Inline code spans dropped.
        let mut line = String::new();
        let mut in_code = false;
        for c in raw.chars() {
            if c == '`' {
                in_code = !in_code;
            } else if !in_code {
                line.push(c);
            }
        }
        let mut rest = line.as_str();
        while let Some(i) = rest.find("![") {
            rest = &rest[i + 2..];
            let Some(close) = rest.find("](") else { break };
            let after = &rest[close + 2..];
            let end = after.find(')').unwrap_or(after.len());
            let target = after[..end].split_whitespace().next().unwrap_or("");
            push(target);
            rest = &after[end.min(after.len())..];
        }
        let lower = line.to_ascii_lowercase();
        let mut from = 0;
        while let Some(i) = lower[from..].find("<img") {
            let tag_start = from + i;
            let tag_end = lower[tag_start..].find('>').map(|e| tag_start + e).unwrap_or(lower.len());
            let tag = &line[tag_start..tag_end];
            let tl = &lower[tag_start..tag_end];
            if let Some(s) = tl.find("src=") {
                let v = &tag[s + 4..];
                let q = v.chars().next().unwrap_or(' ');
                let val = if q == '"' || q == '\'' { v[1..].split(q).next().unwrap_or("") } else { v.split_whitespace().next().unwrap_or("") };
                push(val.trim_end_matches('/'));
            }
            from = tag_end.max(tag_start + 4);
        }
    }
    out
}

/// One screenshot a manifest lists: its file (or a dark and light pair's
/// shared name, `screenshots/editor.png` for `editor-dark.png` and
/// `editor-light.png`) and its caption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotDef {
    pub path: String,
    pub caption: Option<String>,
}

/// A screenshot as shown: its file for a light theme, its file for a dark
/// one (the same when it has one), and its caption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub light: String,
    pub dark: String,
    pub caption: Option<String>,
}

impl Shot {
    /// Its file for the theme (`dark` or light).
    pub fn pick(&self, dark: bool) -> &str {
        if dark {
            &self.dark
        } else {
            &self.light
        }
    }
}

/// A screenshot file's shared name and which theme it is for (`None`:
/// either): `screenshots/editor-dark.png` → (`screenshots/editor.png`,
/// dark).
fn pair_of(name: &str) -> (String, Option<bool>) {
    let Some((stem, ext)) = name.rsplit_once('.') else { return (name.to_string(), None) };
    for (suffix, dark) in [("-dark", true), ("-light", false)] {
        if let Some(base) = stem.strip_suffix(suffix) {
            return (format!("{base}.{ext}"), Some(dark));
        }
    }
    (name.to_string(), None)
}

/// Why screenshot files `names` are too many: more than
/// [`MAX_SCREENSHOTS`] once pairs are joined (and so at most twice that
/// many files).
pub fn check_shot_count(names: &[String]) -> Result<(), String> {
    let shots = names.iter().filter(|n| n.starts_with("screenshots/")).map(|s| pair_of(s).0).collect::<std::collections::BTreeSet<_>>().len();
    if shots > MAX_SCREENSHOTS {
        return Err(format!("It has more than {MAX_SCREENSHOTS} screenshots (a dark and light pair counts once)."));
    }
    Ok(())
}

/// The screenshots made of files `names` (package paths under
/// `screenshots/`, PNG or WebP, sorted) as manifest list `listed` orders
/// and captions them (else by name), pairs joined, at most
/// [`MAX_SCREENSHOTS`]. `Err`: a listed one that isn't there.
pub fn screenshots(names: &[String], listed: &[ShotDef]) -> Result<Vec<Shot>, String> {
    let mut files: Vec<&String> = names.iter().filter(|n| n.starts_with("screenshots/") && n.matches('/').count() == 1 && matches!(Kind::of_name(n), Some(Kind::Png | Kind::Webp))).collect();
    files.sort();
    // Shared name → (light, dark, plain).
    type Group = (String, Option<String>, Option<String>, Option<String>);
    let mut groups: Vec<Group> = Vec::new();
    for f in files {
        let (base, theme) = pair_of(f);
        let i = match groups.iter().position(|g| g.0 == base) {
            Some(i) => i,
            None => {
                groups.push((base, None, None, None));
                groups.len() - 1
            }
        };
        match theme {
            Some(false) => groups[i].1 = Some(f.clone()),
            Some(true) => groups[i].2 = Some(f.clone()),
            None => groups[i].3 = Some(f.clone()),
        }
    }
    let shot = |g: &(String, Option<String>, Option<String>, Option<String>), caption: Option<String>| {
        let light = g.1.clone().or(g.3.clone()).or(g.2.clone()).unwrap_or_default();
        let dark = g.2.clone().or(g.3.clone()).unwrap_or_else(|| light.clone());
        Shot { light, dark, caption }
    };
    let mut out = Vec::new();
    if listed.is_empty() {
        out.extend(groups.iter().map(|g| shot(g, None)));
    } else {
        for d in listed {
            let g = groups.iter().find(|g| g.0 == d.path || g.1.as_deref() == Some(&d.path) || g.2.as_deref() == Some(&d.path) || g.3.as_deref() == Some(&d.path)).ok_or_else(|| format!("its \"screenshots\" lists {}, which isn't in its folder", d.path))?;
            out.push(shot(g, d.caption.clone()));
        }
    }
    out.truncate(MAX_SCREENSHOTS);
    Ok(out)
}

/// The media files of the extension in folder `dir` with manifest
/// `m`, checked: its icon file, its screenshots and the images its
/// README shows from its folder, as package paths. Why not, in words (the
/// first problem). What a README links from the network isn't counted.
pub fn files(dir: &Path, m: &super::Manifest) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let read = |rel: &str, max: u64| -> Result<Vec<u8>, String> {
        let p = dir.join(rel);
        let meta = std::fs::symlink_metadata(&p).map_err(|_| format!("{rel} isn't in its folder"))?;
        if !meta.is_file() {
            return Err(format!("{rel} is a link or isn't a plain file"));
        }
        if meta.len() > max {
            return Err(format!("{rel} is larger than {}", package::size_words(max)));
        }
        std::fs::read(&p).map_err(|e| format!("{rel} can't be read: {e}"))
    };
    if let Some(icon) = &m.icon_file {
        check_icon(icon, &read(icon, MAX_ICON_BYTES)?)?;
        out.push(icon.clone());
    }
    let mut shots: Vec<String> = std::fs::read_dir(dir.join(SCREENSHOTS))
        .map(|r| r.flatten().map(|e| format!("{SCREENSHOTS}/{}", e.file_name().to_string_lossy())).filter(|n| !n.rsplit('/').next().unwrap_or("").starts_with('.')).collect())
        .unwrap_or_default();
    shots.sort();
    for s in &shots {
        if !matches!(Kind::of_name(s), Some(Kind::Png | Kind::Webp)) {
            return Err(format!("{s}: screenshots are PNG or WebP"));
        }
        check_screenshot(s, &read(s, MAX_SCREENSHOT_BYTES)?)?;
    }
    check_shot_count(&shots)?;
    screenshots(&shots, &m.screenshots)?;
    for s in shots {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    if let Ok(text) = std::fs::read_to_string(dir.join(package::README)) {
        let mut n = 0;
        for src in readme_refs(&text) {
            match local_ref(&src) {
                Ref::Local(rel) => {
                    if !IMAGE_FOLDERS.iter().any(|f| rel.starts_with(&format!("{f}/"))) {
                        return Err(format!("its README shows {rel}: put the images it shows in media/ (or assets/, screenshots/) and link them from there"));
                    }
                    check_readme_image(&rel, &read(&rel, MAX_README_IMAGE_BYTES)?)?;
                    n += 1;
                    if !out.contains(&rel) {
                        out.push(rel);
                    }
                }
                Ref::Remote(_) => {}
                Ref::Refused(why) => return Err(format!("its README shows an image it can't: {why}")),
            }
        }
        if n > MAX_README_IMAGES {
            return Err(format!("its README shows more than {MAX_README_IMAGES} images from its folder"));
        }
    }
    Ok(out)
}

/// The images in package files `files` checked by their contents: its
/// icon file ([`check_icon`]), screenshots ([`check_screenshot`]) and the
/// other images ([`check_readme_image`]).
pub fn check_files(files: &[(&str, &[u8])], m: &super::Manifest) -> Result<(), String> {
    for (name, bytes) in files {
        if Some(*name) == m.icon_file.as_deref() {
            check_icon(name, bytes)?;
        } else if name.starts_with("screenshots/") {
            check_screenshot(name, bytes)?;
        } else if name.starts_with("media/") || (name.starts_with("assets/") && Kind::of_name(name).is_some()) {
            check_readme_image(name, bytes)?;
        }
    }
    if let Some(icon) = &m.icon_file {
        if !files.iter().any(|(n, _)| n == icon) {
            return Err(format!("Its icon {icon} isn't in it."));
        }
    }
    Ok(())
}

/// A PNG's first bytes with its IHDR (width, height): enough for the
/// checks, which read no more (tests).
#[cfg(test)]
pub(crate) fn png_header(w: u32, h: u32) -> Vec<u8> {
    png(w, h)
}

#[cfg(test)]
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_are_told_by_their_bytes() {
        assert_eq!(sniff(&png(1, 1)), Some(Kind::Png));
        assert_eq!(sniff(b"GIF89a\x10\x00\x20\x00"), Some(Kind::Gif));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Kind::Jpeg));
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None);
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8X"), Some(Kind::Webp));
        assert_eq!(dimensions(&png(256, 128)), Some((256, 128)));
        assert_eq!(dimensions(b"GIF89a\x10\x00\x20\x00"), Some((16, 32)));
        // A WebP's extended header: 24-bit sizes minus one.
        let mut x = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        x.extend_from_slice(&[0xff, 0, 0, 0xff, 0, 0]);
        assert_eq!(dimensions(&x), Some((256, 256)));
    }

    #[test]
    fn an_icon_file_is_a_square_png_or_webp_of_a_fair_size() {
        assert_eq!(check_icon("icon.png", &png(256, 256)), Ok(256));
        assert!(check_icon("icon.png", &png(256, 200)).unwrap_err().contains("square"));
        assert!(check_icon("icon.png", &png(32, 32)).unwrap_err().contains("64 to 1024"));
        assert!(check_icon("icon.png", &png(2048, 2048)).unwrap_err().contains("64 to 1024"));
        assert!(check_icon("icon.png", b"<svg/>").unwrap_err().contains("isn't a PNG or WebP"));
        let mut big = png(256, 256);
        big.resize((MAX_ICON_BYTES + 1) as usize, 0);
        assert!(check_icon("icon.png", &big).unwrap_err().contains("larger than"));
        assert!(check_icon_path("assets/icon.png").is_ok());
        assert!(check_icon_path("assets/icon.svg").unwrap_err().contains("SVG"));
        assert!(check_icon_path("../icon.png").is_err());
        assert!(is_icon_file("assets/icon.png") && is_icon_file("icon.webp") && !is_icon_file("inbox"));
    }

    #[test]
    fn only_relative_images_inside_the_folder_are_local() {
        assert_eq!(local_ref("media/a.png"), Ref::Local("media/a.png".into()));
        assert_eq!(local_ref("./media/a.png?raw=1"), Ref::Local("media/a.png".into()));
        assert_eq!(local_ref("<media/a.png>"), Ref::Local("media/a.png".into()));
        assert_eq!(local_ref("https://example.com/a.png"), Ref::Remote("https://example.com/a.png".into()));
        assert!(matches!(local_ref("../secret.png"), Ref::Refused(w) if w.contains("..")));
        assert!(matches!(local_ref("media/../../x.png"), Ref::Refused(_)));
        for bad in ["/etc/a.png", "file:///a.png", "data:image/png;base64,AA", "C:/a.png", "//host/a.png", "ftp://x/a.png"] {
            assert!(matches!(local_ref(bad), Ref::Refused(_)), "{bad}");
        }
    }

    #[test]
    fn a_readme_names_its_images_outside_code() {
        let md = "# Hi\n\n![The editor](media/editor.png \"Editor\")\n\nSee `![not](media/code.png)`.\n\n```md\n![no](media/fenced.png)\n```\n\n<img src=\"media/b.webp\" width=\"300\"> and ![net](https://x.dev/a.png) and ![again](media/editor.png)\n";
        assert_eq!(readme_refs(md), ["media/editor.png", "https://x.dev/a.png", "media/b.webp"]);
    }

    #[test]
    fn pairs_are_one_screenshot_and_the_list_orders_and_captions() {
        let names: Vec<String> = ["screenshots/a.png", "screenshots/b-dark.png", "screenshots/b-light.png", "screenshots/c-dark.webp"].iter().map(|s| s.to_string()).collect();
        let all = screenshots(&names, &[]).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!((all[1].pick(true), all[1].pick(false)), ("screenshots/b-dark.png", "screenshots/b-light.png"));
        assert_eq!((all[0].pick(true), all[0].pick(false)), ("screenshots/a.png", "screenshots/a.png"));
        assert_eq!((all[2].pick(true), all[2].pick(false)), ("screenshots/c-dark.webp", "screenshots/c-dark.webp"));
        let listed = [ShotDef { path: "screenshots/b.png".into(), caption: Some("Both themes".into()) }, ShotDef { path: "screenshots/a.png".into(), caption: None }];
        let some = screenshots(&names, &listed).unwrap();
        assert_eq!(some.len(), 2);
        assert_eq!((some[0].light.as_str(), some[0].caption.as_deref()), ("screenshots/b-light.png", Some("Both themes")));
        assert!(screenshots(&names, &[ShotDef { path: "screenshots/zz.png".into(), caption: None }]).unwrap_err().contains("zz.png"));
    }
}
