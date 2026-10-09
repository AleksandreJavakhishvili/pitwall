//! The app's side of `settings.*` (`pitwall settings`): the socket server
//! calls [`AppSettings`] on its own threads; `ui.json` settings are read
//! from a copy of the state the main thread keeps current and applied on
//! the main thread ([`attach`]), so every window follows at once. Settings
//! kept outside `ui.json` (hooks, the CLI link, rulesync's npx opt-in) are
//! read and changed directly, as Settings does.
//!
//! The server has already checked the value and, for `approval` settings,
//! asked the user.

use std::sync::{Arc, Mutex};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use gpui::{App, AppContext, Entity, Global};
use serde_json::Value;

use pitwall_core::Shared;
use pitwall_daemon::SettingsBackend;
use pitwall_proto::settings::{Setting, Stored};

use super::cli_install;

/// What the main thread applies.
#[derive(Debug, Clone)]
pub enum Change {
    /// A `ui.json` setting.
    Ui(&'static Setting, Value),
    /// A setting outside `ui.json` changed: Settings re-reads it.
    Outside(&'static Setting),
}

/// Bumped when a setting outside `ui.json` changed (Settings observes it).
pub struct Outside(pub u64);

/// The entity Settings observes for [`Change::Outside`].
#[derive(Clone)]
pub struct OutsideHandle(pub Entity<Outside>);

impl Global for OutsideHandle {}

pub fn outside(cx: &mut App) -> Entity<Outside> {
    if let Some(h) = cx.try_global::<OutsideHandle>() {
        return h.0.clone();
    }
    let e = cx.new(|_| Outside(0));
    cx.set_global(OutsideHandle(e.clone()));
    e
}

pub struct AppSettings {
    /// `ui.json` as the app has it now.
    file: Mutex<Value>,
    tx: UnboundedSender<Change>,
    rx: Mutex<Option<UnboundedReceiver<Change>>>,
    engine: Option<Shared>,
}

impl AppSettings {
    pub fn new(engine: Option<Shared>) -> Arc<AppSettings> {
        let (tx, rx) = unbounded();
        Arc::new(AppSettings {
            file: Mutex::new(super::data::file_value(&Default::default())),
            tx,
            rx: Mutex::new(Some(rx)),
            engine,
        })
    }

    fn file(&self) -> std::sync::MutexGuard<'_, Value> {
        self.file.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn engine(&self) -> Result<&Shared, String> {
        self.engine.as_ref().ok_or_else(|| "Pitwall's engine isn't running".to_string())
    }

    fn outside_value(&self, s: &'static Setting) -> Result<Value, String> {
        Ok(Value::Bool(match s.key {
            "agents.codexHooks" => pitwall_core::hooks::codex_status(self.engine()?.paths()).installed,
            "general.cliTool" => cli_install::current().installed.is_some(),
            "rules.allowNpx" => pitwall_core::rules::status(&pitwall_core::rules::Dirs::of(self.engine()?.paths())).npx_allowed,
            other => return Err(format!("{other} isn't known to this Pitwall")),
        }))
    }

    fn set_outside(&self, s: &'static Setting, v: &Value) -> Result<(), String> {
        let on = v.as_bool().unwrap_or(false);
        match s.key {
            "agents.codexHooks" => pitwall_core::hooks::install_codex(self.engine()?.paths()).map(|_| ()),
            "general.cliTool" => {
                let st = cli_install::current();
                if st.bin.is_none() {
                    return Err("this build doesn't include the command-line tool".into());
                }
                let dir = st
                    .dirs
                    .iter()
                    .find(|d| d.on_path)
                    .or(st.dirs.first())
                    .ok_or("no folder to install the command-line tool into")?;
                cli_install::install_into(&dir.path).map(|_| ())
            }
            "rules.allowNpx" => pitwall_core::rules::set_npx(&pitwall_core::rules::Dirs::of(self.engine()?.paths()), on)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            other => Err(format!("{other} isn't known to this Pitwall")),
        }
    }
}

impl SettingsBackend for AppSettings {
    fn get(&self, s: &'static Setting) -> Result<Value, String> {
        match s.stored {
            Stored::Ui { .. } => Ok(s
                .read_file(&self.file())
                .and_then(Result::ok)
                .unwrap_or_else(|| s.default_value())),
            Stored::Pitwall => self.outside_value(s),
        }
    }

    fn set(&self, s: &'static Setting, value: Value) -> Result<Value, String> {
        match s.stored {
            Stored::Ui { .. } => {
                if let Value::Object(m) = &mut *self.file() {
                    s.write_file(m, &value);
                }
                self.tx
                    .unbounded_send(Change::Ui(s, value.clone()))
                    .map_err(|_| "Pitwall is quitting".to_string())?;
                Ok(value)
            }
            Stored::Pitwall => {
                self.set_outside(s, &value)?;
                let _ = self.tx.unbounded_send(Change::Outside(s));
                self.outside_value(s)
            }
        }
    }
}

/// On the main thread: keep the backend's copy of `ui.json` current and
/// apply what the CLI changes. Once, after `ui_state::init`.
pub fn attach(backend: Arc<AppSettings>, cx: &mut App) {
    let store = crate::ui_state::store(cx);
    *backend.file() = super::data::file_value(&store.read(cx).state);
    let b = backend.clone();
    cx.observe(&store, move |store, cx| {
        *b.file() = super::data::file_value(&store.read(cx).state);
    })
    .detach();
    let Some(mut rx) = backend.rx.lock().ok().and_then(|mut r| r.take()) else {
        return;
    };
    cx.spawn(async move |cx| {
        while let Some(change) = rx.next().await {
            let _ = cx.update(|cx| apply(change, cx));
        }
    })
    .detach();
}

/// Apply one change from the CLI.
pub fn apply(change: Change, cx: &mut App) {
    match change {
        Change::Ui(s, v) => crate::ui_state::update(cx, |state| *state = super::data::with(state, s, &v)),
        Change::Outside(_) => {
            let e = outside(cx);
            e.update(cx, |o, cx| {
                o.0 += 1;
                cx.notify();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use pitwall_proto::settings::find;
    use serde_json::json;

    /// What the server thread sees and changes reaches the app's state.
    #[gpui::test]
    fn cli_changes_apply_live(cx: &mut TestAppContext) {
        let b = AppSettings::new(None);
        cx.update(|cx| {
            crate::ui_state::store(cx);
            attach(b.clone(), cx);
        });
        let theme = find("appearance.theme").unwrap();
        assert_eq!(b.get(theme).unwrap(), json!("system"));
        let font = find("appearance.terminalFontSize").unwrap();
        assert_eq!(b.set(font, json!(17)).unwrap(), json!(17));
        assert_eq!(b.get(font).unwrap(), json!(17), "read back at once");
        b.set(theme, json!("light")).unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let s = crate::ui_state::get(cx);
            assert_eq!(s.font_size, 17.0);
            assert_eq!(s.theme, crate::theme::ThemePref::Light);
            assert_eq!(crate::theme::appearance(cx).font_size, 17, "applied");
            // A change made in Settings shows in the server's copy.
            crate::ui_state::update(cx, |s| s.hide_elsewhere = true);
        });
        cx.run_until_parked();
        assert_eq!(b.get(find("general.showElsewhere").unwrap()).unwrap(), json!(false));
        assert!(b.get(find("agents.codexHooks").unwrap()).is_err(), "no engine here");
    }
}
