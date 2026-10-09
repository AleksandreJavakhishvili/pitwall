//! Opening the Race Engineer (docs/spec/engineer.md): the top bar's headset
//! button and "Race Engineer" in ⌘K. There is one: a running one is shown,
//! a stopped one is started again (resuming its conversation), and a new one
//! is made when there is none, in the current space. It works in its own
//! folder (`pitwall_core::engineer`) and is shown under the selected
//! project (else home). A new one gets the first prompt queued
//! (`engineer.greeting`).

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{Context, Window};
use pitwall_core::engine::lifecycle;
use pitwall_core::engineer;
use pitwall_core::model::CreateAgentRequest;
use pitwall_proto::settings::find;
use pitwall_proto::AgentView;

use super::{workspace, MainScreen};

/// An open is under way (any window): more clicks meanwhile do nothing,
/// or each would make an engineer before the first one is listed.
static OPENING: AtomicBool = AtomicBool::new(false);

/// Ends the open however it ends (done, failed, its window closed).
struct Opening;

impl Drop for Opening {
    fn drop(&mut self) {
        OPENING.store(false, Ordering::SeqCst);
    }
}

/// What opening found to do.
enum Opened {
    /// Show this agent (it runs, or was started again).
    Show(String),
    /// A new engineer.
    Created(Box<AgentView>),
}

/// `engineer.*` as `ui.json` has them now.
struct Settings {
    agent: String,
    command: String,
    greeting: String,
}

fn text(state: &crate::ui_state::UiState, key: &str) -> String {
    find(key)
        .and_then(|def| crate::settings::data::value(state, def))
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl MainScreen {
    /// The project the engineer is shown under: the focused agent's, else
    /// the project space's, else home.
    fn engineer_project(&self, cx: &gpui::App) -> String {
        if let Some(a) = self.selected(cx).filter(|a| !a.engineer && !a.project.is_empty()) {
            return a.project;
        }
        self.active_space()
            .filter(|s| s.kind == workspace::SpaceKind::Project)
            .and_then(|s| s.project.clone())
            .unwrap_or_else(|| "~".into())
    }

    /// Open the Race Engineer, or show the one there is.
    pub fn open_engineer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            self.failed("open the Race Engineer", "Pitwall's engine isn't running".into(), cx);
            return;
        };
        let state = crate::ui_state::get(cx);
        let set = Settings {
            agent: text(&state, "engineer.agent"),
            command: text(&state, "engineer.command"),
            greeting: text(&state, "engineer.greeting"),
        };
        if OPENING.swap(true, Ordering::SeqCst) {
            return;
        }
        let opening = Opening;
        let project = self.engineer_project(cx);
        let (cols, rows) = self.new_agent_size(cx);
        let task = cx.background_executor().spawn(async move {
            let kinds = engine.list_kinds()?;
            let (kind, custom_command) = engineer::choose(&kinds, &set.agent, &set.command)?;
            // The engine's list, not the window's: it has one made a
            // moment ago that the window hasn't shown yet.
            if let Some(a) = engine.views().into_iter().find(|a| a.engineer) {
                if a.running {
                    return Ok(Opened::Show(a.id));
                }
                if a.kind == kind {
                    lifecycle::restart(&engine, &a.id, cols.zip(rows))?;
                    return Ok(Opened::Show(a.id));
                }
                // Set to run on something else since: a new one.
                lifecycle::remove(&engine, &a.id, false)?;
            }
            let view = lifecycle::create(
                &engine,
                CreateAgentRequest {
                    kind: kind.clone(),
                    custom_command,
                    display_project: Some(project),
                    cols,
                    rows,
                    engineer: true,
                    ..Default::default()
                },
            )?;
            let prompt = engineer::first_prompt(&kind, &set.greeting);
            let view = if prompt.is_empty() { view } else { engine.queue_add(&view.id, prompt).unwrap_or(view) };
            Ok::<_, String>(Opened::Created(Box::new(view)))
        });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            drop(opening);
            let _ = this.update_in(cx, |s, window, cx| match res {
                Ok(Opened::Show(id)) => s.show_agent(&id, window, cx),
                Ok(Opened::Created(view)) => s.created(*view, window, cx),
                Err(e) => s.failed("open the Race Engineer", e, cx),
            });
        })
        .detach();
    }

    /// Debug builds: `PITWALL_DEBUG_OPEN=engineer` opens the Race Engineer
    /// once the window is up, as the top bar button does (checks and
    /// screenshots without synthetic clicks).
    pub(super) fn debug_open_engineer(window: &mut Window, cx: &mut Context<Self>) {
        if !cfg!(debug_assertions) || std::env::var("PITWALL_DEBUG_OPEN").as_deref() != Ok("engineer") {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
            let _ = this.update_in(cx, |s, window, cx| s.open_engineer(window, cx));
        })
        .detach();
    }
}
