//! The `pitwall` CLI's socket, served inside the app (architecture.md §6
//! step 5; `pitwalld` takes it over in step 6), and the approvals it raises:
//! shown in every window as Pitwall's approval dialog (`approvals-changed`),
//! answered by `answer_approval` — this process's own UI, never a socket
//! client.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, UserAttentionType};

use pitwall_core::paths::Paths;
use pitwall_core::Shared;
use pitwall_daemon::{Approvals, Config, Handle, ProcessIdentity};

use crate::windows;

/// How long a request waits for the user before it is denied.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
/// Webview event: the pending approvals (`ApprovalView[]`).
pub const APPROVALS_CHANGED: &str = "approvals-changed";

/// The running server, stopped (socket removed) when the app exits.
#[derive(Default)]
pub struct Running(Mutex<Option<Handle>>);

impl Running {
    pub fn stop(&self) {
        if let Some(h) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            h.stop();
        }
    }
}

/// Start serving. Without the socket Pitwall still works; only the CLI
/// can't reach it.
pub fn start(app: &AppHandle, engine: Shared, paths: &Paths) {
    let approvals = Approvals::new(APPROVAL_TIMEOUT);
    let handle = app.clone();
    let was_empty = Arc::new(Mutex::new(true));
    approvals.on_change(move |list| {
        let _ = handle.emit(APPROVALS_CHANGED, &list);
        // A new request: make sure the user notices (Dock bounce).
        let mut empty = was_empty.lock().unwrap_or_else(|e| e.into_inner());
        if *empty && !list.is_empty() {
            if let Some(w) = handle.get_webview_window(windows::MAIN) {
                let _ = w.request_user_attention(Some(UserAttentionType::Critical));
            }
        }
        *empty = list.is_empty();
    });
    app.manage(approvals.clone());
    let identify = ProcessIdentity::new(engine.clone());
    let cfg = Config { socket: paths.cli_socket(), version: env!("CARGO_PKG_VERSION").into() };
    let running = Running::default();
    match pitwall_daemon::serve(engine, approvals, identify, cfg) {
        Ok(h) => *running.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(h),
        Err(e) => eprintln!("pitwall: the command-line socket is off: {e}"),
    }
    app.manage(running);
}
