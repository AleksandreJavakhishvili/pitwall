//! macOS privacy permissions (roadmap Wave 3): whether Pitwall has Full Disk
//! Access, and the System Settings panes that grant it.
//!
//! Agents and git work inside the user's projects, which often live in
//! Desktop, Documents or Downloads. macOS guards those folders (TCC) and asks
//! once per folder; Full Disk Access covers all of them with one switch.
//!
//! Checking never asks: the probe opens a file that only Full Disk Access
//! unlocks (see `platform::full_disk_access_probes`). Those locations have no
//! consent prompt — without the grant macOS just answers "Operation not
//! permitted". Desktop, Documents and Downloads themselves are never touched
//! here, because the first touch is exactly what makes macOS ask.

use std::io;

use serde::Serialize;

use crate::platform;

/// What Pitwall knows about one permission.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Granted,
    Denied,
    /// Not checkable without making macOS ask (or no such permission here).
    Unknown,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsStatus {
    /// This OS guards folders per app (macOS). Elsewhere everything is `Unknown`.
    pub applies: bool,
    pub full_disk_access: Access,
    /// Readable without a prompt: `Granted` with Full Disk Access, otherwise
    /// `Unknown` (macOS asks on first use; probing would trigger that).
    pub desktop: Access,
    pub documents: Access,
    pub downloads: Access,
}

/// One probe's outcome: the protected item could be opened, or why not.
pub type Probe = io::Result<()>;

/// Full Disk Access from the probes, in order: the first that opened means
/// granted, the first refused (EPERM/EACCES) means denied; missing items are
/// skipped. Nothing conclusive → `Unknown`.
pub fn classify(probes: impl IntoIterator<Item = Probe>) -> Access {
    for p in probes {
        match p {
            Ok(()) => return Access::Granted,
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Access::Denied,
            Err(_) => continue,
        }
    }
    Access::Unknown
}

/// The status for a Full Disk Access answer (folders follow from it).
pub fn status_from(applies: bool, fda: Access) -> PermissionsStatus {
    let folders = if fda == Access::Granted { Access::Granted } else { Access::Unknown };
    PermissionsStatus { applies, full_disk_access: fda, desktop: folders, documents: folders, downloads: folders }
}

/// Read-only and prompt-free (module docs). Blocking but fast (a few opens).
pub fn status() -> PermissionsStatus {
    let probes = platform::full_disk_access_probes();
    let applies = !probes.is_empty();
    status_from(applies, if applies { classify(probes) } else { Access::Unknown })
}

/// The System Settings pane for a permission: `fullDiskAccess` or
/// `filesAndFolders`. Anything else is refused (no arbitrary URLs).
pub fn settings_url(kind: &str) -> Result<&'static str, String> {
    match kind {
        "fullDiskAccess" => Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"),
        "filesAndFolders" => Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders"),
        _ => Err(format!("unknown privacy setting: {kind}")),
    }
}

/// Opens that pane. Only shows it; nothing is changed for the user.
pub fn open_settings(kind: &str) -> Result<(), String> {
    platform::open_system_url(settings_url(kind)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(kind: io::ErrorKind) -> Probe {
        Err(io::Error::from(kind))
    }

    #[test]
    fn first_opened_probe_means_granted() {
        assert_eq!(classify([Ok(())]), Access::Granted);
        assert_eq!(classify([err(io::ErrorKind::NotFound), Ok(())]), Access::Granted);
    }

    #[test]
    fn refused_probe_means_denied() {
        // EPERM (TCC) and EACCES both map to PermissionDenied.
        assert_eq!(classify([Err(io::Error::from_raw_os_error(1))]), Access::Denied);
        assert_eq!(classify([err(io::ErrorKind::NotFound), err(io::ErrorKind::PermissionDenied)]), Access::Denied);
    }

    #[test]
    fn nothing_conclusive_is_unknown() {
        assert_eq!(classify(Vec::<Probe>::new()), Access::Unknown);
        assert_eq!(classify([err(io::ErrorKind::NotFound), err(io::ErrorKind::Other)]), Access::Unknown);
    }

    #[test]
    fn folders_are_only_known_with_full_disk_access() {
        let s = status_from(true, Access::Granted);
        assert_eq!((s.desktop, s.documents, s.downloads), (Access::Granted, Access::Granted, Access::Granted));
        let s = status_from(true, Access::Denied);
        assert_eq!((s.desktop, s.documents, s.downloads), (Access::Unknown, Access::Unknown, Access::Unknown));
    }

    #[test]
    fn serializes_for_the_ui() {
        let v = serde_json::to_value(status_from(true, Access::Denied)).unwrap();
        assert_eq!(v["fullDiskAccess"], "denied");
        assert_eq!(v["desktop"], "unknown");
        assert_eq!(v["applies"], true);
    }

    #[test]
    fn only_known_panes_open() {
        assert!(settings_url("fullDiskAccess").unwrap().ends_with("Privacy_AllFiles"));
        assert!(settings_url("filesAndFolders").unwrap().ends_with("Privacy_FilesAndFolders"));
        assert!(settings_url("https://example.com").is_err());
    }

    #[test]
    fn checking_never_fails() {
        // Read-only, whatever this machine grants.
        let s = status();
        if !s.applies {
            assert_eq!(s.full_disk_access, Access::Unknown);
        }
    }
}
