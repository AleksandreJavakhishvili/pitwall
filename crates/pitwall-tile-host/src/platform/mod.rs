//! The OS-specific bits of the tile host (runtime.md §2.2: only in
//! `platform/`): folders and files only this user may read, for compiled
//! code and tiles' data (runtime.md §2.6).

use std::path::Path;

/// Create `dir` (and its parents) and limit it to this user: mode 700 on
/// Unix. On Windows a folder under the user's profile already inherits an
/// ACL limited to the user and administrators; a narrower one is README
/// §10.3 item 18's open point.
pub fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Write `bytes` to `path` through a temporary file in the same folder and
/// a rename, so a reader never sees half a file; mode 600 on Unix.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent folder"))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut f = options.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
