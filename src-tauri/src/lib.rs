mod agents;
mod attention;
mod commands;
mod git;
mod hooks;
mod kinds;
mod launch;
mod lifecycle;
mod menu;
mod model;
mod onboarding;
mod paths;
mod persist;
mod project_list;
mod projects;
mod registry;
mod review;
mod rules;
mod scan;
mod session;
mod shell;
mod status;
mod tasks;
mod ticker;
mod windows;
mod worktree;

use tauri::{Manager, RunEvent, WindowEvent};

use registry::{Agent, Core, Shared};

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    kinds::warm_up();
    if let Err(e) = hooks::install_script() {
        eprintln!("pitwall: could not install hook script: {e}");
    }

    // Agents whose terminal holder survived the last run are running again,
    // without a restart; the rest are stopped.
    let agents = persist::load()
        .into_iter()
        .map(|rec| {
            let session = lifecycle::reattach(&rec);
            Agent::new(rec, session)
        })
        .collect();
    let core = Core::new(app.handle().clone(), agents);
    app.manage(core.clone());

    let hook_core = core.clone();
    if let Err(e) = hooks::serve(move |id, payload| {
        // First: an agent that made its own worktree reports where it works.
        worktree::on_hook(&hook_core, id, &payload);
        hook_core.apply_hook(id, hooks::map_hook(&payload));
        tasks::on_hook(&hook_core, id, &payload);
    }) {
        // Hooks are optional: screen/activity detection still works.
        eprintln!("pitwall: hooks disabled: {e}");
    }
    tasks::start_worker(core.clone());
    ticker::start(core);
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
            commands::list_kinds,
            commands::recent_projects,
            commands::list_agents,
            commands::create_agent,
            commands::attach_output,
            commands::detach_output,
            commands::write_input,
            commands::resize,
            commands::send_prompt,
            commands::queue_add,
            commands::queue_remove,
            commands::queue_send_now,
            commands::set_auto_send,
            commands::mark_seen,
            commands::get_changes,
            commands::get_file_diff,
            commands::stop_agent,
            commands::restart_agent,
            commands::remove_agent,
            commands::codex_hooks_status,
            commands::install_codex_hooks,
            commands::get_ui_state,
            commands::set_ui_state,
            commands::open_window,
            commands::focus_window,
            commands::list_windows,
            rules::rules_status,
            rules::set_rules_npx,
            rules::list_rule_library,
            rules::reveal_rule_library,
            rules::list_rule_sets,
            rules::save_rule_set,
            rules::delete_rule_set,
            rules::import_rules,
            rules::list_rule_sources,
            rules::pull_rule_source,
            rules::remove_rule_source,
            rules::set_project_rules,
            rules::list_project_rules,
            rules::get_project_rules,
            rules::apply_rules,
            rules::set_agent_rules,
            rules::agent_rules,
            review::list_tasks,
            review::get_task_changes,
            review::get_file_versions,
            review::discard_file,
            review::commit_agent,
            review::merge_agent,
            review::get_merge_status,
            onboarding::scan_environment,
            onboarding::get_onboarded,
            onboarding::list_projects,
            onboarding::add_project,
            onboarding::remove_project,
            onboarding::complete_onboarding,
            onboarding::continue_conversation,
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
                let _ = persist::save(core.records());
            }
        }
        _ => {}
    });
}
