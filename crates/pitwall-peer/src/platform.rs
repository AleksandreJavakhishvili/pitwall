//! Files only their owner may read (the instance key), per OS. Ported from
//! the parked collaboration branch's core `platform`.

use std::io;
use std::path::Path;

/// Write a secret only its owner may read (0600): a fresh file renamed over
/// `path`, so a reader never sees half of it and the mode is never wider.
#[cfg(unix)]
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// Make an existing secret readable by its owner only again (a key file
/// copied with wider permissions).
#[cfg(unix)]
pub fn make_private(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Whether only the owner may read and write `path` (tests).
#[cfg(all(test, unix))]
pub fn is_private(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o077 == 0)
}

/// The data folder is under the user's own `%APPDATA%`, which only they
/// (and admins) can read; a fresh file is renamed over `path` so a reader
/// never sees half.
#[cfg(windows)]
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Nothing to tighten beyond the per-user data folder.
#[cfg(windows)]
pub fn make_private(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(all(test, windows))]
pub fn is_private(path: &Path) -> bool {
    path.is_file()
}
