//! The Pitwall app: windows, the webview bridge, notifications, the badge
//! (Dock, or taskbar overlay + tray), the tray and the menu, each chosen by
//! `HostInfo` capabilities. All logic lives in `pitwall_core`; this crate
//! only hosts the engine and adapts it to Tauri (architecture.md §1).

mod attention;
mod bench;
mod cli_install;
mod commands;
mod events;
mod holder;
mod menu;
mod platform;
mod server;
mod windows;

use std::sync::Arc;

use tauri::{Manager, RunEvent, WindowEvent};

use pitwall_core::clock::SystemClock;
use pitwall_core::paths::Paths;
use pitwall_core::store::FileStore;
use pitwall_core::{Deps, Engine, Shared};
use pitwall_providers::agw::{AgwConfig, AgwProvider};
use pitwall_providers::local::{LocalConfig, LocalProvider};

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    bench::prepare(app);
    let paths = Paths::new(Paths::default_root());
    let local = LocalProvider::new(LocalConfig {
        hold_dir: paths.hold_dir(),
        holder_bin: holder::holder_bin(),
        hook_socket: paths.hook_socket(),
        cli_socket: paths.cli_socket(),
    });
    // agw sessions (adopted, architecture.md §2.7); agw is looked up on
    // first use, and without it the provider offers nothing.
    let agw = AgwProvider::new(AgwConfig::new(paths.hold_dir(), holder::holder_bin()));
    let engine = Engine::open(Deps {
        store: Arc::new(FileStore::new(paths.state_file())),
        paths: paths.clone(),
        events: Arc::new(events::AppEvents::new(app.handle().clone())),
        clock: Arc::new(SystemClock::new()),
        providers: vec![Arc::new(local), Arc::new(agw)],
    });
    app.manage(engine.clone());
    app.manage(Arc::new(pitwall_core::onboarding::elsewhere::Elsewhere::new()));
    engine.start();
    // The `pitwall` CLI's socket (architecture.md §4).
    server::start(app.handle(), engine.clone(), &paths);
    windows::restore(app.handle());
    if pitwall_core::host::HostInfo::current(&paths).tray {
        platform::setup_tray(app.handle())?;
    }
    bench::start(app.handle(), started());
    Ok(())
}

/// The Dock's "reopen" (macOS) and the tray icon's click (Windows).
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
pub(crate) fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// When `run` began (bench: cold start to first window).
fn started() -> std::time::Instant {
    *START.get_or_init(std::time::Instant::now)
}
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    started();
    platform::prepare_env();
    let host = pitwall_core::host::HostInfo::current(&Paths::new(Paths::default_root()));
    let hides_on_close = host.hides_on_close();
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        // Native menu items and the tray's menu.
        .on_menu_event(menu::on_event);
    if let Some(build) = menu::for_host(&host) {
        builder = builder.menu(build);
    }
    let app = builder
        .setup(setup)
        .on_window_event(move |window, event| match event {
            // Closing main hides it (agents keep running; the Dock or the
            // tray brings it back); Quit quits. Without either nothing could
            // bring it back, so closing main quits — agents still keep
            // running in their holders. Secondary windows close for real.
            WindowEvent::CloseRequested { api, .. } => {
                if window.label() == windows::MAIN && hides_on_close {
                    api.prevent_close();
                    let _ = window.hide();
                } else if window.label() == windows::MAIN {
                    window.app_handle().exit(0);
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
            commands::agents::list_machines,
            commands::agents::create_form,
            commands::agents::attach_output,
            commands::agents::detach_output,
            commands::agents::watch_screen,
            commands::agents::unwatch_screen,
            commands::agents::write_input,
            commands::agents::resize,
            commands::agents::send_prompt,
            commands::agents::queue_add,
            commands::agents::queue_remove,
            commands::agents::queue_send_now,
            commands::agents::set_auto_send,
            commands::agents::mark_seen,
            commands::agents::get_changes,
            commands::agents::refresh_changes,
            commands::agents::get_file_diff,
            commands::agents::stop_agent,
            commands::agents::restart_agent,
            commands::agents::remove_agent,
            commands::agents::codex_hooks_status,
            commands::agents::install_codex_hooks,
            commands::cli::list_approvals,
            commands::cli::answer_approval,
            commands::cli::cli_status,
            commands::cli::install_cli,
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
            commands::explorer::list_files,
            commands::explorer::list_all_files,
            commands::explorer::read_file,
            commands::explorer::search_files,
            commands::explorer::cancel_search,
            commands::worktrees::list_worktrees,
            commands::worktrees::get_worktree_changes,
            commands::worktrees::refresh_worktrees,
            commands::worktrees::get_worktree_file_versions,
            commands::worktrees::get_worktree_merge_status,
            commands::worktrees::commit_worktree,
            commands::worktrees::merge_worktree,
            commands::worktrees::remove_worktree,
            commands::onboarding::scan_environment,
            commands::onboarding::get_onboarded,
            commands::onboarding::list_projects,
            commands::onboarding::add_project,
            commands::onboarding::remove_project,
            commands::onboarding::complete_onboarding,
            commands::onboarding::continue_conversation,
            commands::onboarding::adopt_session,
            commands::terminals::list_elsewhere,
            commands::permissions::permissions_status,
            commands::permissions::open_privacy_settings,
            commands::host::host_info,
            commands::host::set_window_glass,
            commands::host::quit_app,
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
            if let Some(server) = app.try_state::<server::Running>() {
                server.stop();
            }
        }
        _ => {}
    });
}
