//! The command-line tool: approvals it raised (answered here, by the user
//! in Pitwall's dialog) and installing the `pitwall` link.

use std::sync::Arc;

use tauri::State;

use pitwall_daemon::Approvals;
use pitwall_proto::{ApprovalAnswer, ApprovalView};

use super::Res;
use crate::cli_install::{self, CliStatus};

#[tauri::command]
pub fn list_approvals(approvals: State<'_, Arc<Approvals>>) -> Vec<ApprovalView> {
    approvals.pending()
}

/// The user's answer in the approval dialog. Only this app's own UI calls
/// it; socket clients can't (they aren't Pitwall's window).
#[tauri::command]
pub fn answer_approval(approvals: State<'_, Arc<Approvals>>, id: String, allow: bool, remember: bool) -> Res<()> {
    approvals.answer(&ApprovalAnswer { id, allow, remember })
}

#[tauri::command]
pub fn cli_status() -> CliStatus {
    let bin = cli_install::cli_bin().ok();
    let dirs = cli_install::candidate_dirs(&pitwall_core::paths::home());
    cli_install::status(bin.as_deref(), &dirs, &pitwall_core::shell::spawn_path().or_else(|| std::env::var("PATH").ok()).unwrap_or_default())
}

/// Create the `pitwall` link in `dir` (one of `cli_status().dirs`), after
/// the user agreed in Settings.
#[tauri::command]
pub fn install_cli(dir: String) -> Res<CliStatus> {
    let dirs = cli_install::candidate_dirs(&pitwall_core::paths::home());
    if !dirs.iter().any(|d| d.to_string_lossy() == dir) {
        return Err(format!("{dir} is not a place Pitwall installs the command-line tool"));
    }
    let bin = cli_install::cli_bin()?;
    cli_install::install(&bin, std::path::Path::new(&dir))?;
    Ok(cli_status())
}
