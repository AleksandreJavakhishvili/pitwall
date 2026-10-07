//! Tauri commands (see docs/CONTRACT.md): thin adapters from the webview to
//! the core's services. Anything that may block runs on a blocking worker so
//! the main thread stays responsive. Command names and argument names are
//! the UI contract; the logic lives in `pitwall_core`.

pub mod agents;
pub mod cli;
pub mod onboarding;
pub mod permissions;
pub mod review;
pub mod rules;
pub mod terminals;
pub mod windows;

pub type Res<T> = Result<T, String>;

pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("internal error: {e}"))?
}
