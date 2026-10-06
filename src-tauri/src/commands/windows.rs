//! Windows and the UI-state blob shared by all windows.

use tauri::{AppHandle, WebviewWindow};

use super::Res;
use crate::windows;

#[tauri::command]
pub fn get_ui_state() -> serde_json::Value {
    windows::get_ui_state()
}

#[tauri::command]
pub fn set_ui_state(app: AppHandle, window: WebviewWindow, state: serde_json::Value) -> Res<()> {
    windows::set_ui_state(&app, state, window.label())
}

#[tauri::command]
pub async fn open_window(app: AppHandle, space_id: String) -> Res<String> {
    windows::open_window(&app, &space_id)
}

#[tauri::command]
pub fn focus_window(app: AppHandle, label: String) -> Res<()> {
    windows::focus_window(&app, &label)
}

#[tauri::command]
pub fn list_windows(app: AppHandle) -> Vec<String> {
    windows::list_windows(&app)
}
