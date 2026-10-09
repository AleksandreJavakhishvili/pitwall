//! The app's side of `space.*` and `agent.move` (`pitwall space`, `pitwall
//! agent move`): the socket server calls [`AppWorkspace`] on its own
//! threads; each request runs on the main thread ([`attach`]) against the
//! one `ui.json` store (`crate::ui_state`) and the window registry
//! (`crate::windows`), with the same state functions the UI's own gestures
//! use, so every window follows at once.

use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use gpui::App;

use pitwall_daemon::workspace::find;
use pitwall_daemon::WorkspaceBackend;
use pitwall_proto::{code, ErrorBody, SpaceView};

use crate::main_screen::workspace::{SpaceKind, UiState, ALL_SPACE};
use crate::windows::{self, ownership, MAIN};

/// How long a request waits for the main thread.
const ANSWER_WITHIN: Duration = Duration::from_secs(10);

type Answer = Result<SpaceView, ErrorBody>;
type Job = Box<dyn FnOnce(&mut App) + Send>;

pub struct AppWorkspace {
    tx: UnboundedSender<Job>,
    rx: Mutex<Option<UnboundedReceiver<Job>>>,
}

impl AppWorkspace {
    pub fn new() -> Arc<AppWorkspace> {
        let (tx, rx) = unbounded();
        Arc::new(AppWorkspace {
            tx,
            rx: Mutex::new(Some(rx)),
        })
    }

    /// Run `f` on the main thread and wait for its answer.
    fn on_main<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut App) -> Result<T, ErrorBody> + Send + 'static,
    ) -> Result<T, ErrorBody> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .unbounded_send(Box::new(move |cx: &mut App| {
                let _ = tx.send(f(cx));
            }))
            .map_err(|_| ErrorBody::new(code::NOT_RUNNING, "Pitwall is quitting"))?;
        rx.recv_timeout(ANSWER_WITHIN)
            .map_err(|_| ErrorBody::new(code::OTHER, "Pitwall's windows didn't answer in time"))?
    }
}

/// On the main thread: run what the server asks. Once, after
/// `ui_state::init` and `windows::init`.
pub fn attach(backend: Arc<AppWorkspace>, cx: &mut App) {
    let Some(mut rx) = backend.rx.lock().ok().and_then(|mut r| r.take()) else {
        return;
    };
    cx.spawn(async move |cx| {
        while let Some(job) = rx.next().await {
            let _ = cx.update(job);
        }
    })
    .detach();
}

/// A space as the CLI shows it.
pub fn view(ui: &UiState, id: &str) -> Option<SpaceView> {
    let s = ui.space(id)?;
    Some(SpaceView {
        id: s.id.clone(),
        name: s.name.clone(),
        kind: match s.kind {
            SpaceKind::All => "all",
            SpaceKind::Project => "project",
            SpaceKind::Custom => "custom",
        }
        .into(),
        project: s.project.clone(),
        window: ui.window_of_space(&s.id).to_string(),
        shown: s.layout.agents(),
        members: s.members.clone(),
    })
}

pub fn views(ui: &UiState) -> Vec<SpaceView> {
    ui.spaces.iter().filter_map(|s| view(ui, &s.id)).collect()
}

fn resolve(ui: &UiState, key: &str) -> Result<String, ErrorBody> {
    Ok(find(&views(ui), key)?.id.clone())
}

fn current(id: &str, cx: &App) -> Answer {
    view(&crate::ui_state::get(cx), id).ok_or_else(|| ErrorBody::new(code::NOT_FOUND, "the space is gone"))
}

/// New custom space `name` in the main window.
pub fn create(name: &str, cx: &mut App) -> Answer {
    let mut made = String::new();
    crate::ui_state::update(cx, |ui| {
        let (next, id) = ui.clone().create_custom_space(MAIN);
        *ui = next.rename_space(&id, name);
        made = id;
    });
    current(&made, cx)
}

pub fn rename(space: &str, name: &str, cx: &mut App) -> Answer {
    let id = resolve(&crate::ui_state::get(cx), space)?;
    if id == ALL_SPACE {
        return Err(ErrorBody::new(code::BAD_PARAMS, "the All space can't be renamed"));
    }
    crate::ui_state::update(cx, |ui| *ui = ui.clone().rename_space(&id, name));
    current(&id, cx)
}

/// Into window `to`: `new`, `main` or an open window's label.
pub fn move_to_window(space: &str, to: &str, cx: &mut App) -> Answer {
    let ui = crate::ui_state::get(cx);
    let id = resolve(&ui, space)?;
    if id == ALL_SPACE {
        return Err(ErrorBody::new(code::BAD_PARAMS, "the All space stays in the main window"));
    }
    let from = ui.window_of_space(&id).to_string();
    match to {
        "new" => {
            windows::move_to_new_window(&id, &from, cx);
            if crate::ui_state::get(cx).window_of_space(&id) == from {
                return Err(ErrorBody::new(code::OTHER, "Pitwall couldn't open a new window"));
            }
        }
        label => {
            let open = windows::labels(cx);
            if label != MAIN && !open.iter().any(|l| l == label) {
                return Err(ErrorBody::new(
                    code::NOT_FOUND,
                    format!("no open window \"{label}\" (open: {}; or new)", open.join(", ")),
                ));
            }
            crate::ui_state::update(cx, |ui| *ui = ownership::move_space(ui.clone(), &id, label));
            windows::show_space(&id, cx);
        }
    }
    current(&id, cx)
}

/// Show `agent` in `space`, as dropping it on the space's tab.
pub fn move_agent(agent: &str, space: &str, cx: &mut App) -> Answer {
    let id = resolve(&crate::ui_state::get(cx), space)?;
    crate::ui_state::update(cx, |ui| *ui = ui.clone().drop_on_space(&id, agent, None).0);
    current(&id, cx)
}

impl WorkspaceBackend for AppWorkspace {
    fn spaces(&self) -> Result<Vec<SpaceView>, ErrorBody> {
        self.on_main(|cx| Ok(views(&crate::ui_state::get(cx))))
    }

    fn create(&self, name: &str) -> Answer {
        let name = name.to_string();
        self.on_main(move |cx| create(&name, cx))
    }

    fn rename(&self, space: &str, name: &str) -> Answer {
        let (space, name) = (space.to_string(), name.to_string());
        self.on_main(move |cx| rename(&space, &name, cx))
    }

    fn move_to_window(&self, space: &str, window: &str) -> Answer {
        let (space, window) = (space.to_string(), window.to_string());
        self.on_main(move |cx| move_to_window(&space, &window, cx))
    }

    fn move_agent(&self, agent: &str, space: &str) -> Answer {
        let (agent, space) = (agent.to_string(), space.to_string());
        self.on_main(move |cx| move_agent(&agent, &space, cx))
    }
}

#[cfg(test)]
mod tests;
