//! The screen's commands as public calls, for the command palette
//! (`crate::palette`) and other modules: what the React `CommandPalette`
//! gets as its `commands` prop in `App.tsx`.

use gpui::{App, Context, Window};

use pitwall_proto::AgentView;

use crate::agents::group_by_project;
use crate::theme::Density;

use super::model::with_projects;
use super::strip::Toast;
use super::tree::{self, Preset};
use super::{workspace, MainScreen, Route};

/// What the palette lists, read from the screen when it opens or changes.
#[derive(Debug, Clone, Default)]
pub struct CommandState {
    /// Agents in sidebar order.
    pub agents: Vec<AgentView>,
    /// The focused pane's agent.
    pub selected: Option<String>,
    /// The sidebar's projects as (path, display): "Terminal in <project>".
    pub projects: Vec<(String, String)>,
    /// Layout presets that fit the active space.
    pub presets: Vec<Preset>,
}

impl MainScreen {
    pub fn command_state(&self, cx: &App) -> CommandState {
        let agents = self.ordered(cx);
        let projects = with_projects(group_by_project(self.agents(cx)), &self.projects)
            .into_iter()
            .map(|g| (g.project, g.display))
            .collect();
        let presets = match self.active_space() {
            Some(sp) => {
                let min = workspace::fold_min(self.ui.density_of(sp), self.ui.font_size);
                let area = self
                    .area
                    .map(|a| (f32::from(a.width) as f64, f32::from(a.height) as f64));
                tree::available_presets(area, min)
            }
            None => Preset::ALL.to_vec(),
        };
        CommandState {
            agents,
            selected: self.focused_agent_id(),
            projects,
            presets,
        }
    }

    /// ⌘N
    pub fn cmd_new_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_new_agent(None, window, cx);
    }

    /// ⌘T (`None`: where you are) or a terminal in `path`.
    pub fn cmd_new_terminal(
        &mut self,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = path.unwrap_or_else(|| self.here(cx));
        self.open_terminal(path, window, cx);
    }

    /// ⌘⇧T
    pub fn cmd_new_terminal_at(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_terminal_dialog(window, cx);
    }

    /// ⌘J
    pub fn cmd_next_blocked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.next_blocked(window, cx);
    }

    /// "Remove <name>…": the Remove dialog.
    pub fn cmd_remove(&mut self, id: &str, cx: &mut Context<Self>) {
        self.open_remove(id, cx);
    }

    /// ⌘E
    pub fn cmd_toggle_wall(&mut self, cx: &mut Context<Self>) {
        self.toggle_route(Route::Wall, cx);
    }

    /// ⌘R
    pub fn cmd_toggle_review(&mut self, cx: &mut Context<Self>) {
        self.toggle_route(Route::Review, cx);
    }

    /// ⌘B
    pub fn cmd_toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_sidebar(window, cx);
    }

    /// ⌘.
    pub fn cmd_toggle_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_details(window, cx);
    }

    /// ⌘⇧N: the active space to a new window.
    pub fn cmd_move_space_to_window(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.active_space().map(|s| s.id.clone()) {
            self.move_space(id, cx);
        }
    }

    /// "Tile layout: <preset>" on the active space.
    pub fn cmd_preset(&mut self, preset: Preset, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_preset(preset, window, cx);
    }

    /// The default density (`space: false`), or the active space's own
    /// (`None` there: follow the default).
    pub fn cmd_density(&mut self, density: Option<Density>, space: bool, cx: &mut Context<Self>) {
        if space {
            self.with_active(|s, id| s.set_density(density, Some(id)), cx);
        } else if let Some(d) = density {
            self.update_ui(|s| s.set_density(Some(d), None), cx);
        }
    }

    /// Queue `text` for an agent ("name: prompt" in the palette).
    pub fn cmd_queue(&mut self, id: &str, text: String, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let id = id.to_string();
        let task = cx
            .background_executor()
            .spawn(async move { engine.queue_add(&id, text) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            let _ = this.update(cx, |s, cx| match res {
                Ok(view) => s.store.update(cx, |st, cx| {
                    if let Some(a) = st.agents.iter_mut().find(|a| a.id == view.id) {
                        *a = view;
                        cx.notify();
                    }
                }),
                Err(e) => s.toast(Toast::error("Couldn't queue prompt".into(), e), cx),
            });
        })
        .detach();
    }
}
