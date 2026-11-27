//! The OS-specific bits of the tile host (runtime.md §2.2: only in
//! `platform/`): folders and files only this user may read, for compiled
//! code and tiles' data (runtime.md §2.6).

use std::path::Path;

/// Whether a compiled file from the cache can run where it is mapped.
/// Not on macOS: under the hardened runtime (how the app is signed) a
/// file's pages can't be made executable ("Permission denied"), so its
/// bytes are read and loaded into memory of the process's own, which
/// the app's `allow-unsigned-executable-memory` entitlement lets run.
pub const RUN_MAPPED_FILES: bool = !cfg!(target_os = "macos");

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

/// Move `path` (a file, a folder or a link, absolute) to the system's
/// Trash, where the user can put it back (`files.delete`): Finder's on
/// macOS, the Recycle Bin on Windows, the freedesktop.org home Trash
/// elsewhere. Nothing is deleted when it can't be moved there.
pub fn trash(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSFileManager, NSString, NSURL};
        let s = path.to_str().ok_or_else(|| std::io::Error::other("the path isn't valid UTF-8"))?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(s));
        NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error(&url, None).map_err(|e| std::io::Error::other(e.localizedDescription().to_string()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE, SHFILEOPSTRUCTW};
        // A list of paths, each ended by a NUL, the list by another.
        let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
        from.push(0);
        from.push(0);
        let mut op = SHFILEOPSTRUCTW { hwnd: std::ptr::null_mut(), wFunc: FO_DELETE, pFrom: from.as_ptr(), pTo: std::ptr::null(), fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16, fAnyOperationsAborted: 0, hNameMappings: std::ptr::null_mut(), lpszProgressTitle: std::ptr::null() };
        // SAFETY: `op` and the NUL-ended list it points to live across the call.
        let r = unsafe { SHFileOperationW(&mut op) };
        if r != 0 || op.fAnyOperationsAborted != 0 {
            return Err(std::io::Error::other(format!("the Recycle Bin refused it (error {r})")));
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        freedesktop_trash(path)
    }
}

/// The freedesktop.org Trash in the user's home (`$XDG_DATA_HOME/Trash`,
/// else `~/.local/share/Trash`): the item moved into `files/`, with its
/// `.trashinfo` (where it was, when) in `info/`. Only on the same disk:
/// across disks it is refused rather than copied.
#[cfg(not(any(target_os = "macos", windows)))]
fn freedesktop_trash(path: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let base = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => std::path::PathBuf::from(d),
        None => std::path::PathBuf::from(std::env::var_os("HOME").ok_or_else(|| std::io::Error::other("no home folder"))?).join(".local").join("share"),
    };
    let trash = base.join("Trash");
    let (files, info) = (trash.join("files"), trash.join("info"));
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("item").to_string();
    let when = {
        let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
        let (days, secs) = (s.div_euclid(86_400), s.rem_euclid(86_400));
        // Days since 1970 to a civil date (Howard Hinnant's algorithm).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
    };
    for n in 0..1000 {
        let stem = if n == 0 { name.clone() } else { format!("{name}.{n}") };
        let info_path = info.join(format!("{stem}.trashinfo"));
        // Claim the name with the info file first, as the spec says.
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(&info_path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let encoded: String = path.to_string_lossy().bytes().map(|b| if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
        f.write_all(format!("[Trash Info]\nPath={encoded}\nDeletionDate={when}\n").as_bytes())?;
        drop(f);
        return std::fs::rename(path, files.join(&stem)).inspect_err(|_| {
            let _ = std::fs::remove_file(&info_path);
        });
    }
    Err(std::io::Error::other("the Trash has too many items of that name"))
}
