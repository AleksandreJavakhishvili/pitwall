//! The Pitwall app: windows, the webview bridge, notifications, the Dock
//! badge and the menu. All logic lives in `pitwall_core`; this crate only
//! hosts the engine and adapts it to Tauri (architecture.md §1).

mod attention;
mod commands;
mod events;
mod holder;
mod menu;
mod windows;

use std::sync::Arc;

use tauri::{Manager, RunEvent, WindowEvent};

use pitwall_core::clock::SystemClock;
use pitwall_core::paths::Paths;
use pitwall_core::store::FileStore;
use pitwall_core::{Deps, Engine, Shared};

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let paths = Paths::new(Paths::default_root());
    let engine = Engine::open(Deps {
        store: Arc::new(FileStore::new(paths.state_file())),
        paths,
        events: Arc::new(events::AppEvents::new(app.handle().clone())),
        clock: Arc::new(SystemClock::new()),
        holder_bin: holder::holder_bin(),
    });
    app.manage(engine.clone());
    engine.start();
    windows::restore(app.handle());
    Ok(())
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .menu(menu::build)
        .on_menu_event(menu::on_event)
        .setup(setup)
        .on_window_event(|window, event| match event {
            // Closing main hides it (agents keep running); ⌘Q quits.
            // Secondary windows close for real.
            WindowEvent::CloseRequested { api, .. } => {
                if window.label() == windows::MAIN {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    windows::on_closed(window.app_handle(), window.label());
                }
            }
            WindowEvent::Moved(pos) => windows::on_bounds(window.label(), Some(*pos), None),
            WindowEvent::Resized(size) => windows::on_bounds(window.label(), None, Some(*size)),
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::agents::list_kinds,
            commands::agents::recent_projects,
            commands::agents::list_agents,
            commands::agents::create_agent,
            commands::agents::attach_output,
            commands::agents::detach_output,
            commands::agents::write_input,
            commands::agents::resize,
            commands::agents::send_prompt,
            commands::agents::queue_add,
            commands::agents::queue_remove,
            commands::agents::queue_send_now,
            commands::agents::set_auto_send,
            commands::agents::mark_seen,
            commands::agents::get_changes,
            commands::agents::get_file_diff,
            commands::agents::stop_agent,
            commands::agents::restart_agent,
            commands::agents::remove_agent,
            commands::agents::codex_hooks_status,
            commands::agents::install_codex_hooks,
            commands::windows::get_ui_state,
            commands::windows::set_ui_state,
            commands::windows::open_window,
            commands::windows::focus_window,
            commands::windows::list_windows,
            commands::rules::rules_status,
            commands::rules::set_rules_npx,
            commands::rules::list_rule_library,
            commands::rules::reveal_rule_library,
            commands::rules::list_rule_sets,
            commands::rules::save_rule_set,
            commands::rules::delete_rule_set,
            commands::rules::import_rules,
            commands::rules::list_rule_sources,
            commands::rules::pull_rule_source,
            commands::rules::remove_rule_source,
            commands::rules::set_project_rules,
            commands::rules::list_project_rules,
            commands::rules::get_project_rules,
            commands::rules::apply_rules,
            commands::rules::set_agent_rules,
            commands::rules::agent_rules,
            commands::review::list_tasks,
            commands::review::get_task_changes,
            commands::review::get_file_versions,
            commands::review::discard_file,
            commands::review::commit_agent,
            commands::review::merge_agent,
            commands::review::get_merge_status,
            commands::onboarding::scan_environment,
            commands::onboarding::get_onboarded,
            commands::onboarding::list_projects,
            commands::onboarding::add_project,
            commands::onboarding::remove_project,
            commands::onboarding::complete_onboarding,
            commands::onboarding::continue_conversation,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| match event {
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_main(app),
        RunEvent::ExitRequested { .. } => windows::quitting(),
        RunEvent::Exit => {
            windows::quitting();
            // Agents keep running in their holders; only "Quit and Stop
            // Agents" (menu.rs) ends them.
            if let Some(core) = app.try_state::<Shared>() {
                let _ = core.save();
            }
        }
        _ => {}
    });
}
