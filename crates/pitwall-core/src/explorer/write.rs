//! Saving a file the user edited in the viewer: on this computer only, a
//! text file already there (nothing is created), inside the folder with
//! every symlink resolved, never over a file that changed since it was
//! read unless the user said so, and atomically (a temporary file in the
//! same folder, the file's permissions kept, renamed over it).

use std::fs;
use std::io::Write as _;
use std::path::Path;

use super::read::{self, MAX_TEXT};
use super::Res;

/// What a save did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    Written,
    /// The file on disk is no longer what was read (`base`): nothing was
    /// written.
    ChangedOnDisk,
}

/// Why `rel` can't be edited, by its name alone (images, binary types;
/// an SVG's source can be).
pub fn refused_by_name(rel: &str) -> Option<&'static str> {
    if read::is_image(rel) && !read::is_svg(rel) {
        return Some("images can't be edited here");
    }
    if read::binary_by_name(rel) {
        return Some("binary files can't be edited here");
    }
    None
}

/// Write `text` to `real` (resolved, inside the folder, checked by the
/// caller). `base`: the file as it was read; when the file now differs,
/// nothing is written ([`Saved::ChangedOnDisk`]). `None`: write anyway.
pub(super) fn write(rel: &str, real: &str, text: &str, base: Option<&str>) -> Res<Saved> {
    if let Some(why) = refused_by_name(rel) {
        return Err(format!("{rel}: {why}"));
    }
    if text.len() as u64 > MAX_TEXT {
        return Err(format!("{rel}: too large to save from the viewer (over {} MiB)", MAX_TEXT / (1024 * 1024)));
    }
    let target = Path::new(real);
    let meta = fs::metadata(target).map_err(|e| format!("{rel}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{rel}: not a file"));
    }
    if meta.len() > MAX_TEXT {
        return Err(format!("{rel}: too large to edit here"));
    }
    let now = fs::read(target).map_err(|e| format!("{rel}: {e}"))?;
    if read::looks_binary(&now) {
        return Err(format!("{rel}: binary files can't be edited here"));
    }
    if std::str::from_utf8(&now).is_err() {
        return Err(format!("{rel}: not UTF-8 text; it can't be edited here"));
    }
    if base.is_some_and(|b| b.as_bytes() != now.as_slice()) {
        return Ok(Saved::ChangedOnDisk);
    }
    let dir = target.parent().ok_or_else(|| format!("{rel}: no folder"))?;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = dir.join(format!(".{name}.pitwall-{}-{stamp}.tmp", std::process::id()));
    let done = (|| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.set_permissions(meta.permissions())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, target)
    })();
    if let Err(e) = done {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{rel}: couldn't be saved: {e}"));
    }
    Ok(Saved::Written)
}
