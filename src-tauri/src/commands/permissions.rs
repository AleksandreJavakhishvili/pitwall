//! macOS privacy permissions (Full Disk Access) for the welcome screen and
//! Settings → Permissions. The logic lives in `pitwall_core::permissions`.

use pitwall_core::permissions::{self, PermissionsStatus};

use super::{blocking, Res};

/// Read-only; never makes macOS ask (see `pitwall_core::permissions`).
#[tauri::command]
pub async fn permissions_status() -> Res<PermissionsStatus> {
    blocking(|| Ok(permissions::status())).await
}

/// Shows a System Settings pane: `kind` is "fullDiskAccess" or
/// "filesAndFolders". The user makes any change there themselves.
#[tauri::command]
pub async fn open_privacy_settings(kind: String) -> Res<()> {
    blocking(move || permissions::open_settings(&kind)).await
}
